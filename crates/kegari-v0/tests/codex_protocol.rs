use kegari_codex::*;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use kegari_v0::isolation::*;
use std::collections::BTreeMap;

#[derive(Default)]
struct ObservingBroker {
    turn: Option<String>,
    called: bool,
    intervention: bool,
    checkpoint: Option<Vec<u8>>,
}
impl ToolBroker for ObservingBroker {
    fn observe_session(&mut self, state: &SessionState) -> kegari_types::error::Result<()> {
        self.turn = state.turn_id.clone();
        self.intervention = state.intervention.is_some();
        self.checkpoint = Some(serde_json::to_vec(state).unwrap());
        Ok(())
    }
    fn call<'a>(
        &'a mut self,
        call: &'a ToolCall,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = kegari_types::error::Result<Value>> + Send + 'a>,
    > {
        Box::pin(async move {
            assert_eq!(self.turn.as_deref(), Some(call.turn_id.as_str()));
            self.called = true;
            Ok(json!({"observed":true}))
        })
    }
}

fn binding() -> SessionBinding {
    SessionBinding {
        schema_version: 1,
        account_id: "account-one".into(),
        work_identity: "linear:workspace:child".into(),
        operation_id: "operation-one".into(),
        generation: "generation-one".into(),
        instruction_digest: "sha256:v1:instructions".into(),
        isolation_digest: "sha256:v1:scope".into(),
        profile: CodexProfile {
            model: "fixture-model".into(),
            reasoning: "high".into(),
            speed: "standard".into(),
        },
        cwd: PathBuf::from("/private/tmp/fixture/repository"),
    }
}

async fn read_json<R: tokio::io::AsyncRead + Unpin>(reader: &mut BufReader<R>) -> Value {
    let mut line = String::new();
    reader.read_line(&mut line).await.unwrap();
    serde_json::from_str(&line).unwrap()
}

async fn write_json<W: tokio::io::AsyncWrite + Unpin>(writer: &mut W, value: Value) {
    let mut bytes = serde_json::to_vec(&value).unwrap();
    bytes.push(b'\n');
    writer.write_all(&bytes).await.unwrap();
}

async fn handshake<R: tokio::io::AsyncRead + Unpin, W: tokio::io::AsyncWrite + Unpin>(
    r: &mut BufReader<R>,
    w: &mut W,
) {
    let request = read_json(r).await;
    assert_eq!(request["method"], "initialize");
    assert_eq!(request["params"]["capabilities"]["experimentalApi"], true);
    write_json(w,json!({"id":request["id"],"result":{"userAgent":"fixture","platformFamily":"unix","platformOs":"macos","codexHome":"/fixture/auth","codexHome":"/fixture/auth"}})).await;
    assert_eq!(read_json(r).await["method"], "initialized");
}

fn conforms(value: &Value, schema: &Value, root: &Value) -> bool {
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        return root
            .pointer(reference.strip_prefix('#').unwrap())
            .is_some_and(|target| conforms(value, target, root));
    }
    if schema == &json!(true) {
        return true;
    }
    if schema == &json!(false) {
        return false;
    }
    if schema
        .get("oneOf")
        .and_then(Value::as_array)
        .is_some_and(|options| options.iter().filter(|s| conforms(value, s, root)).count() != 1)
    {
        return false;
    }
    if schema
        .get("anyOf")
        .and_then(Value::as_array)
        .is_some_and(|options| !options.iter().any(|s| conforms(value, s, root)))
    {
        return false;
    }
    if schema
        .get("allOf")
        .and_then(Value::as_array)
        .is_some_and(|options| !options.iter().all(|s| conforms(value, s, root)))
    {
        return false;
    }
    if schema
        .get("enum")
        .and_then(Value::as_array)
        .is_some_and(|options| !options.contains(value))
    {
        return false;
    }
    let matches_type = |kind: &str| match kind {
        "null" => value.is_null(),
        "boolean" => value.is_boolean(),
        "string" => value.is_string(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "number" => value.is_number(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        _ => false,
    };
    if schema
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|kind| !matches_type(kind))
    {
        return false;
    }
    if schema
        .get("type")
        .and_then(Value::as_array)
        .is_some_and(|kinds| !kinds.iter().any(|k| k.as_str().is_some_and(matches_type)))
    {
        return false;
    }
    if let Some(object) = value.as_object() {
        if schema
            .get("required")
            .and_then(Value::as_array)
            .is_some_and(|required| {
                required
                    .iter()
                    .any(|k| !object.contains_key(k.as_str().unwrap()))
            })
        {
            return false;
        }
        let properties = schema.get("properties").and_then(Value::as_object);
        for (key, value) in object {
            match properties.and_then(|p| p.get(key)) {
                Some(property) if !conforms(value, property, root) => return false,
                None if schema.get("additionalProperties") == Some(&json!(false)) => return false,
                _ => {}
            }
        }
    }
    if let (Some(array), Some(items)) = (value.as_array(), schema.get("items"))
        && !array.iter().all(|v| conforms(v, items, root))
    {
        return false;
    }
    true
}

fn assert_wire_schema(params: &Value, name: &str) {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../kegari-codex/protocol/codex-0.164.0-alpha.11.1")
        .join(name);
    let schema: Value = serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap();
    assert!(
        conforms(params, &schema, &schema),
        "emitted params do not conform to {name}: {params}"
    );
}

fn fixture() -> (tempfile::TempDir, IsolationPlan) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    for d in [
        "repository",
        "repository/src",
        "scratch",
        "privileged",
        "auth",
    ] {
        std::fs::create_dir(root.join(d)).unwrap();
    }
    let plan = IsolationPlan {
        managed_execution: None,
        schema_version: 1,
        role: ExecutionRole::Child,
        execution_root: root.clone(),
        repository: root.join("repository"),
        cwd: root.join("repository"),
        scratch: root.join("scratch"),
        read_roots: vec![],
        resource_writes: vec![],
        source_writes: vec![PathGrant {
            path: root.join("repository/src"),
            directory: true,
        }],
        forbidden_roots: vec![root.join("privileged")],
    };
    (temp, plan)
}

fn thread_response(cwd: &Path, permissions: &PermissionConfiguration) -> Value {
    json!({"thread":{"id":"owned-thread"},"cwd":cwd,"model":"fixture-model","modelProvider":"openai","reasoningEffort":"high","approvalPolicy":"never","approvalsReviewer":"user","activePermissionProfile":{"id":permissions.profile_id},"sandbox":{"type":"readOnly","networkAccess":false},"serviceTier":"default","instructionSources":[cwd.join("AGENTS.md")]})
}

#[test]
fn sandbox_paths_roles_and_environment_fail_closed() {
    let (_temp, mut plan) = fixture();
    let isolated = plan.validate().unwrap();
    let profile = isolated.seatbelt_profile().unwrap();
    assert!(profile.contains("(deny default)"));
    assert!(!profile.contains("(allow network"));
    let permissions = kegari_v0::codex_host::permission_configuration(
        &isolated,
        &plan.execution_root.join("auth"),
    )
    .unwrap();
    let fs = &permissions.settings["permissions"][&permissions.profile_id]["filesystem"];
    assert_eq!(fs[":root"], "deny");
    assert_eq!(
        permissions.settings["permissions"][&permissions.profile_id]["network"]["enabled"],
        false
    );
    plan.role = ExecutionRole::Coordination;
    assert!(plan.validate().is_err());
    plan.source_writes.clear();
    plan.validate().unwrap();
    plan.role = ExecutionRole::Child;
    plan.managed_execution =
        Some(kegari_v0::isolation::BoundManagedExecution::automatic(&plan).unwrap());
    plan.role = ExecutionRole::IndependentReview;
    assert!(plan.validate().is_err());
    assert!(
        worker_environment(
            &plan.scratch,
            &BTreeMap::from([("PRIVATE_ALIAS".into(), "secret".into())]),
            &["PRIVATE_ALIAS".into()]
        )
        .is_err()
    );
    assert!(
        worker_environment(
            &plan.scratch,
            &BTreeMap::from([("CODEX_HOME".into(), "elsewhere".into())]),
            &[]
        )
        .is_err()
    );
}

#[test]
fn artifact_resolution_uses_candidate_and_refuses_symlink_escape() {
    let (_temp, plan) = fixture();
    let isolated = plan.validate().unwrap();
    std::fs::write(plan.cwd.join("results.json"), "candidate").unwrap();
    std::fs::write(plan.execution_root.join("results.json"), "workflow-copy").unwrap();
    assert_eq!(
        isolated.artifact(Path::new("results.json")).unwrap(),
        plan.cwd.join("results.json")
    );
    assert!(isolated.artifact(Path::new("../results.json")).is_err());
    assert!(
        isolated
            .artifact(&plan.execution_root.join("results.json"))
            .is_err()
    );
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&plan.forbidden_roots[0], plan.cwd.join("escape")).unwrap();
        assert!(isolated.artifact(Path::new("escape/secret")).is_err());
    }
}

#[test]
fn identity_directory_names_do_not_collide_or_escape() {
    let a = execution_name("../../same title", "issue-one", "repo", "g1").unwrap();
    let b = execution_name("../../same title", "issue-two", "repo", "g1").unwrap();
    assert_ne!(a, b);
    assert!(!a.contains('/'));
    assert!(!a.starts_with('.'));
    assert!(
        a.chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
    );
}

#[test]
fn writable_directory_rejects_hardlinks_to_unrelated_files() {
    let (_temp, plan) = fixture();
    let outside = plan.forbidden_roots[0].join("secret");
    std::fs::write(&outside, "private").unwrap();
    std::fs::hard_link(outside, plan.repository.join("src/linked")).unwrap();
    assert_eq!(plan.validate().unwrap_err().code, "scope_violation");
}

#[test]
fn bare_executable_uses_admitted_path_without_rebasing_arguments() {
    let (_temp, mut plan) = fixture();
    plan.managed_execution =
        Some(kegari_v0::isolation::BoundManagedExecution::automatic(&plan).unwrap());
    let isolation = plan.validate().unwrap();
    assert_eq!(
        isolation.resolve_executable("sh").unwrap(),
        Path::new("/bin/sh").canonicalize().unwrap()
    );
    assert!(isolation.resolve_executable("./sh").is_err());
    // Unknown names are resolved generically; there is no configured tool list.
    assert!(
        isolation
            .resolve_executable("kegari-nonexistent-command-fixture")
            .is_err()
    );
}

#[tokio::test]
async fn production_session_requests_follow_pinned_schema_and_completion_races() {
    let (_temp, plan) = fixture();
    let isolation = plan.validate().unwrap();
    let permissions = kegari_v0::codex_host::permission_configuration(
        &isolation,
        &plan.execution_root.join("auth"),
    )
    .unwrap();
    let response = thread_response(&plan.cwd, &permissions);
    let effective = permissions.settings.clone();
    let (client, server) = tokio::io::duplex(65536);
    let (cr, cw) = tokio::io::split(client);
    let (sr, mut sw) = tokio::io::split(server);
    let mut sr = BufReader::new(sr);
    let server = tokio::spawn(async move {
        handshake(&mut sr, &mut sw).await;
        let req = read_json(&mut sr).await;
        assert_wire_schema(&req["params"], "v2_GetAccountParams.json");
        write_json(&mut sw,json!({"id":req["id"],"result":{"account":{"type":"chatgpt","email":null,"planType":"unknown"},"requiresOpenaiAuth":true,"workspaceRouting":{"chatgptAccountId":"account-one"}}})).await;
        let req = read_json(&mut sr).await;
        assert_wire_schema(&req["params"], "v2_ModelListParams.json");
        write_json(&mut sw,json!({"id":req["id"],"result":{"data":[{"model":"fixture-model","supportedReasoningEfforts":[{"reasoningEffort":"high"}],"serviceTiers":[]}],"nextCursor":null}})).await;
        let req = read_json(&mut sr).await;
        assert_eq!(req["method"], "config/read");
        write_json(
            &mut sw,
            json!({"id":req["id"],"result":{"config":effective,"layers":[],"origins":{}}}),
        )
        .await;
        let req = read_json(&mut sr).await;
        assert_eq!(req["method"], "thread/start");
        assert_wire_schema(&req["params"], "v2_ThreadStartParams.json");
        assert_eq!(
            req["params"]["developerInstructions"],
            "runtime authority and delivered child guidance"
        );
        assert_eq!(req["params"]["allowProviderModelFallback"], false);
        write_json(&mut sw, json!({"id":req["id"],"result":response})).await;
        let req = read_json(&mut sr).await;
        assert_wire_schema(&req["params"], "v2_TurnStartParams.json");
        assert_eq!(req["params"]["serviceTierForTurn"], "default");
        assert_eq!(req["params"]["effort"], "high");
        // A fast turn can finish before its synchronous start response arrives.
        write_json(&mut sw,json!({"method":"turn/started","params":{"threadId":"owned-thread","turn":{"id":"turn-fast","status":"inProgress"}}})).await;
        write_json(&mut sw,json!({"id":"interleaved","method":"item/tool/call","params":{"threadId":"owned-thread","turnId":"turn-fast","callId":"interleaved-call","tool":"observe","arguments":{}}})).await;
        let tool_reply = read_json(&mut sr).await;
        assert_eq!(tool_reply["id"], "interleaved");
        assert_eq!(tool_reply["result"]["success"], true);
        write_json(&mut sw,json!({"method":"turn/completed","params":{"threadId":"owned-thread","turn":{"id":"turn-fast","status":"completed","items":[]}}})).await;
        write_json(
            &mut sw,
            json!({"id":req["id"],"result":{"turn":{"id":"turn-fast","status":"completed","items":[]}}}),
        )
        .await;
        let req = read_json(&mut sr).await;
        assert_eq!(req["method"], "review/start");
        assert_wire_schema(&req["params"], "v2_ReviewStartParams.json");
        assert_eq!(
            req["params"]["target"]["instructions"],
            "delivered review guidance; exact revision abc"
        );
        write_json(&mut sw,json!({"id":req["id"],"result":{"reviewThreadId":"owned-thread","turn":{"id":"turn-review","status":"inProgress"}}})).await;
        write_json(&mut sw,json!({"method":"turn/completed","params":{"threadId":"owned-thread","turn":{"id":"turn-review","status":"completed","items":[]}}})).await;
        let req = read_json(&mut sr).await;
        assert_eq!(req["method"], "thread/read");
        assert_wire_schema(&req["params"], "v2_ThreadReadParams.json");
        let mut read_thread = response["thread"].clone();
        read_thread["cwd"] = response["cwd"].clone();
        read_thread["modelProvider"] = response["modelProvider"].clone();
        read_thread["status"] = json!({"type":"idle"});
        read_thread["turns"] = json!([{"id":"turn-fast","status":"completed","items":[]},{"id":"turn-review","status":"completed","items":[]}]);
        write_json(
            &mut sw,
            json!({"id":req["id"],"result":{"thread":read_thread}}),
        )
        .await;
        let req = read_json(&mut sr).await;
        assert_eq!(req["method"], "thread/resume");
        assert_wire_schema(&req["params"], "v2_ThreadResumeParams.json");
        assert!(req["params"].get("path").is_none());
        write_json(&mut sw, json!({"id":req["id"],"result":response})).await;
        let req = read_json(&mut sr).await;
        assert!(
            req["params"]["input"][0]["text"]
                .as_str()
                .unwrap()
                .contains("provider confirms PR remains open at abc")
        );
        write_json(
            &mut sw,
            json!({"id":req["id"],"result":{"turn":{"id":"turn-resumed","status":"inProgress"}}}),
        )
        .await;
    });
    let mut rpc = RpcClient::from_transport(cr, cw, Duration::from_secs(2)).unwrap();
    rpc.initialize().await.unwrap();
    rpc.verify_account("account-one").await.unwrap();
    rpc.discover_models().await.unwrap();
    rpc.verify_execution_configuration(&permissions, &plan.cwd)
        .await
        .unwrap();
    let mut b = binding();
    b.cwd = plan.cwd.clone();
    b.isolation_digest = isolation.digest().into();
    let mut state = SessionState::new(b, 3).unwrap();
    rpc.start_thread(
        &mut state,
        &isolation,
        &permissions,
        "runtime authority and delivered child guidance",
        vec![json!({"type":"function","name":"observe","description":"Fixture observation","inputSchema":{"type":"object"}})],
    )
    .await
    .unwrap();
    let mut observer = ObservingBroker::default();
    rpc.start_turn(
        &mut state,
        &permissions,
        "implement scoped child",
        &mut observer,
    )
    .await
    .unwrap();
    assert!(observer.called);
    assert!(state.turn_id.is_none());
    assert!(state.completion_is_current());
    rpc.review(
        &mut state,
        "delivered review guidance; exact revision abc",
        &mut DenyTools,
    )
    .await
    .unwrap();
    assert_eq!(state.turn_id.as_deref(), Some("turn-review"));
    rpc.pump(
        &mut state,
        &mut DenyTools,
        Duration::from_secs(2),
        Duration::from_secs(2),
    )
    .await
    .unwrap();
    let previous = state.binding.clone();
    state.binding.operation_id = "resumed-operation".into();
    rpc.resume_thread(
        &mut state,
        &previous,
        &isolation,
        &permissions,
        "provider confirms PR remains open at abc",
        &mut DenyTools,
    )
    .await
    .unwrap();
    rpc.start_turn(
        &mut state,
        &permissions,
        "continue remaining validation",
        &mut DenyTools,
    )
    .await
    .unwrap();
    assert_eq!(state.turns_started, 3);
    assert!(state.reconciliation_digest.is_some());
    server.await.unwrap();
}

#[tokio::test]
async fn connection_metadata_uses_only_bounded_read_protocol_without_model_work() {
    let (_temp, plan) = fixture();
    let isolation = plan.validate().unwrap();
    let permissions = kegari_v0::codex_host::permission_configuration(
        &isolation,
        &plan.execution_root.join("auth"),
    )
    .unwrap();
    let effective = permissions.settings.clone();
    let (client, server) = tokio::io::duplex(65536);
    let (cr, cw) = tokio::io::split(client);
    let (sr, mut sw) = tokio::io::split(server);
    let mut sr = BufReader::new(sr);
    let task = tokio::spawn(async move {
        handshake(&mut sr, &mut sw).await;
        let req = read_json(&mut sr).await;
        assert_eq!(req["method"], "account/read");
        assert_eq!(req["params"]["refreshToken"], false);
        write_json(&mut sw,json!({"id":req["id"],"result":{"account":{"type":"chatgpt"},"workspaceRouting":{"chatgptAccountId":"account-one"}}})).await;
        let req = read_json(&mut sr).await;
        assert_eq!(req["method"], "model/list");
        write_json(
            &mut sw,
            json!({"id":req["id"],"result":{"data":[{"model":"fixture-model"}],"nextCursor":null}}),
        )
        .await;
        let req = read_json(&mut sr).await;
        assert_eq!(req["method"], "account/rateLimits/read");
        write_json(
            &mut sw,
            json!({"id":req["id"],"result":{"rateLimits":null}}),
        )
        .await;
        let req = read_json(&mut sr).await;
        assert_eq!(req["method"], "config/read");
        write_json(
            &mut sw,
            json!({"id":req["id"],"result":{"config":effective,"layers":[],"origins":{}}}),
        )
        .await;
    });
    let mut rpc = RpcClient::from_transport(cr, cw, Duration::from_secs(2)).unwrap();
    let observed = rpc
        .connection_metadata("account-one", &permissions, &plan.cwd)
        .await
        .unwrap();
    assert_eq!(observed.account_id, "account-one");
    assert_eq!(observed.model_catalog[0]["model"], "fixture-model");
    assert!(
        observed.argument_digest.is_none(),
        "injected transport cannot claim native argv provenance"
    );
    assert_eq!(
        observed.permission_digest,
        kegari_digest::canonical_digest(1, &permissions.settings).unwrap()
    );
    task.await.unwrap();
}

#[tokio::test]
async fn owned_fixture_provider_cancellation_waits_and_reaps_without_pid_adoption() {
    use std::os::unix::fs::PermissionsExt;
    let (temp, plan) = fixture();
    let root = temp.path().canonicalize().unwrap();
    let binary = root.join("fixture-adapter");
    std::fs::write(&binary,format!("#!/bin/sh\nif [ \"$1\" = --version ]; then printf '%s\\n' '{SUPPORTED_CODEX_VERSION}'; else exec /bin/sleep 30; fi\n")).unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::set_permissions(root.join("auth"), std::fs::Permissions::from_mode(0o700)).unwrap();
    let permissions = kegari_v0::codex_host::permission_configuration(
        &plan.validate().unwrap(),
        &root.join("auth"),
    )
    .unwrap();
    let verified = kegari_v0::codex_host::verify_executable(&binary, Duration::from_secs(2))
        .await
        .unwrap();
    let mut client = kegari_v0::codex_host::start_verified(
        &verified,
        &root.join("auth"),
        &permissions,
        Duration::from_secs(2),
    )
    .unwrap();
    let owned = client.cancellation();
    assert!(!owned.exited().unwrap());
    owned.request_cancel().unwrap();
    tokio::time::timeout(Duration::from_secs(2), owned.join_termination())
        .await
        .unwrap()
        .unwrap();
    assert!(owned.exited().unwrap());
    client.terminate().await.unwrap();
    std::fs::write(&binary, "#!/bin/sh\nexit 0\n").unwrap();
    assert!(
        kegari_v0::codex_host::start_verified(
            &verified,
            &root.join("auth"),
            &permissions,
            Duration::from_secs(2)
        )
        .is_err()
    );
}

#[tokio::test]
async fn metadata_probe_limits_output_and_reaps_timed_out_fixture_processes() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    for (name, body, expected) in [
        (
            "flood",
            "while :; do printf 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx'; done",
            "protocol_error",
        ),
        ("timeout", "exec /bin/sleep 30", "timeout"),
    ] {
        let binary = root.join(name);
        let pid_file = root.join(format!("{name}.pid"));
        std::fs::write(
            &binary,
            format!(
                "#!/bin/sh\nprintf '%s' \"$$\" > '{}'\n{body}\n",
                pid_file.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let deadline = if name == "flood" {
            Duration::from_secs(2)
        } else {
            Duration::from_secs(1)
        };
        let error = kegari_v0::codex_host::verify_version(&binary, deadline)
            .await
            .unwrap_err();
        assert_eq!(error.code, expected, "{error:?}");
        let pid = std::fs::read_to_string(pid_file)
            .unwrap()
            .parse::<i32>()
            .unwrap();
        let mut status = 0;
        assert_eq!(
            unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) },
            -1
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ECHILD)
        );
    }
}

#[test]
fn test_resource_writes_are_separate_exact_grants_and_never_reopen_source_or_control() {
    let (_temp, mut plan) = fixture();
    let resource = plan.execution_root.join("disposable-resource");
    std::fs::create_dir(&resource).unwrap();
    std::fs::write(resource.join("data"), "fixture").unwrap();
    let grant = PathGrant {
        path: resource.clone(),
        directory: true,
    };
    plan.resource_writes.push(grant.clone());
    assert_eq!(plan.validate().unwrap_err().code, "scope_violation");
    plan.read_roots.push(grant.clone());
    let boundary = plan.validate().unwrap();
    let permissions = kegari_v0::codex_host::permission_configuration(
        &boundary,
        &plan.execution_root.join("auth"),
    )
    .unwrap();
    assert_eq!(
        permissions.settings["permissions"][&permissions.profile_id]["filesystem"]
            [resource.to_str().unwrap()],
        "write"
    );
    assert_eq!(permissions.settings["features"]["shell_tool"], false);
    let profile = boundary.seatbelt_profile().unwrap();
    assert!(profile.contains(&format!(
        "(allow file-write* (subpath \"{}\"))",
        resource.display()
    )));
    plan.role = ExecutionRole::Coordination;
    plan.source_writes.clear();
    assert_eq!(plan.validate().unwrap_err().code, "scope_violation");
    plan.role = ExecutionRole::Validation;
    plan.validate().unwrap();
    let original = plan.resource_writes.clone();
    for path in [&plan.repository, &plan.scratch, &plan.forbidden_roots[0]] {
        let mut unsafe_plan = plan.clone();
        let grant = PathGrant {
            path: path.clone(),
            directory: true,
        };
        unsafe_plan.read_roots.push(grant.clone());
        unsafe_plan.resource_writes = vec![grant];
        assert_eq!(unsafe_plan.validate().unwrap_err().code, "scope_violation");
    }
    plan.resource_writes = original;
    std::os::unix::fs::symlink(&plan.repository, resource.join("escape")).unwrap();
    assert_eq!(plan.validate().unwrap_err().code, "scope_violation");
}

#[tokio::test]
async fn dropping_native_client_cancels_and_reaps_its_owned_fixture() {
    use std::os::unix::fs::PermissionsExt;
    let (temp, plan) = fixture();
    let root = temp.path().canonicalize().unwrap();
    let binary = root.join("fixture-adapter");
    std::fs::write(
        &binary,
        format!("#!/bin/sh\nif [ \"$1\" = --version ]; then printf '%s\\n' '{SUPPORTED_CODEX_VERSION}'; else exec /bin/sleep 30; fi\n"),
    )
    .unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::set_permissions(root.join("auth"), std::fs::Permissions::from_mode(0o700)).unwrap();
    let permissions = kegari_v0::codex_host::permission_configuration(
        &plan.validate().unwrap(),
        &root.join("auth"),
    )
    .unwrap();
    let verified = kegari_v0::codex_host::verify_executable(&binary, Duration::from_secs(2))
        .await
        .unwrap();
    let client = kegari_v0::codex_host::start_verified(
        &verified,
        &root.join("auth"),
        &permissions,
        Duration::from_secs(2),
    )
    .unwrap();
    let owned = client.cancellation();
    assert!(!owned.exited().unwrap());
    drop(client);
    // Observe only: joining termination here would itself request termination and
    // could conceal a missing cancellation operation in the wrapper's Drop.
    let observed = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if owned.exited()? {
                return Ok::<(), kegari_types::error::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    if !matches!(observed, Ok(Ok(()))) {
        // Bound failure cleanup before preserving the original observation error.
        let _ = tokio::time::timeout(Duration::from_secs(12), owned.join_termination()).await;
    }
    observed.unwrap().unwrap();
    assert!(owned.exited().unwrap());
}
