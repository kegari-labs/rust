//! Shared dispatcher composed with real storage/lifecycle and explicit read-only
//! provider/configuration fixtures. No native registration or live accounts.
use axum::{
    body::{Body, to_bytes},
    http::Request,
};
use kegari_config::Config;
use kegari_control_api::{
    api::{self, Dispatcher, Peer},
    identity::{Actor, Principal},
    transport::TransportAuthentication,
};
use kegari_controller::app::{HostDispatcher, HostServices};
use kegari_secrets::Secret;
use kegari_storage::PrivateDir;
use kegari_types::api::{CommandName, CommandRequest, Envelope};
use kegari_types::error::{Error, Result};
use kegari_v0::{
    control::*,
    control_command::{command_effect_id, resolve_workspace, verify_current_actor},
    reconcile::RuntimeProjection,
    runtime_host::{
        ConfigurationControl, HostFuture, LocalControlOwner, RuntimeInspection, WorkflowControl,
    },
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::sync::Notify;
use tower::ServiceExt;
use uuid::Uuid;

struct Configuration {
    value: Mutex<Config>,
    fences: Mutex<kegari_v0::configuration::AuthorityFences>,
    writes: AtomicUsize,
    job_gate: Mutex<Option<Arc<Notify>>>,
}
impl ConfigurationControl for Configuration {
    fn authority_fences(&self) -> Result<kegari_v0::configuration::AuthorityFences> {
        Ok(self.fences.lock().unwrap().clone())
    }
    fn snapshot(&self) -> Result<Config> {
        Ok(self.value.lock().unwrap().clone())
    }
    fn execute(
        &self,
        _: Actor,
        _: CommandRequest,
        _: Option<Secret>,
        _: Receipt,
    ) -> HostFuture<'_, Receipt> {
        Box::pin(async {
            self.writes.fetch_add(1, Ordering::SeqCst);
            let gate = self.job_gate.lock().unwrap().clone();
            if let Some(gate) = gate {
                gate.notified().await;
            }
            Err(Error::new(
                "fixture_only",
                "unexpected configuration mutation",
            ))
        })
    }
    fn check(&self, _: Option<Uuid>) -> HostFuture<'_, Value> {
        Box::pin(async {
            Ok(
                json!({"verified":["typed configuration"],"unverified":["fixture provider"],"blocked":[]}),
            )
        })
    }
}
struct Workflow {
    config: Arc<Configuration>,
    reads: AtomicUsize,
    writes: AtomicUsize,
    release_history: Mutex<Vec<kegari_v0::inspection::ReleaseInspectionRow>>,
    no_release_history: Mutex<Vec<kegari_v0::inspection::NoReleaseInspectionRow>>,
    history: Mutex<Vec<kegari_v0::reconcile::HistoryRow>>,
    release_details: Mutex<BTreeMap<(Uuid, kegari_v0::authority::IssueKey), Value>>,
}
impl WorkflowControl for Workflow {
    fn inspect(&self) -> HostFuture<'_, RuntimeInspection> {
        Box::pin(async {
            self.reads.fetch_add(1, Ordering::SeqCst);
            let config = self.config.snapshot()?;
            Ok(RuntimeInspection {
                projection: RuntimeProjection {
                    runtime_id: config.runtime.id,
                    config_revision: kegari_v0::config::config_revision(&config)?,
                    observed_at: chrono::Utc::now(),
                    workspaces: BTreeMap::new(),
                },
                delivery: BTreeMap::new(),
                outcomes: Default::default(),
                activities: vec![],
                history: self.history.lock().unwrap().clone(),
                release_details: self.release_details.lock().unwrap().clone(),
                release_history: self.release_history.lock().unwrap().clone(),
                no_release_history: self.no_release_history.lock().unwrap().clone(),
            })
        })
    }
    fn health(&self) -> Result<Value> {
        Ok(json!({"provider":"explicit deterministic fixture","live_validation":"not_run"}))
    }
    fn command_targets(&self, _: &CommandRequest) -> Result<Vec<String>> {
        Ok(vec!["fixture-parent".into()])
    }
    fn execute(
        &self,
        _: Actor,
        request: CommandRequest,
        parent: Receipt,
    ) -> HostFuture<'_, Receipt> {
        Box::pin(async move {
            assert_eq!(request.request_id, command_effect_id(parent.operation_id));
            assert_eq!(parent.canonical_targets, vec!["fixture-parent"]);
            self.writes.fetch_add(1, Ordering::SeqCst);
            let gate = self.config.job_gate.lock().unwrap().clone();
            if let Some(gate) = gate {
                gate.notified().await;
            }
            Err(Error::new("fixture_only", "unexpected workflow mutation"))
        })
    }
    fn cleanup_plan(&self, _: Uuid) -> HostFuture<'_, Value> {
        Box::pin(async { Ok(json!({"paths":[],"eligible":false})) })
    }
}

#[tokio::test]
async fn human_workflow_commands_keep_owned_lineage_and_scoped_targets_after_http_response() {
    let f = Fixture::new();
    {
        let mut config = f.config.value.lock().unwrap();
        config
            .control
            .local_users
            .get_mut("501")
            .unwrap()
            .roles
            .insert(kegari_config::ControlRole::Correct);
        f.auth.replace_config(config.control.clone()).unwrap();
    }
    let gate = Arc::new(Notify::new());
    *f.config.job_gate.lock().unwrap() = Some(gate.clone());
    let request = f.request(
        CommandName::Correct,
        json!({"parent":"fixture-parent","message":"Correct this prepared outcome"}),
    );
    let original = request.request_id;
    let response = tokio::time::timeout(
        std::time::Duration::from_millis(250),
        f.api(request.clone()),
    )
    .await
    .unwrap();
    assert!(response.ok, "{:?}", response.error);
    assert_eq!(response.operation_id, Some(original));
    assert_eq!(
        f.control.operation(original).unwrap().canonical_targets,
        vec!["fixture-parent"]
    );
    let replay = f.api(request.clone()).await;
    assert!(replay.ok);
    assert_eq!(replay.operation_id, Some(original));
    tokio::task::yield_now().await;
    assert_eq!(f.workflow.writes.load(Ordering::SeqCst), 1);
    let mut changed = request;
    changed.args["message"] = json!("different correction");
    assert_eq!(f.api(changed).await.error.unwrap().code, "request_conflict");
    gate.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while !f.control.snapshot().unwrap().background_commands.is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        f.control
            .operation(original)
            .unwrap()
            .observed_result
            .unwrap()["error"]["code"],
        "fixture_only"
    );
    assert_eq!(f.workflow.writes.load(Ordering::SeqCst), 1);
}
#[derive(Default)]
struct Owner {
    reservations: AtomicUsize,
}
impl LocalControlOwner for Owner {
    fn status(&self) -> Result<Value> {
        Ok(json!({"ownership":"fixture"}))
    }
    fn reserve_shutdown(&self) -> Result<()> {
        self.reservations.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
struct Fixture {
    _temp: tempfile::TempDir,
    config: Arc<Configuration>,
    control: Arc<ControlHost>,
    workflow: Arc<Workflow>,
    owner: Arc<Owner>,
    dispatcher: Arc<HostDispatcher>,
    auth: Arc<TransportAuthentication>,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let runtime = Uuid::new_v4();
        let config:Config=serde_json::from_value(json!({"schema_version":1,"runtime":{"id":runtime,"host_id":"fixture-host","max_agents":1},"connections":{},"workspaces":{},"control":{"local_users":{"501":{"id":"fixture-operator","linear_user_id":Uuid::new_v4(),"roles":["inspect","lifecycle","configure"]}}}})).unwrap();
        config.validate().unwrap();
        let auth = Arc::new(TransportAuthentication::new(config.control.clone(), None));
        let control = Arc::new(
            ControlHost::setup(
                PrivateDir::create(&temp.path().canonicalize().unwrap().join("state")).unwrap(),
                runtime,
                Uuid::new_v4(),
            )
            .unwrap(),
        );
        let config = Arc::new(Configuration {
            value: Mutex::new(config),
            fences: Mutex::new(Default::default()),
            writes: AtomicUsize::new(0),
            job_gate: Mutex::new(None),
        });
        let workflow = Arc::new(Workflow {
            config: config.clone(),
            reads: AtomicUsize::new(0),
            writes: AtomicUsize::new(0),
            release_history: Mutex::new(vec![]),
            no_release_history: Mutex::new(vec![]),
            history: Mutex::new(vec![]),
            release_details: Mutex::new(BTreeMap::new()),
        });
        let owner = Arc::new(Owner::default());
        let dispatcher = Arc::new(
            HostDispatcher::new(HostServices {
                control: control.clone(),
                configuration: config.clone(),
                workflow: workflow.clone(),
                owner: owner.clone(),
                wake: Arc::new(Notify::new()),
            })
            .unwrap(),
        );
        Self {
            _temp: temp,
            config,
            control,
            workflow,
            owner,
            dispatcher,
            auth,
        }
    }
    fn request(&self, name: CommandName, args: Value) -> CommandRequest {
        CommandRequest {
            api_version: 1,
            runtime_id: self.config.snapshot().unwrap().runtime.id,
            request_id: Uuid::new_v4(),
            command: name,
            args,
            expected_config_revision: None,
            expected_control_generation: None,
        }
    }
    fn actor(&self) -> Actor {
        Actor {
            identity: "fixture-operator".into(),
            roles: ["inspect".into(), "lifecycle".into(), "configure".into()].into(),
            principal: Some(Principal::Unix { uid: 501 }),
        }
    }
    async fn api(&self, command: CommandRequest) -> Envelope {
        let runtime = command.runtime_id;
        let state = api::ApiState {
            runtime_id: runtime,
            expected_authority: "kegari.local".into(),
            authentication: self.auth.clone(),
            dispatcher: self.dispatcher.clone(),
        };
        let request = Request::builder()
            .method("POST")
            .uri("/v1/commands")
            .header("host", "kegari.local")
            .header("x-kegari-protocol", "1")
            .header("x-kegari-runtime", runtime.to_string())
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&command).unwrap()))
            .unwrap();
        let response = api::connection_router(state, Peer::Unix { uid: 501 })
            .oneshot(request)
            .await
            .unwrap();
        serde_json::from_slice(&to_bytes(response.into_body(), api::MAX_BODY).await.unwrap())
            .unwrap()
    }
}
#[tokio::test]
async fn stopped_inspection_uses_shared_dispatcher_and_creates_no_receipts_or_work() {
    let f = Fixture::new();
    for command in [
        CommandName::Overview,
        CommandName::ConnectionList,
        CommandName::WorkspaceList,
        CommandName::ConfigShow,
        CommandName::ConfigCheck,
    ] {
        let request = f.request(command, json!({}));
        let id = request.request_id;
        let response = f.api(request).await;
        assert!(response.ok, "{:?}", response.error);
        let page: kegari_types::snapshot::Page = serde_json::from_value(response.data).unwrap();
        let data = kegari_control_api::snapshot::assemble(&[page]).unwrap();
        assert!(data.is_object());
        assert_eq!(f.control.operation(id).unwrap_err().code, "record_missing");
    }
    let response = f.api(f.request(CommandName::Status, json!({}))).await;
    assert!(response.ok);
    assert_eq!(response.state.as_deref(), Some("stopped"));
    assert_eq!(
        f.control.snapshot().unwrap().intent.desired,
        Intent::Stopped
    );
    assert_eq!(f.workflow.writes.load(Ordering::SeqCst), 0);
    assert_eq!(f.config.writes.load(Ordering::SeqCst), 0);
    assert_eq!(f.workflow.reads.load(Ordering::SeqCst), 1);
}

// Explicit presentation fixture, not a signed release or completion capability.
// Authenticated retained-record producer coverage lives with runtime delivery.
fn historical_display(
    runtime: Uuid,
    workspace: Uuid,
) -> kegari_v0::inspection::ReleaseInspectionRow {
    use kegari_digest::bytes_digest;
    use kegari_v0::{
        authority::IssueKey,
        evidence::{Generation, WorkIdentity},
        inspection::{ParentDisplay, ReleaseInspectionRow},
    };
    let parent = IssueKey::new(
        "https://linear.invalid",
        Uuid::from_u128(40),
        Uuid::from_u128(41),
    )
    .unwrap();
    let generation = Generation {
        authorization: bytes_digest(b"display authorization"),
        definition: bytes_digest(b"display definition"),
        correction: bytes_digest(b"display correction"),
        scope: bytes_digest(b"display scope"),
    };
    let work = WorkIdentity {
        linear_endpoint: parent.endpoint.clone(),
        linear_workspace: parent.workspace,
        parent: parent.issue,
        child: None,
        generation: generation.clone(),
    };
    let attempt = serde_json::from_value(json!({
        "plan":{"schema_version":1,"runtime_id":runtime,"operation_id":Uuid::new_v4(),"parent_id":parent.issue,"linear_workspace":parent.workspace,"generation":generation,
        "evidence_token":bytes_digest(b"display evidence"),"acceptance_digest":bytes_digest(b"display acceptance"),"carried_acceptance":null,"resources":[],"dependencies":[],"spacing_seconds":1,"deployment_free":[],"steps":[]},
        "admitted_at":chrono::Utc::now(),"control_generation":1,"owner_generation":1,"steps":{},"step_history":{},"cancellations":{},"hold_reason":null,"abandoned":false,"resolved":true,"assigned_successor":null,"recovery_records":[]
    })).unwrap();
    ReleaseInspectionRow {
        workspace,
        parent,
        work,
        attempt,
        active_hold: false,
        completed_by: None,
        completion: None,
        diagnostic: Some(Error::new(
            "record_missing",
            "display fixture has no completion authority",
        )),
        display: ParentDisplay {
            workspace_name: Some("removed-workspace".into()),
            identifier: Some("OLD-41".into()),
            title: None,
            url: None,
        },
    }
}

fn page_data(response: Envelope) -> Value {
    assert!(response.ok, "{:?}", response.error);
    let page = serde_json::from_value(response.data).unwrap();
    kegari_control_api::snapshot::assemble(&[page]).unwrap()
}

#[tokio::test]
async fn historical_show_and_overview_use_read_only_workspace_identity_without_mutation() {
    let f = Fixture::new();
    let runtime = f.config.snapshot().unwrap().runtime.id;
    let workspace = Uuid::from_u128(30);
    f.workflow
        .release_history
        .lock()
        .unwrap()
        .push(historical_display(runtime, workspace));
    let before = f.control.operations().unwrap().len();
    assert!(
        resolve_workspace(&f.config.snapshot().unwrap(), Some(&workspace.to_string())).is_err()
    );
    let overview = page_data(
        f.api(f.request(CommandName::Overview, json!({"workspace":workspace})))
            .await,
    );
    assert_eq!(overview["release_history"].as_array().unwrap().len(), 1);
    assert!(overview["history"].as_array().unwrap().is_empty());
    assert!(overview["ready_for_release"].as_array().unwrap().is_empty());
    let shown = page_data(
        f.api(f.request(
            CommandName::Show,
            json!({"workspace":workspace,"parent":"OLD-41"}),
        ))
        .await,
    );
    assert!(shown["parent"].is_null());
    assert!(shown["evidence_token"].is_null());
    assert_eq!(shown["inspection_identity"]["workspace"], json!(workspace));
    assert_eq!(
        shown["release_history"][0]["diagnostic"]["code"],
        "record_missing"
    );
    let missing_url = f
        .api(f.request(
            CommandName::Open,
            json!({"workspace":workspace,"parent":"OLD-41"}),
        ))
        .await;
    assert_eq!(missing_url.error.unwrap().code, "unknown_resource");
    f.workflow.release_history.lock().unwrap()[0].display.url =
        Some("file:///private/tmp/untrusted".into());
    let unsafe_url = f
        .api(f.request(
            CommandName::Open,
            json!({"workspace":workspace,"parent":"OLD-41"}),
        ))
        .await;
    assert_eq!(unsafe_url.error.unwrap().code, "unsafe_url");
    f.workflow.release_history.lock().unwrap()[0].display.url =
        Some("https://linear.invalid/issue/OLD-41".into());
    let opened = f
        .api(f.request(
            CommandName::Open,
            json!({"workspace":workspace,"parent":"OLD-41"}),
        ))
        .await;
    assert!(opened.ok, "{:?}", opened.error);
    assert_eq!(opened.data["url"], "https://linear.invalid/issue/OLD-41");
    let other = Uuid::from_u128(31);
    f.workflow
        .release_history
        .lock()
        .unwrap()
        .push(historical_display(runtime, other));
    for row in f.workflow.release_history.lock().unwrap().iter() {
        f.workflow
            .history
            .lock()
            .unwrap()
            .push(kegari_v0::reconcile::HistoryRow {
                parent_reference: row.parent_reference(),
                parent: row.parent.clone(),
                outcome: "no-release-required".into(),
                generation: row.work.generation.clone(),
                completed_at: row.attempt.admitted_at,
                links: vec![],
            });
        f.workflow.release_details.lock().unwrap().insert(
            (row.workspace, row.parent.clone()),
            json!({"fixture_origin_workspace":row.workspace}),
        );
    }
    let ambiguous = f
        .api(f.request(CommandName::Show, json!({"parent":"OLD-41"})))
        .await;
    assert_eq!(ambiguous.error.unwrap().code, "ambiguous_reference");
    let exact = page_data(
        f.api(f.request(
            CommandName::Show,
            json!({"parent":format!("{workspace}/OLD-41")}),
        ))
        .await,
    );
    assert_eq!(exact["release_history"].as_array().unwrap().len(), 1);
    assert_eq!(exact["history"].as_array().unwrap().len(), 1);
    assert_eq!(
        exact["history"][0]["parent_reference"],
        format!("{workspace}/{}", Uuid::from_u128(41))
    );
    assert_eq!(
        exact["release"]["fixture_origin_workspace"],
        json!(workspace)
    );
    let other_exact = page_data(
        f.api(f.request(
            CommandName::Show,
            json!({"workspace":other,"parent":"OLD-41"}),
        ))
        .await,
    );
    assert_eq!(other_exact["history"].as_array().unwrap().len(), 1);
    assert_eq!(
        other_exact["history"][0]["parent_reference"],
        format!("{other}/{}", Uuid::from_u128(41))
    );
    assert_eq!(
        other_exact["release"]["fixture_origin_workspace"],
        json!(other)
    );
    assert_eq!(f.control.operations().unwrap().len(), before);
    assert!(f.control.snapshot().unwrap().active_operations.is_empty());
    assert_eq!(f.workflow.writes.load(Ordering::SeqCst), 0);
    assert_eq!(f.config.writes.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn lifecycle_wait_observes_actual_admission_and_never_invents_start_completion() {
    let f = Fixture::new();
    let mut start = f.request(CommandName::Start, json!({}));
    start.expected_control_generation = Some(0);
    let id = start.request_id;
    let response = f.api(start).await;
    assert!(response.ok);
    assert_eq!(response.state.as_deref(), Some("reconciling"));
    let observe = || f.request(CommandName::OperationShow, json!({"operation":id}));
    let response = f.api(observe()).await;
    assert_eq!(response.operation_id, Some(id));
    assert_eq!(response.state.as_deref(), Some("reconciling"));
    f.control
        .attach_reconciled(Lease {
            runtime_id: f.config.snapshot().unwrap().runtime.id,
            owner_generation: 1,
            owner_id: Uuid::new_v4(),
        })
        .unwrap();
    let response = f.api(observe()).await;
    assert_eq!(response.state.as_deref(), Some("completed"));
    assert_eq!(f.workflow.writes.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn principal_reassignment_cannot_reuse_cached_operator_role() {
    let f = Fixture::new();
    let actor = f.actor();
    f.config
        .value
        .lock()
        .unwrap()
        .control
        .local_users
        .get_mut("501")
        .unwrap()
        .id = "replacement-operator".into();
    let mut request = f.request(CommandName::Start, json!({}));
    request.expected_control_generation = Some(0);
    let error = f
        .dispatcher
        .dispatch(actor, request, None)
        .await
        .unwrap_err();
    assert_eq!(error.code, "access_denied");
    assert_eq!(
        f.control.snapshot().unwrap().intent.desired,
        Intent::Stopped
    );
}
#[tokio::test]
async fn service_stop_reserves_before_ack_and_following_start_is_fenced() {
    let f = Fixture::new();
    let response = f.api(f.request(CommandName::ServiceStop, json!({}))).await;
    assert!(response.ok, "{:?}", response.error);
    assert_eq!(f.owner.reservations.load(Ordering::SeqCst), 1);
    let mut request = f.request(CommandName::Start, json!({}));
    request.expected_control_generation = Some(0);
    let response = f.api(request).await;
    assert!(!response.ok);
    assert_eq!(response.error.unwrap().code, "service_stopping");
    assert_eq!(
        f.control.snapshot().unwrap().intent.desired,
        Intent::Stopped
    );
}

#[tokio::test]
async fn paged_read_retains_original_revision_and_never_refetches_after_config_change() {
    let f = Fixture::new();
    {
        let mut config = f.config.value.lock().unwrap();
        for i in 0..110 {
            config.connections.insert(format!("fixture-{i}"),serde_json::from_value(json!({"kind":"github","account_id":format!("account-{i}"),"endpoint":"https://api.github.com/","credential_ref":"keychain:fixture"})).unwrap());
        }
    }
    let first = f.api(f.request(CommandName::ConfigShow, json!({}))).await;
    assert!(first.ok);
    let original = first.config_revision.clone();
    let mut pages =
        vec![serde_json::from_value::<kegari_types::snapshot::Page>(first.data).unwrap()];
    f.config.value.lock().unwrap().runtime.max_agents = 7;
    while let Some(cursor) = pages.last().unwrap().next_cursor {
        let snapshot = pages[0].snapshot_id;
        let next = f
            .api(f.request(
                CommandName::ConfigShow,
                json!({"snapshot_id":snapshot,"cursor":cursor}),
            ))
            .await;
        assert!(next.ok, "{:?}", next.error);
        assert_eq!(next.config_revision, original);
        assert_eq!(next.state.as_deref(), Some("observed"));
        pages.push(serde_json::from_value(next.data).unwrap());
    }
    let complete = kegari_control_api::snapshot::assemble(&pages).unwrap();
    assert_eq!(complete["configuration"]["runtime"]["max_agents"], 1);
    assert_eq!(
        complete["configuration"]["connections"]
            .as_object()
            .unwrap()
            .len(),
        110
    );
    assert_eq!(f.workflow.reads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn long_configuration_command_returns_receipt_and_replay_never_spawns_twice() {
    let f = Fixture::new();
    let gate = Arc::new(Notify::new());
    *f.config.job_gate.lock().unwrap() = Some(gate.clone());
    let request = f.request(
        CommandName::WorkspaceAdd,
        json!({"name":"fixture","workspace_configuration":"typed fixture input"}),
    );
    let id = request.request_id;
    let accepted = tokio::time::timeout(
        std::time::Duration::from_millis(250),
        f.api(request.clone()),
    )
    .await
    .unwrap();
    assert!(accepted.ok, "{:?}", accepted.error);
    assert_eq!(accepted.operation_id, Some(id));
    assert_eq!(accepted.state.as_deref(), Some("pending"));
    let replay = f.api(request.clone()).await;
    assert!(replay.ok);
    assert_eq!(replay.operation_id, Some(id));
    tokio::task::yield_now().await;
    assert_eq!(f.config.writes.load(Ordering::SeqCst), 1);
    assert!(
        f.control
            .snapshot()
            .unwrap()
            .background_commands
            .contains(&id)
    );
    let denied = f.api(f.request(CommandName::ServiceStop, json!({}))).await;
    assert_eq!(denied.error.unwrap().code, "stop_in_progress");
    let mut changed = request;
    changed.args["workspace_configuration"] = json!("different input");
    let rejected = f.api(changed).await;
    assert_eq!(rejected.error.unwrap().code, "request_conflict");
    gate.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if f.control.snapshot().unwrap().background_commands.is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let observed = f
        .api(f.request(CommandName::OperationShow, json!({"operation":id})))
        .await;
    assert_eq!(observed.state.as_deref(), Some("failed"));
    assert_eq!(f.config.writes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn disabled_remote_listener_revokes_previously_authenticated_actor() {
    let f = Fixture::new();
    {
        let mut config = f.config.value.lock().unwrap();
        let operator = config.control.local_users["501"].clone();
        config.control.tailscale_users.insert("77".into(), operator);
        config.control.https=Some(serde_json::from_value(json!({"addresses":["100.64.0.1"],"port":8443,"dns_name":"fixture.tail.example","certificate":"/private/tmp/cert.pem","private_key":"/private/tmp/key.pem","tailscale_executable":"/private/tmp/tailscale"})).unwrap());
    }
    let mut actor = f.actor();
    actor.principal = Some(Principal::Tailscale {
        user_id: 77,
        node_id: "fixture-node".into(),
    });
    verify_current_actor(&f.config.snapshot().unwrap(), &actor, "lifecycle").unwrap();
    f.config.value.lock().unwrap().control.https = None;
    let mut request = f.request(CommandName::Start, json!({}));
    request.expected_control_generation = Some(0);
    assert_eq!(
        f.dispatcher
            .dispatch(actor, request, None)
            .await
            .unwrap_err()
            .code,
        "access_denied"
    );
    assert_eq!(
        f.control.snapshot().unwrap().intent.desired,
        Intent::Stopped
    );
    assert!(f.control.operations().unwrap().is_empty());
}
#[tokio::test]
async fn known_role_reduction_blocks_dispatch_even_while_last_good_config_retains_role() {
    let f = Fixture::new();
    f.config.fences.lock().unwrap().denied_roles.insert(
        "unix:501".into(),
        ["lifecycle".to_string(), "configure".to_string()].into(),
    );
    let mut request = f.request(CommandName::Start, json!({}));
    request.expected_control_generation = Some(0);
    assert_eq!(
        f.dispatcher
            .dispatch(f.actor(), request, None)
            .await
            .unwrap_err()
            .code,
        "access_denied"
    );
    assert_eq!(
        f.control.snapshot().unwrap().intent.desired,
        Intent::Stopped
    );
    assert!(f.control.operations().unwrap().is_empty());
    assert!(
        f.dispatcher
            .dispatch(f.actor(), f.request(CommandName::Status, json!({})), None)
            .await
            .is_ok()
    );
}

// Synthetic display fixture only. Actual signed publication/seal validation is
// covered by the no-release producer; this helper supplies no capability.
fn no_release_historical_display(
    runtime: Uuid,
    workspace: Uuid,
    operation: Uuid,
) -> kegari_v0::inspection::NoReleaseInspectionRow {
    use kegari_digest::bytes_digest;
    use kegari_v0::{
        authority::IssueKey,
        evidence::{Generation, NoReleaseRequired, WorkIdentity},
        inspection::{NoReleaseInspectionRow, NoReleasePublicationInspection, ParentDisplay},
    };
    let parent = IssueKey::new(
        "https://linear.invalid",
        Uuid::from_u128(40),
        Uuid::from_u128(41),
    )
    .unwrap();
    let work = WorkIdentity {
        linear_endpoint: parent.endpoint.clone(),
        linear_workspace: parent.workspace,
        parent: parent.issue,
        child: None,
        generation: Generation {
            authorization: bytes_digest(b"no-release display authorization"),
            definition: bytes_digest(b"no-release display definition"),
            correction: bytes_digest(b"no-release display correction"),
            scope: bytes_digest(b"no-release display scope"),
        },
    };
    NoReleaseInspectionRow {
        workspace,
        parent,
        work: work.clone(),
        admission: operation,
        publication: NoReleasePublicationInspection {
            effect: Uuid::new_v4(),
            comment: None,
            url: None,
        },
        record: NoReleaseRequired {
            schema_version: 1,
            runtime_id: runtime,
            operation_id: operation,
            work,
            evidence_token: bytes_digest(b"no-release display evidence"),
            acceptance_digest: bytes_digest(b"no-release display acceptance"),
            baselines: vec![],
            targets: vec![],
            recorded_at: chrono::Utc::now() - chrono::Duration::days(3),
        },
        record_digest: bytes_digest(b"synthetic original signed no-release digest"),
        completion: None,
        diagnostic: Some(Error::new(
            "record_missing",
            "display fixture has no actual seal reference",
        )),
        display: ParentDisplay {
            workspace_name: Some("removed-no-release-workspace".into()),
            identifier: Some("OLD-41".into()),
            title: Some("Previously no release required".into()),
            url: None,
        },
    }
}

#[tokio::test]
async fn no_release_history_facade_is_read_only_scoped_and_keeps_completion_separate() {
    use kegari_v0::inspection::NoReleaseCompletionInspection;
    let f = Fixture::new();
    let runtime = f.config.snapshot().unwrap().runtime.id;
    let workspace = Uuid::from_u128(30);
    let other = Uuid::from_u128(31);
    f.workflow
        .no_release_history
        .lock()
        .unwrap()
        .push(no_release_historical_display(
            runtime,
            workspace,
            Uuid::from_u128(60),
        ));
    let before = f.control.operations().unwrap().len();
    assert!(
        resolve_workspace(&f.config.snapshot().unwrap(), Some(&workspace.to_string())).is_err()
    );
    let overview = page_data(
        f.api(f.request(CommandName::Overview, json!({"workspace":workspace})))
            .await,
    );
    assert_eq!(overview["no_release_history"].as_array().unwrap().len(), 1);
    assert!(
        overview["release_history"].as_array().unwrap().is_empty(),
        "no release attempt is invented"
    );
    assert!(overview["history"].as_array().unwrap().is_empty());
    assert!(overview["ready_for_release"].as_array().unwrap().is_empty());
    let show = page_data(
        f.api(f.request(
            CommandName::Show,
            json!({"workspace":workspace,"parent":"OLD-41"}),
        ))
        .await,
    );
    assert!(show["parent"].is_null());
    assert!(show["evidence_token"].is_null());
    assert_eq!(
        show["no_release_history"][0]["diagnostic"]["code"],
        "record_missing"
    );
    assert_eq!(
        show["no_release_history"][0]["admission"],
        json!(Uuid::from_u128(60))
    );
    assert!(show["no_release_history"][0]["completion"].is_null());
    let missing = f
        .api(f.request(
            CommandName::Open,
            json!({"workspace":workspace,"parent":"OLD-41"}),
        ))
        .await;
    assert_eq!(missing.error.unwrap().code, "unknown_resource");
    f.workflow.no_release_history.lock().unwrap()[0].display.url =
        Some("https://user:secret@linear.invalid/issue/OLD-41".into());
    let unsafe_url = f
        .api(f.request(
            CommandName::Open,
            json!({"workspace":workspace,"parent":"OLD-41"}),
        ))
        .await;
    assert_eq!(unsafe_url.error.unwrap().code, "unsafe_url");
    let recorded_at = {
        let mut rows = f.workflow.no_release_history.lock().unwrap();
        let row = &mut rows[0];
        row.display.url = Some("https://linear.invalid/issue/OLD-41".into());
        row.publication.comment = Some(Uuid::from_u128(61));
        row.publication.url = Some("https://linear.invalid/comment/61".into());
        row.diagnostic = None;
        row.completion = Some(NoReleaseCompletionInspection {
            completed_at: row.record.recorded_at,
        });
        row.record.recorded_at
    };
    let overview = page_data(
        f.api(f.request(CommandName::Overview, json!({"workspace":workspace})))
            .await,
    );
    assert_eq!(overview["history"].as_array().unwrap().len(), 1);
    assert_eq!(overview["history"][0]["outcome"], "no-release-required");
    assert_eq!(overview["history"][0]["completed_at"], json!(recorded_at));
    assert_eq!(
        overview["history"][0]["links"],
        json!(["https://linear.invalid/comment/61"])
    );
    assert!(overview["ready_for_release"].as_array().unwrap().is_empty());
    let open = f
        .api(f.request(
            CommandName::Open,
            json!({"workspace":workspace,"parent":"OLD-41"}),
        ))
        .await;
    assert!(open.ok, "{:?}", open.error);
    assert_eq!(open.data["url"], "https://linear.invalid/issue/OLD-41");
    f.workflow
        .no_release_history
        .lock()
        .unwrap()
        .push(no_release_historical_display(
            runtime,
            other,
            Uuid::from_u128(62),
        ));
    for row in f.workflow.no_release_history.lock().unwrap().iter() {
        f.workflow.release_details.lock().unwrap().insert(
            (row.workspace, row.parent.clone()),
            json!({"fixture_origin_workspace":row.workspace}),
        );
    }
    let ambiguous = f
        .api(f.request(CommandName::Show, json!({"parent":"OLD-41"})))
        .await;
    assert_eq!(ambiguous.error.unwrap().code, "ambiguous_reference");
    for selected in [workspace, other] {
        let show = page_data(
            f.api(f.request(
                CommandName::Show,
                json!({"workspace":selected,"parent":"OLD-41"}),
            ))
            .await,
        );
        assert_eq!(show["no_release_history"].as_array().unwrap().len(), 1);
        assert_eq!(show["no_release_history"][0]["workspace"], json!(selected));
        assert_eq!(show["release"]["fixture_origin_workspace"], json!(selected));
        assert!(show["release_history"].as_array().unwrap().is_empty());
    }
    let overview = page_data(
        f.api(f.request(CommandName::Overview, json!({"workspace":other})))
            .await,
    );
    assert!(
        overview["history"].as_array().unwrap().is_empty(),
        "the completed first mapping cannot credit the second mapping"
    );
    assert_eq!(f.control.operations().unwrap().len(), before);
    assert_eq!(f.workflow.writes.load(Ordering::SeqCst), 0);
    assert_eq!(f.config.writes.load(Ordering::SeqCst), 0);
}
