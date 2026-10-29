//! A protected, already retired predecessor observed after a coordinator restart.
//! This never constructs a joined RunCompletion or adopts a process from its PID.
use super::*;
use kegari_execution::journal::RecoveryState;
use kegari_storage::PrivateDir;

#[derive(Clone)]
pub(crate) enum ResumePredecessor {
    Joined(Arc<RunCompletion>),
    Cold(Arc<ColdResumeSession>),
}
impl ResumePredecessor {
    pub(crate) fn operation_id(&self) -> Uuid {
        match self {
            Self::Joined(run) => run.operation_id,
            Self::Cold(run) => run.execution.binding.operation_id,
        }
    }
    pub(crate) fn state(&self) -> &SessionState {
        match self {
            Self::Joined(run) => &run.state,
            Self::Cold(run) => run.retained.state(),
        }
    }
    pub(crate) fn contract(&self) -> Result<&Arc<WorkerContract>> {
        match self {
            Self::Joined(run) => run
                .contract
                .as_ref()
                .ok_or_else(|| cold_error("original contract is missing")),
            Self::Cold(run) => Ok(&run.contract),
        }
    }
    pub(crate) fn execution(&self) -> Result<&Arc<WorkerExecution>> {
        match self {
            Self::Joined(run) => run
                .execution
                .as_ref()
                .ok_or_else(|| cold_error("original execution is missing")),
            Self::Cold(run) => Ok(&run.execution),
        }
    }
    pub(crate) fn verify(&self, joined: Option<&Arc<RunCompletion>>) -> Result<()> {
        match self {
            Self::Joined(expected)
                if joined.is_some_and(|actual| Arc::ptr_eq(expected, actual)) =>
            {
                Ok(())
            }
            Self::Joined(_) => Err(cold_error("actual joined predecessor changed")),
            Self::Cold(cold) if joined.is_none() => cold.revalidate(),
            Self::Cold(_) => Err(cold_error(
                "cold predecessor conflicts with a registered completion",
            )),
        }
    }
}

pub(crate) struct ColdResumeSession {
    retained: RetainedResumeSession,
    contract: Arc<WorkerContract>,
    execution: Arc<WorkerExecution>,
    records: PathBuf,
    control: Arc<ControlHost>,
    processes: Arc<crate::process_records::ProcessJournal>,
    process_digest: String,
}
impl ColdResumeSession {
    pub(crate) fn observe(
        retained: RetainedResumeSession,
        contract: Arc<WorkerContract>,
        execution: Arc<WorkerExecution>,
        records: PathBuf,
        control: Arc<ControlHost>,
        processes: Arc<crate::process_records::ProcessJournal>,
    ) -> Result<Arc<Self>> {
        let process_digest = canonical_digest(
            1,
            &processes.retained_records(execution.binding.operation_id)?,
        )?;
        let result = Arc::new(Self {
            retained,
            contract,
            execution,
            records,
            control,
            processes,
            process_digest,
        });
        result.revalidate()?;
        Ok(result)
    }
    pub(crate) fn retained(&self) -> &RetainedResumeSession {
        &self.retained
    }
    pub(crate) fn revalidate(&self) -> Result<()> {
        let operation = self.execution.binding.operation_id;
        let receipt = self.control.operation(operation)?;
        require_closed_receipt(&receipt)?;
        if canonical_digest(1, &receipt)? != self.retained.admission_digest()
            || self.contract.instructions().manifest.operation_id != operation
            || self.contract.instructions().manifest_digest
                != self.retained.reference().instruction_digest
            || self.contract.work_key()? != self.retained.reference().phase.work
            || self.contract.instructions().manifest.asset_version
                != kegari_context::instructions::ASSET_VERSION
            || self
                .contract
                .workflow()
                .front_matter
                .hooks
                .before_run
                .is_some()
            || self
                .contract
                .workflow()
                .front_matter
                .hooks
                .after_run
                .is_some()
            || !matches!(
                self.contract.instructions().manifest.role,
                AgentRole::Implementation | AgentRole::Documentation | AgentRole::Testing
            )
        {
            return Err(cold_error(
                "cold session changed its original contract or unsupported lifecycle",
            ));
        }
        self.execution
            .ownership
            .verify(&self.execution.binding, self.contract.isolation())?;
        let records = PrivateDir::open(&self.records)?;
        let observed = crate::worker::recover_session_with_validation(
            &records,
            &receipt,
            self.retained.validation(),
        )?
        .ok_or_else(|| cold_error("protected final session is missing"))?
        .resume_candidate()?;
        if observed.digest() != self.retained.digest()
            || receipt
                .observed_result
                .as_ref()
                .and_then(|v| v.pointer("/observation/session"))
                != Some(
                    &serde_json::to_value(observed.state())
                        .map_err(|_| Error::invalid("encode retained session"))?,
                )
        {
            return Err(cold_error(
                "cold session differs from its final protected observation",
            ));
        }
        observed.verify_prior_receipts(&self.control.operations()?)?;
        verify_native_history(&records, operation, observed.state())?;
        let process_records = self.processes.retained_records(operation)?;
        let processes = self.processes.observe(operation)?;
        if canonical_digest(1, &process_records)? != self.process_digest
            || processes.is_empty()
            || !processes
                .iter()
                .any(|p| p.binding.kind == "codex-app-server")
            || processes.len() != process_records.len()
            || processes.iter().any(|p| {
                p.state != RecoveryState::OwnedGroupExitRecorded
                    || !p.owned_group_exited
                    || p.operation_id != operation
                    || p.binding.work_key != self.retained.reference().phase.work
                    || p.binding.scope_digest != self.contract.scope_digest()
            })
        {
            return Err(cold_error(
                "original host-process exit is unrecorded, changed or still held",
            ));
        }
        Ok(())
    }
}

pub(crate) fn require_closed_receipt(receipt: &crate::control::Receipt) -> Result<()> {
    let value = receipt
        .observed_result
        .as_ref()
        .ok_or_else(|| cold_error("cold run has no terminal observation"))?;
    if receipt.phase != crate::control::ReceiptPhase::Reconciled
        || value["local_execution"] != "exited"
        || value["unresolved_effects"]
            .as_array()
            .is_none_or(|effects| !effects.is_empty())
        || value
            .pointer("/observation/outcome")
            .and_then(Value::as_str)
            != Some("failed")
        || value
            .pointer("/observation/error/code")
            .and_then(Value::as_str)
            != Some("turn_incomplete")
        || value.pointer("/observation/local_exit_confirmed") != Some(&Value::Bool(true))
        || value
            .pointer("/observation/hook_error")
            .is_none_or(|v| !v.is_null())
        || value
            .pointer("/observation/validation_error")
            .is_some_and(|v| !v.is_null())
        || value
            .pointer("/observation/validation_plan_digest")
            .is_some_and(|v| !v.is_null())
            && value
                .pointer("/observation/validation_checkpoint")
                .and_then(Value::as_str)
                .is_none()
        || [
            "verified_check_count",
            "failed_check_count",
            "validation_attempt_count",
        ]
        .iter()
        .any(|key| value["observation"][key].as_u64().unwrap_or(1) != 0)
            && value
                .pointer("/observation/validation_checkpoint")
                .and_then(Value::as_str)
                .is_none()
        || [
            "review_round",
            "source_advanced",
            "continuation",
            "parent_reassembly",
        ]
        .iter()
        .any(|key| receipt.expected.contains_key(*key))
    {
        return Err(cold_error(
            "cold resume requires a retired ordinary incomplete session without effects or uncheckpointed validation",
        ));
    }
    Ok(())
}
pub(crate) fn verify_native_history(
    records: &PrivateDir,
    operation: Uuid,
    state: &SessionState,
) -> Result<()> {
    use kegari_codex::{
        NativeExecutionItem, NativeExecutionObservation, NativeExecutionStatus, NativeItemPhase,
    };
    let journal = PrivateDir::open(&records.path().join("journal"))?;
    let mut active = BTreeMap::new();
    let mut completed = std::collections::BTreeSet::new();
    for file in journal.records_bounded(65_537)? {
        if file == "binding.json" {
            continue;
        }
        let value: Value = journal.read(&file)?;
        if value["kind"] != "codex_execution_observed" {
            continue;
        }
        let mut data = value["record"].clone();
        data.as_object_mut()
            .ok_or_else(|| cold_error("native observation is malformed"))?
            .remove("observation_digest");
        let event: NativeExecutionObservation = serde_json::from_value(data)
            .map_err(|_| cold_error("native observation is malformed"))?;
        if event.operation_id != operation.to_string()
            || Some(event.thread_id.as_str()) != state.thread_id.as_deref()
        {
            return Err(cold_error("native history changed session identity"));
        }
        let key = (event.thread_id, event.turn_id, event.item_id);
        match event.phase {
            NativeItemPhase::Started => {
                if completed.contains(&key) || active.insert(key, event.item).is_some() {
                    return Err(cold_error("native item identity repeats"));
                }
            }
            NativeItemPhase::Completed => {
                let prior = active
                    .remove(&key)
                    .ok_or_else(|| cold_error("native completion has no original start"))?;
                let settled = match (&prior, &event.item) {
                    (
                        NativeExecutionItem::CommandExecution(a),
                        NativeExecutionItem::CommandExecution(b),
                    ) => {
                        a.command == b.command
                            && a.cwd == b.cwd
                            && (b.status == NativeExecutionStatus::Declined
                                || matches!(
                                    b.status,
                                    NativeExecutionStatus::Completed
                                        | NativeExecutionStatus::Failed
                                ) && b.exit_code.is_some())
                    }
                    (NativeExecutionItem::FileChange(_), NativeExecutionItem::FileChange(b)) => {
                        b.status != NativeExecutionStatus::InProgress
                    }
                    _ => false,
                };
                if !settled || !completed.insert(key) {
                    return Err(cold_error("native execution outcome is unknown or changed"));
                }
            }
        }
    }
    if !active.is_empty() {
        return Err(cold_error("native commands or edits remain unsettled"));
    }
    Ok(())
}
fn cold_error(message: &str) -> Error {
    Error::new("resume_incompatible", message)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cold_resume_requires_retired_receipt_and_settled_native_history() {
        let corpus: Value = serde_json::from_str(include_str!(
            "../../kegari-codex/tests/fixtures/codex-final-sessions.json"
        ))
        .unwrap();
        let state = SessionState::restore(
            corpus["cases"]["two_prefix"]["serialized"]
                .as_str()
                .unwrap()
                .as_bytes(),
        )
        .unwrap();
        let operation: Uuid = state.binding.operation_id.parse().unwrap();
        let receipt = crate::control::Receipt {
            operation_id: operation,
            request_id: operation,
            runtime_id: Uuid::from_u128(1),
            owner_generation: 1,
            actor: "fixture".into(),
            kind: "admission".into(),
            canonical_targets: vec![state.binding.work_identity.clone()],
            payload_digest: canonical_digest(1, &"fixture").unwrap(),
            expected: BTreeMap::new(),
            phase: crate::control::ReceiptPhase::Reconciled,
            remote_ids: BTreeMap::new(),
            observed_result: Some(
                json!({"local_execution":"exited","unresolved_effects":[],"observation":{
                    "outcome":"failed","error":{"code":"turn_incomplete"},"local_exit_confirmed":true,
                    "hook_error":null,"validation_error":null,"validation_plan_digest":null,
                    "verified_check_count":0,"failed_check_count":0,"validation_attempt_count":0
                }}),
            ),
            cancellation: "none".into(),
            updated_at: chrono::Utc::now(),
        };
        require_closed_receipt(&receipt).unwrap();
        for pointer in [
            "/unresolved_effects",
            "/observation/local_exit_confirmed",
            "/observation/validation_attempt_count",
        ] {
            let mut changed = receipt.clone();
            *changed
                .observed_result
                .as_mut()
                .unwrap()
                .pointer_mut(pointer)
                .unwrap() = match pointer {
                "/unresolved_effects" => json!(["uncertain-effect"]),
                "/observation/local_exit_confirmed" => json!(false),
                _ => json!(1),
            };
            assert_eq!(
                require_closed_receipt(&changed).unwrap_err().code,
                "resume_incompatible"
            );
        }
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let records = PrivateDir::create(&root.join("records")).unwrap();
        let journal = records.child("journal").unwrap();
        let event = |phase: &str, status: &str, exit: Value| {
            json!({"kind":"codex_execution_observed","record":{
                "operation_id":operation.to_string(), "thread_id":state.thread_id.as_ref().unwrap(),
                "turn_id":"turn-fixture", "item_id":"command-fixture", "phase":phase,
                "observation_digest":canonical_digest(1,&"recorded result").unwrap(),
                "item":{"type":"commandExecution", "id":"command-fixture", "command":"project-check", "cwd":state.binding.cwd,
                    "status":status,"commandActions":[],"aggregatedOutput":null,"exitCode":exit,
                    "durationMs":null,"processId":null,"source":"agent","pluginId":null,"scriptPath":null}
            }})
        };
        journal
            .write(
                "event-00000000.json",
                &event("started", "inProgress", Value::Null),
            )
            .unwrap();
        assert_eq!(
            verify_native_history(&records, operation, &state)
                .unwrap_err()
                .code,
            "resume_incompatible"
        );
        journal
            .write(
                "event-00000001.json",
                &event("completed", "completed", Value::Null),
            )
            .unwrap();
        assert!(verify_native_history(&records, operation, &state).is_err());
        journal
            .write(
                "event-00000001.json",
                &event("completed", "completed", json!(0)),
            )
            .unwrap();
        verify_native_history(&records, operation, &state).unwrap();
        let mut wrong = event("completed", "completed", json!(0));
        wrong["record"]["item"]["command"] = json!("different-check");
        journal.write("event-00000001.json", &wrong).unwrap();
        assert!(verify_native_history(&records, operation, &state).is_err());
    }
}
