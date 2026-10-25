//! A new child session needs a concrete unmet obligation and actual progress.
//! These capabilities are never reconstructed from serialized worker summaries.
use super::*;
use crate::{
    control::{ReceiptPhase, RunReference},
    execution::CheckExecutionRecord,
    instructions::AgentRole,
    runtime::VerifiedAuthorizedOutcome,
    scheduler::PhaseKey,
    worker_run::RunCompletion,
};

#[derive(Clone)]
pub struct UnfinishedChildWork {
    inner: Arc<UnfinishedProof>,
}
struct UnfinishedProof {
    original: Arc<CompletedChildClaim>,
    _failure: Arc<RunCompletion>,
    authority: FreshGate,
    source: DeliverySource,
    reference: RunReference,
    predecessor_receipt: Receipt,
    predecessor_receipt_digest: String,
    failure_operation: Uuid,
    failure_receipt_digest: String,
    failure_phase: RunReference,
    failure_phase_inventory_digest: String,
    remaining_task: Value,
    progress_digest: String,
    digest: String,
    role: AgentRole,
    contract_digest: String,
    review_round: bool,
    review_rounds_consumed: u32,
}
impl UnfinishedChildWork {
    pub fn predecessor(&self) -> Uuid {
        self.inner.original.operation_id()
    }
    pub fn phase(&self) -> &PhaseKey {
        &self.inner.reference.phase
    }
    pub fn authority(&self) -> &FreshGate {
        &self.inner.authority
    }
    pub fn source(&self) -> &DeliverySource {
        &self.inner.source
    }
    pub fn remaining_task(&self) -> &Value {
        &self.inner.remaining_task
    }
    pub fn digest(&self) -> &str {
        &self.inner.digest
    }
    pub fn progress_digest(&self) -> &str {
        &self.inner.progress_digest
    }
    pub fn failure_operation(&self) -> Uuid {
        self.inner.failure_operation
    }
    pub fn predecessor_receipt_digest(&self) -> &str {
        &self.inner.predecessor_receipt_digest
    }
    pub fn predecessor_receipt(&self) -> &Receipt {
        &self.inner.predecessor_receipt
    }
    pub fn failure_receipt_digest(&self) -> &str {
        &self.inner.failure_receipt_digest
    }
    pub fn failure_phase_inventory_digest(&self) -> &str {
        &self.inner.failure_phase_inventory_digest
    }
    pub fn verify_failure_inventory(&self, receipts: &[Receipt]) -> Result<()> {
        if phase_inventory(
            receipts,
            self.inner.predecessor_receipt.runtime_id,
            &self.inner.failure_phase,
        )? != self.inner.failure_phase_inventory_digest
        {
            return Err(Error::new(
                "stale_result",
                "another validation or review admission superseded unfinished evidence",
            ));
        }
        Ok(())
    }
    pub fn reference(&self) -> &RunReference {
        &self.inner.reference
    }
    pub fn review_rounds_consumed(&self) -> u32 {
        self.inner.review_rounds_consumed
    }
    pub fn verify_contract(&self, contract: &crate::worker::WorkerContract) -> Result<()> {
        self.verify_contract_with_task(self.remaining_task(), contract)
    }
    fn verify_contract_with_task(
        &self,
        task: &Value,
        contract: &crate::worker::WorkerContract,
    ) -> Result<()> {
        let crate::worker::WorkerAuthority::Authorized(gate) = contract.authority() else {
            return Err(Error::new(
                "worker_binding",
                "repair cannot use independent review authority",
            ));
        };
        let manifest = &contract.instructions().manifest;
        assignment::verify_assignment_contract(
            contract,
            self.authority(),
            &self.phase().phase,
            task,
        )?;
        if gate.digest()? != self.authority().digest()?
            || manifest.role != self.role()
            || manifest.phase != self.phase().phase
            || manifest.repository_revision.as_deref()
                != Some(self.source().revision().commit.as_str())
            || manifest.workflow_digest != self.authority().workflow_digest()
            || manifest.work_generation != self.authority().work().digest()?
            || manifest.correction_revision != self.authority().work().generation.correction
            || manifest.workspace_id != self.authority().workspace_id()
            || canonical_digest(1, &manifest.profile)?
                != canonical_digest(1, self.authority().profile())?
            || gate
                .child_scope()
                .and_then(|scope| scope.write_repository.as_deref())
                != Some(self.source().repository().id.as_str())
            || manifest.repository_root.as_deref()
                != Some(contract.isolation().plan().repository.as_path())
        {
            return Err(Error::new(
                "worker_binding",
                "assembled repair contract differs from sealed remaining obligation",
            ));
        }
        Ok(())
    }
    pub fn verify_assignment(&self, assignment: &DeliveryWorkerAssignment) -> Result<()> {
        if assignment.authority().digest()? != self.authority().digest()?
            || assignment.phase() != self.phase().phase
            || assignment.role() != self.role()
            || assignment.context() != self.remaining_task()
            || assignment
                .source()
                .map(DeliverySource::digest)
                .transpose()?
                != Some(self.source().digest()?)
            || !assignment.read_sources().is_empty()
            || assignment.candidate_vector_digest().is_some()
            || assignment.contract_digest() != self.contract_digest()
        {
            return Err(Error::new(
                "worker_binding",
                "repair assignment differs from sealed unfinished obligation",
            ));
        }
        Ok(())
    }
    pub fn consumes_review_round(&self) -> bool {
        self.inner.review_round
    }
    pub(super) fn role(&self) -> AgentRole {
        self.inner.role
    }
    pub(super) fn contract_digest(&self) -> &str {
        &self.inner.contract_digest
    }
}

pub struct RepairSource<'a> {
    pub admission: &'a DeliveryAdmission,
    pub original: Arc<CompletedChildClaim>,
    pub source: &'a SealedChildSource,
    pub completion: Arc<RunCompletion>,
}
pub struct ValidationRepairRequest<'a> {
    pub source: RepairSource<'a>,
    pub workflow: &'a Workflow,
    pub outcome: &'a VerifiedAuthorizedOutcome,
    pub completion: Arc<RunCompletion>,
}
pub struct ReviewRepairRequest<'a> {
    pub source: RepairSource<'a>,
    pub assignment: &'a ChildReviewAssignment,
    pub outcome: &'a VerifiedAuthorizedOutcome,
    pub completion: Arc<RunCompletion>,
}

impl DeliveryCoordinator {
    pub async fn unfinished_validation(
        &self,
        request: ValidationRepairRequest<'_>,
    ) -> Result<UnfinishedChildWork> {
        let admission = request.source.admission;
        self.current(admission, &[DeliveryPhase::LocalValidation])?;
        self.repair_worker(
            admission,
            request.outcome,
            &request.completion,
            AgentRole::Validation,
        )?;
        if request.workflow.digest != admission.gate.workflow_digest() {
            return Err(Error::new(
                "stale_result",
                "repair validation workflow changed",
            ));
        }
        let required = local_requirements(self.contract(admission)?, request.workflow)?;
        let completion = &request.completion;
        // Retained old failures are historical: compare both private capability
        // sets before selecting the latest result for each configured check.
        let mut records = Vec::new();
        for check in &completion.checks {
            records.push((check.record(), false));
        }
        for check in &completion.failed_checks {
            records.push((check.record(), true));
        }
        let selected = latest_required_failures(&required, &records)?;
        let execution = completion.execution.as_ref().ok_or_else(|| {
            Error::new(
                "execution_ownership",
                "validation source ownership is missing",
            )
        })?;
        let contract = completion
            .contract
            .as_ref()
            .ok_or_else(|| Error::new("worker_binding", "validation contract is missing"))?;
        let mut findings = Vec::new();
        let mut names = BTreeSet::new();
        for record in selected {
            let check = completion
                .failed_checks
                .iter()
                .find(|c| std::ptr::eq(c.record(), record))
                .ok_or_else(|| {
                    Error::new("worker_binding", "failed result lacks retained capability")
                })?;
            check.production_failure()?;
            validate_failed_record(record, admission, request.outcome, execution, contract)?;
            let configured = &request.workflow.front_matter.checks.local[&record.check_name];
            if record.command.argv != configured.argv {
                return Err(Error::new(
                    "validation_gate",
                    "failed command differs from configured obligation",
                ));
            }
            names.insert(record.check_name.clone());
            findings.push(json!({"check":record.check_name,"argv":record.command.argv,
                "exit_code":record.command.exit_code,"stdout":record.command.stdout,"stderr":record.command.stderr,
                "record_digest":canonical_digest(1,record)?}));
        }
        let cause = json!({"kind":"required_local_checks","checks":findings});
        self.finish_repair(
            request.source,
            request.outcome,
            request.completion,
            cause,
            json!({"kind":"required_local_checks","checks":names}),
            false,
        )
        .await
    }
    pub async fn unfinished_review(
        &self,
        request: ReviewRepairRequest<'_>,
    ) -> Result<UnfinishedChildWork> {
        let admission = request.source.admission;
        self.current(
            admission,
            &[DeliveryPhase::ReviewPublication, DeliveryPhase::NoChange],
        )?;
        self.repair_worker(
            admission,
            request.outcome,
            &request.completion,
            AgentRole::Review,
        )?;
        let claim = Self::completed_review_claim(
            admission,
            request.assignment,
            request.outcome,
            &request.completion,
        )?;
        if claim.verdict != evidence::Verdict::Findings || claim.unresolved_findings.is_empty() {
            return Err(Error::new(
                "no_work_trigger",
                "only concrete unresolved review findings authorize scoped repair",
            ));
        }
        let cause = json!({"kind":"review_findings","findings":claim.unresolved_findings,
            "supporting_links":claim.supporting_links,"review_claim":request.outcome.digest(),
            "review_context":request.outcome.outcome().instructions.context_digest});
        self.finish_repair(
            request.source,
            request.outcome,
            request.completion,
            cause,
            json!({"kind":"review_findings"}),
            true,
        )
        .await
    }
    fn repair_worker(
        &self,
        admission: &DeliveryAdmission,
        outcome: &VerifiedAuthorizedOutcome,
        completion: &RunCompletion,
        role: AgentRole,
    ) -> Result<()> {
        child::completed_run(outcome, completion)?;
        outcome.effective_authority().compare_current(
            &admission.gate,
            Utc::now(),
            chrono::Duration::seconds(60),
        )?;
        let worker = outcome.outcome();
        let contract = completion.contract.as_ref().ok_or_else(|| {
            Error::new(
                "worker_binding",
                "repair evidence lacks actual retained contract",
            )
        })?;
        if worker.instructions.role != role
            || worker.instructions.repository_revision.as_deref()
                != Some(&admission.scope.revision.head)
            || worker.instruction_digest != contract.instructions().manifest_digest
            || worker.tool_contract_digest != contract.tools().digest()
            || canonical_digest(1, &worker.instructions.profile)?
                != canonical_digest(1, &admission.gate.profile())?
            || completion.state.intervention.is_some()
            || completion.state.input_required
            || completion
                .failed_checks
                .iter()
                .any(|c| c.record().binding.operation_id != completion.operation_id)
        {
            return Err(Error::new(
                "worker_binding",
                "repair evidence differs from actual current scoped session/profile",
            ));
        }
        if let Some(execution) = &completion.execution {
            execution
                .ownership
                .verify(&execution.binding, contract.isolation())?;
        }
        Ok(())
    }
    async fn finish_repair(
        &self,
        input: RepairSource<'_>,
        outcome: &VerifiedAuthorizedOutcome,
        failure: Arc<RunCompletion>,
        cause: Value,
        obligation: Value,
        review_round: bool,
    ) -> Result<UnfinishedChildWork> {
        self.current_child_claim(input.admission, &input.original)?;
        let original = input.original.outcome();
        let original_contract = input
            .completion
            .contract
            .as_ref()
            .ok_or_else(|| Error::new("worker_binding", "original contract missing"))?;
        if original_contract.instructions().manifest_digest != original.outcome().instruction_digest
            || canonical_digest(1, &original.outcome().instructions.profile)?
                != canonical_digest(1, &input.admission.gate.profile())?
        {
            return Err(Error::new(
                "worker_binding",
                "original scoped profile or contract changed",
            ));
        }
        let owned = assignment::child_source(
            input.admission,
            &input.original,
            input.source,
            &input.completion,
        )?;
        let git = self.git.clone();
        let checked = owned.clone();
        tokio::task::spawn_blocking(move || checked.verify_for_clone(&git))
            .await
            .map_err(|_| {
                Error::new(
                    "operation_uncertain",
                    "repair source verification task was lost",
                )
            })??;
        let predecessor_receipt = closed_run(&self.host, input.original.operation_id())?;
        let failure_receipt = closed_run(&self.host, failure.operation_id)?;
        let reference = RunReference::from_receipt(&predecessor_receipt)?.ok_or_else(|| {
            Error::new(
                "worker_binding",
                "original operation is not an admitted worker",
            )
        })?;
        if predecessor_receipt.expected != original.run_receipt().expected
            || failure_receipt.expected != outcome.run_receipt().expected
            || reference.workspace != input.admission.gate.workspace_id()
            || reference.phase.work != input.admission.gate.work_key()?
            || reference.phase.generation
                != canonical_digest(
                    1,
                    &input
                        .original
                        .outcome()
                        .original_authority()
                        .work()
                        .generation,
                )?
            || reference.instruction_digest != original.outcome().instruction_digest
            || reference.profile_digest
                != canonical_digest(1, &original.outcome().instructions.profile)?
        {
            return Err(Error::new(
                "worker_binding",
                "repair predecessor differs from actual original admission",
            ));
        }
        // A correction creates its own defined phase budget. This trigger never
        // silently converts the previous generation into that new phase.
        if original.original_authority().work().generation != input.admission.gate.work().generation
        {
            return Err(Error::new(
                "correction_execution_binding",
                "repair needs current-generation admission after correction",
            ));
        }
        let mut review_rounds_consumed = 0u32;
        for receipt in self.host.operations()? {
            if let Some(review) = RunReference::from_receipt(&receipt)?
                && review.workspace == reference.workspace
                && review.phase.work == reference.phase.work
                && review.phase.generation == reference.phase.generation
                && review.phase.phase == "child_codex_review"
            {
                closed_run(&self.host, receipt.operation_id)?;
                review_rounds_consumed =
                    review_rounds_consumed.checked_add(1).ok_or_else(|| {
                        Error::new(
                            "attempts_exhausted",
                            "review round count exceeds finite range",
                        )
                    })?;
            }
        }
        if review_round && review_rounds_consumed == 0 {
            return Err(Error::new(
                "worker_binding",
                "review findings lack the actual admitted review round",
            ));
        }
        let failure_phase = RunReference::from_receipt(&failure_receipt)?.ok_or_else(|| {
            Error::new("worker_binding", "failure admission lacks phase identity")
        })?;
        let expected_failure_phase = if review_round {
            "child_codex_review"
        } else {
            "child_local_validation"
        };
        if failure_phase.phase.phase != expected_failure_phase
            || failure_phase.workspace != reference.workspace
            || failure_phase.phase.work != reference.phase.work
            || failure_phase.phase.generation != reference.phase.generation
        {
            return Err(Error::new(
                "worker_binding",
                "failure admission is not the exact child validation/review phase",
            ));
        }
        let inventory = self.host.operations()?;
        require_latest_phase(&inventory, predecessor_receipt.runtime_id, &failure_phase)?;
        let failure_phase_inventory_digest =
            phase_inventory(&inventory, predecessor_receipt.runtime_id, &failure_phase)?;
        let predecessor_receipt_digest = canonical_digest(1, &predecessor_receipt)?;
        let failure_receipt_digest = canonical_digest(1, &failure_receipt)?;
        let progress_digest = progress_marker(
            &input.admission.scope.work,
            owned.repository(),
            &owned.revision().tree,
            &obligation,
        )?;
        let remaining_task = json!({"purpose":"complete the remaining verified obligation within the original authorized child scope",
            "work":input.admission.scope.work,"source":owned.revision(),"prior_claim":input.original.claim(),
            "remaining":cause,"outcome_claim":{"schema_version":1,"summary":"verified scoped result",
            "fulfilled_criteria":input.original.claim().fulfilled_criteria,"disposition":{"kind":"changed"}}});
        let digest = canonical_digest(
            1,
            &json!({"purpose":"unfinished_child_work","predecessor":predecessor_receipt_digest,
            "failure":failure_receipt_digest,"failure_inventory":failure_phase_inventory_digest,"source":owned.digest()?,"authority":input.admission.gate.digest()?,
            "remaining_task":remaining_task,"progress":progress_digest,"review_round":review_round}),
        )?;
        let role = original.outcome().instructions.role;
        Ok(UnfinishedChildWork {
            inner: Arc::new(UnfinishedProof {
                original: input.original,
                _failure: failure,
                authority: input.admission.gate.clone(),
                source: owned,
                reference,
                predecessor_receipt,
                predecessor_receipt_digest,
                failure_operation: outcome.outcome().operation_id,
                failure_receipt_digest,
                failure_phase,
                failure_phase_inventory_digest,
                remaining_task,
                progress_digest,
                digest,
                role,
                contract_digest: input.admission.scope.contract_digest.clone(),
                review_round,
                review_rounds_consumed,
            }),
        })
    }
}

fn closed_run(host: &ControlHost, operation: Uuid) -> Result<Receipt> {
    let receipt = host.operation(operation)?;
    let result = receipt.observed_result.as_ref();
    if receipt.phase != ReceiptPhase::Reconciled
        || result
            .and_then(|v| v.get("local_execution"))
            .and_then(Value::as_str)
            != Some("exited")
        || result
            .and_then(|v| v.get("unresolved_effects"))
            .and_then(Value::as_array)
            .is_none_or(|a| !a.is_empty())
    {
        return Err(Error::new(
            "reconciliation_required",
            "repair requires actual exit and reconciled original worker effects",
        ));
    }
    Ok(receipt)
}
pub(super) fn latest_required_failures<'a>(
    required: &[CheckRequirement],
    records: &[(&'a CheckExecutionRecord, bool)],
) -> Result<Vec<&'a CheckExecutionRecord>> {
    let mut failed = Vec::new();
    for requirement in required {
        let mut matching: Vec<_> = records
            .iter()
            .filter(|(r, _)| r.check_name == requirement.id && r.kind == requirement.kind)
            .collect();
        matching.sort_by_key(|(r, _)| r.completed_at);
        let Some((latest, is_failure)) = matching.last().copied() else {
            continue;
        };
        if matching.len() > 1 && matching[matching.len() - 2].0.completed_at >= latest.started_at {
            return Err(Error::new(
                "validation_gate",
                "overlapping named checks have no unambiguous latest result",
            ));
        }
        if *is_failure {
            failed.push(*latest);
        }
    }
    if failed.is_empty() {
        return Err(Error::new(
            "no_work_trigger",
            "no latest required local check remains failed",
        ));
    }
    Ok(failed)
}
fn validate_failed_record(
    record: &CheckExecutionRecord,
    admission: &DeliveryAdmission,
    outcome: &VerifiedAuthorizedOutcome,
    execution: &crate::worker::WorkerExecution,
    contract: &crate::worker::WorkerContract,
) -> Result<()> {
    if record.binding.work != admission.scope.work
        || record.workflow_digest != admission.gate.workflow_digest()
        || record.binding.repository != admission.scope.repository
        || record.binding.operation_id != outcome.outcome().operation_id
        || record.binding.revision.commit != admission.scope.revision.tested_commit
        || record.binding.revision.tree != admission.scope.revision.tested_tree
        || record.before.revision != record.binding.revision
        || record.before != record.after
        || record.kind != CheckKind::Local
        || record.binding.isolation_digest != contract.isolation().digest()
        || record.command.cwd != contract.isolation().plan().repository
        || record.command.isolation_digest != contract.isolation().digest()
        || canonical_digest(1, &record.binding)? != canonical_digest(1, &execution.binding)?
        || record.correction.is_some()
    {
        return Err(Error::new(
            "validation_gate",
            "failed check is not exact unchanged current validation source",
        ));
    }
    Ok(())
}

fn progress_marker(
    work: &WorkIdentity,
    repository: &RepositoryId,
    tree: &str,
    obligation: &Value,
) -> Result<String> {
    crate::git::validate_oid(tree)?;
    canonical_digest(
        1,
        &json!({"work":work,"repository":repository,"tree":tree,"obligation":obligation}),
    )
}
/// A new exact source requires its original finite validation/review phase again.
/// This is separate from evidence that an implementation has unfinished work.
#[derive(Clone)]
pub struct VerifiedSourceAdvanced {
    assignment: DeliveryWorkerAssignment,
    reference: RunReference,
    predecessor: Receipt,
    predecessor_digest: String,
    phase_inventory_digest: String,
    source_operation: Uuid,
    source_receipt_digest: String,
    progress_digest: String,
    digest: String,
    review_rounds_consumed: u32,
    _original: Arc<CompletedChildClaim>,
    _previous_completion: Arc<RunCompletion>,
}
impl VerifiedSourceAdvanced {
    pub fn assignment(&self) -> &DeliveryWorkerAssignment {
        &self.assignment
    }
    pub fn predecessor(&self) -> Uuid {
        self.predecessor.operation_id
    }
    pub fn predecessor_receipt_digest(&self) -> &str {
        &self.predecessor_digest
    }
    pub fn predecessor_phase_inventory_digest(&self) -> &str {
        &self.phase_inventory_digest
    }
    pub fn verify_predecessor_inventory(&self, receipts: &[Receipt]) -> Result<()> {
        if phase_inventory(receipts, self.predecessor.runtime_id, &self.reference)?
            != self.phase_inventory_digest
        {
            return Err(Error::new(
                "stale_result",
                "another required-check admission superseded source advancement",
            ));
        }
        Ok(())
    }
    pub fn source_operation(&self) -> Uuid {
        self.source_operation
    }
    pub fn source_receipt_digest(&self) -> &str {
        &self.source_receipt_digest
    }
    pub fn phase(&self) -> &PhaseKey {
        &self.reference.phase
    }
    pub fn authority(&self) -> &FreshGate {
        self.assignment.authority()
    }
    pub fn source(&self) -> &DeliverySource {
        self.assignment
            .source()
            .expect("verified source assignment")
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
    pub fn progress_digest(&self) -> &str {
        &self.progress_digest
    }
    pub fn review_rounds_consumed(&self) -> u32 {
        self.review_rounds_consumed
    }
    pub fn reference(&self) -> &RunReference {
        &self.reference
    }
    pub fn verify_assignment(&self, assignment: &DeliveryWorkerAssignment) -> Result<()> {
        if assignment.digest()? != self.assignment.digest()? {
            return Err(Error::new(
                "worker_binding",
                "renewed validation differs from sealed source assignment",
            ));
        }
        Ok(())
    }
    pub fn verify_contract(&self, contract: &crate::worker::WorkerContract) -> Result<()> {
        assignment::verify_assignment_contract(
            contract,
            self.authority(),
            &self.phase().phase,
            self.assignment.context(),
        )?;
        if contract.instructions().manifest.role != self.assignment.role()
            || contract
                .instructions()
                .manifest
                .repository_revision
                .as_deref()
                != Some(self.source().revision().commit.as_str())
        {
            return Err(Error::new(
                "worker_binding",
                "renewed check contract has another role or source",
            ));
        }
        Ok(())
    }
}
pub struct SourceAdvancedRequest<'a> {
    pub source: RepairSource<'a>,
    pub assignment: &'a DeliveryWorkerAssignment,
    pub predecessor: &'a VerifiedAuthorizedOutcome,
    pub completion: Arc<RunCompletion>,
}
impl DeliveryCoordinator {
    pub async fn source_advanced(
        &self,
        request: SourceAdvancedRequest<'_>,
    ) -> Result<VerifiedSourceAdvanced> {
        let admission = request.source.admission;
        self.current_child_claim(admission, &request.source.original)?;
        let role = match request.assignment.phase() {
            "child_local_validation" => AgentRole::Validation,
            "child_codex_review" => AgentRole::Review,
            _ => {
                return Err(Error::new(
                    "no_work_trigger",
                    "only original child validation/review phases can follow advanced source",
                ));
            }
        };
        child::completed_run(request.predecessor, &request.completion)?;
        let old_contract = request.completion.contract.as_ref().ok_or_else(|| {
            Error::new(
                "worker_binding",
                "previous validation lacks actual contract",
            )
        })?;
        let old = request.predecessor.outcome();
        if old.instructions.role != role
            || old.instruction_digest != old_contract.instructions().manifest_digest
            || request.predecessor.effective_authority().work() != &admission.scope.work
            || request.predecessor.original_authority().config_revision()
                != admission.gate.config_revision()
            || canonical_digest(1, &old.instructions.profile)?
                != canonical_digest(1, admission.gate.profile())?
            || request.assignment.role() != role
            || request.assignment.authority().digest()? != admission.gate.digest()?
        {
            return Err(Error::new(
                "worker_binding",
                "previous phase or current assignment is outside exact source authority",
            ));
        }
        let source = assignment::child_source(
            admission,
            &request.source.original,
            request.source.source,
            &request.source.completion,
        )?;
        if request
            .assignment
            .source()
            .map(DeliverySource::digest)
            .transpose()?
            != Some(source.digest()?)
        {
            return Err(Error::new(
                "worker_binding",
                "renewed assignment does not use actual sealed source",
            ));
        }
        let previous_revision = old
            .instructions
            .repository_revision
            .clone()
            .ok_or_else(|| {
                Error::new(
                    "worker_binding",
                    "previous validation has no checked source",
                )
            })?;
        let checked = source.clone();
        let git = self.git.clone();
        let prior = tokio::task::spawn_blocking(move || {
            checked.verify_for_clone(&git)?;
            git.revision(checked.root(), &previous_revision)
        })
        .await
        .map_err(|_| {
            Error::new(
                "operation_uncertain",
                "source advancement verification did not join",
            )
        })??;
        if prior.tree == source.revision().tree {
            return Err(Error::new(
                "no_progress",
                "source tree is unchanged since the previous required check",
            ));
        }
        let predecessor = closed_run(&self.host, old.operation_id)?;
        let source_operation = request.source.original.operation_id();
        let source_receipt = closed_run(&self.host, source_operation)?;
        let reference = RunReference::from_receipt(&predecessor)?
            .ok_or_else(|| Error::new("worker_binding", "previous check lacks worker admission"))?;
        let source_run = RunReference::from_receipt(&source_receipt)?.ok_or_else(|| {
            Error::new(
                "worker_binding",
                "advanced source lacks implementation admission",
            )
        })?;
        if source_run.phase.phase != "implementation"
            || reference.phase.phase != request.assignment.phase()
            || source_run.phase.work != reference.phase.work
            || source_run.phase.generation != reference.phase.generation
            || source_run.workspace != reference.workspace
            || source_run.account != reference.account
            || reference.workspace != admission.gate.workspace_id()
            || predecessor.expected != request.predecessor.run_receipt().expected
            || source_receipt.expected != request.source.original.outcome().run_receipt().expected
        {
            return Err(Error::new(
                "worker_binding",
                "source and prior check admissions have different authority",
            ));
        }
        let mut review_rounds_consumed = 0u32;
        for receipt in self.host.operations()? {
            if let Some(run) = RunReference::from_receipt(&receipt)? {
                if (run.phase == reference.phase && run.attempt > reference.attempt)
                    || (run.phase == source_run.phase && run.attempt > source_run.attempt)
                {
                    return Err(Error::new(
                        "stale_result",
                        "a later source or check admission already exists",
                    ));
                }
                if run.workspace == reference.workspace
                    && run.phase.work == reference.phase.work
                    && run.phase.generation == reference.phase.generation
                    && run.phase.phase == "child_codex_review"
                {
                    closed_run(&self.host, receipt.operation_id)?;
                    review_rounds_consumed =
                        review_rounds_consumed.checked_add(1).ok_or_else(|| {
                            Error::new("attempts_exhausted", "review round count overflow")
                        })?;
                }
            }
        }
        let inventory = self.host.operations()?;
        require_latest_phase(&inventory, predecessor.runtime_id, &reference)?;
        let phase_inventory_digest =
            phase_inventory(&inventory, predecessor.runtime_id, &reference)?;
        let predecessor_digest = canonical_digest(1, &predecessor)?;
        let source_receipt_digest = canonical_digest(1, &source_receipt)?;
        let progress_digest = progress_marker(
            &admission.scope.work,
            source.repository(),
            &source.revision().tree,
            &json!({"phase":reference.phase.phase}),
        )?;
        let digest = canonical_digest(
            1,
            &json!({"purpose":"source_advanced","previous":predecessor_digest,"previous_inventory":phase_inventory_digest,"source":source_receipt_digest,
            "source_seal":request.source.source.receipt().payload_digest,"assignment":request.assignment.digest()?,"progress":progress_digest,"review_rounds_consumed":review_rounds_consumed}),
        )?;
        Ok(VerifiedSourceAdvanced {
            assignment: request.assignment.clone(),
            reference,
            predecessor,
            predecessor_digest,
            phase_inventory_digest,
            source_operation,
            source_receipt_digest,
            progress_digest,
            digest,
            review_rounds_consumed,
            _original: request.source.original,
            _previous_completion: request.completion,
        })
    }
}

fn same_phase_group(
    runtime: Uuid,
    expected: &RunReference,
    receipt: &Receipt,
    run: &RunReference,
) -> bool {
    receipt.runtime_id == runtime
        && run.workspace == expected.workspace
        && run.phase.work == expected.phase.work
        && run.phase.generation == expected.phase.generation
        && run.phase.phase == expected.phase.phase
}
fn phase_inventory(receipts: &[Receipt], runtime: Uuid, expected: &RunReference) -> Result<String> {
    let mut selected = BTreeMap::new();
    for receipt in receipts {
        if let Some(run) = RunReference::from_receipt(receipt)?
            && same_phase_group(runtime, expected, receipt, &run)
            && selected.insert(receipt.operation_id, receipt).is_some()
        {
            return Err(Error::new(
                "record_corrupt",
                "duplicate phase admission inventory",
            ));
        }
    }
    canonical_digest(1, &selected)
}
fn require_latest_phase(
    receipts: &[Receipt],
    runtime: Uuid,
    expected: &RunReference,
) -> Result<()> {
    for receipt in receipts {
        if let Some(run) = RunReference::from_receipt(receipt)?
            && same_phase_group(runtime, expected, receipt, &run)
            && (run.phase.retry_grant != expected.phase.retry_grant
                || run.attempt > expected.attempt)
        {
            return Err(Error::new(
                "stale_result",
                "later or differently granted check history prevents automatic continuation",
            ));
        }
    }
    Ok(())
}

#[path = "delivery_continuation_retry.rs"]
mod retry;
pub use retry::RetriedUnfinishedWork;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        execution::{ExecutionBinding, ExecutionEvidenceClass, SourceSnapshot},
        git::Revision,
        isolation::CommandOutcome,
    };
    fn work() -> WorkIdentity {
        let digest = bytes_digest(b"continuation fixture");
        WorkIdentity {
            linear_endpoint: "https://linear.invalid/graphql".into(),
            linear_workspace: Uuid::from_u128(2),
            parent: Uuid::from_u128(10),
            child: Some(Uuid::from_u128(11)),
            generation: evidence::Generation {
                authorization: digest.clone(),
                definition: digest.clone(),
                correction: digest.clone(),
                scope: digest,
            },
        }
    }
    fn record(name: &str, started: i64, completed: i64, success: bool) -> CheckExecutionRecord {
        let revision = Revision {
            commit: "a".repeat(40),
            tree: "b".repeat(40),
        };
        let isolation = bytes_digest(b"fixture isolation");
        CheckExecutionRecord {
            schema_version: 1,
            correction: None,
            binding: ExecutionBinding {
                runtime_id: Uuid::from_u128(1),
                operation_id: Uuid::from_u128(7),
                work: work(),
                repository: RepositoryId {
                    hostname: "github.com".into(),
                    id: "42".into(),
                },
                revision: revision.clone(),
                candidate_vector_digest: None,
                isolation_digest: isolation.clone(),
            },
            workflow_digest: bytes_digest(b"workflow"),
            check_name: name.into(),
            kind: CheckKind::Local,
            before: SourceSnapshot {
                revision: revision.clone(),
                files: BTreeMap::new(),
            },
            after: SourceSnapshot {
                revision,
                files: BTreeMap::new(),
            },
            artifacts: vec![],
            command: CommandOutcome {
                schema_version: 1,
                argv: vec!["test".into()],
                cwd: PathBuf::from("/fixture"),
                isolation_digest: isolation,
                exit_code: Some(if success { 0 } else { 1 }),
                timed_out: false,
                stdout: String::new(),
                stderr: String::new(),
                output_truncated: false,
                elapsed_ms: 1,
            },
            started_at: DateTime::from_timestamp(started, 0).unwrap(),
            completed_at: DateTime::from_timestamp(completed, 0).unwrap(),
            evidence_class: ExecutionEvidenceClass::DeterministicFixture,
            successful: success,
            blocker: (!success).then(|| "known fixture failure".into()),
        }
    }
    fn required() -> Vec<CheckRequirement> {
        vec![CheckRequirement {
            id: "unit".into(),
            kind: CheckKind::Local,
            workflow_id: None,
            command: vec!["test".into()],
        }]
    }
    #[test]
    fn latest_required_failure_uses_both_outcome_sets_and_rejects_overlap() {
        let failure = record("unit", 1, 2, false);
        let pass = record("unit", 3, 4, true);
        assert_eq!(
            latest_required_failures(&required(), &[(&failure, true), (&pass, false)])
                .unwrap_err()
                .code,
            "no_work_trigger"
        );
        let later_failure = record("unit", 5, 6, false);
        let selected = latest_required_failures(
            &required(),
            &[(&failure, true), (&pass, false), (&later_failure, true)],
        )
        .unwrap();
        assert!(std::ptr::eq(selected[0], &later_failure));
        let overlap = record("unit", 3, 5, false);
        assert_eq!(
            latest_required_failures(&required(), &[(&pass, false), (&overlap, true)])
                .unwrap_err()
                .code,
            "validation_gate"
        );
        let unrelated = record("optional", 7, 8, false);
        assert_eq!(
            latest_required_failures(&required(), &[(&unrelated, true)])
                .unwrap_err()
                .code,
            "no_work_trigger"
        );
    }
    #[test]
    fn progress_is_exact_tree_obligation_and_generation_not_attempt_or_diagnostics() {
        let record = record("unit", 1, 2, false);
        let obligation = json!({"kind":"required_local_checks","checks":["unit"]});
        let marker = progress_marker(
            &record.binding.work,
            &record.binding.repository,
            &record.binding.revision.tree,
            &obligation,
        )
        .unwrap();
        let mut repeated = record.clone();
        repeated.binding.operation_id = Uuid::new_v4();
        repeated.started_at = Utc::now();
        repeated.command.stderr = "different clock text".into();
        repeated.binding.revision.commit = "c".repeat(40);
        assert_eq!(
            marker,
            progress_marker(
                &repeated.binding.work,
                &repeated.binding.repository,
                &repeated.binding.revision.tree,
                &obligation
            )
            .unwrap()
        );
        repeated.binding.revision.tree = "d".repeat(40);
        assert_ne!(
            marker,
            progress_marker(
                &repeated.binding.work,
                &repeated.binding.repository,
                &repeated.binding.revision.tree,
                &obligation
            )
            .unwrap()
        );
        repeated.binding.revision.tree = record.binding.revision.tree.clone();
        repeated.binding.work.generation.correction = bytes_digest(b"new correction");
        assert_ne!(
            marker,
            progress_marker(
                &repeated.binding.work,
                &repeated.binding.repository,
                &repeated.binding.revision.tree,
                &obligation
            )
            .unwrap()
        );
    }
}
