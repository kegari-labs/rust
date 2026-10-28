//! Protocol/storage component evidence only. These synthetic session values do
//! not construct a worker, native run, permit, check result or source capability.
use super::*;
use crate::CodexProfile;
use kegari_storage::PrivateDir;
use std::path::PathBuf;

fn active() -> SessionState {
    let mut state = SessionState::new(
        SessionBinding {
            schema_version: 1,
            account_id: "component-account".into(),
            work_identity: "component-work".into(),
            operation_id: Uuid::from_u128(1).to_string(),
            generation: "component-generation".into(),
            instruction_digest: kegari_digest::bytes_digest(b"component instructions"),
            isolation_digest: kegari_digest::bytes_digest(b"component isolation"),
            profile: CodexProfile {
                model: "component".into(),
                reasoning: "high".into(),
                speed: "standard".into(),
            },
            cwd: PathBuf::from("/private/tmp/explicit-component"),
        },
        4,
    )
    .unwrap();
    state.thread_id = Some("thread".into());
    state.turn_id = Some("turn".into());
    state.turns_started = 1;
    state
}
fn capture() -> ProtocolCapture {
    ProtocolCapture::new(
        ProtocolOperationBinding {
            operation: Uuid::from_u128(1),
            runtime: Uuid::from_u128(2),
            workspace: Uuid::from_u128(3),
        },
        kegari_digest::bytes_digest(b"scope"),
        kegari_digest::bytes_digest(b"component instructions"),
        Some(kegari_digest::bytes_digest(b"component trial")),
        kegari_digest::bytes_digest(b"component suite"),
        kegari_digest::bytes_digest(b"component fixture"),
    )
}
#[derive(Default)]
struct Records(BTreeMap<String, Value>);
impl Records {
    fn write(&mut self, value: &Value) -> Result<String> {
        let leaf = format!("event-{:08}.json", self.0.len());
        self.0.insert(
            leaf.clone(),
            json!({"kind":"protocol_observation","record":value}),
        );
        Ok(leaf)
    }
    fn read(&self, leaf: &str) -> Result<Value> {
        self.0
            .get(leaf)
            .cloned()
            .ok_or_else(|| Error::new("record_missing", "component record absent"))
    }
}
fn establish(c: &mut ProtocolCapture, records: &mut Records, state: &SessionState) {
    if !c.established {
        c.observe(ProtocolObservation::ThreadEstablished { state }, &mut |v| {
            records.write(v)
        });
    }
}
fn terminal(c: &mut ProtocolCapture, records: &mut Records, state: &mut SessionState, text: &str) {
    establish(c, records, state);
    let params = json!({"threadId":"thread","turn":{"id":"turn","status":"completed","items":[
        {"id":"message","type":"agentMessage","text":text},
        {"id":"private","type":"reasoning","content":["hidden reasoning must not be copied"]}
    ]}});
    let accepted = state.notification("turn/completed", &params).is_ok();
    c.observe(
        ProtocolObservation::TerminalTurn {
            state,
            params: &params,
            accepted,
        },
        &mut |v| records.write(v),
    );
}
fn request(c: &mut ProtocolCapture, records: &mut Records, state: &SessionState, params: &Value) {
    establish(c, records, state);
    c.observe(
        ProtocolObservation::ServerRequest {
            state: Some(state),
            rpc_id: &json!(1),
            sequence: c.last_request_sequence + 1,
            method: "item/tool/call",
            params,
        },
        &mut |v| records.write(v),
    );
}
fn params(text: &str) -> Value {
    json!({"threadId":"thread","turnId":"turn","callId":"call","tool":"unsupported","arguments":{"text":text}})
}

#[test]
fn declared_capture_retains_denied_arguments_and_terminal_text_without_reasoning() {
    let mut c = capture();
    let mut r = Records::default();
    let mut s = active();
    s.input_required = true;
    request(&mut c, &mut r, &s, &params("attempted output"));
    c.observe(
        ProtocolObservation::ToolResult {
            state: Some(&s),
            rpc_id: &json!(1),
            error_code: Some("unsupported_tool"),
        },
        &mut |v| r.write(v),
    );
    terminal(&mut c, &mut r, &mut s, "complete public answer");
    let report = c.finish(Some(&s), None, &|leaf| r.read(leaf));
    report.validate().unwrap();
    assert_eq!(report.state, ProtocolCaptureState::Incomplete);
    assert_eq!(report.tool_request_attempts, 1);
    assert_eq!(report.terminal_turns, 1);
    let raw = serde_json::to_string(&r.0).unwrap();
    assert!(raw.contains("attempted output"));
    assert!(raw.contains("complete public answer"));
    assert!(raw.contains("unsupported_tool"));
    assert!(!raw.contains("hidden reasoning must not be copied"));
    let public = serde_json::to_string(&report).unwrap();
    assert!(!public.contains("attempted output"));
    assert!(!public.contains("complete public answer"));
}
#[test]
fn missing_foreign_conflicting_and_incomplete_terminal_data_never_become_empty_complete() {
    let mut c = capture();
    let mut r = Records::default();
    let mut s = active();
    assert_eq!(
        c.finish(None, None, &|leaf| r.read(leaf)).state,
        ProtocolCaptureState::Unavailable
    );
    request(&mut c, &mut r, &s, &params("first"));
    request(&mut c, &mut r, &s, &params("changed"));
    terminal(&mut c, &mut r, &mut s, "answer");
    assert_eq!(
        c.finish(Some(&s), None, &|leaf| r.read(leaf)).state,
        ProtocolCaptureState::Conflict
    );
    assert!(
        c.report
            .issues
            .contains(&ProtocolCaptureIssue::ConflictingDuplicate)
    );
    let mut c = capture();
    let mut foreign = params("foreign");
    foreign["threadId"] = json!("other");
    request(&mut c, &mut r, &s, &foreign);
    assert!(
        c.report
            .issues
            .contains(&ProtocolCaptureIssue::ForeignOrUnknownTurn)
    );
    let mut c = capture();
    let mut s = active();
    let p = json!({"threadId":"thread","turn":{"id":"turn","status":"completed","itemsView":"summary","items":[]}});
    establish(&mut c, &mut r, &s);
    let accepted = s.notification("turn/completed", &p).is_ok();
    c.observe(
        ProtocolObservation::TerminalTurn {
            state: &s,
            params: &p,
            accepted,
        },
        &mut |v| r.write(v),
    );
    assert_eq!(
        c.finish(Some(&s), None, &|leaf| r.read(leaf)).state,
        ProtocolCaptureState::Incomplete
    );
    assert!(
        c.report
            .issues
            .contains(&ProtocolCaptureIssue::MissingFullTurn)
    );
}
#[test]
fn exact_terminal_duplicate_deduplicates_but_conflicting_duplicate_remains_visible() {
    let mut c = capture();
    let mut r = Records::default();
    let mut s = active();
    terminal(&mut c, &mut r, &mut s, "first");
    let count = r.0.len();
    terminal(&mut c, &mut r, &mut s, "first");
    assert_eq!(r.0.len(), count);
    terminal(&mut c, &mut r, &mut s, "changed");
    assert_eq!(
        c.finish(Some(&s), None, &|leaf| r.read(leaf)).state,
        ProtocolCaptureState::Conflict
    );
    assert!(
        c.report
            .issues
            .contains(&ProtocolCaptureIssue::RejectedTerminal)
    );
}
#[test]
fn overflow_and_failed_protected_capture_remain_explicit() {
    let mut c = capture();
    let mut r = Records::default();
    let s = active();
    establish(&mut c, &mut r, &s);
    let before = r.0.clone();
    let before_refs = serde_json::to_value(&c.report.records).unwrap();
    request(
        &mut c,
        &mut r,
        &s,
        &params(&"x".repeat(MAX_RECORD_BYTES + 1)),
    );
    assert_eq!(
        c.finish(Some(&s), None, &|leaf| r.read(leaf)).state,
        ProtocolCaptureState::Overflow
    );
    // Metadata already exists; the oversized request writes no payload or
    // truncated replacement and adds no retained reference.
    assert_eq!(r.0, before);
    assert_eq!(
        serde_json::to_value(&c.report.records).unwrap(),
        before_refs
    );
    let mut c = capture();
    establish(&mut c, &mut r, &s);
    c.observe(
        ProtocolObservation::ServerRequest {
            state: Some(&s),
            rpc_id: &json!(1),
            sequence: c.last_request_sequence + 1,
            method: "item/tool/call",
            params: &params("actual attempt"),
        },
        &mut |_| Err(Error::new("io_error", "component write failure")),
    );
    assert_eq!(
        c.finish(Some(&s), None, &|leaf| r.read(leaf)).state,
        ProtocolCaptureState::Unavailable
    );
}
#[test]
fn actual_protected_capture_readback_detects_removed_or_changed_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let dir = PrivateDir::create(&root.join("records")).unwrap();
    let mut c = capture();
    let mut s = active();
    let p = json!({"threadId":"thread","turn":{"id":"turn","status":"completed","items":[{"id":"message","type":"agentMessage","text":"observed"}]}});
    s.notification("turn/completed", &p).unwrap();
    let mut next = 0;
    let mut writer = |value: &Value| {
        let leaf = format!("event-{next:08}.json");
        next += 1;
        dir.write(
            &leaf,
            &json!({"kind":"protocol_observation","record":value}),
        )?;
        Ok(leaf)
    };
    c.observe(
        ProtocolObservation::ThreadEstablished { state: &s },
        &mut writer,
    );
    c.observe(
        ProtocolObservation::TerminalTurn {
            state: &s,
            params: &p,
            accepted: true,
        },
        &mut writer,
    );
    assert_eq!(
        c.finish(Some(&s), None, &|leaf| dir.read_bounded(leaf, 256 * 1024))
            .state,
        ProtocolCaptureState::Incomplete
    );
    let (leaf, original) = c
        .report
        .records
        .iter()
        .find_map(|reference| {
            let value: Value = dir
                .read_bounded(&reference.journal_leaf, 256 * 1024)
                .unwrap();
            (value.pointer("/record/channel").and_then(Value::as_str) == Some("agent_message")
                && value.pointer("/record/message/id").and_then(Value::as_str) == Some("message")
                && value
                    .pointer("/record/message/text")
                    .and_then(Value::as_str)
                    == Some("observed"))
            .then(|| (reference.journal_leaf.clone(), value))
        })
        .expect("the actual retained agent-message payload must exist before mutation");
    let mut changed = original.clone();
    changed["record"]["message"]["text"] = json!("changed payload");
    dir.write(&leaf, &changed).unwrap();
    assert_eq!(
        c.finish(Some(&s), None, &|leaf| dir.read_bounded(leaf, 256 * 1024))
            .state,
        ProtocolCaptureState::Unavailable
    );
    dir.write(&leaf, &original).unwrap();
    assert_eq!(
        c.finish(Some(&s), None, &|leaf| dir.read_bounded(leaf, 256 * 1024))
            .state,
        ProtocolCaptureState::Incomplete
    );
    std::fs::remove_file(dir.path().join(&leaf)).unwrap();
    assert_eq!(
        c.finish(Some(&s), None, &|leaf| dir.read_bounded(leaf, 256 * 1024))
            .state,
        ProtocolCaptureState::Unavailable
    );
}

struct WireBroker {
    capture: ProtocolCapture,
    records: Records,
    dispatched: usize,
}
impl ToolBroker for WireBroker {
    fn observe_protocol(&mut self, event: ProtocolObservation<'_>) {
        self.capture.observe(event, &mut |v| self.records.write(v));
    }
    fn call<'a>(
        &'a mut self,
        _: &'a ToolCall,
    ) -> Pin<Box<dyn Future<Output = Result<Value>> + Send + 'a>> {
        self.dispatched += 1;
        Box::pin(async {
            Err(Error::new(
                "component_refusal",
                "component broker denies effect",
            ))
        })
    }
}
#[tokio::test]
async fn actual_adapter_observes_requests_before_whitelist_and_intervention_then_terminal() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let (client, server) = tokio::io::duplex(32 * 1024);
    let (cr, cw) = tokio::io::split(client);
    let (sr, mut sw) = tokio::io::split(server);
    let task = tokio::spawn(async move {
        let mut sr = BufReader::new(sr);
        let mut line = String::new();
        sr.read_line(&mut line).await.unwrap();
        let initialize: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(initialize["method"], "initialize");
        sw.write_all(format!("{}\n",json!({"id":initialize["id"],"result":{"userAgent":"component","platformFamily":"unix","platformOs":"macos","codexHome":"/component/auth"}})).as_bytes()).await.unwrap();
        line.clear();
        sr.read_line(&mut line).await.unwrap();
        let initialized: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(initialized["method"], "initialized");
        line.clear();
        sr.read_line(&mut line).await.unwrap();
        let request: Value = serde_json::from_str(&line).unwrap();
        // This is the ordinary request loop over actual JSON-RPC frames. The
        // synthetic broker below has no execution/Control permission.
        for (index, tool) in [(1, "known"), (2, "unsupported")] {
            sw.write_all(format!("{}\n",json!({"id":index,"method":"item/tool/call","params":{"threadId":"thread","turnId":"turn","callId":format!("call-{index}"),"tool":tool,"arguments":{"text":format!("attempt-{index}")}}})).as_bytes()).await.unwrap();
            line.clear();
            sr.read_line(&mut line).await.unwrap();
            let reply: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(reply["result"]["success"], false);
        }
        sw.write_all(format!("{}\n",json!({"method":"turn/started","params":{"threadId":"thread","turn":{"id":"turn","status":"inProgress","error":{"message":"restricted","codexErrorInfo":"cyberPolicy"}}}})).as_bytes()).await.unwrap();
        sw.write_all(format!("{}\n",json!({"id":3,"method":"item/tool/call","params":{"threadId":"thread","turnId":"turn","callId":"call-3","tool":"known","arguments":{"text":"intervention attempt"}}})).as_bytes()).await.unwrap();
        line.clear();
        sr.read_line(&mut line).await.unwrap();
        let reply: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(reply["result"]["success"], false);
        sw.write_all(format!("{}\n",json!({"method":"turn/completed","params":{"threadId":"thread","turn":{"id":"turn","status":"completed","items":[{"id":"public","type":"agentMessage","text":"finished output"}]}}})).as_bytes()).await.unwrap();
        sw.write_all(format!("{}\n", json!({"id":request["id"],"result":{}})).as_bytes())
            .await
            .unwrap();
    });
    let mut rpc = RpcClient::from_transport(cr, cw, Duration::from_secs(2)).unwrap();
    // Component adapter schema setup, not a production tool-grant capability.
    rpc.initialize().await.unwrap();
    rpc.allowed_tools.insert("known".into());
    let mut state = active();
    let mut broker = WireBroker {
        capture: capture(),
        records: Records::default(),
        dispatched: 0,
    };
    broker.observe_protocol(ProtocolObservation::ThreadEstablished { state: &state });
    rpc.request(
        "turn/interrupt",
        json!({"threadId":"thread","turnId":"turn"}),
        Some(&mut state),
        &mut broker,
    )
    .await
    .unwrap();
    task.await.unwrap();
    assert_eq!(broker.dispatched, 1);
    assert!(state.intervention.is_some());
    broker.observe_protocol(ProtocolObservation::AdapterInventory {
        state: &state,
        processed_requests: Some(rpc.observed_server_request_count()),
    });
    let report = broker
        .capture
        .finish(Some(&state), None, &|leaf| broker.records.read(leaf));
    assert_eq!(report.observed_server_requests, 3);
    assert_eq!(report.processed_server_requests, Some(3));
    assert!(
        !report
            .issues
            .contains(&ProtocolCaptureIssue::MissingRequestInventory)
    );
    assert!(
        report
            .issues
            .contains(&ProtocolCaptureIssue::UnobservedTransportTail)
    );
    assert_eq!(report.tool_request_attempts, 3);
    assert_eq!(report.state, ProtocolCaptureState::Incomplete);
    let raw = serde_json::to_string(&broker.records.0).unwrap();
    for text in [
        "attempt-1",
        "attempt-2",
        "intervention attempt",
        "finished output",
        "component_refusal",
        "unsupported_tool",
    ] {
        assert!(raw.contains(text), "retained {text}");
    }
}

#[test]
fn completed_item_and_delta_identity_require_exact_terminal_inventory() {
    for completed_item in [false, true] {
        let mut c = capture();
        let mut r = Records::default();
        let mut s = active();
        let params = if completed_item {
            json!({"threadId":"thread","turnId":"turn","item":{"id":"prior","type":"agentMessage","text":"completed earlier"}})
        } else {
            json!({"threadId":"thread","turnId":"turn","itemId":"prior","delta":"not retained as full output"})
        };
        let method = if completed_item {
            "item/completed"
        } else {
            "item/agentMessage/delta"
        };
        establish(&mut c, &mut r, &s);
        let accepted = s.notification(method, &params).is_ok();
        let event = if completed_item {
            ProtocolObservation::CompletedItem {
                state: &s,
                params: &params,
                accepted,
            }
        } else {
            ProtocolObservation::AgentMessageDelta {
                state: &s,
                params: &params,
                accepted,
            }
        };
        c.observe(event, &mut |v| r.write(v));
        terminal(&mut c, &mut r, &mut s, "a different terminal item");
        let report = c.finish(Some(&s), None, &|leaf| r.read(leaf));
        assert_eq!(report.state, ProtocolCaptureState::Conflict);
        assert!(
            report
                .issues
                .contains(&ProtocolCaptureIssue::ConflictingDuplicate)
        );
        assert!(
            !serde_json::to_string(&r.0)
                .unwrap()
                .contains("not retained as full output")
        );
    }
}
#[test]
fn scope_not_declared_never_records_raw_messages_or_claims_zero_attempts() {
    let mut c = capture();
    let mut r = Records::default();
    let s = active();
    c.disable(ProtocolCaptureIssue::ScopeNotDeclared);
    request(
        &mut c,
        &mut r,
        &s,
        &params("must not enter general worker journal"),
    );
    let report = c.finish(Some(&s), None, &|leaf| r.read(leaf));
    report.validate().unwrap();
    assert_eq!(report.state, ProtocolCaptureState::NotDeclared);
    assert!(r.0.is_empty());
    assert!(
        report
            .issues
            .contains(&ProtocolCaptureIssue::ScopeNotDeclared)
    );
}

#[test]
fn processed_request_inventory_detects_missing_prefix_but_never_proves_stream_end() {
    for (first_sequence, total) in [(1, Some(1)), (2, Some(2)), (1, None)] {
        let mut c = capture();
        let mut r = Records::default();
        let mut s = active();
        establish(&mut c, &mut r, &s);
        c.observe(
            ProtocolObservation::ServerRequest {
                state: Some(&s),
                rpc_id: &json!(1),
                sequence: first_sequence,
                method: "item/tool/call",
                params: &params("observed attempt"),
            },
            &mut |v| r.write(v),
        );
        c.observe(
            ProtocolObservation::ToolResult {
                state: Some(&s),
                rpc_id: &json!(1),
                error_code: Some("unsupported_tool"),
            },
            &mut |v| r.write(v),
        );
        terminal(&mut c, &mut r, &mut s, "full terminal text");
        c.observe(
            ProtocolObservation::AdapterInventory {
                state: &s,
                processed_requests: total,
            },
            &mut |v| r.write(v),
        );
        let report = c.finish(Some(&s), None, &|leaf| r.read(leaf));
        report.validate().unwrap();
        assert_eq!(report.state, ProtocolCaptureState::Incomplete);
        assert!(
            report
                .issues
                .contains(&ProtocolCaptureIssue::UnobservedTransportTail)
        );
        assert_eq!(
            report
                .issues
                .contains(&ProtocolCaptureIssue::MissingRequestInventory),
            first_sequence != 1 || total.is_none()
        );
        assert_eq!(report.observed_server_requests, 1);
        assert_eq!(report.processed_server_requests, total);
        let encoded = serde_json::to_value(&report).unwrap();
        let restored: DeclaredProtocolObservation =
            serde_json::from_value(encoded.clone()).unwrap();
        assert_eq!(serde_json::to_value(restored).unwrap(), encoded);
    }
}

#[path = "transport_observation_tests.rs"]
mod transport_tests;

// Actual JSON-RPC transport ordering only. This component sink is deliberately
// not a ScopedWorker, canary enrollment, declared trial or native capability.
mod output_callback_ordering {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    #[derive(Default)]
    struct AttemptBroker {
        requests: Vec<(u64, Value)>,
        sequences: Vec<u64>,
        order: Vec<(&'static str, u64)>,
    }
    impl ToolBroker for AttemptBroker {
        fn observe_protocol(&mut self, event: ProtocolObservation<'_>) {
            match event {
                ProtocolObservation::ServerRequest {
                    rpc_id,
                    sequence,
                    method,
                    params,
                    ..
                } => {
                    assert_eq!(method, "item/tool/call");
                    let id = rpc_id.as_u64().unwrap();
                    self.requests.push((id, params.clone()));
                    self.sequences.push(sequence);
                    self.order.push(("raw", id));
                }
                ProtocolObservation::ToolResult {
                    rpc_id, error_code, ..
                } => {
                    let id = rpc_id.as_u64().unwrap();
                    assert_eq!(
                        error_code,
                        Some(if id <= 2 {
                            "invalid_input"
                        } else {
                            "unsupported_tool"
                        })
                    );
                    self.order.push(("result", id));
                }
                _ => (),
            }
        }
        fn observe_session(&mut self, _: &SessionState) -> Result<()> {
            let id = self.requests.last().expect("raw request arrived first").0;
            self.order.push(("checkpoint", id));
            Ok(())
        }
        fn call<'a>(
            &'a mut self,
            call: &'a ToolCall,
        ) -> Pin<Box<dyn Future<Output = Result<Value>> + Send + 'a>> {
            let id = call.call_id.parse::<u64>().unwrap();
            assert_eq!(self.order.last(), Some(&("checkpoint", id)));
            let (observed_id, params) = self.requests.last().unwrap();
            assert_eq!(*observed_id, id);
            assert_eq!(params["arguments"], call.arguments);
            self.order.push(("dispatch", id));
            Box::pin(async move {
                // Explicit component object-shape refusal. The production
                // ScopedWorker::validate_call uses its own unchanged gate.
                assert!(!call.arguments.is_object());
                Err(Error::new(
                    "invalid_input",
                    "component requires object arguments",
                ))
            })
        }
    }
    async fn read_frame<R: tokio::io::AsyncBufRead + Unpin>(reader: &mut R) -> Value {
        let mut line = String::new();
        assert!(reader.read_line(&mut line).await.unwrap() > 0);
        serde_json::from_str(&line).unwrap()
    }
    async fn write_frame<W: tokio::io::AsyncWrite + Unpin>(writer: &mut W, value: Value) {
        writer
            .write_all(format!("{value}\n").as_bytes())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn actual_duplex_retains_root_arguments_before_parse_whitelist_and_intervention_refusal()
    {
        let (client, server) = tokio::io::duplex(16 * 1024);
        let (cr, cw) = tokio::io::split(client);
        let (sr, mut sw) = tokio::io::split(server);
        let string = json!("component root string");
        let array = json!(["component array text", {"nested": "component value"}]);
        let expected = [string.clone(), array.clone()];
        let server = async move {
            let mut sr = BufReader::new(sr);
            let initialize = read_frame(&mut sr).await;
            assert_eq!(initialize["method"], "initialize");
            write_frame(&mut sw, json!({"id":initialize["id"],"result":{"userAgent":"component","platformFamily":"unix","platformOs":"macos","codexHome":"/component/auth"}})).await;
            assert_eq!(read_frame(&mut sr).await["method"], "initialized");
            assert_eq!(read_frame(&mut sr).await["method"], "turn/interrupt");
            for (id, tool, arguments) in [
                (1, "known", string),
                (2, "known", array),
                (3, "unsupported", json!("unsupported root text")),
                (4, "known", json!(["blocked root text"])),
            ] {
                if id == 4 {
                    write_frame(&mut sw, json!({"method":"turn/started","params":{"threadId":"thread","turn":{"id":"turn","status":"inProgress","error":{"message":"restricted","codexErrorInfo":"cyberPolicy"}}}})).await;
                }
                write_frame(&mut sw, json!({"id":id,"method":"item/tool/call","params":{"threadId":"thread","turnId":"turn","callId":id.to_string(),"tool":tool,"arguments":arguments}})).await;
                let response = read_frame(&mut sr).await;
                assert_eq!(response["id"], id);
                assert_eq!(response["result"]["success"], false);
                let result: Value = serde_json::from_str(
                    response["result"]["contentItems"][0]["text"]
                        .as_str()
                        .unwrap(),
                )
                .unwrap();
                assert_eq!(
                    result["error"]["code"],
                    if id <= 2 {
                        "invalid_input"
                    } else {
                        "unsupported_tool"
                    }
                );
            }
            // Malformed metadata fails actual adapter parsing. Its arguments
            // must already have reached the raw callback, with no dispatch or
            // manufactured successful result event.
            write_frame(&mut sw, json!({"id":5,"method":"item/tool/call","params":{"threadId":"thread","turnId":"turn","tool":"known","arguments":"before malformed callId refusal"}})).await;
        };
        let client = async move {
            let mut rpc = RpcClient::from_transport(cr, cw, Duration::from_secs(2)).unwrap();
            rpc.initialize().await.unwrap();
            // Component adapter whitelist only; no grant or runtime admission.
            rpc.allowed_tools.insert("known".into());
            let mut broker = AttemptBroker::default();
            let mut state = active();
            let error = rpc
                .request(
                    "turn/interrupt",
                    json!({"threadId":"thread","turnId":"turn"}),
                    Some(&mut state),
                    &mut broker,
                )
                .await
                .unwrap_err();
            assert_eq!(error.code, "protocol_error");
            assert!(state.intervention.is_some());
            assert_eq!(rpc.observed_server_request_count(), 5);
            broker
        };
        // Both duplex halves/futures stay owned here and are dropped together
        // on timeout/panic. There is no detached task, process or cleanup claim.
        let (_, broker) = tokio::time::timeout(Duration::from_secs(4), async {
            tokio::join!(server, client)
        })
        .await
        .expect("bounded actual protocol exchange");
        assert_eq!(broker.sequences, [1, 2, 3, 4, 5]);
        assert_eq!(broker.requests.len(), 5);
        for (index, value) in expected.iter().enumerate() {
            assert_eq!(&broker.requests[index].1["arguments"], value);
        }
        assert_eq!(broker.requests[2].1["arguments"], "unsupported root text");
        assert_eq!(
            broker.requests[3].1["arguments"],
            json!(["blocked root text"])
        );
        assert_eq!(
            broker.requests[4].1["arguments"],
            "before malformed callId refusal"
        );
        let order: Vec<_> = broker
            .order
            .into_iter()
            .filter(|(kind, _)| *kind != "checkpoint")
            .collect();
        assert_eq!(
            order,
            [
                ("raw", 1),
                ("dispatch", 1),
                ("result", 1),
                ("raw", 2),
                ("dispatch", 2),
                ("result", 2),
                ("raw", 3),
                ("result", 3),
                ("raw", 4),
                ("result", 4),
                ("raw", 5),
            ]
        );
    }
}
