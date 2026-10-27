//! Real ScopedWorker/Control/executor and protected journal components. Command
//! outcomes are injected; these tests grant no native qualification or checks.
use super::*;
use crate::worker::Journal;
use crate::worker::command::{CommandEntryReference, MAX_HISTORY_BYTES};
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::AtomicUsize;

fn request(f: &Fixture) -> ToolCall {
    let path = f
        .contract
        .isolation()
        .plan()
        .scratch
        .join("unfamiliar-command");
    std::fs::write(&path, b"#!/bin/sh\nexit 97\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    // The injected boundary must never invoke these bytes. The actual adapter
    // still observes the executable identity and literal argv/cwd.
    f.call(
        "command_run",
        json!({"argv":[path,"--literal","$(not-a-shell)"],"cwd":"."}),
    )
}
fn receipts(f: &Fixture) -> Vec<Receipt> {
    f.control
        .operations()
        .unwrap()
        .into_iter()
        .filter(|r| r.kind == "worker_command_run")
        .collect()
}
fn entries(f: &Fixture) -> Vec<(String, Value)> {
    f.worker
        .journal
        .dir
        .records()
        .unwrap()
        .into_iter()
        .filter(|name| name.starts_with("event-"))
        .map(|name| {
            let value = f.worker.journal.dir.read(&name).unwrap();
            (name, value)
        })
        .collect()
}
fn completed_record(f: &Fixture) -> CommandExecutionRecord {
    let all = entries(f);
    for (_, event) in &all {
        if event["kind"] != "command_entry" {
            continue;
        }
        let reference: CommandEntryReference =
            serde_json::from_value(event["record"].clone()).unwrap();
        let bytes: Vec<u8> = reference
            .chunks
            .iter()
            .flat_map(|leaf| {
                let event: Value = f.worker.journal.dir.read(leaf).unwrap();
                hex::decode(event["record"]["payload_hex"].as_str().unwrap()).unwrap()
            })
            .collect();
        assert_eq!(bytes.len() as u64, reference.bytes);
        assert_eq!(bytes_digest(&bytes), reference.digest);
        if let CommandJournalEntry::Completed { record, .. } =
            serde_json::from_slice(&bytes).unwrap()
        {
            return *record;
        }
    }
    panic!("actual worker did not persist Completed");
}
fn history(f: &Fixture, events: &[(String, Value)]) -> Result<()> {
    let admission = f.control.operation(f.permit.operation_id)?;
    let mut reader = crate::worker::command_history::CommandHistory::new(
        &admission,
        &f.contract.instructions().manifest,
        f.contract.tools().digest(),
    );
    for (leaf, event) in events {
        reader.observe(leaf, event)?;
    }
    reader.ensure_settled()
}
fn no_check_capability(f: &mut Fixture) {
    assert!(f.worker.take_verified_checks().is_empty());
    assert!(f.worker.take_failed_checks().is_empty());
    assert!(f.worker.check_records.is_empty());
}
#[tokio::test]
async fn unfamiliar_literal_command_without_checks_records_facts_and_replays_once() {
    let mut f = Fixture::without_checks(AgentRole::Implementation);
    assert!(f.input.workflow.front_matter.checks.local.is_empty());
    assert!(f.input.workflow.front_matter.checks.integrated.is_empty());
    let call = request(&f);
    let result = f.worker.call(&call).await.unwrap();
    let original = f
        .boundary
        .last_cancel
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .clone();
    assert!(original.exited().unwrap());
    assert_eq!(result["outcome"]["exit_code"], 0);
    assert_eq!(result["evidence_class"], "deterministic_fixture");
    assert!(result.get("successful").is_none());
    let record = completed_record(&f);
    assert_eq!(
        record.preparation.request.argv,
        call.arguments["argv"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a.as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    );
    assert_eq!(record.preparation.request.cwd, ".");
    assert_eq!(
        record.preparation.observation.cwd,
        f.contract.isolation().plan().cwd
    );
    let receipt = receipts(&f).pop().unwrap();
    assert_eq!(receipt.operation_id, record.preparation.invocation_id);
    assert_eq!(receipt.phase, ReceiptPhase::Acknowledged);
    assert_eq!(receipt.observed_result, Some(result.clone()));
    assert!(f.executions.active_operations().unwrap().is_empty());
    assert_eq!(f.worker.call(&call).await.unwrap(), result);
    assert!(Arc::ptr_eq(
        &original,
        f.boundary.last_cancel.lock().unwrap().as_ref().unwrap()
    ));
    assert_eq!(receipts(&f).len(), 1);
    let mut conflict = call.clone();
    conflict.arguments["argv"][1] = json!("changed");
    assert_eq!(
        f.worker.call(&conflict).await.unwrap_err().code,
        "request_conflict"
    );
    no_check_capability(&mut f);
    assert!(f.worker.observe().unwrap().checks.is_empty());
    let context = f.call("context_read", json!({"key":"command_history"}));
    assert_eq!(f.worker.call(&context).await.unwrap()["total"], 1);
    let events = entries(&f);
    history(&f, &events).unwrap();
    let missing = events
        .iter()
        .filter(|(_, v)| v["kind"] != "command_effect")
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        history(&f, &missing).unwrap_err().code,
        "reconciliation_required"
    );
    let mut corrupt = events.clone();
    let chunk = corrupt
        .iter_mut()
        .find(|(_, v)| v["kind"] == "command_chunk")
        .unwrap();
    chunk.1["record"]["index"] = json!(1);
    assert_eq!(history(&f, &corrupt).unwrap_err().code, "record_corrupt");
    let mut duplicate = events.clone();
    duplicate.push(
        events
            .iter()
            .find(|(_, v)| v["kind"] == "command_effect")
            .unwrap()
            .clone(),
    );
    assert_eq!(history(&f, &duplicate).unwrap_err().code, "record_corrupt");
    let mut substituted = events.clone();
    let reference = substituted
        .iter_mut()
        .find(|(_, v)| v["kind"] == "command_entry")
        .unwrap();
    reference.1["record"]["digest"] = json!(bytes_digest(b"substituted"));
    assert_eq!(
        history(&f, &substituted).unwrap_err().code,
        "record_corrupt"
    );
    let mut changed = events.clone();
    let effect = changed
        .iter_mut()
        .find(|(_, v)| v["kind"] == "command_effect")
        .unwrap();
    effect.1["record"]["receipt"]["observed_result"]["outcome"]["exit_code"] = json!(47);
    assert!(history(&f, &changed).is_err());
}
#[tokio::test]
async fn command_nonzero_is_observed_without_failed_check_or_repair_authority() {
    let mut f = Fixture::without_checks(AgentRole::Testing);
    f.boundary.fail.store(true, Ordering::SeqCst);
    let call = request(&f);
    assert_eq!(
        f.worker.call(&call).await.unwrap()["outcome"]["exit_code"],
        1
    );
    assert_eq!(receipts(&f)[0].phase, ReceiptPhase::Acknowledged);
    no_check_capability(&mut f);
    history(&f, &entries(&f)).unwrap();
}
struct WriteBoundary {
    original: Arc<Boundary>,
}
impl CommandBoundary for WriteBoundary {
    fn evidence_class(&self) -> ExecutionEvidenceClass {
        ExecutionEvidenceClass::DeterministicFixture
    }
    fn start(
        &self,
        isolation: &ValidatedIsolation,
        argv: &[String],
        timeout: StdDuration,
        limit: usize,
    ) -> Result<Box<dyn StartedCommand>> {
        let owned = self.original.start(isolation, argv, timeout, limit)?;
        std::fs::write(
            isolation.plan().repository.join("a.txt"),
            b"actual injected command edit\n",
        )
        .unwrap();
        Ok(owned)
    }
}
fn boundary(f: &mut Fixture, boundary: Arc<dyn CommandBoundary>) {
    let original = f.execution.as_ref().unwrap();
    f.worker.execution = Some(Arc::new(WorkerExecution {
        git: original.git.clone(),
        git_executable: original.git_executable.clone(),
        boundary,
        ownership: original.ownership.clone(),
        binding: original.binding.clone(),
        limits: original.limits.clone(),
        check_outputs: original.check_outputs.clone(),
    }));
}
#[tokio::test]
async fn command_reports_changed_authorized_source_without_excluding_outputs() {
    let mut f = Fixture::without_checks(AgentRole::Implementation);
    let writer = Arc::new(WriteBoundary {
        original: f.boundary.clone(),
    });
    boundary(&mut f, writer);
    let call = request(&f);
    let value = f.worker.call(&call).await.unwrap();
    assert_ne!(value["before_digest"], value["after_digest"]);
    let record = completed_record(&f);
    assert_ne!(
        record.preparation.before.files["a.txt"],
        record.after.files["a.txt"]
    );
    no_check_capability(&mut f);
}
#[tokio::test]
async fn command_refuses_parent_coordination_independent_and_narrowed_permissions() {
    for role in [AgentRole::Coordination, AgentRole::IndependentReview] {
        let mut f = Fixture::without_checks(role);
        let call = request(&f);
        assert_eq!(
            f.worker.call(&call).await.unwrap_err().code,
            "unsupported_tool"
        );
        assert!(receipts(&f).is_empty());
        assert!(f.boundary.last_cancel.lock().unwrap().is_none());
    }
    assert!(
        operation_tool_definitions(AgentRole::Implementation, true, false)
            .iter()
            .all(|(tool, _)| tool.name != "command_run")
    );
    assert!(
        operation_tool_definitions(AgentRole::Validation, false, true)
            .iter()
            .any(|(tool, _)| tool.name == "command_run")
    );
    let mut f = Fixture::without_checks(AgentRole::Implementation);
    // An instruction permission loss cannot be hidden behind a still-present
    // tool name/domain; this private mutation is a negative only.
    Arc::make_mut(&mut f.worker.contract)
        .assembled
        .manifest
        .permissions
        .repository_code_execution = false;
    let call = request(&f);
    assert!(f.worker.call(&call).await.is_err());
    assert!(receipts(&f).is_empty());
    assert!(f.boundary.last_cancel.lock().unwrap().is_none());
}
#[tokio::test]
async fn parent_validation_command_retains_read_only_candidate_role() {
    let mut f = Fixture::without_checks(AgentRole::Validation);
    assert!(f.contract.isolation().plan().source_writes.is_empty());
    assert!(f.contract.authority.work().unwrap().child.is_none());
    let call = request(&f);
    f.worker.call(&call).await.unwrap();
    assert_eq!(receipts(&f)[0].phase, ReceiptPhase::Acknowledged);
    no_check_capability(&mut f);
}
struct RefuseBoundary;
impl CommandBoundary for RefuseBoundary {
    fn evidence_class(&self) -> ExecutionEvidenceClass {
        ExecutionEvidenceClass::DeterministicFixture
    }
    fn preflight(&self, _: &ValidatedIsolation) -> Result<()> {
        Err(Error::new(
            "managed_profile_unqualified",
            "injected known pre-spawn refusal",
        ))
    }
    fn start(
        &self,
        _: &ValidatedIsolation,
        _: &[String],
        _: StdDuration,
        _: usize,
    ) -> Result<Box<dyn StartedCommand>> {
        panic!("refused preflight must not dispatch")
    }
}
#[tokio::test]
async fn known_preflight_refusal_records_not_started_without_effect_or_registry() {
    let mut f = Fixture::without_checks(AgentRole::Implementation);
    boundary(&mut f, Arc::new(RefuseBoundary));
    let call = request(&f);
    assert_eq!(
        f.worker.call(&call).await.unwrap_err().code,
        "managed_profile_unqualified"
    );
    assert!(receipts(&f).is_empty());
    assert!(f.executions.active_operations().unwrap().is_empty());
    history(&f, &entries(&f)).unwrap();
}
#[tokio::test]
async fn pending_correction_and_revoked_owner_refuse_command_before_dispatch() {
    let mut f = Fixture::without_checks(AgentRole::Implementation);
    let call = request(&f);
    f.host.waiting.store(true, Ordering::SeqCst);
    assert_eq!(
        f.worker.call(&call).await.unwrap_err().code,
        "correction_classification_pending"
    );
    f.host.waiting.store(false, Ordering::SeqCst);
    f.host.owner.store(false, Ordering::SeqCst);
    assert_eq!(
        f.worker.call(&call).await.unwrap_err().code,
        "ownership_conflict"
    );
    assert!(receipts(&f).is_empty());
    assert!(f.boundary.last_cancel.lock().unwrap().is_none());
}
#[tokio::test]
async fn unknown_command_retains_original_effect_registry_and_unsettled_history() {
    let mut f = Fixture::without_checks(AgentRole::Implementation);
    // Preserve owned component evidence even if any later assertion fails.
    let retained = std::mem::replace(&mut f._temp, TempDir::new().unwrap()).keep();
    eprintln!(
        "retained generic command uncertainty: {}",
        retained.display()
    );
    f.boundary.unknown.store(true, Ordering::SeqCst);
    let call = request(&f);
    assert_eq!(
        f.worker.call(&call).await.unwrap_err().code,
        "operation_uncertain"
    );
    assert_eq!(receipts(&f)[0].phase, ReceiptPhase::Sent);
    assert_eq!(
        f.executions.observe_exited(f.permit.operation_id).unwrap(),
        0
    );
    assert!(
        f.executions
            .active_operations()
            .unwrap()
            .contains(&f.permit.operation_id)
    );
    let mut uncertain = entries(&f);
    assert_eq!(
        history(&f, &uncertain).unwrap_err().code,
        "reconciliation_required"
    );
    // Repeating a retained original entry cannot turn uncertainty into closure.
    uncertain.extend(
        entries(&f).into_iter().filter(|(_, event)| {
            event["kind"] == "command_chunk" || event["kind"] == "command_entry"
        }),
    );
    assert_eq!(history(&f, &uncertain).unwrap_err().code, "record_corrupt");
    let prior = f
        .boundary
        .last_cancel
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .clone();
    assert!(f.worker.call(&call).await.is_err());
    assert!(Arc::ptr_eq(
        &prior,
        f.boundary.last_cancel.lock().unwrap().as_ref().unwrap()
    ));
    assert_eq!(receipts(&f).len(), 1);
    no_check_capability(&mut f);
}
#[tokio::test]
async fn fenced_command_cancels_original_owner_but_does_not_claim_current_completion() {
    let mut f = Fixture::without_checks(AgentRole::Implementation);
    let retained = std::mem::replace(&mut f._temp, TempDir::new().unwrap()).keep();
    eprintln!("retained fenced command history: {}", retained.display());
    f.boundary.block.store(true, Ordering::SeqCst);
    let call = request(&f);
    let registry = f.executions.clone();
    let control = f.control.clone();
    let operation = f.permit.operation_id;
    let boundary = f.boundary.clone();
    let task = tokio::spawn(async move {
        let result = f.worker.call(&call).await;
        (f, result)
    });
    tokio::time::timeout(StdDuration::from_secs(10), async {
        while boundary.last_cancel.lock().unwrap().is_none() {
            tokio::time::sleep(StdDuration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    control.fence(operation).unwrap();
    registry.cancel_fenced(&control, operation).unwrap();
    let (mut f, result) = tokio::time::timeout(StdDuration::from_secs(10), task)
        .await
        .unwrap()
        .unwrap();
    assert!(result.is_err());
    assert!(
        boundary
            .last_cancel
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .exited()
            .unwrap()
    );
    assert_eq!(registry.observe_exited(operation).unwrap(), 0);
    assert!(registry.active_operations().unwrap().contains(&operation));
    assert!(history(&f, &entries(&f)).is_err());
    no_check_capability(&mut f);
}
fn journal(root: &PrivateDir) -> Journal {
    let dir = root.child("journal").unwrap();
    let lock = dir.lock("worker.lock").unwrap();
    Journal {
        dir,
        _lock: lock,
        next: Mutex::new(0),
        bytes: AtomicUsize::new(0),
        failed: AtomicBool::new(false),
    }
}
fn large_entry(invocation_id: Uuid) -> CommandJournalEntry {
    CommandJournalEntry::Uncertain {
        invocation_id,
        operation_id: Uuid::new_v4(),
        preparation_digest: bytes_digest(b"component"),
        error: Error::new("component", "actual protected data test")
            .detail(json!({"data":"x".repeat(20_000)})),
        outcome: None,
    }
}
#[test]
fn command_chunks_share_total_journal_budget_and_reserve_before_any_write() {
    let temp = TempDir::new().unwrap();
    let root = PrivateDir::create(&temp.path().canonicalize().unwrap().join("owned")).unwrap();
    let journal = journal(&root);
    journal
        .append("session", &json!({"component":"ordinary event"}))
        .unwrap();
    assert!(journal.bytes.load(Ordering::SeqCst) > 0);
    journal
        .bytes
        .store(MAX_HISTORY_BYTES - 100, Ordering::SeqCst);
    let before = journal.dir.records().unwrap();
    let id = Uuid::new_v4();
    assert_eq!(
        journal
            .append_command(id, &large_entry(id))
            .unwrap_err()
            .code,
        "journal_limit"
    );
    assert_eq!(journal.dir.records().unwrap(), before);
    assert_eq!(*journal.next.lock().unwrap(), 1);
}
#[test]
fn partial_command_chunk_write_retains_original_prefix_and_poisons_later_appends() {
    let temp = TempDir::new().unwrap();
    let root = PrivateDir::create(&temp.path().canonicalize().unwrap().join("owned")).unwrap();
    let journal = journal(&root);
    std::fs::create_dir(journal.dir.path().join("event-00000001.json")).unwrap();
    let id = Uuid::new_v4();
    assert!(journal.append_command(id, &large_entry(id)).is_err());
    let original = journal
        .dir
        .read_bytes("event-00000000.json", 256 * 1024)
        .unwrap();
    assert_eq!(
        journal
            .append("session", &json!({"component":"must not refill"}))
            .unwrap_err()
            .code,
        "journal_failed"
    );
    assert_eq!(
        journal
            .dir
            .read_bytes("event-00000000.json", 256 * 1024)
            .unwrap(),
        original
    );
    assert!(journal.bytes.load(Ordering::SeqCst) > original.len());
}

#[tokio::test]
async fn command_history_rejects_rewritten_authority_deadline_and_backend_schema() {
    let mut f = Fixture::without_checks(AgentRole::Implementation);
    let call = request(&f);
    f.worker.call(&call).await.unwrap();
    let actual = completed_record(&f).preparation;
    let prefix = |preparation: CommandPreparationRecord| {
        let temp = TempDir::new().unwrap();
        let root = PrivateDir::create(&temp.path().canonicalize().unwrap().join("owned")).unwrap();
        let journal = journal(&root);
        let invocation_id = preparation.invocation_id;
        let entry = CommandJournalEntry::Prepared {
            invocation_id,
            operation_id: preparation.original_binding.operation_id,
            preparation_digest: preparation.digest().unwrap(),
            preparation: Box::new(preparation),
        };
        journal.append_command(invocation_id, &entry).unwrap();
        let events = journal
            .dir
            .records()
            .unwrap()
            .into_iter()
            .map(|leaf| {
                let value = journal.dir.read(&leaf).unwrap();
                (leaf, value)
            })
            .collect::<Vec<_>>();
        history(&f, &events).unwrap_err().code
    };
    // Actual producer bytes decode; a preparation alone is not proof of closure.
    assert_eq!(prefix(actual.clone()), "reconciliation_required");
    let mut changed = actual.clone();
    changed.deadline_at += Duration::milliseconds(1);
    assert_eq!(prefix(changed), "record_corrupt");
    let mut changed = actual.clone();
    changed.observation.backend_policy_digest = changed
        .observation
        .backend_policy_digest
        .replace("sha256:v5:", "sha256:v1:");
    assert_eq!(prefix(changed), "record_corrupt");

    // Synthetic descriptive correction shape only; this cannot mint or restore
    // a verified transition, admit a command or satisfy a validation obligation.
    let mut corrected = actual;
    corrected.binding.work.generation.correction = bytes_digest(b"later correction");
    corrected.correction = Some(CorrectionExecutionProvenance {
        original_binding_digest: canonical_digest(1, &corrected.original_binding).unwrap(),
        transition_digest: bytes_digest(b"component transition shape"),
        watermark: 1,
        input_digest: bytes_digest(b"component input shape"),
    });
    assert_eq!(prefix(corrected.clone()), "reconciliation_required");
    for field in ["authorization", "definition", "scope"] {
        let mut changed = corrected.clone();
        let generation = &mut changed.binding.work.generation;
        let destination = match field {
            "authorization" => &mut generation.authorization,
            "definition" => &mut generation.definition,
            _ => &mut generation.scope,
        };
        *destination = bytes_digest(b"broadened unrelated generation");
        assert_eq!(prefix(changed), "record_corrupt", "{field}");
    }
}
