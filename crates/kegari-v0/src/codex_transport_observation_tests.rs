//! Real duplex/parser components with explicitly synthetic session/catalog data.
//! Nothing here constructs a native run, process-exit or Control capability.
use super::*;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

struct TailBroker {
    capture: ProtocolCapture,
    records: Records,
    checkpoints: Vec<SessionState>,
    fail_checkpoint: bool,
    calls: usize,
}
impl TailBroker {
    fn new(state: Option<&SessionState>) -> Self {
        let mut value = Self {
            capture: capture(),
            records: Records::default(),
            checkpoints: vec![],
            fail_checkpoint: false,
            calls: 0,
        };
        if let Some(state) = state {
            value.observe_protocol(ProtocolObservation::ThreadEstablished { state });
        }
        value
    }
    fn result(
        &mut self,
        state: &SessionState,
        end: &ProtocolTransportEnd,
    ) -> DeclaredProtocolObservation {
        self.observe_protocol(ProtocolObservation::TransportEnd { state, end });
        self.capture
            .finish(Some(state), None, &|leaf| self.records.read(leaf))
    }
}
impl ToolBroker for TailBroker {
    fn observe_protocol(&mut self, event: ProtocolObservation<'_>) {
        self.capture
            .observe(event, &mut |value| self.records.write(value));
    }
    fn observe_session(&mut self, state: &SessionState) -> Result<()> {
        self.checkpoints.push(state.clone());
        if self.fail_checkpoint {
            Err(Error::new(
                "component_checkpoint",
                "component cannot retain hold",
            ))
        } else {
            Ok(())
        }
    }
    fn call<'a>(
        &'a mut self,
        _: &'a ToolCall,
    ) -> Pin<Box<dyn Future<Output = Result<Value>> + Send + 'a>> {
        self.calls += 1;
        Box::pin(async { Err(Error::new("component_dispatch", "tail must not dispatch")) })
    }
}
fn completed() -> SessionState {
    let mut s = active();
    s.notification(
        "turn/completed",
        &json!({"threadId":"thread","turn":{"id":"turn","status":"completed","items":[]}}),
    )
    .unwrap();
    s
}
fn bytes(frames: &[Value]) -> Vec<u8> {
    frames
        .iter()
        .flat_map(|value| {
            let mut b = serde_json::to_vec(value).unwrap();
            b.push(b'\n');
            b
        })
        .collect()
}
async fn rpc_with_bytes(payload: Vec<u8>) -> (RpcClient, tokio::task::JoinHandle<()>) {
    let (client, mut server) = tokio::io::duplex(65536);
    let (reader, writer) = tokio::io::split(client);
    let task = tokio::spawn(async move {
        server.write_all(&payload).await.unwrap();
        server.shutdown().await.unwrap();
    });
    (
        RpcClient::from_transport(reader, writer, Duration::from_secs(5)).unwrap(),
        task,
    )
}
fn is_clean(end: &ProtocolTransportEnd) -> bool {
    matches!(
        end.diagnostic(),
        ProtocolTransportEndDiagnostic::CleanEof { .. }
    )
}

#[tokio::test]
async fn actual_queued_tail_reaches_eof_without_dispatch_or_complete_claim() {
    let frames = vec![
        json!({"method":"turn/completed","params":{"threadId":"thread","turn":{"id":"turn","status":"completed","items":[]}}}),
        json!({"id":"late","method":"item/tool/call","params":{"threadId":"thread","turnId":"turn","callId":"late","tool":"unsupported","arguments":{"text":"late attempted output"}}}),
    ];
    let (client, mut server) = tokio::io::duplex(65536);
    server.write_all(&bytes(&frames)).await.unwrap();
    server.shutdown().await.unwrap();
    let (reader, writer) = tokio::io::split(client);
    let mut rpc = RpcClient::from_transport(reader, writer, Duration::from_secs(5)).unwrap();
    let mut state = active();
    let mut broker = TailBroker::new(Some(&state));
    let RpcMessage::Notification { method, params } = rpc.read_message().await.unwrap() else {
        panic!("first terminal")
    };
    checkpoint_notification(&mut state, &mut broker, &method, &params).unwrap();
    assert!(
        !rpc.messages.is_empty(),
        "the test must cover actual queued tail data"
    );
    let prior = state.completed_turns.clone();
    let end = rpc
        .observe_declared_tail(
            &mut state,
            &mut broker,
            Instant::now() + Duration::from_secs(5),
        )
        .await;
    assert!(is_clean(&end));
    assert!(rpc.clean_read_eof);
    assert_eq!(broker.calls, 0);
    let mut response_byte = [0_u8; 1];
    assert!(
        tokio::time::timeout(Duration::from_millis(1), server.read(&mut response_byte))
            .await
            .is_err(),
        "tail must not write a denial or new request"
    );
    assert_eq!(state.completed_turns, prior);
    let report = broker.result(&state, &end);
    assert_eq!(report.state, ProtocolCaptureState::Incomplete);
    assert!(
        report
            .issues
            .contains(&ProtocolCaptureIssue::LateUnservedRequest)
    );
    assert!(
        report
            .issues
            .contains(&ProtocolCaptureIssue::CoverageNotQualified)
    );
    assert!(
        !report
            .issues
            .contains(&ProtocolCaptureIssue::UnobservedTransportTail)
    );
    assert!(
        serde_json::to_string(&broker.records.0)
            .unwrap()
            .contains("late attempted output")
    );
}

#[tokio::test]
async fn actual_tail_preserves_same_thread_unknown_turn_policy_before_rejection() {
    for foreign in [false, true] {
        let frames = [
            json!({"method":"turn/started","params":{"threadId":if foreign {"foreign"}else{"thread"},"turn":{"id":"unadmitted-turn","status":"inProgress","error":{"message":"held","codexErrorInfo":"cyberPolicy"}}}}),
        ];
        let (mut rpc, task) = rpc_with_bytes(bytes(&frames)).await;
        let mut state = completed();
        let before = state.completed_turns.clone();
        let mut broker = TailBroker::new(Some(&state));
        let end = rpc
            .observe_declared_tail(
                &mut state,
                &mut broker,
                Instant::now() + Duration::from_secs(5),
            )
            .await;
        task.await.unwrap();
        assert!(is_clean(&end));
        assert_eq!(state.intervention.is_some(), !foreign);
        assert_eq!(broker.checkpoints.len(), usize::from(!foreign));
        assert_eq!(state.completed_turns, before);
        assert!(state.turn_id.is_none());
        assert_eq!(broker.calls, 0);
        assert!(
            broker
                .result(&state, &end)
                .issues
                .contains(&ProtocolCaptureIssue::UnsupportedTailFrame)
        );
    }
}

#[tokio::test]
async fn actual_late_interaction_hold_uses_checkpoint_and_preserves_failure() {
    for (method, fail) in [
        ("item/commandExecution/requestApproval", false),
        ("item/fileChange/requestApproval", false),
        ("execCommandApproval", false),
        ("applyPatchApproval", false),
        ("item/permissions/requestApproval", false),
        ("item/tool/requestUserInput", false),
        ("mcpServer/elicitation/request", false),
        ("item/tool/requestUserInput", true),
    ] {
        let frames = [
            json!({"id":"late-input","method":method,"params":{"threadId":"thread","turnId":"turn","questions":[]}}),
        ];
        let (mut rpc, task) = rpc_with_bytes(bytes(&frames)).await;
        let mut state = completed();
        let mut broker = TailBroker::new(Some(&state));
        broker.fail_checkpoint = fail;
        let end = rpc
            .observe_declared_tail(
                &mut state,
                &mut broker,
                Instant::now() + Duration::from_secs(5),
            )
            .await;
        task.await.unwrap();
        assert!(state.input_required);
        assert_eq!(broker.checkpoints.len(), 1);
        assert_eq!(broker.calls, 0);
        if fail {
            assert!(
                matches!(end.diagnostic(),ProtocolTransportEndDiagnostic::Incomplete{error,..} if error.code=="component_checkpoint")
            );
        } else {
            assert!(is_clean(&end));
        }
    }
}

#[tokio::test]
async fn original_deadline_and_single_use_refuse_new_eof_window() {
    let (client, _held_open_server) = tokio::io::duplex(1024);
    let (reader, writer) = tokio::io::split(client);
    let mut rpc = RpcClient::from_transport(reader, writer, Duration::from_secs(5)).unwrap();
    let mut state = completed();
    let mut broker = TailBroker::new(Some(&state));
    let end = rpc
        .observe_declared_tail(&mut state, &mut broker, Instant::now())
        .await;
    assert!(
        matches!(end.diagnostic(),ProtocolTransportEndDiagnostic::Incomplete{error,..}if error.code=="timeout")
    );
    assert!(!rpc.clean_read_eof);
    let again = rpc
        .observe_declared_tail(
            &mut state,
            &mut broker,
            Instant::now() + Duration::from_secs(5),
        )
        .await;
    assert!(
        matches!(again.diagnostic(),ProtocolTransportEndDiagnostic::Incomplete{error,..}if error.code=="ownership_conflict")
    );
}

#[tokio::test]
async fn decoder_error_or_partial_line_never_produces_typed_eof() {
    for payload in [
        b"{\"method\":\"tick\"}".to_vec(),
        b"not-json\n".to_vec(),
        b"{\"method\":\"tick\"}\nnot-json\n".to_vec(),
    ] {
        let (mut rpc, task) = rpc_with_bytes(payload).await;
        let mut state = completed();
        let mut broker = TailBroker::new(Some(&state));
        let end = rpc
            .observe_declared_tail(
                &mut state,
                &mut broker,
                Instant::now() + Duration::from_secs(5),
            )
            .await;
        task.await.unwrap();
        assert!(!is_clean(&end));
        assert!(!rpc.clean_read_eof);
        assert!(
            matches!(end.diagnostic(),ProtocolTransportEndDiagnostic::Incomplete{error,..}if error.code=="protocol_error")
        );
    }
}

#[tokio::test]
async fn tail_wire_bytes_include_whitespace_and_oversize_stream_poisoning() {
    let payload = format!("{{{}\"id\":7,\"result\":{{}}}}\r\n", " ".repeat(128)).into_bytes();
    let expected = payload.len() as u64;
    let (mut rpc, task) = rpc_with_bytes(payload).await;
    let mut state = completed();
    let mut broker = TailBroker::new(Some(&state));
    let end = rpc
        .observe_declared_tail(
            &mut state,
            &mut broker,
            Instant::now() + Duration::from_secs(5),
        )
        .await;
    task.await.unwrap();
    match end.diagnostic() {
        ProtocolTransportEndDiagnostic::CleanEof {
            decoded_bytes,
            wire_bytes,
            ..
        } => {
            assert_eq!(*wire_bytes, expected);
            assert!(*decoded_bytes < *wire_bytes);
        }
        _ => panic!("actual clean stream"),
    }
    let (mut rpc, task) = rpc_with_bytes(vec![b' '; 8 * 1024 * 1024 + 1]).await;
    let mut broker = TailBroker::new(Some(&state));
    let end = rpc
        .observe_declared_tail(
            &mut state,
            &mut broker,
            Instant::now() + Duration::from_secs(5),
        )
        .await;
    task.await.unwrap();
    assert!(
        matches!(end.diagnostic(),ProtocolTransportEndDiagnostic::Incomplete{error,..}if error.code=="bounded_input")
    );
    assert!(rpc.decoder.poisoned);
    assert!(!rpc.clean_read_eof);
}

#[tokio::test]
async fn bootstrap_denies_and_retains_unbound_requests_without_tentative_thread_adoption() {
    let (client, server) = tokio::io::duplex(65536);
    let (reader, writer) = tokio::io::split(client);
    let (sr, mut sw) = tokio::io::split(server);
    let task = tokio::spawn(async move {
        let mut sr = BufReader::new(sr);
        let mut line = String::new();
        sr.read_line(&mut line).await.unwrap();
        let request: Value = serde_json::from_str(&line).unwrap();
        sw.write_all(&bytes(&[
            json!({"id":"early","method":"item/tool/call","params":{"threadId":"thread","turnId":"turn","callId":"early","tool":"known","arguments":{"text":"bootstrap attempted output"}}}),
        ])).await.unwrap();
        line.clear();
        sr.read_line(&mut line).await.unwrap();
        let denial: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(denial["result"]["success"], false);
        sw.write_all(&bytes(&[
            json!({"method":"item/completed","params":{"threadId":"thread","turnId":"turn","item":{"id":"early-message","type":"agentMessage","text":"bootstrap message"}}}),
            json!({"id":request["id"],"result":{"userAgent":"component","platformFamily":"unix","platformOs":"macos","codexHome":"/component/auth"}}),
        ])).await.unwrap();
        line.clear();
        sr.read_line(&mut line).await.unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&line).unwrap()["method"],
            "initialized"
        );
        sw.shutdown().await.unwrap();
    });
    let mut rpc = RpcClient::from_transport(reader, writer, Duration::from_secs(5)).unwrap();
    let mut broker = TailBroker::new(None);
    rpc.initialize_observed(&mut broker).await.unwrap();
    task.await.unwrap();
    assert_eq!(broker.calls, 0);
    let mut tentative = active();
    broker.observe_protocol(ProtocolObservation::AdapterInventory {
        state: &tentative,
        processed_requests: Some(rpc.observed_server_request_count()),
    });
    let end = rpc
        .observe_declared_tail(
            &mut tentative,
            &mut broker,
            Instant::now() + Duration::from_secs(5),
        )
        .await;
    let report = broker.result(&tentative, &end);
    assert!(is_clean(&end));
    assert!(report.thread_id.is_none());
    assert!(report.session_binding_digest.is_none());
    assert_eq!(report.state, ProtocolCaptureState::Unavailable);
    assert!(
        report
            .issues
            .contains(&ProtocolCaptureIssue::UnestablishedThread)
    );
    let raw = serde_json::to_string(&broker.records.0).unwrap();
    assert!(raw.contains("bootstrap attempted output"));
    assert!(raw.contains("bootstrap message"));
}

#[tokio::test]
async fn matched_response_records_full_message_without_completing_or_dispatching() {
    let (client, server) = tokio::io::duplex(65536);
    let (reader, writer) = tokio::io::split(client);
    let (sr, mut sw) = tokio::io::split(server);
    let task = tokio::spawn(async move {
        let mut sr = BufReader::new(sr);
        let mut line = String::new();
        sr.read_line(&mut line).await.unwrap();
        let req: Value = serde_json::from_str(&line).unwrap();
        sw.write_all(&bytes(&[json!({"id":req["id"],"result":{"reviewThreadId":"thread","turn":{"id":"turn","status":"inProgress","items":[{"id":"response-message","type":"agentMessage","text":"matched response output"}]}}})])).await.unwrap();
    });
    let mut rpc = RpcClient::from_transport(reader, writer, Duration::from_secs(5)).unwrap();
    rpc.initialized = true; // explicit component precondition, no native/account claim
    let mut state = active();
    let mut broker = TailBroker::new(Some(&state));
    let before = state.completed_turns.clone();
    rpc.request(
        "review/start",
        json!({"threadId":"thread","delivery":"inline"}),
        Some(&mut state),
        &mut broker,
    )
    .await
    .unwrap();
    task.await.unwrap();
    assert_eq!(state.completed_turns, before);
    assert_eq!(broker.calls, 0);
    assert!(
        serde_json::to_string(&broker.records.0)
            .unwrap()
            .contains("matched response output")
    );
}

#[tokio::test]
async fn actual_thread_start_failure_does_not_establish_tentative_thread() {
    use crate::isolation::{ExecutionRole, IsolationPlan};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    for leaf in ["repository", "scratch", "private", "auth"] {
        std::fs::create_dir(root.join(leaf)).unwrap();
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
        source_writes: vec![],
        forbidden_roots: vec![root.join("private")],
    };
    let isolation = plan.validate().unwrap();
    let permissions =
        PermissionConfiguration::for_isolation(&isolation, &root.join("auth")).unwrap();
    let mut binding = active().binding;
    binding.cwd = plan.cwd.clone();
    binding.isolation_digest = isolation.digest().into();
    let mut state = SessionState::new(binding, 4).unwrap();
    let response = json!({"thread":{"id":"thread"},"cwd":plan.cwd,"model":"component","modelProvider":"openai","reasoningEffort":"high","approvalPolicy":"never","activePermissionProfile":{"id":permissions.profile_id},"serviceTier":"default"});
    let (client, server) = tokio::io::duplex(65536);
    let (reader, writer) = tokio::io::split(client);
    let (sr, mut sw) = tokio::io::split(server);
    let task = tokio::spawn(async move {
        let mut sr = BufReader::new(sr);
        let mut line = String::new();
        sr.read_line(&mut line).await.unwrap();
        let request: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(request["method"], "thread/start");
        sw.write_all(&bytes(&[json!({"id":request["id"],"result":response})]))
            .await
            .unwrap();
    });
    let mut rpc = RpcClient::from_transport(reader, writer, Duration::from_secs(5)).unwrap();
    // Explicit component metadata inputs for the final response-validation
    // seam; no real account verification or native launch is asserted.
    rpc.initialized = true;
    rpc.account_id = Some(state.binding.account_id.clone());
    rpc.models = Some(vec![
        json!({"model":"component","supportedReasoningEfforts":[{"reasoningEffort":"high"}]}),
    ]);
    rpc.verified_permissions = Some(permissions.isolation_digest.clone());
    let mut broker = TailBroker::new(None);
    let error = rpc
        .start_thread_observed(
            &mut state,
            &isolation,
            &permissions,
            "component instruction",
            vec![],
            &mut broker,
        )
        .await
        .unwrap_err();
    task.await.unwrap();
    assert_eq!(error.code, "instruction_discovery_unavailable");
    assert_eq!(state.thread_id.as_deref(), Some("thread"));
    broker.observe_protocol(ProtocolObservation::AdapterInventory {
        state: &state,
        processed_requests: Some(0),
    });
    let report = broker
        .capture
        .finish(Some(&state), None, &|leaf| broker.records.read(leaf));
    assert!(report.thread_id.is_none());
    assert!(report.session_binding_digest.is_none());
    assert_eq!(report.state, ProtocolCaptureState::Unavailable);
}

#[tokio::test]
async fn actual_watchdog_interrupt_keeps_observation_and_denial() {
    let (client, server) = tokio::io::duplex(65536);
    let (reader, writer) = tokio::io::split(client);
    let (sr, mut sw) = tokio::io::split(server);
    let task = tokio::spawn(async move {
        let mut sr = BufReader::new(sr);
        let mut line = String::new();
        sr.read_line(&mut line).await.unwrap();
        let request: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(request["method"], "turn/interrupt");
        sw.write_all(&bytes(&[json!({"id":"interrupt-tool","method":"item/tool/call","params":{"threadId":"thread","turnId":"turn","callId":"interrupt-call","tool":"known","arguments":{"text":"interleaved during interrupt"}}})])).await.unwrap();
        line.clear();
        sr.read_line(&mut line).await.unwrap();
        let denial: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(denial["result"]["success"], false);
        sw.write_all(&bytes(&[json!({"id":request["id"],"result":{}})]))
            .await
            .unwrap();
    });
    let mut rpc = RpcClient::from_transport(reader, writer, Duration::from_secs(5)).unwrap();
    rpc.initialized = true;
    rpc.allowed_tools.insert("known".into());
    let mut state = active();
    let mut broker = TailBroker::new(Some(&state));
    let event = rpc
        .pump_until(&mut state, &mut broker, Instant::now(), None)
        .await
        .unwrap()
        .unwrap();
    task.await.unwrap();
    assert_eq!(event.method, "watchdog_expired");
    assert_eq!(broker.calls, 0);
    assert!(
        serde_json::to_string(&broker.records.0)
            .unwrap()
            .contains("interleaved during interrupt")
    );
    assert_eq!(broker.capture.report.tool_request_attempts, 1);
}

#[tokio::test]
async fn original_partial_decoder_bytes_count_against_tail_bound() {
    struct PrefixThenPending(std::io::Cursor<Vec<u8>>);
    impl tokio::io::AsyncRead for PrefixThenPending {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
            buf: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            let position = self.0.position() as usize;
            let bytes = self.0.get_ref();
            if position == bytes.len() {
                return std::task::Poll::Pending;
            }
            let count = buf.remaining().min(bytes.len() - position);
            buf.put_slice(&bytes[position..position + count]);
            self.0.set_position((position + count) as u64);
            std::task::Poll::Ready(Ok(()))
        }
    }
    let prefix = vec![b' '; 8 * 1024 * 1024 + 1];
    let mut rpc = RpcClient::from_transport(
        PrefixThenPending(std::io::Cursor::new(prefix)),
        tokio::io::sink(),
        Duration::from_secs(5),
    )
    .unwrap();
    // Poll the actual original read until its supplied transport is pending.
    // There is no EOF and no fabricated decoded frame or native process.
    let mut read = Box::pin(rpc.read_message());
    std::future::poll_fn(|cx| {
        assert!(read.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    drop(read);
    assert_eq!(rpc.decoder.pending.len(), 8 * 1024 * 1024 + 1);
    assert_eq!(rpc.transport_read_bytes, rpc.decoder.pending.len() as u64);
    let mut state = completed();
    let mut broker = TailBroker::new(Some(&state));
    let end = rpc
        .observe_declared_tail(
            &mut state,
            &mut broker,
            Instant::now() + Duration::from_secs(5),
        )
        .await;
    assert!(
        matches!(end.diagnostic(), ProtocolTransportEndDiagnostic::Incomplete {
        wire_bytes, error, ..
    } if *wire_bytes == 8 * 1024 * 1024 + 1 && error.code == "bounded_input")
    );
    assert!(!rpc.clean_read_eof);
    assert!(
        broker
            .result(&state, &end)
            .issues
            .contains(&ProtocolCaptureIssue::CaptureBound)
    );
}

#[tokio::test]
async fn queued_frame_limit_stops_without_claiming_eof() {
    let frames = vec![json!({"id":1,"result":{}}); 4097];
    let (mut rpc, task) = rpc_with_bytes(bytes(&frames)).await;
    let mut state = completed();
    let mut broker = TailBroker::new(Some(&state));
    let end = rpc
        .observe_declared_tail(
            &mut state,
            &mut broker,
            Instant::now() + Duration::from_secs(5),
        )
        .await;
    task.await.unwrap();
    assert!(
        matches!(end.diagnostic(), ProtocolTransportEndDiagnostic::Incomplete {
        frames: 4097, error, ..
    } if error.code == "bounded_input")
    );
    assert!(!rpc.clean_read_eof);
    assert!(
        broker
            .result(&state, &end)
            .issues
            .contains(&ProtocolCaptureIssue::CaptureBound)
    );
}

#[tokio::test]
async fn thread_start_some_state_keeps_unbound_completed_text_without_adopting_identity() {
    for terminal in [false, true] {
        let (client, server) = tokio::io::duplex(65536);
        let (reader, writer) = tokio::io::split(client);
        let (sr, mut sw) = tokio::io::split(server);
        let task = tokio::spawn(async move {
            let mut sr = BufReader::new(sr);
            let mut line = String::new();
            sr.read_line(&mut line).await.unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(request["method"], "thread/start");
            let item =
                json!({"id":"early-item","type":"agentMessage","text":"early Some-state output"});
            let frame = if terminal {
                json!({"method":"turn/completed","params":{"threadId":"tentative","turn":{"id":"unadmitted","status":"completed","items":[item]}}})
            } else {
                json!({"method":"item/completed","params":{"threadId":"tentative","turnId":"unadmitted","item":item}})
            };
            sw.write_all(&bytes(&[
                frame,
                json!({"id":request["id"],"result":{"thread":{"id":"tentative"}}}),
            ]))
            .await
            .unwrap();
        });
        let mut rpc = RpcClient::from_transport(reader, writer, Duration::from_secs(5)).unwrap();
        rpc.initialized = true; // explicit component handshake precondition
        let mut state = SessionState::new(active().binding, 4).unwrap();
        let mut broker = TailBroker::new(None);
        let result = rpc
            .request("thread/start", json!({}), Some(&mut state), &mut broker)
            .await;
        task.await.unwrap();
        if terminal {
            let error = result.unwrap_err();
            assert_eq!(error.code, "protocol_error");
            assert_eq!(error.message, "completion is not for the active turn");
        } else {
            assert_eq!(result.unwrap()["thread"]["id"], "tentative");
        }
        assert!(state.thread_id.is_none());
        assert!(state.turn_id.is_none());
        assert!(state.completed_turns.is_empty());
        assert_eq!(broker.calls, 0);
        assert!(broker.capture.messages.is_empty());
        assert!(broker.capture.turns.is_empty());
        let report = broker
            .capture
            .finish(Some(&state), None, &|leaf| broker.records.read(leaf));
        assert_eq!(
            report.state,
            if terminal {
                ProtocolCaptureState::Conflict
            } else {
                ProtocolCaptureState::Unavailable
            }
        );
        assert!(report.thread_id.is_none());
        assert!(report.session_binding_digest.is_none());
        assert!(
            report
                .issues
                .contains(&ProtocolCaptureIssue::UnestablishedThread)
        );
        assert_eq!(
            report
                .issues
                .contains(&ProtocolCaptureIssue::RejectedTerminal),
            terminal
        );
        let raw = serde_json::to_string(&broker.records.0).unwrap();
        assert!(raw.contains("early Some-state output"));
        assert!(raw.contains("unbound_agent_message"));
        assert!(
            !serde_json::to_string(&report)
                .unwrap()
                .contains("early Some-state output")
        );
    }
}

#[test]
fn established_foreign_output_never_uses_unbound_projection() {
    let mut state = completed();
    let mut broker = TailBroker::new(Some(&state));
    for (method, params) in [
        (
            "item/completed",
            json!({"threadId":"foreign","turnId":"turn","item":{"id":"foreign","type":"agentMessage","text":"must remain foreign"}}),
        ),
        (
            "turn/completed",
            json!({"threadId":"foreign","turn":{"id":"turn","status":"completed","items":[{"id":"foreign","type":"agentMessage","text":"must remain foreign"}]}}),
        ),
    ] {
        assert!(checkpoint_notification(&mut state, &mut broker, method, &params).is_err());
    }
    let mut changed = state.clone();
    changed.binding.instruction_digest = crate::digest::bytes_digest(b"different binding");
    broker.observe_protocol(ProtocolObservation::ThreadEstablished { state: &changed });
    // A failed rebind must not reopen the initial unbound-prefix path.
    let _ = checkpoint_notification(
        &mut changed,
        &mut broker,
        "item/completed",
        &json!({
            "threadId":"thread","turnId":"turn",
            "item":{"id":"changed","type":"agentMessage","text":"must remain changed"}
        }),
    );
    let raw = serde_json::to_string(&broker.records.0).unwrap();
    assert!(!raw.contains("must remain changed"));
    assert!(
        broker
            .capture
            .report
            .issues
            .contains(&ProtocolCaptureIssue::BindingChanged)
    );
    assert!(!raw.contains("must remain foreign"));
    assert!(!raw.contains("unbound_agent_message"));
    assert!(
        broker
            .capture
            .report
            .issues
            .contains(&ProtocolCaptureIssue::ForeignOrUnknownTurn)
    );
}
