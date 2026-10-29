//! Native Codex repository-tool observations. These describe provider output;
//! the host still binds it to source state and decides whether it satisfies work.
use super::*;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NativeItemPhase {
    Started,
    Completed,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum NativeExecutionStatus {
    InProgress,
    Completed,
    Failed,
    Declined,
}

/// The pinned `commandExecution` item, retaining absent results as unknown.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NativeCommandExecution {
    pub id: String,
    pub command: String,
    pub cwd: String,
    pub status: NativeExecutionStatus,
    pub command_actions: Vec<Value>,
    pub aggregated_output: Option<String>,
    pub exit_code: Option<i32>,
    pub duration_ms: Option<i64>,
    pub process_id: Option<String>,
    #[serde(default = "agent_source")]
    pub source: String,
    pub plugin_id: Option<String>,
    pub script_path: Option<String>,
}

fn agent_source() -> String {
    "agent".into()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct NativeFileChange {
    pub path: String,
    pub kind: Value,
    pub diff: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct NativeFileChanges {
    pub id: String,
    pub status: NativeExecutionStatus,
    pub changes: Vec<NativeFileChange>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum NativeExecutionItem {
    CommandExecution(NativeCommandExecution),
    FileChange(NativeFileChanges),
}

/// A native tool item observed on the admitted thread and active turn.
/// Operation identity comes from the bound session, never provider text.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NativeExecutionObservation {
    pub operation_id: String,
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub phase: NativeItemPhase,
    pub item: NativeExecutionItem,
}

impl NativeExecutionObservation {
    pub(super) fn from_notification(
        state: &SessionState,
        method: &str,
        params: &Value,
    ) -> Result<Option<Self>> {
        let phase = match method {
            "item/started" => NativeItemPhase::Started,
            "item/completed" => NativeItemPhase::Completed,
            _ => return Ok(None),
        };
        let Some(item) = params.get("item") else {
            return Ok(None);
        };
        if !matches!(
            item.get("type").and_then(Value::as_str),
            Some("commandExecution" | "fileChange")
        ) {
            return Ok(None);
        }
        let thread_id = required_str(params, "threadId")?;
        let turn_id = required_str(params, "turnId")?;
        if state.thread_id.as_deref() != Some(thread_id)
            || (state.turn_id.as_deref() != Some(turn_id)
                && state.unresolved_turn.as_deref() != Some(turn_id))
            || state.completed_turns.contains_key(turn_id)
        {
            return Err(Error::new(
                "identity_mismatch",
                "native execution item is not for the bound active turn",
            ));
        }
        let item_id = required_str(item, "id")?.to_owned();
        let item: NativeExecutionItem = serde_json::from_value(item.clone())
            .map_err(|_| Error::new("protocol_error", "malformed native execution item"))?;
        let status = match &item {
            NativeExecutionItem::CommandExecution(command) => {
                if ![
                    "agent",
                    "userShell",
                    "unifiedExecStartup",
                    "unifiedExecInteraction",
                ]
                .contains(&command.source.as_str())
                {
                    return Err(Error::new(
                        "protocol_error",
                        "unknown native command source",
                    ));
                }
                command.status
            }
            NativeExecutionItem::FileChange(files) => {
                for change in &files.changes {
                    if !matches!(
                        change.kind.get("type").and_then(Value::as_str),
                        Some("add" | "delete" | "update")
                    ) {
                        return Err(Error::new("protocol_error", "unknown native file change"));
                    }
                }
                files.status
            }
        };
        if phase == NativeItemPhase::Completed && status == NativeExecutionStatus::InProgress {
            return Err(Error::new(
                "protocol_error",
                "completed native item retains an in-progress status",
            ));
        }
        Ok(Some(Self {
            operation_id: state.binding.operation_id.clone(),
            thread_id: thread_id.into(),
            turn_id: turn_id.into(),
            item_id,
            phase,
            item,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> SessionState {
        let mut state = SessionState::new(
            SessionBinding {
                schema_version: 1,
                account_id: "account".into(),
                work_identity: "work".into(),
                operation_id: "operation".into(),
                generation: "generation".into(),
                instruction_digest: "instructions".into(),
                isolation_digest: "scope".into(),
                profile: CodexProfile {
                    model: "model".into(),
                    reasoning: "high".into(),
                    speed: "standard".into(),
                },
                cwd: "/fixture/repository".into(),
            },
            3,
        )
        .unwrap();
        state.thread_id = Some("thread".into());
        state.turn_id = Some("turn".into());
        state.turns_started = 1;
        state
    }

    fn command(status: &str, exit: Option<i32>) -> Value {
        json!({"threadId":"thread","turnId":"turn","item":{
            "type":"commandExecution","id":"check","command":"project-check",
            "cwd":"/fixture/repository","commandActions":[],"status":status,
            "exitCode":exit,"aggregatedOutput":"actual output","durationMs":12,
            "processId":"terminal-1","source":"agent"
        }})
    }

    #[derive(Default)]
    struct Broker {
        events: Vec<NativeExecutionObservation>,
        refuse: bool,
    }
    impl ToolBroker for Broker {
        fn observe_execution(
            &mut self,
            _: &SessionState,
            event: &NativeExecutionObservation,
        ) -> Result<()> {
            if self.refuse {
                return Err(Error::new("source_changed", "fixture source changed"));
            }
            self.events.push(event.clone());
            Ok(())
        }
        fn call<'a>(
            &'a mut self,
            _: &'a ToolCall,
        ) -> Pin<Box<dyn Future<Output = Result<Value>> + Send + 'a>> {
            Box::pin(async { Err(Error::new("unsupported_tool", "fixture has no tools")) })
        }
    }

    #[tokio::test]
    async fn native_execution_transport_retains_actual_results_in_order() {
        let frames = [
            json!({"method":"item/started","params":command("inProgress",None)}),
            json!({"method":"item/completed","params":command("completed",Some(0))}),
            json!({"method":"item/completed","params":{"threadId":"thread","turnId":"turn","item":{
                "type":"fileChange","id":"edit","status":"completed","changes":[{
                    "path":"src/main.rs","kind":{"type":"update","move_path":null},"diff":"actual diff"
                }]
            }}}),
        ];
        let mut bytes = Vec::new();
        for frame in frames {
            bytes.extend(serde_json::to_vec(&frame).unwrap());
            bytes.push(b'\n');
        }
        let mut client = RpcClient::from_transport(
            std::io::Cursor::new(bytes),
            tokio::io::sink(),
            Duration::from_secs(1),
        )
        .unwrap();
        let mut state = state();
        let mut broker = Broker::default();
        for _ in 0..3 {
            client
                .pump(
                    &mut state,
                    &mut broker,
                    Duration::from_secs(1),
                    Duration::from_secs(1),
                )
                .await
                .unwrap();
        }
        assert_eq!(broker.events.len(), 3);
        assert_eq!(broker.events[0].phase, NativeItemPhase::Started);
        assert_eq!(broker.events[1].phase, NativeItemPhase::Completed);
        assert_eq!(broker.events[1].operation_id, "operation");
        assert_eq!(broker.events[1].thread_id, "thread");
        assert_eq!(broker.events[1].turn_id, "turn");
        assert_eq!(broker.events[1].item_id, "check");
        let NativeExecutionItem::CommandExecution(command) = &broker.events[1].item else {
            panic!("expected native command");
        };
        assert_eq!(command.exit_code, Some(0));
        assert_eq!(command.aggregated_output.as_deref(), Some("actual output"));
        assert_eq!(command.process_id.as_deref(), Some("terminal-1"));
        assert!(matches!(
            broker.events[2].item,
            NativeExecutionItem::FileChange(_)
        ));
    }

    #[test]
    fn native_execution_rejects_stale_identity_and_keeps_missing_results_unknown() {
        let mut state = state();
        let missing = NativeExecutionObservation::from_notification(
            &state,
            "item/completed",
            &command("completed", None),
        )
        .unwrap()
        .unwrap();
        let NativeExecutionItem::CommandExecution(command_item) = missing.item else {
            panic!("expected native command");
        };
        assert_eq!(command_item.exit_code, None);
        let mut stale = command("completed", Some(0));
        stale["turnId"] = json!("previous-turn");
        let mut broker = Broker::default();
        assert_eq!(
            client::checkpoint_notification(&mut state, &mut broker, "item/completed", &stale)
                .unwrap_err()
                .code,
            "identity_mismatch"
        );
        assert!(broker.events.is_empty());
        assert_eq!(
            client::checkpoint_notification(
                &mut state,
                &mut broker,
                "item/completed",
                &command("inProgress", Some(0))
            )
            .unwrap_err()
            .code,
            "protocol_error"
        );
        broker.refuse = true;
        assert_eq!(
            client::checkpoint_notification(
                &mut state,
                &mut broker,
                "item/completed",
                &command("completed", Some(0))
            )
            .unwrap_err()
            .code,
            "source_changed"
        );
        assert!(broker.events.is_empty());
    }
}
