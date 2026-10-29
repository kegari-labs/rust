use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Waker};
use tokio::io::{AsyncBufReadExt, BufReader};

struct Scope {
    cwd: PathBuf,
    digest: String,
    calls: AtomicUsize,
    failure: Option<Error>,
}
impl Scope {
    fn new(failure: Option<Error>) -> Self {
        Self {
            cwd: PathBuf::from("/private/tmp/component-repository"),
            digest: "scope-digest".into(),
            calls: AtomicUsize::new(0),
            failure,
        }
    }
}
impl SessionScope for Scope {
    fn revalidate(&self) -> Result<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        match &self.failure {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }
    fn cwd(&self) -> &Path {
        &self.cwd
    }
    fn isolation_digest(&self) -> &str {
        &self.digest
    }
}
fn state() -> SessionState {
    SessionState::new(
        SessionBinding {
            schema_version: 1,
            account_id: "account".into(),
            work_identity: "work".into(),
            operation_id: "operation".into(),
            generation: "generation".into(),
            instruction_digest: "instructions".into(),
            isolation_digest: "scope-digest".into(),
            profile: CodexProfile {
                model: "component".into(),
                reasoning: "high".into(),
                speed: "standard".into(),
            },
            cwd: PathBuf::from("/private/tmp/component-repository"),
        },
        3,
    )
    .unwrap()
}
fn permissions() -> PermissionConfiguration {
    PermissionConfiguration {
        profile_id: "component-profile".into(),
        settings: json!({}),
        isolation_digest: "scope-digest".into(),
    }
}
fn catalog() -> Vec<Value> {
    vec![json!({"model":"component","supportedReasoningEfforts":[{"reasoningEffort":"high"}]})]
}

#[test]
fn session_scope_preserves_validation_order_and_revalidates_once() {
    let mut client = RpcClient::from_transport(
        tokio::io::empty(),
        tokio::io::sink(),
        Duration::from_secs(1),
    )
    .unwrap();
    let permissions = permissions();
    let failed_scope = Scope::new(Some(Error::new("scope_refused", "fixture scope refusal")));
    let mut state = state();
    state.binding.account_id.clear();
    assert_eq!(
        client
            .validate_session(&state, &failed_scope, &permissions)
            .unwrap_err()
            .code,
        "invalid_config"
    );
    assert_eq!(failed_scope.calls.load(Ordering::SeqCst), 0);
    state.binding.account_id = "account".into();
    assert_eq!(
        client
            .validate_session(&state, &failed_scope, &permissions)
            .unwrap_err(),
        Error::new("scope_refused", "fixture scope refusal")
    );
    assert_eq!(failed_scope.calls.load(Ordering::SeqCst), 1);

    let scope = Scope::new(None);
    assert_eq!(
        client
            .validate_session(&state, &scope, &permissions)
            .unwrap_err()
            .code,
        "identity_mismatch"
    );
    client.account_id = Some(state.binding.account_id.clone());
    assert_eq!(
        client
            .validate_session(&state, &scope, &permissions)
            .unwrap_err()
            .code,
        "unsupported_profile"
    );
    client.models = Some(catalog());
    assert_eq!(
        client
            .validate_session(&state, &scope, &permissions)
            .unwrap_err()
            .code,
        "isolation_unverified"
    );
    client.verified_permissions = Some(permissions.isolation_digest.clone());
    state.binding.cwd = PathBuf::from("/private/tmp/foreign-repository");
    assert_eq!(
        client
            .validate_session(&state, &scope, &permissions)
            .unwrap_err()
            .code,
        "scope_violation"
    );
    state.binding.cwd = scope.cwd.clone();
    client
        .validate_session(&state, &scope, &permissions)
        .unwrap();
    assert_eq!(scope.calls.load(Ordering::SeqCst), 5);
}

#[tokio::test]
async fn successful_thread_start_revalidates_the_supplied_scope_once() {
    let scope = Scope::new(None);
    let mut state = state();
    let permissions = permissions();
    let response = json!({"id":1,"result":{"thread":{"id":"thread"},"cwd":scope.cwd,"model":"component","modelProvider":"openai","reasoningEffort":"high","approvalPolicy":"never","activePermissionProfile":{"id":permissions.profile_id},"serviceTier":"default","instructionSources":[]}});
    let (stream, server) = tokio::io::duplex(8192);
    let (reader, writer) = tokio::io::split(stream);
    let mut client = RpcClient::from_transport(reader, writer, Duration::from_secs(1)).unwrap();
    client.initialized = true;
    client.account_id = Some(state.binding.account_id.clone());
    client.models = Some(catalog());
    client.verified_permissions = Some(permissions.isolation_digest.clone());
    let task = tokio::spawn(async move {
        let mut server = BufReader::new(server);
        let mut line = String::new();
        server.read_line(&mut line).await.unwrap();
        let request: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(request["method"], "thread/start");
        server
            .get_mut()
            .write_all(format!("{response}\n").as_bytes())
            .await
            .unwrap();
    });
    client
        .start_thread(
            &mut state,
            &scope,
            &permissions,
            "component instructions",
            vec![],
        )
        .await
        .unwrap();
    task.await.unwrap();
    assert_eq!(scope.calls.load(Ordering::SeqCst), 1);
    assert_eq!(state.thread_id.as_deref(), Some("thread"));
}

#[tokio::test]
async fn host_startup_error_refuses_before_any_protocol_write() {
    let (stream, mut server) = tokio::io::duplex(8192);
    let (reader, writer) = tokio::io::split(stream);
    let error = Error::new("worker_binding", "fixture identity capture failed");
    let mut client = RpcClient::from_host_transport(
        reader,
        writer,
        Duration::from_secs(1),
        PathBuf::from("/private/tmp/auth"),
        Some(error.clone()),
    )
    .unwrap();
    assert_eq!(client.initialize().await.unwrap_err(), error);
    let mut byte = [0];
    let mut read = Box::pin(server.read(&mut byte));
    let mut context = Context::from_waker(Waker::noop());
    assert!(read.as_mut().poll(&mut context).is_pending());
}

#[tokio::test]
async fn host_home_mismatch_refuses_before_initialized_notification() {
    let (stream, server) = tokio::io::duplex(8192);
    let (reader, writer) = tokio::io::split(stream);
    let mut client = RpcClient::from_host_transport(
        reader,
        writer,
        Duration::from_secs(1),
        PathBuf::from("/private/tmp/expected-auth"),
        None,
    )
    .unwrap();
    let task = tokio::spawn(async move {
        let mut server = BufReader::new(server);
        let mut line = String::new();
        server.read_line(&mut line).await.unwrap();
        let request: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(request["method"], "initialize");
        let response = json!({"id":request["id"],"result":{"userAgent":SUPPORTED_CODEX_VERSION,"platformFamily":"unix","platformOs":"macos","codexHome":"/private/tmp/other-auth"}});
        server
            .get_mut()
            .write_all(format!("{response}\n").as_bytes())
            .await
            .unwrap();
        server
    });
    assert_eq!(
        client.initialize().await.unwrap_err(),
        Error::new(
            "identity_mismatch",
            "app-server used another Codex auth home"
        )
    );
    assert!(!client.initialized);
    let mut server = task.await.unwrap();
    let mut byte = [0];
    let mut read = Box::pin(server.read(&mut byte));
    let mut context = Context::from_waker(Waker::noop());
    assert!(read.as_mut().poll(&mut context).is_pending());
}

#[test]
fn digest_syntax_preserves_backend_golden_cases() {
    let corpus: Value =
        serde_json::from_str(include_str!("../tests/fixtures/digest-syntax.json")).unwrap();
    let expected: Error = serde_json::from_value(corpus["error"].clone()).unwrap();
    for digest in corpus["accepted"].as_array().unwrap() {
        let digest = digest.as_str().unwrap();
        assert_eq!(validate_digest(digest), Ok(()), "{digest:?}");
    }
    for digest in corpus["rejected"].as_array().unwrap() {
        let digest = digest.as_str().unwrap();
        assert_eq!(validate_digest(digest), Err(expected.clone()), "{digest:?}");
    }
}

#[derive(Default)]
struct LifecycleGate {
    dispatched: AtomicUsize,
    observed: AtomicUsize,
}
impl RequestGate for LifecycleGate {
    fn dispatch(
        &self,
        _: u64,
        method: &str,
        _: &Value,
        transmit: &mut dyn FnMut() -> Result<()>,
    ) -> Result<()> {
        assert_eq!(method, "command/exec");
        self.dispatched.fetch_add(1, Ordering::SeqCst);
        transmit()
    }
    fn observed(&self, _: u64, method: &str, result: &Result<Value>) -> Result<()> {
        assert_eq!(method, "command/exec");
        assert!(result.is_ok());
        self.observed.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn uncertain(&self, _: u64, _: &str) -> Result<()> {
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lifecycle_command_uses_verified_sandbox_and_actual_nonzero_result() {
    let (transport, server) = tokio::io::duplex(8192);
    let (read, write) = tokio::io::split(transport);
    let mut client = RpcClient::from_transport(read, write, Duration::from_secs(1)).unwrap();
    client.initialized = true;
    client.account_id = Some("account".into());
    let scope = Scope::new(None);
    let mut permissions = permissions();
    permissions.settings = json!({"features":{"shell_tool":true},"shell_environment_policy":{"set":{"HOME":"/Users/component","PATH":"/niche/tool/bin:/usr/bin","TMPDIR":"/private/tmp/component-scratch","LANG":"C.UTF-8"}}});
    let command = vec!["/bin/sh".into(), "-c".into(), "exit 7".into()];
    assert_eq!(
        client
            .execute_command(
                &scope,
                &permissions,
                &command,
                "operation:before_run",
                Duration::from_secs(1),
                1024
            )
            .await
            .unwrap_err()
            .code,
        "isolation_unverified"
    );
    client.verified_permissions = Some(scope.digest.clone());
    let gate = std::sync::Arc::new(LifecycleGate::default());
    client.bind_request_gate(gate.clone()).unwrap();
    let server = tokio::spawn(async move {
        let mut server = BufReader::new(server);
        let mut line = String::new();
        server.read_line(&mut line).await.unwrap();
        let frame: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(frame["method"], "command/exec");
        assert_eq!(
            frame["params"]["command"],
            json!(["/bin/sh", "-c", "exit 7"])
        );
        assert_eq!(frame["params"]["cwd"], "/private/tmp/component-repository");
        assert_eq!(frame["params"]["permissionProfile"], "component-profile");
        assert_eq!(frame["params"]["processId"], "operation:before_run");
        assert_eq!(frame["params"]["timeoutMs"], 1000);
        assert_eq!(frame["params"]["outputBytesCap"], 1024);
        assert_eq!(frame["params"]["disableTimeout"], false);
        assert_eq!(frame["params"]["disableOutputCap"], false);
        assert!(frame["params"].get("sandboxPolicy").is_none());
        assert_eq!(frame["params"]["env"]["PATH"], "/niche/tool/bin:/usr/bin");
        assert_eq!(frame["params"]["env"]["HOME"], "/Users/component");
        assert_eq!(frame["params"]["env"]["CODEX_HOME"], Value::Null);
        assert_eq!(frame["params"]["env"].as_object().unwrap().len(), 5);
        server.get_mut().write_all(format!("{}\n", json!({"id":frame["id"],"result":{"exitCode":7,"stdout":"observed stdout","stderr":"observed failure"}})).as_bytes()).await.unwrap();
    });
    let result = client
        .execute_command(
            &scope,
            &permissions,
            &command,
            "operation:before_run",
            Duration::from_secs(1),
            1024,
        )
        .await
        .unwrap();
    assert_eq!(result.exit_code, 7);
    assert_eq!(result.stdout, "observed stdout");
    assert_eq!(result.stderr, "observed failure");
    assert!(client.active_command.is_none());
    assert_eq!(gate.dispatched.load(Ordering::SeqCst), 1);
    assert_eq!(gate.observed.load(Ordering::SeqCst), 1);
    server.await.unwrap();
}

#[tokio::test]
async fn cancelled_lifecycle_command_keeps_identity_for_supported_termination() {
    let (transport, server) = tokio::io::duplex(8192);
    let (read, write) = tokio::io::split(transport);
    let mut client = RpcClient::from_transport(read, write, Duration::from_secs(1)).unwrap();
    client.initialized = true;
    client.account_id = Some("account".into());
    let scope = Scope::new(None);
    client.verified_permissions = Some(scope.digest.clone());
    let mut permissions = permissions();
    permissions.settings = json!({"features":{"shell_tool":true},"shell_environment_policy":{"set":{"HOME":"/Users/component","PATH":"/niche/tool/bin:/usr/bin","TMPDIR":"/private/tmp/component-scratch","LANG":"C.UTF-8"}}});
    let command = vec!["/bin/sh".into(), "-c".into(), "sleep 10".into()];
    let sent = std::sync::Arc::new(tokio::sync::Notify::new());
    let observed = sent.clone();
    let server = tokio::spawn(async move {
        let mut server = BufReader::new(server);
        let mut line = String::new();
        server.read_line(&mut line).await.unwrap();
        let original: Value = serde_json::from_str(&line).unwrap();
        observed.notify_one();
        line.clear();
        server.read_line(&mut line).await.unwrap();
        let stop: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(stop["method"], "command/exec/terminate");
        assert_eq!(stop["params"]["processId"], original["params"]["processId"]);
        server.get_mut().write_all(format!("{}\n{}\n", json!({"id":original["id"],"result":{"exitCode":0,"stdout":"late output","stderr":""}}), json!({"id":stop["id"],"result":{}})).as_bytes()).await.unwrap();
    });
    let mut running = Box::pin(client.execute_command(
        &scope,
        &permissions,
        &command,
        "owned-hook",
        Duration::from_secs(10),
        1024,
    ));
    tokio::select! { _ = sent.notified() => {}, result = &mut running => panic!("unexpected early result: {result:?}") }
    drop(running);
    assert_eq!(client.active_command.as_deref(), Some("owned-hook"));
    client.uncertain_request = Some(1);
    client.terminate_command().await.unwrap();
    assert_eq!(client.uncertain_request, Some(1));
    server.await.unwrap();
}
