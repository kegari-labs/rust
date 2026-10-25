use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use kegari_v0::{
    api::*,
    control::Actor,
    error::{Error, Result},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use tower::ServiceExt;
use uuid::Uuid;

struct Auth {
    calls: Arc<AtomicUsize>,
    roles: BTreeSet<String>,
}
impl Authentication for Auth {
    fn authenticate(&self, peer: Peer) -> Pin<Box<dyn Future<Output = Result<Actor>> + Send + '_>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if !matches!(peer, Peer::Unix { uid: 501 }) {
                return Err(Error::new("access_denied", "unmapped OS identity"));
            }
            Ok(Actor {
                identity: "os:501".into(),
                principal: None,
                roles: self.roles.clone(),
            })
        })
    }
}
struct Echo {
    calls: Arc<AtomicUsize>,
}

struct MappedPeer;
impl Authentication for MappedPeer {
    fn authenticate(&self, _: Peer) -> Pin<Box<dyn Future<Output = Result<Actor>> + Send + '_>> {
        Box::pin(async {
            Ok(Actor {
                identity: "mapped-operator".into(),
                principal: None,
                roles: BTreeSet::from(["lifecycle".into()]),
            })
        })
    }
}

#[tokio::test]
async fn mapped_remote_operator_cannot_shutdown_local_control_service() {
    let (mut state, _, calls) = state(&["lifecycle"]);
    state.authentication = Arc::new(MappedPeer);
    let command = json!({"api_version":1,"runtime_id":state.runtime_id,"request_id":Uuid::new_v4(),"command":"service_stop","args":{}});
    let response = connection_router(
        state.clone(),
        Peer::DirectTls {
            address: "100.64.0.2:17000".parse().unwrap(),
        },
    )
    .oneshot(request(
        &state,
        "POST",
        "/v1/commands",
        Body::from(command.to_string()),
    ))
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let response = connection_router(state.clone(), Peer::Unix { uid: 501 })
        .oneshot(request(
            &state,
            "POST",
            "/v1/commands",
            Body::from(command.to_string()),
        ))
        .await
        .unwrap();
    assert!(envelope(response).await.ok);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
impl Dispatcher for Echo {
    fn dispatch(
        &self,
        _: Actor,
        request: CommandRequest,
        _: Option<kegari_v0::secrets::Secret>,
    ) -> Pin<Box<dyn Future<Output = Result<DispatchResult>> + Send + '_>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(DispatchResult {
                config_revision: Some("test-revision".into()),
                operation_id: None,
                state: Some("fixture".into()),
                data: json!({"command":request.command,"args":request.args}),
            })
        })
    }
}
fn state(roles: &[&str]) -> (ApiState, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let auth = Arc::new(AtomicUsize::new(0));
    let dispatch = Arc::new(AtomicUsize::new(0));
    (
        ApiState {
            runtime_id: Uuid::new_v4(),
            expected_authority: "runtime.example:443".into(),
            authentication: Arc::new(Auth {
                calls: auth.clone(),
                roles: roles.iter().map(|s| (*s).into()).collect(),
            }),
            dispatcher: Arc::new(Echo {
                calls: dispatch.clone(),
            }),
        },
        auth,
        dispatch,
    )
}
fn request(state: &ApiState, method: &str, path: &str, body: Body) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("host", &state.expected_authority)
        .header("x-kegari-protocol", "1")
        .header("x-kegari-runtime", state.runtime_id.to_string())
        .header("content-type", "application/json")
        .body(body)
        .unwrap()
}
async fn envelope(response: axum::response::Response) -> Envelope {
    assert_eq!(response.headers()["content-type"], "application/json");
    serde_json::from_slice(&to_bytes(response.into_body(), MAX_BODY).await.unwrap()).unwrap()
}

#[tokio::test]
async fn transport_peer_is_required_and_identity_is_refreshed_per_request() {
    let (state, auth, dispatch) = state(&["inspect"]);
    let response = router(state.clone())
        .oneshot(request(&state, "GET", "/v1/status", Body::empty()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(!envelope(response).await.ok);
    assert_eq!(auth.load(Ordering::SeqCst), 0);
    for _ in 0..2 {
        let response = connection_router(state.clone(), Peer::Unix { uid: 501 })
            .oneshot(request(&state, "GET", "/v1/status", Body::empty()))
            .await
            .unwrap();
        assert!(envelope(response).await.ok);
    }
    assert_eq!(auth.load(Ordering::SeqCst), 2);
    assert_eq!(dispatch.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn browser_spoofed_identity_and_wrong_target_do_not_dispatch() {
    let (state, _, dispatch) = state(&["inspect", "lifecycle"]);
    for (name, value) in [
        ("origin", "null"),
        ("forwarded", "for=100.64.0.1"),
        ("tailscale-user-login", "operator@example.com"),
        ("host", "evil.example"),
        ("x-kegari-protocol", "2"),
        ("x-kegari-runtime", "00000000-0000-0000-0000-000000000000"),
    ] {
        let mut request = request(&state, "GET", "/v1/status", Body::empty());
        request.headers_mut().insert(
            axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
            value.parse().unwrap(),
        );
        let response = connection_router(state.clone(), Peer::Unix { uid: 501 })
            .oneshot(request)
            .await
            .unwrap();
        assert!(!envelope(response).await.ok, "{name}");
    }
    assert_eq!(dispatch.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn closed_commands_validate_tokens_roles_and_lifecycle_generation() {
    let (state, _, dispatch) = state(&["inspect"]);
    for (command, args, generation) in [
        ("pause", json!({"workspace":"one"}), Some(1)),
        ("pause", json!({}), None),
        ("approve", json!({"parent":"ws/issue"}), None),
        (
            "release",
            json!({"parent":"ws/issue","result":"token"}),
            None,
        ),
        ("shell", json!({"cmd":"touch forbidden"}), None),
    ] {
        let body = json!({"api_version":1,"runtime_id":state.runtime_id,"request_id":Uuid::new_v4(),"command":command,"args":args,"expected_control_generation":generation});
        let response = connection_router(state.clone(), Peer::Unix { uid: 501 })
            .oneshot(request(
                &state,
                "POST",
                "/v1/commands",
                Body::from(body.to_string()),
            ))
            .await
            .unwrap();
        assert!(!envelope(response).await.ok);
    }
    assert_eq!(dispatch.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn routes_and_protocol_errors_share_json_envelope() {
    let (state, _, dispatch) = state(&["inspect"]);
    let good = connection_router(state.clone(), Peer::Unix { uid: 501 })
        .oneshot(request(
            &state,
            "GET",
            "/v1/overview?workspace=one",
            Body::empty(),
        ))
        .await
        .unwrap();
    let result = envelope(good).await;
    assert_eq!(result.data["args"]["workspace"], "one");
    assert_eq!(dispatch.load(Ordering::SeqCst), 1);
    for (method, path) in [
        ("GET", "/unknown"),
        ("GET", "/v1/commands"),
        ("GET", "/v1/parents/ws/%FF"),
        ("OPTIONS", "/v1/status"),
        ("GET", "/v1/operations/not-a-uuid"),
    ] {
        let response = connection_router(state.clone(), Peer::Unix { uid: 501 })
            .oneshot(request(&state, method, path, Body::empty()))
            .await
            .unwrap();
        assert!(!envelope(response).await.ok, "{path}");
    }
    assert_eq!(dispatch.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn request_body_and_header_limits_fail_before_effects() {
    let (state, _, dispatch) = state(&["inspect"]);
    let response = connection_router(state.clone(), Peer::Unix { uid: 501 })
        .oneshot(request(
            &state,
            "POST",
            "/v1/commands",
            Body::from(vec![b'x'; MAX_BODY + 1]),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert!(!envelope(response).await.ok);
    let mut oversized = request(&state, "GET", "/v1/status", Body::empty());
    oversized
        .headers_mut()
        .insert("x-long", "a".repeat(MAX_HEADERS).parse().unwrap());
    let response = connection_router(state.clone(), Peer::Unix { uid: 501 })
        .oneshot(oversized)
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert!(!envelope(response).await.ok);
    assert_eq!(dispatch.load(Ordering::SeqCst), 0);
}

#[test]
fn correction_limits_and_recovery_actions_are_closed() {
    let runtime = Uuid::new_v4();
    let mut command = CommandRequest {
        api_version: 1,
        runtime_id: runtime,
        request_id: Uuid::new_v4(),
        command: CommandName::Correct,
        args: json!({"parent":"p","message":"x".repeat(1024*1024+1)}),
        expected_config_revision: None,
        expected_control_generation: None,
    };
    assert!(command.validate(runtime).is_err());
    command.command = CommandName::ReleaseRecover;
    command.args = json!({"parent":"p","operation":Uuid::new_v4(),"reason":"inspection","action":"mark_successful"});
    assert!(command.validate(runtime).is_err());
    command.args["action"] = Value::String("supersede".into());
    assert!(command.validate(runtime).is_err());
}

#[tokio::test]
async fn optional_filters_and_get_bodies_cannot_broaden_inspection() {
    let (state, _, dispatch) = state(&["inspect"]);
    for args in [
        json!({"workspace":[]}),
        json!({"snapshot_id":123,"cursor":{}}),
        json!({"cursor":Uuid::new_v4()}),
        json!({"workspace":""}),
    ] {
        let command = CommandRequest {
            api_version: 1,
            runtime_id: state.runtime_id,
            request_id: Uuid::new_v4(),
            command: CommandName::Overview,
            args,
            expected_config_revision: None,
            expected_control_generation: None,
        };
        assert!(command.validate(state.runtime_id).is_err());
    }
    for (path, bytes) in [
        ("/v1/status", vec![b'x']),
        ("/v1/status", vec![b'x'; MAX_BODY + 1]),
        ("/v1/overview?workspace=a&workspace=b", vec![]),
        ("/v1/status?workspace=ignored", vec![]),
    ] {
        let result = connection_router(state.clone(), Peer::Unix { uid: 501 })
            .oneshot(request(&state, "GET", path, Body::from(bytes)))
            .await
            .unwrap();
        assert!(!envelope(result).await.ok);
    }
    assert_eq!(dispatch.load(Ordering::SeqCst), 0);
}

struct Paged {
    snapshots: std::sync::Mutex<kegari_v0::snapshot::Snapshots>,
    projection: Value,
}
impl Dispatcher for Paged {
    fn dispatch(
        &self,
        actor: Actor,
        mut request: CommandRequest,
        _: Option<kegari_v0::secrets::Secret>,
    ) -> Pin<Box<dyn Future<Output = Result<DispatchResult>> + Send + '_>> {
        Box::pin(async move {
            let snapshot = request.args.as_object_mut().unwrap().remove("snapshot_id");
            let cursor = request.args.as_object_mut().unwrap().remove("cursor");
            let filters = json!({"command":request.command,"args":request.args});
            let mut snapshots = self.snapshots.lock().unwrap();
            let page = match (snapshot, cursor) {
                (Some(snapshot), Some(cursor)) => snapshots.page(
                    Uuid::parse_str(snapshot.as_str().unwrap()).unwrap(),
                    Uuid::parse_str(cursor.as_str().unwrap()).unwrap(),
                    &actor.identity,
                    request.runtime_id,
                    &filters,
                    chrono::Utc::now(),
                )?,
                _ => snapshots.capture(
                    &actor.identity,
                    request.runtime_id,
                    &filters,
                    &self.projection,
                    chrono::Utc::now(),
                )?,
            };
            Ok(DispatchResult {
                config_revision: None,
                operation_id: None,
                state: None,
                data: serde_json::to_value(page).unwrap(),
            })
        })
    }
}

#[tokio::test]
async fn collection_and_operation_snapshots_retrieve_every_page() {
    let projection = json!({"children":(0..501).map(|n| json!({"id":n,"state":"required"})).collect::<Vec<_>>()});
    let (mut state, _, _) = state(&["inspect"]);
    state.dispatcher = Arc::new(Paged {
        snapshots: std::sync::Mutex::new(Default::default()),
        projection: projection.clone(),
    });
    for command in [
        CommandName::ConnectionList,
        CommandName::WorkspaceList,
        CommandName::OperationShow,
    ] {
        let operation = Uuid::new_v4();
        let mut pages = Vec::new();
        let mut args = if command == CommandName::OperationShow {
            json!({"operation":operation})
        } else {
            json!({})
        };
        loop {
            let req = if command == CommandName::OperationShow {
                let query = match (args.get("snapshot_id"), args.get("cursor")) {
                    (Some(id), Some(cursor)) => format!(
                        "?snapshot_id={}&cursor={}",
                        id.as_str().unwrap(),
                        cursor.as_str().unwrap()
                    ),
                    _ => String::new(),
                };
                request(
                    &state,
                    "GET",
                    &format!("/v1/operations/{operation}{query}"),
                    Body::empty(),
                )
            } else {
                let body = CommandRequest {
                    api_version: 1,
                    runtime_id: state.runtime_id,
                    request_id: Uuid::new_v4(),
                    command: command.clone(),
                    args: args.clone(),
                    expected_config_revision: None,
                    expected_control_generation: None,
                };
                request(
                    &state,
                    "POST",
                    "/v1/commands",
                    Body::from(serde_json::to_vec(&body).unwrap()),
                )
            };
            let result = envelope(
                connection_router(state.clone(), Peer::Unix { uid: 501 })
                    .oneshot(req)
                    .await
                    .unwrap(),
            )
            .await;
            assert!(result.ok, "{:?}", result.error);
            let page: kegari_v0::snapshot::Page = serde_json::from_value(result.data).unwrap();
            assert!(page.entries.len() <= 100);
            let next = page.next_cursor;
            args["snapshot_id"] = json!(page.snapshot_id);
            args["cursor"] = json!(next);
            pages.push(page);
            if next.is_none() {
                break;
            }
        }
        assert!(pages.len() > 1);
        assert_eq!(kegari_v0::snapshot::assemble(&pages).unwrap(), projection);
    }
}

#[tokio::test]
async fn transient_secret_is_removed_before_dispatch_and_never_echoed() {
    let (state, _, dispatch) = state(&["configure"]);
    let command = json!({"api_version":1,"runtime_id":state.runtime_id,"request_id":Uuid::new_v4(),"command":"connection_add","args":{"name":"fixture","connection":"typed fixture","secret_input":"sentinel-transient-secret"}});
    let result = envelope(
        connection_router(state.clone(), Peer::Unix { uid: 501 })
            .oneshot(request(
                &state,
                "POST",
                "/v1/commands",
                Body::from(command.to_string()),
            ))
            .await
            .unwrap(),
    )
    .await;
    assert!(result.ok);
    assert_eq!(dispatch.load(Ordering::SeqCst), 1);
    assert!(
        !serde_json::to_string(&result)
            .unwrap()
            .contains("sentinel-transient-secret")
    );
    assert!(result.data["args"].get("secret_input").is_none());
    let decoded: CommandRequest = serde_json::from_value(command.clone()).unwrap();
    assert!(!format!("{decoded:?}").contains("sentinel-transient-secret"));
    let mut denied = command;
    denied["command"] = json!("status");
    let result = envelope(
        connection_router(state.clone(), Peer::Unix { uid: 501 })
            .oneshot(request(
                &state,
                "POST",
                "/v1/commands",
                Body::from(denied.to_string()),
            ))
            .await
            .unwrap(),
    )
    .await;
    assert!(!result.ok);
    assert_eq!(dispatch.load(Ordering::SeqCst), 1);
}
