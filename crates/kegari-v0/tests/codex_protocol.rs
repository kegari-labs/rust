use kegari_v0::codex::*;
use kegari_v0::isolation::*;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

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
fn active() -> SessionState {
    let mut s = SessionState::new(binding(), 20).unwrap();
    s.thread_id = Some("thread-one".into());
    s.turn_id = Some("turn-one".into());
    s.turns_started = 1;
    s
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

#[test]
fn fragmented_coalesced_json_rpc_is_atomic_and_bounded() {
    let mut decoder = JsonRpcDecoder::default();
    assert!(decoder.push(b"{\"id\":1,\"res").unwrap().is_empty());
    let messages = decoder
        .push(b"ult\":{}}\n{\"method\":\"tick\",\"params\":{}}\n")
        .unwrap();
    assert_eq!(messages.len(), 2);
    assert!(matches!(&messages[0],RpcMessage::Response{id,..} if id==1));
    decoder.finish().unwrap();
    let mut decoder = JsonRpcDecoder::default();
    assert!(
        decoder
            .push(&vec![b'x'; MAX_FRAME_BYTES])
            .unwrap()
            .is_empty()
    );
    assert_eq!(decoder.push(b"x").unwrap_err().code, "protocol_error");
    assert!(decoder.push(b"{}\n").is_err());
    let mut decoder = JsonRpcDecoder::default();
    assert!(decoder.push(b"{\"id\":null,\"result\":{}}\n").is_err());
}
#[test]
fn malformed_required_protocol_is_not_success() {
    for bytes in [
        b"[]\n".as_slice(),
        b"{\"id\":1,\"result\":{},\"error\":{}}\n",
        b"{\"method\":\"\"}\n",
        b"not-json\n",
    ] {
        assert!(JsonRpcDecoder::default().push(bytes).is_err());
    }
    let mut d = JsonRpcDecoder::default();
    d.push(b"{\"id\":1").unwrap();
    assert!(d.finish().is_err());
    let mut s = active();
    assert!(
        s.notification(
            "turn/completed",
            &json!({"threadId":"thread-one","turn":{"id":"turn-one","status":"inProgress"}})
        )
        .is_err()
    );
    assert!(!s.completion_is_current());
}
#[test]
fn provider_verification_survives_timeout_exit_and_restart() {
    let mut s = active();
    let event=s.notification("model/verification",&json!({"threadId":"thread-one","turnId":"turn-one","verifications":["trustedAccessForCyber"]})).unwrap();
    assert_eq!(event.kind, EventKind::ProviderIntervention);
    assert_eq!(
        s.interruption("watchdog_expired").kind,
        EventKind::ProviderIntervention
    );
    let mut restored = SessionState::restore(&serde_json::to_vec(&s).unwrap()).unwrap();
    restored.observe_correction(7);
    restored.binding.profile.reasoning = "ultra".into();
    assert_eq!(
        restored.next_turn_allowed().unwrap_err().code,
        "provider_intervention"
    );
    assert_eq!(
        restored.interruption("process_exit").kind,
        EventKind::ProviderIntervention
    );
    assert!(!restored.completion_is_current());
    assert!(
        restored
            .intervention
            .unwrap()
            .source_digest
            .starts_with("sha256:v1:")
    );
}
#[test]
fn provider_policy_error_cannot_be_downgraded_by_later_completed_event() {
    for code in [
        "cyberPolicy",
        "misalignmentPolicyViolation",
        "tooManyDenials",
    ] {
        let mut s = active();
        s.notification("error",&json!({"threadId":"thread-one","turnId":"turn-one","willRetry":false,"error":{"message":"provider-only text","codexErrorInfo":code}})).unwrap();
        let e=s.notification("turn/completed",&json!({"threadId":"thread-one","turn":{"id":"turn-one","status":"completed","items":[],"error":null}})).unwrap();
        assert_eq!(e.kind, EventKind::ProviderIntervention);
        assert!(!s.completion_is_current());
    }
}
#[test]
fn legacy_session_fields_keep_exact_serialization_without_resume_history_authority() {
    let mut state = active();
    state.notification("turn/completed", &json!({"threadId":"thread-one","turn":{"id":"turn-one","status":"completed","items":[]}})).unwrap();
    let mut legacy = serde_json::to_value(&state).unwrap();
    legacy.as_object_mut().unwrap().remove("provider_history");
    legacy.as_object_mut().unwrap().remove("unresolved_turn");
    let restored = SessionState::restore(&serde_json::to_vec(&legacy).unwrap()).unwrap();
    assert!(restored.provider_history.is_none());
    assert!(restored.unresolved_turn.is_none());
    assert_eq!(serde_json::to_value(restored).unwrap(), legacy);
}
#[test]
fn buffering_is_transient_and_a_normal_exit_alone_is_not_success() {
    let mut s = active();
    s.notification("model/safetyBuffering/updated",&json!({"threadId":"thread-one","turnId":"turn-one","model":"fixture-model","reasons":[],"useCases":[],"showBufferingUi":true})).unwrap();
    assert!(s.intervention.is_none());
    s.notification("turn/completed",&json!({"threadId":"thread-one","turn":{"id":"turn-one","status":"completed","items":[],"error":null}})).unwrap();
    assert!(s.completion_is_current());
    let mut exited = active();
    exited.interruption("process_exit");
    assert!(!exited.completion_is_current());
}
#[test]
fn correction_completion_requires_current_consumption_not_delivery() {
    let mut s = active();
    s.observe_correction(1);
    s.notification(
        "turn/completed",
        &json!({"threadId":"thread-one","turn":{"id":"turn-one","status":"completed"}}),
    )
    .unwrap();
    assert!(!s.completion_is_current());
    assert!(s.acknowledge_correction(2).is_err());
    s.acknowledge_correction(1).unwrap();
    assert!(s.completion_is_current());
    s.observe_correction(2);
    assert!(!s.completion_is_current());
}
#[test]
fn usage_preserves_account_thread_baselines_on_recovery() {
    let mut usage = UsageLedger::default();
    assert_eq!(usage.observe("a", "t", 100, true), 100);
    assert_eq!(usage.observe("a", "t", 100, false), 0);
    let mut restored: UsageLedger =
        serde_json::from_slice(&serde_json::to_vec(&usage).unwrap()).unwrap();
    assert_eq!(restored.observe("a", "t", 110, false), 10);
    assert_eq!(restored.observe("a", "t", 80, false), 0);
    assert_eq!(restored.observe("a", "t", 115, false), 5);
    assert_eq!(restored.observe("b", "t", 60, false), 0);
    assert!(restored.earlier_usage_unknown.contains("b\0t"));
    restored.finish_elapsed("operation", 300);
    restored.finish_elapsed("operation", 500);
    assert_eq!(restored.ended_operations["operation"], 300);
    let prior = serde_json::to_value(&restored).unwrap();
    let mut next = restored.resumed_session("a", "t");
    assert!(next.observed_tokens.is_empty());
    assert!(next.ended_operations.is_empty());
    assert_eq!(next.observe("a", "t", 140, false), 25);
    assert_eq!(next.observe("a", "t", 140, false), 0);
    next.finish_elapsed("next-operation", 200);
    assert_eq!(next.ended_operations.len(), 1);
    assert_eq!(serde_json::to_value(&restored).unwrap(), prior);
    let mut unknown = restored.resumed_session("a", "unobserved-thread");
    assert!(
        unknown
            .earlier_usage_unknown
            .contains("a\0unobserved-thread")
    );
    assert_eq!(unknown.observe("a", "unobserved-thread", 75, false), 0);
    assert_eq!(unknown.observe("a", "unobserved-thread", 80, false), 5);
}
#[test]
fn interrupted_turn_identity_survives_restart_until_exact_terminal_observation() {
    let mut state = active();
    let event = state.interruption("process_exit");
    assert_eq!(event.turn_id.as_deref(), Some("turn-one"));
    assert!(state.turn_id.is_none());
    assert_eq!(state.unresolved_turn.as_deref(), Some("turn-one"));
    let mut restored = SessionState::restore(&serde_json::to_vec(&state).unwrap()).unwrap();
    assert_eq!(
        restored.next_turn_allowed().unwrap_err().code,
        "reconciliation_required"
    );
    assert!(restored.notification("turn/completed", &json!({"threadId":"thread-one","turn":{"id":"other","status":"completed","items":[]}})).is_err());
    restored.notification("turn/completed", &json!({"threadId":"thread-one","turn":{"id":"turn-one","status":"failed","items":[],"error":{"message":"temporary disconnect","codexErrorInfo":"internalServerError"}}})).unwrap();
    assert!(restored.unresolved_turn.is_none());
    assert_eq!(restored.completed_turns["turn-one"], "failed");
    assert!(!restored.completion_is_current());
    SessionState::restore(&serde_json::to_vec(&restored).unwrap()).unwrap();
}
#[test]
fn profile_and_resume_cannot_fallback_or_cross_scope() {
    let p = binding().profile;
    let catalog = vec![
        json!({"model":"fixture-model","supportedReasoningEfforts":[{"reasoningEffort":"high"}],"serviceTiers":[]}),
    ];
    p.validate_catalog(&catalog).unwrap();
    let mut fast = p.clone();
    fast.speed = "fast".into();
    assert!(fast.validate_catalog(&catalog).is_err());
    let mut other = binding();
    other.account_id = "other".into();
    assert!(other.compatible_resume(&binding()).is_err());
    let mut resumed = binding();
    resumed.operation_id = "new-operation".into();
    resumed.compatible_resume(&binding()).unwrap();
    resumed.instruction_digest = "changed".into();
    assert!(resumed.compatible_resume(&binding()).is_err());
}
#[test]
fn sandbox_paths_roles_and_environment_fail_closed() {
    let (_temp, mut plan) = fixture();
    let isolated = plan.validate().unwrap();
    let profile = isolated.seatbelt_profile().unwrap();
    assert!(profile.contains("(deny default)"));
    assert!(!profile.contains("(allow network"));
    let permissions =
        PermissionConfiguration::for_isolation(&isolated, &plan.execution_root.join("auth"))
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

#[tokio::test]
async fn interleaved_input_and_unknown_tool_receive_replies_without_questions() {
    let (client, server) = tokio::io::duplex(8192);
    let (cr, cw) = tokio::io::split(client);
    let (sr, mut sw) = tokio::io::split(server);
    let mut sr = BufReader::new(sr);
    let server = tokio::spawn(async move {
        handshake(&mut sr, &mut sw).await;
        let request = read_json(&mut sr).await;
        write_json(&mut sw,json!({"id":"input","method":"item/tool/requestUserInput","params":{"threadId":"thread-one","turnId":"turn-one","questions":[]}})).await;
        assert_eq!(
            read_json(&mut sr).await,
            json!({"id":"input","result":{"answers":{}}})
        );
        write_json(&mut sw,json!({"id":"tool","method":"item/tool/call","params":{"threadId":"thread-one","turnId":"turn-one","callId":"c","tool":"dangerous","arguments":{}}})).await;
        let result = read_json(&mut sr).await;
        assert_eq!(result["result"]["success"], false);
        write_json(
            &mut sw,
            json!({"id":"unknown","method":"madeUpTool","params":{}}),
        )
        .await;
        assert_eq!(read_json(&mut sr).await["error"]["code"], -32601);
        write_json(&mut sw, json!({"id":request["id"],"result":{}})).await;
    });
    let mut rpc = RpcClient::from_transport(cr, cw, Duration::from_secs(2)).unwrap();
    rpc.initialize().await.unwrap();
    let mut state = active();
    rpc.request(
        "turn/interrupt",
        json!({"threadId":"thread-one","turnId":"turn-one"}),
        Some(&mut state),
        &mut DenyTools,
    )
    .await
    .unwrap();
    assert!(state.input_required);
    assert!(
        rpc.drain_events()
            .iter()
            .any(|e| e.kind == EventKind::UnsupportedTool)
    );
    server.await.unwrap();
}
#[tokio::test]
async fn account_identity_and_complete_model_pages_are_required() {
    let (client, server) = tokio::io::duplex(8192);
    let (cr, cw) = tokio::io::split(client);
    let (sr, mut sw) = tokio::io::split(server);
    let mut sr = BufReader::new(sr);
    let server = tokio::spawn(async move {
        handshake(&mut sr, &mut sw).await;
        let req = read_json(&mut sr).await;
        assert_eq!(req["method"], "account/read");
        write_json(&mut sw,json!({"id":req["id"],"result":{"account":{"type":"chatgpt","email":null,"planType":"unknown"},"requiresOpenaiAuth":true,"workspaceRouting":{"chatgptAccountId":"account-one","backendOrigin":"fixture","accountRoutingOverride":"NO_CONSTRAINT"}}})).await;
        let req = read_json(&mut sr).await;
        assert_eq!(req["params"]["cursor"], Value::Null);
        write_json(
            &mut sw,
            json!({"id":req["id"],"result":{"data":[{"model":"first"}],"nextCursor":"last"}}),
        )
        .await;
        let req = read_json(&mut sr).await;
        assert_eq!(req["params"]["cursor"], "last");
        write_json(
            &mut sw,
            json!({"id":req["id"],"error":{"code":-32000,"message":"last page unavailable"}}),
        )
        .await;
    });
    let mut rpc = RpcClient::from_transport(cr, cw, Duration::from_secs(2)).unwrap();
    rpc.initialize().await.unwrap();
    rpc.verify_account("account-one").await.unwrap();
    assert!(rpc.discover_models().await.is_err());
    server.await.unwrap();
}
#[tokio::test]
async fn steering_completion_race_keeps_correction_pending() {
    let (client, server) = tokio::io::duplex(8192);
    let (cr, cw) = tokio::io::split(client);
    let (sr, mut sw) = tokio::io::split(server);
    let mut sr = BufReader::new(sr);
    let server = tokio::spawn(async move {
        handshake(&mut sr, &mut sw).await;
        let request = read_json(&mut sr).await;
        assert_eq!(request["method"], "turn/steer");
        assert_eq!(request["params"]["expectedTurnId"], "turn-one");
        write_json(&mut sw,json!({"method":"turn/completed","params":{"threadId":"thread-one","turn":{"id":"turn-one","status":"completed"}}})).await;
        write_json(
            &mut sw,
            json!({"id":request["id"],"error":{"code":-32602,"message":"stale turn"}}),
        )
        .await;
    });
    let mut rpc = RpcClient::from_transport(cr, cw, Duration::from_secs(2)).unwrap();
    rpc.initialize().await.unwrap();
    let mut state = active();
    assert!(
        !rpc.steer(&mut state, 1, "correct existing child", &mut DenyTools)
            .await
            .unwrap()
    );
    assert_eq!(state.correction_required, 1);
    assert_eq!(state.correction_consumed, 0);
    assert!(!state.completion_is_current());
    assert!(state.turn_id.is_none());
    server.await.unwrap();
}
#[tokio::test]
async fn synchronous_request_timeout_is_bounded_and_identified() {
    let (client, server) = tokio::io::duplex(8192);
    let (cr, cw) = tokio::io::split(client);
    let (sr, _sw) = tokio::io::split(server);
    let _sr = BufReader::new(sr);
    let mut rpc = RpcClient::from_transport(cr, cw, Duration::from_millis(10)).unwrap();
    let error = rpc.initialize().await.unwrap_err();
    assert_eq!(error.code, "timeout");
    assert_eq!(error.details["request_id"], 1);
    assert_eq!(error.details["outcome"], "uncertain");
}
#[test]
fn checked_in_protocol_manifest_matches_bytes_and_schema_capabilities() {
    use sha2::{Digest, Sha256};
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("protocol/codex-0.164.0-alpha.11.1");
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["cli_version"], SUPPORTED_CODEX_VERSION);
    for file in manifest["files"].as_array().unwrap() {
        let bytes = std::fs::read(root.join(file["file"].as_str().unwrap())).unwrap();
        assert_eq!(
            hex::encode(Sha256::digest(&bytes)),
            file["sha256"].as_str().unwrap()
        );
        let _: Value = serde_json::from_slice(&bytes).unwrap();
    }
    let steer: Value =
        serde_json::from_slice(&std::fs::read(root.join("v2_TurnSteerParams.json")).unwrap())
            .unwrap();
    assert!(
        steer["required"]
            .as_array()
            .unwrap()
            .contains(&json!("expectedTurnId"))
    );
    let errors: Value =
        serde_json::from_slice(&std::fs::read(root.join("v2_ErrorNotification.json")).unwrap())
            .unwrap();
    assert!(
        errors["definitions"]["CodexErrorInfo"]["anyOf"][0]["enum"]
            .as_array()
            .unwrap()
            .contains(&json!("misalignmentPolicyViolation"))
    );
}

#[test]
fn duplicate_terminal_events_preserve_one_outcome_and_no_new_turn() {
    let mut state = active();
    let completion = json!({"threadId":"thread-one","turn":{"id":"turn-one","status":"completed"}});
    state.notification("turn/completed", &completion).unwrap();
    assert_eq!(
        state
            .notification("turn/completed", &completion)
            .unwrap()
            .kind,
        EventKind::Notification
    );
    state
        .notification(
            "turn/started",
            &json!({"threadId":"thread-one","turn":{"id":"turn-one"}}),
        )
        .unwrap();
    assert!(state.turn_id.is_none());
    assert_eq!(state.completed_turns.len(), 1);
    assert!(
        state
            .notification(
                "turn/completed",
                &json!({"threadId":"thread-one","turn":{"id":"turn-one","status":"failed"}})
            )
            .is_err()
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
async fn timed_out_transport_never_sends_a_second_mutation() {
    let (client, server) = tokio::io::duplex(8192);
    let (cr, cw) = tokio::io::split(client);
    let (sr, _sw) = tokio::io::split(server);
    let mut sr = BufReader::new(sr);
    let mut rpc = RpcClient::from_transport(cr, cw, Duration::from_millis(10)).unwrap();
    assert_eq!(rpc.initialize().await.unwrap_err().code, "timeout");
    assert_eq!(
        rpc.initialize().await.unwrap_err().code,
        "operation_uncertain"
    );
    let sent = read_json(&mut sr).await;
    assert_eq!(sent["id"], 1);
    let mut extra = String::new();
    assert!(
        tokio::time::timeout(Duration::from_millis(10), sr.read_line(&mut extra))
            .await
            .is_err()
    );
}

fn thread_response(cwd: &Path, permissions: &PermissionConfiguration) -> Value {
    json!({"thread":{"id":"owned-thread"},"cwd":cwd,"model":"fixture-model","modelProvider":"openai","reasoningEffort":"high","approvalPolicy":"never","approvalsReviewer":"user","activePermissionProfile":{"id":permissions.profile_id},"sandbox":{"type":"readOnly","networkAccess":false},"serviceTier":"default","instructionSources":[cwd.join("AGENTS.md")]})
}

/// Small draft-07 structural assertion for the exact checked-in wire schemas.
/// It validates emitted fields/required values, never interprets model output.
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
        .join("protocol/codex-0.164.0-alpha.11.1")
        .join(name);
    let schema: Value = serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap();
    assert!(
        conforms(params, &schema, &schema),
        "emitted params do not conform to {name}: {params}"
    );
}

#[tokio::test]
async fn production_session_requests_follow_pinned_schema_and_completion_races() {
    let (_temp, plan) = fixture();
    let isolation = plan.validate().unwrap();
    let permissions =
        PermissionConfiguration::for_isolation(&isolation, &plan.execution_root.join("auth"))
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
    let permissions =
        PermissionConfiguration::for_isolation(&isolation, &plan.execution_root.join("auth"))
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
        kegari_v0::digest::canonical_digest(1, &permissions.settings).unwrap()
    );
    task.await.unwrap();
}

#[derive(Default)]
struct FixtureRequestGate {
    observations: std::sync::Mutex<Vec<String>>,
    deny: std::sync::atomic::AtomicBool,
}
impl RequestGate for FixtureRequestGate {
    fn dispatch(
        &self,
        id: u64,
        method: &str,
        _: &Value,
        transmit: &mut dyn FnMut() -> kegari_types::error::Result<()>,
    ) -> kegari_types::error::Result<()> {
        self.observations
            .lock()
            .unwrap()
            .push(format!("dispatch:{id}:{method}"));
        if self.deny.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(kegari_types::error::Error::new(
                "access_denied",
                "fixture already fenced",
            ));
        }
        transmit()
    }
    fn observed(
        &self,
        id: u64,
        method: &str,
        result: &kegari_types::error::Result<Value>,
    ) -> kegari_types::error::Result<()> {
        self.observations
            .lock()
            .unwrap()
            .push(format!("observed:{id}:{method}:{}", result.is_ok()));
        Ok(())
    }
    fn uncertain(&self, id: u64, method: &str) -> kegari_types::error::Result<()> {
        self.observations
            .lock()
            .unwrap()
            .push(format!("uncertain:{id}:{method}"));
        Ok(())
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mutation_gate_surrounds_only_pipe_dispatch_and_records_actual_response() {
    let (client, server) = tokio::io::duplex(4096);
    let (cr, cw) = tokio::io::split(client);
    let (sr, mut sw) = tokio::io::split(server);
    let server = tokio::spawn(async move {
        let mut lines = BufReader::new(sr).lines();
        let init: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        sw.write_all(format!("{}\n",json!({"id":init["id"],"result":{"userAgent":"fixture","platformFamily":"unix","platformOs":"macos","codexHome":"/fixture/auth"}})).as_bytes()).await.unwrap();
        let _: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        let request: Value =
            serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(request["method"], "turn/start");
        sw.write_all(
            format!(
                "{}\n",
                json!({"id":request["id"],"result":{"turn":{"id":"fixture-turn"}}})
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    });
    let mut client = RpcClient::from_transport(cr, cw, Duration::from_secs(1)).unwrap();
    let gate = std::sync::Arc::new(FixtureRequestGate::default());
    client.bind_request_gate(gate.clone()).unwrap();
    client.initialize().await.unwrap();
    client
        .request("turn/start", json!({}), None, &mut DenyTools)
        .await
        .unwrap();
    server.await.unwrap();
    assert_eq!(
        *gate.observations.lock().unwrap(),
        ["dispatch:2:turn/start", "observed:2:turn/start:true"]
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fenced_request_gate_prevents_provider_bytes_and_keeps_uncertainty_distinct() {
    let (client, server) = tokio::io::duplex(4096);
    let (cr, cw) = tokio::io::split(client);
    let (sr, mut sw) = tokio::io::split(server);
    let server = tokio::spawn(async move {
        let mut lines = BufReader::new(sr).lines();
        let init: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        sw.write_all(format!("{}\n",json!({"id":init["id"],"result":{"userAgent":"fixture","platformFamily":"unix","platformOs":"macos","codexHome":"/fixture/auth"}})).as_bytes()).await.unwrap();
        lines.next_line().await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(35), lines.next_line())
                .await
                .is_err()
        );
    });
    let mut client = RpcClient::from_transport(cr, cw, Duration::from_secs(1)).unwrap();
    let gate = std::sync::Arc::new(FixtureRequestGate::default());
    gate.deny.store(true, std::sync::atomic::Ordering::SeqCst);
    client.bind_request_gate(gate.clone()).unwrap();
    client.initialize().await.unwrap();
    assert_eq!(
        client
            .request("turn/start", json!({}), None, &mut DenyTools)
            .await
            .unwrap_err()
            .code,
        "access_denied"
    );
    server.await.unwrap();
    assert_eq!(
        *gate.observations.lock().unwrap(),
        ["dispatch:2:turn/start", "uncertain:2:turn/start"]
    );
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
    let permissions =
        PermissionConfiguration::for_isolation(&plan.validate().unwrap(), &root.join("auth"))
            .unwrap();
    let verified = RpcClient::verify_executable(&binary, Duration::from_secs(2))
        .await
        .unwrap();
    let mut client = RpcClient::start_verified(
        &verified,
        &root.join("auth"),
        &permissions,
        Duration::from_secs(2),
    )
    .unwrap();
    let owned = client.cancellation().unwrap();
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
        RpcClient::start_verified(
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
        let error = RpcClient::verify_version(&binary, deadline)
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
    let permissions =
        PermissionConfiguration::for_isolation(&boundary, &plan.execution_root.join("auth"))
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
async fn owned_thread_history_rejects_divergence_and_retains_policy_before_resume() {
    for scenario in [
        "current",
        "changed_content",
        "summary",
        "missing_items",
        "null_view",
        "object_view",
        "boolean_view",
        "unknown_view",
        "legacy",
        "cwd",
        "unrecorded",
        "active",
        "duplicate",
        "failed",
        "policy",
    ] {
        let (client, server) = tokio::io::duplex(16_384);
        let (cr, cw) = tokio::io::split(client);
        let (sr, mut sw) = tokio::io::split(server);
        let mut sr = BufReader::new(sr);
        let task = tokio::spawn(async move {
            handshake(&mut sr, &mut sw).await;
            let request = read_json(&mut sr).await;
            assert_eq!(request["method"], "account/read");
            write_json(&mut sw, json!({"id":request["id"],"result":{"account":{"type":"chatgpt"},"workspaceRouting":{"chatgptAccountId":"account-one"}}})).await;
            let request = read_json(&mut sr).await;
            assert_eq!(request["method"], "thread/read");
            assert_wire_schema(&request["params"], "v2_ThreadReadParams.json");
            let turn = json!({"id":"turn-one","status":"completed","items":[{"id":"reply","type":"agentMessage","text":"original"}]});
            let mut thread = json!({"id":"thread-one","cwd":"/private/tmp/fixture/repository","modelProvider":"openai","status":{"type":"idle"},"turns":[turn.clone()]});
            match scenario {
                "current" | "legacy" => (),
                "changed_content" => thread["turns"][0]["items"][0]["text"] = json!("edited"),
                "summary" => thread["turns"][0]["itemsView"] = json!("summary"),
                "missing_items" => {
                    thread["turns"][0].as_object_mut().unwrap().remove("items");
                }
                "null_view" => thread["turns"][0]["itemsView"] = Value::Null,
                "object_view" => thread["turns"][0]["itemsView"] = json!({"type":"full"}),
                "boolean_view" => thread["turns"][0]["itemsView"] = json!(true),
                "unknown_view" => thread["turns"][0]["itemsView"] = json!("future-view"),
                "cwd" => thread["cwd"] = json!("/private/tmp/unrelated"),
                "unrecorded" => thread["turns"][0]["id"] = json!("another-turn"),
                "active" => thread["status"] = json!({"type":"active","activeFlags":[]}),
                "duplicate" => thread["turns"] = json!([turn.clone(), turn]),
                "failed" => {
                    thread["turns"][0]["status"] = json!("failed");
                    thread["turns"][0]["error"] = json!({"message":"transport failed","codexErrorInfo":"internalServerError"});
                }
                "policy" => {
                    // An unknown provider turn cannot hide an actual policy signal
                    // behind the subsequent exact-history mismatch.
                    thread["turns"][0]["id"] = json!("provider-new-turn");
                    thread["turns"][0]["status"] = json!("failed");
                    thread["turns"][0]["error"] =
                        json!({"message":"restricted","codexErrorInfo":"cyberPolicy"});
                }
                _ => unreachable!(),
            }
            write_json(
                &mut sw,
                json!({"id":request["id"],"result":{"thread":thread}}),
            )
            .await;
        });
        let mut rpc = RpcClient::from_transport(cr, cw, Duration::from_secs(2)).unwrap();
        rpc.initialize().await.unwrap();
        rpc.verify_account("account-one").await.unwrap();
        let mut state = active();
        state
            .notification(
                "turn/completed",
                &json!({"threadId":"thread-one","turn":{"id":"turn-one","status":"completed","items":[{"id":"reply","type":"agentMessage","text":"original"}]}}),
            )
            .unwrap();
        if scenario == "legacy" {
            state.provider_history = None;
        }
        let mut broker = ObservingBroker::default();
        let result = rpc.observe_owned_thread(&mut state, &mut broker).await;
        match scenario {
            "current" => assert!(result.unwrap().starts_with("sha256:v1:")),
            "cwd" => assert_eq!(result.unwrap_err().code, "identity_mismatch"),
            "null_view" | "object_view" | "boolean_view" | "unknown_view" => {
                assert_eq!(result.unwrap_err().code, "protocol_error");
            }
            "policy" => {
                assert_eq!(result.unwrap_err().code, "provider_intervention");
                assert!(
                    broker.intervention,
                    "actual blocker must be checkpointed immediately"
                );
                assert!(state.intervention.is_some());
                assert_eq!(
                    state.next_turn_allowed().unwrap_err().code,
                    "provider_intervention"
                );
            }
            _ => assert_eq!(result.unwrap_err().code, "reconciliation_required"),
        }
        assert!(state.turn_id.is_none());
        assert_eq!(state.turns_started, 1);
        task.await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn terminal_transient_classification_requires_full_native_failed_observation() {
    for (code, shape, expected) in [
        (
            "serverOverloaded",
            "full",
            Some(TransientTurnFailure::ServerOverloaded),
        ),
        (
            "internalServerError",
            "full",
            Some(TransientTurnFailure::InternalServerError),
        ),
        ("unauthorized", "full", None),
        ("other", "full", None),
        ("badRequest", "full", None),
        ("cyberPolicy", "full", None),
        ("serverOverloaded", "misalignment", None),
        ("serverOverloaded", "summary", None),
        ("serverOverloaded", "missing_items", None),
        ("serverOverloaded", "completed", None),
    ] {
        let (client, server) = tokio::io::duplex(16_384);
        let (cr, cw) = tokio::io::split(client);
        let (sr, mut sw) = tokio::io::split(server);
        let mut sr = BufReader::new(sr);
        let task = tokio::spawn(async move {
            handshake(&mut sr, &mut sw).await;
            let mut turn = json!({"id":"turn-one","status":"failed","items":[],
                "error":{"message":"private diagnostic sentinel","codexErrorInfo":code}});
            match shape {
                "misalignment" => turn["error"]["misalignment"] = json!({"reason":"policy detail"}),
                "summary" => turn["itemsView"] = json!("summary"),
                "missing_items" => {
                    turn.as_object_mut().unwrap().remove("items");
                }
                "completed" => turn["status"] = json!("completed"),
                _ => (),
            }
            write_json(
                &mut sw,
                json!({"method":"turn/completed","params":{"threadId":"thread-one","turn":turn}}),
            )
            .await;
        });
        let mut rpc = RpcClient::from_transport(cr, cw, Duration::from_secs(2)).unwrap();
        rpc.initialize().await.unwrap();
        let mut state = active();
        let mut broker = ObservingBroker::default();
        rpc.pump(
            &mut state,
            &mut broker,
            Duration::from_secs(2),
            Duration::from_secs(2),
        )
        .await
        .unwrap();
        task.await.unwrap();
        assert_eq!(
            state.terminal_transient_failure().unwrap(),
            expected,
            "{code}/{shape}"
        );
        let serialized = serde_json::to_vec(&state).unwrap();
        assert!(!String::from_utf8_lossy(&serialized).contains("private diagnostic sentinel"));
        assert_eq!(
            SessionState::restore(&serialized)
                .unwrap()
                .terminal_transient_failure()
                .unwrap(),
            expected
        );
        if expected.is_some() {
            let retained: Value = serde_json::from_slice(&serialized).unwrap();
            for field in ["terminal_digest", "policy_free"] {
                let mut changed = retained.clone();
                changed["provider_history"]["turns"][0]["transient_failure"][field] = if field
                    == "policy_free"
                {
                    json!(false)
                } else {
                    json!(kegari_v0::digest::canonical_digest(1, &"different terminal").unwrap())
                };
                assert!(SessionState::restore(&serde_json::to_vec(&changed).unwrap()).is_err());
            }
            let mut legacy = retained;
            legacy["provider_history"]["turns"][0]
                .as_object_mut()
                .unwrap()
                .remove("transient_failure");
            assert!(
                SessionState::restore(&serde_json::to_vec(&legacy).unwrap())
                    .unwrap()
                    .terminal_transient_failure()
                    .unwrap()
                    .is_none(),
                "legacy history must not acquire inferred transient provenance"
            );
        }
    }
}

#[tokio::test]
async fn late_policy_completions_checkpoint_before_protocol_rejection_on_both_read_paths() {
    for request_path in [false, true] {
        for scenario in [
            "same_outcome",
            "different_outcome",
            "unknown",
            "missing_status",
            "malformed_history",
            "foreign",
        ] {
            let (client, server) = tokio::io::duplex(16_384);
            let (cr, cw) = tokio::io::split(client);
            let (sr, mut sw) = tokio::io::split(server);
            let mut sr = BufReader::new(sr);
            let error = json!({"message":"provider policy detail","codexErrorInfo":"cyberPolicy"});
            let mut params = json!({"threadId":"thread-one","turn":{"id":"turn-one","status":"completed","items":[],"error":error}});
            match scenario {
                "different_outcome" => params["turn"]["status"] = json!("failed"),
                "unknown" => params["turn"]["id"] = json!("late-unrecorded-turn"),
                "missing_status" => {
                    params["turn"].as_object_mut().unwrap().remove("status");
                }
                "malformed_history" => params["turn"]["itemsView"] = Value::Null,
                "foreign" => params["threadId"] = json!("unrelated-thread"),
                _ => (),
            }
            let sent = params.clone();
            let task = tokio::spawn(async move {
                handshake(&mut sr, &mut sw).await;
                if request_path {
                    let request = read_json(&mut sr).await;
                    assert_eq!(request["method"], "thread/read");
                }
                write_json(&mut sw, json!({"method":"turn/completed","params":sent})).await;
            });
            let mut rpc = RpcClient::from_transport(cr, cw, Duration::from_secs(2)).unwrap();
            rpc.initialize().await.unwrap();
            let mut state = active();
            if scenario != "malformed_history" {
                state.notification("turn/completed", &json!({"threadId":"thread-one","turn":{"id":"turn-one","status":"completed","items":[]}})).unwrap();
            }
            let before = state.completed_turns.clone();
            let mut broker = ObservingBroker::default();
            let result = if request_path {
                rpc.request(
                    "thread/read",
                    json!({"threadId":"thread-one","includeTurns":true}),
                    Some(&mut state),
                    &mut broker,
                )
                .await
                .map(|_| ())
            } else {
                rpc.pump(
                    &mut state,
                    &mut broker,
                    Duration::from_secs(2),
                    Duration::from_secs(2),
                )
                .await
                .map(|_| ())
            };
            assert_eq!(result.unwrap_err().code, "protocol_error", "{scenario}");
            assert_eq!(
                state.completed_turns, before,
                "invalid completion must not be adopted"
            );
            if scenario == "foreign" {
                assert!(state.intervention.is_none());
                assert!(broker.checkpoint.is_none());
            } else {
                assert!(
                    broker.intervention,
                    "{scenario} must checkpoint the native policy hold"
                );
                let restored = SessionState::restore(broker.checkpoint.as_ref().unwrap()).unwrap();
                let observed = restored.intervention.as_ref().unwrap();
                assert_eq!(observed.account_id, "account-one");
                assert_eq!(observed.thread_id, "thread-one");
                assert_eq!(observed.turn_id, params["turn"]["id"].as_str().unwrap());
                assert_eq!(observed.source_method, "turn/completed");
                assert_eq!(observed.source_digest, kegari_v0::digest::canonical_digest(1, &json!({"threadId":"thread-one","turnId":params["turn"]["id"],"error":error})).unwrap());
                assert_eq!(
                    restored.next_turn_allowed().unwrap_err().code,
                    "provider_intervention"
                );
                assert!(!restored.completion_is_current());
            }
            task.await.unwrap();
        }
    }
}
