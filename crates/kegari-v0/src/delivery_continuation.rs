//! A new child session needs a concrete unmet obligation and actual progress.
//! These capabilities are never reconstructed from serialized worker summaries.
use super::*;
use crate::{
    control::ReceiptPhase, execution::CheckExecutionRecord, runtime::VerifiedAuthorizedOutcome,
    worker_run::RunCompletion,
};
use kegari_context::instructions::AgentRole;
use kegari_types::run::PhaseKey;
use kegari_types::run::RunReference;

#[path = "delivery_validation_budget.rs"]
mod validation_budget;
pub(crate) use validation_budget::ValidationAdvanceBasis;
use validation_budget::ValidationBudgetLineage;

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
    granted_review: Option<CompletedReviewLineage>,
    paired_repair: Option<Arc<VerifiedReviewRoundPlan>>,
    granted_validation: Option<Arc<CompletedGrantedValidation>>,
    source_validation: Option<Arc<VerifiedSourceAdvanced>>,
    paired_completion: Option<Arc<VerifiedCompletedPairedResume>>,
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
    pub fn verify_current(
        &self,
        receipts: &[Receipt],
        grants: &[crate::decisions::VerifiedRetryGrant],
    ) -> Result<()> {
        self.verify_failure_inventory(receipts)?;
        if let Some(proof) = &self.inner.paired_completion {
            proof.verify_immutable(receipts, grants)?;
        }
        if let Some(review) = &self.inner.granted_review {
            review.verify_current(receipts, grants)?;
        }
        if let Some(validation) = &self.inner.granted_validation {
            validation.verify_current(receipts, grants)?;
        }
        if let Some(source) = &self.inner.source_validation {
            source.verify_completed_inventory(receipts, grants, self.failure_operation())?;
        }
        if let Some(pair) = &self.inner.paired_repair {
            pair.verify_completed_repair_result(
                receipts,
                grants,
                &self.inner.predecessor_receipt,
                self.inner.paired_completion.as_ref(),
            )?;
        }
        Ok(())
    }
    pub(crate) fn granted_review(&self) -> Option<&CompletedReviewLineage> {
        self.inner.granted_review.as_ref()
    }
    pub(crate) fn requires_review_round(&self) -> bool {
        matches!(
            &self.inner.granted_review,
            Some(CompletedReviewLineage::Round(_))
        )
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
        self.authority().compare_admitted_authority(gate)?;
        if manifest.role != self.role()
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

enum RepairReview {
    None,
    Findings(Option<Box<CompletedReviewLineage>>),
    ContinuedPair {
        previous: Arc<VerifiedReviewRoundPlan>,
        validation_source: Arc<VerifiedSourceAdvanced>,
        grants: Vec<crate::decisions::VerifiedRetryGrant>,
    },
    ContinuedPairAfterValidation {
        previous: Arc<VerifiedReviewRoundPlan>,
        validation: Arc<CompletedGrantedValidation>,
        grants: Vec<crate::decisions::VerifiedRetryGrant>,
    },
}

pub struct RepairSource<'a> {
    pub(crate) paired_completion: Option<Arc<VerifiedCompletedPairedResume>>,
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
        self.unfinished_validation_inner(request, RepairReview::None)
            .await
    }
    pub(super) async fn unfinished_validation_in_round(
        &self,
        request: ValidationRepairRequest<'_>,
        previous: Arc<VerifiedReviewRoundPlan>,
        validation_source: Arc<VerifiedSourceAdvanced>,
        grants: &[crate::decisions::VerifiedRetryGrant],
    ) -> Result<UnfinishedChildWork> {
        let actual = self
            .host
            .operation(request.source.original.operation_id())?;
        previous.verify_completed_repair_result(
            &self.host.operations()?,
            grants,
            &actual,
            request.source.paired_completion.as_ref(),
        )?;
        validation_source.verify_completed_inventory(
            &self.host.operations()?,
            grants,
            request.completion.operation_id,
        )?;
        let result = self
            .unfinished_validation_inner(
                request,
                RepairReview::ContinuedPair {
                    previous,
                    validation_source,
                    grants: grants.to_vec(),
                },
            )
            .await?;
        result.verify_current(&self.host.operations()?, grants)?;
        Ok(result)
    }
    pub(super) async fn unfinished_validation_after_granted_round(
        &self,
        request: ValidationRepairRequest<'_>,
        previous: Arc<VerifiedReviewRoundPlan>,
        validation: Arc<CompletedGrantedValidation>,
        grants: &[crate::decisions::VerifiedRetryGrant],
    ) -> Result<UnfinishedChildWork> {
        validation.verify_current(&self.host.operations()?, grants)?;
        if validation.outcome().digest() != request.outcome.digest()
            || !Arc::ptr_eq(validation.completion(), &request.completion)
        {
            return Err(Error::new(
                "worker_binding",
                "granted validation differs from actual repair failure",
            ));
        }
        let result = self
            .unfinished_validation_inner(
                request,
                RepairReview::ContinuedPairAfterValidation {
                    previous,
                    validation: validation.clone(),
                    grants: grants.to_vec(),
                },
            )
            .await?;
        result.verify_current(&self.host.operations()?, grants)?;
        Ok(result)
    }
    async fn unfinished_validation_inner(
        &self,
        request: ValidationRepairRequest<'_>,
        review: RepairReview,
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
        let prior = crate::validation::ValidationPrefix::from_completion(&request.completion)?;
        let required = if let Some(prefix) = &prior {
            let current = prefix
                .history()
                .last()
                .ok_or_else(|| Error::invalid("empty validation prefix"))?
                .current()?;
            if current.binding.work != admission.scope.work
                || current.binding.repository != admission.scope.repository
            {
                return Err(Error::new(
                    "validation_history_binding",
                    "discovered failure differs from current work and repository",
                ));
            }
            current
                .requirements
                .iter()
                .map(|requirement| CheckRequirement {
                    id: requirement.id.clone(),
                    kind: CheckKind::Local,
                    workflow_id: None,
                    command: requirement.argv.clone(),
                })
                .collect()
        } else {
            local_requirements(self.contract(admission)?, request.workflow)?
        };
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
        if let Some(plan) = &completion.validation_plan {
            records.retain(|(record, _)| {
                record
                    .codex
                    .as_ref()
                    .and_then(|provenance| provenance.plan.as_ref())
                    .is_some_and(|original| {
                        canonical_digest(1, original).ok().as_deref() == Some(plan.digest())
                    })
            });
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
            let expected = required
                .iter()
                .find(|requirement| requirement.id == record.check_name)
                .ok_or_else(|| {
                    Error::new(
                        "validation_gate",
                        "failed result is outside the current obligations",
                    )
                })?;
            if record.command.argv != expected.command {
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
        let cause = json!({"kind":"required_local_checks","checks":findings,
            "retained_validation":prior.as_ref().map(|prefix|prefix.descriptor()).transpose()?});
        self.finish_repair(
            request.source,
            request.outcome,
            request.completion,
            cause,
            json!({"kind":"required_local_checks","checks":names}),
            review,
        )
        .await
    }
    pub async fn unfinished_review(
        &self,
        request: ReviewRepairRequest<'_>,
    ) -> Result<UnfinishedChildWork> {
        self.unfinished_review_inner(request, None).await
    }
    #[cfg(test)]
    pub(crate) async fn unfinished_retried_review(
        &self,
        request: ReviewRepairRequest<'_>,
        proof: &ReviewRetryWork,
        grants: &[crate::decisions::VerifiedRetryGrant],
    ) -> Result<UnfinishedChildWork> {
        let lineage = self
            .completed_granted_review(
                request.source.admission,
                request.assignment,
                request.outcome,
                &request.completion,
                proof,
                grants,
            )
            .await?;
        self.unfinished_review_inner(
            request,
            Some(CompletedReviewLineage::Legacy(Box::new(lineage))),
        )
        .await
    }
    /// Common consumer: the actual initial/retried/source-advanced review
    /// remains opaque. No raw claim, scalar or marker can create this lineage.
    pub(crate) async fn unfinished_completed_round(
        &self,
        request: ReviewRepairRequest<'_>,
        completed: Arc<CompletedReviewRound>,
        grants: &[crate::decisions::VerifiedRetryGrant],
    ) -> Result<UnfinishedChildWork> {
        completed.verify_current(&self.host.operations()?, grants)?;
        if completed.outcome().digest() != request.outcome.digest()
            || !Arc::ptr_eq(completed.completion(), &request.completion)
        {
            return Err(Error::new(
                "worker_binding",
                "Findings differ from actual completed round",
            ));
        }
        self.unfinished_review_inner(request, Some(CompletedReviewLineage::Round(completed)))
            .await
    }
    async fn unfinished_review_inner(
        &self,
        request: ReviewRepairRequest<'_>,
        granted_review: Option<CompletedReviewLineage>,
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
        let mut cause = json!({"kind":"review_findings","findings":claim.unresolved_findings,
            "supporting_links":claim.supporting_links,"review_claim":request.outcome.digest(),
            "review_context":request.outcome.outcome().instructions.context_digest});
        if let Some(assessment) = claim.validation {
            cause["validation_assessment"] = json!(assessment);
        }
        self.finish_repair(
            request.source,
            request.outcome,
            request.completion,
            cause,
            json!({"kind":"review_findings"}),
            RepairReview::Findings(granted_review.map(Box::new)),
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
        review: RepairReview,
    ) -> Result<UnfinishedChildWork> {
        let (
            review_round,
            granted_review,
            paired_repair,
            granted_validation,
            source_validation,
            validation_grants,
        ) = match review {
            RepairReview::None => (false, None, None, None, None, Vec::new()),
            RepairReview::Findings(lineage) => (
                true,
                lineage.map(|value| *value),
                None,
                None,
                None,
                Vec::new(),
            ),
            RepairReview::ContinuedPair {
                previous,
                validation_source,
                grants,
            } => {
                let findings = previous.completed_findings()?;
                (
                    false,
                    Some(CompletedReviewLineage::Round(findings)),
                    Some(previous),
                    None,
                    Some(validation_source),
                    grants,
                )
            }
            RepairReview::ContinuedPairAfterValidation {
                previous,
                validation,
                grants,
            } => {
                let findings = previous.completed_findings()?;
                (
                    false,
                    Some(CompletedReviewLineage::Round(findings)),
                    Some(previous),
                    Some(validation),
                    None,
                    grants,
                )
            }
        };
        self.current_child_claim(input.admission, &input.original)?;
        input.verify_paired_resume(&self.host.operations()?)?;
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
        input.verify_paired_resume(&self.host.operations()?)?;
        let predecessor_receipt = closed_run(&self.host, input.original.operation_id())?;
        let failure_receipt = closed_run(&self.host, failure.operation_id)?;
        let reference = crate::control::run_reference_from_receipt(&predecessor_receipt)?
            .ok_or_else(|| {
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
            if let Some(review) = crate::control::run_reference_from_receipt(&receipt)?
                && review.workspace == reference.workspace
                && review.phase.work == reference.phase.work
                && review.phase.generation == reference.phase.generation
                && review.phase.phase == "child_codex_review"
            {
                closed_run(&self.host, receipt.operation_id)?;
                if !matches!(&granted_review, Some(CompletedReviewLineage::Round(_))) {
                    review_rounds_consumed =
                        review_rounds_consumed.checked_add(1).ok_or_else(|| {
                            Error::new(
                                "attempts_exhausted",
                                "review round count exceeds finite range",
                            )
                        })?;
                }
            }
        }
        if let Some(CompletedReviewLineage::Round(completed)) = &granted_review {
            review_rounds_consumed =
                retained_round_scalar_floor(&self.host.operations()?, &reference)?
                    .max(completed.plan().reference().ordinal);
        }
        if review_round && review_rounds_consumed == 0 {
            return Err(Error::new(
                "worker_binding",
                "review findings lack the actual admitted review round",
            ));
        }
        let failure_phase = crate::control::run_reference_from_receipt(&failure_receipt)?
            .ok_or_else(|| {
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
        if let Some(previous) = &paired_repair {
            if previous.subject_phase() != &reference.phase
                || previous.reference().repair_findings().is_none()
                || granted_review
                    .as_ref()
                    .is_none_or(|r| r.phase() != &previous.reference().review_phase)
            {
                return Err(Error::new(
                    "review_round_lineage",
                    "continued validation loses its actual original Findings pair",
                ));
            }
            if let Some(validation) = &granted_validation {
                validation.verify_current(&inventory, &validation_grants)?;
                if validation.outcome().digest() != outcome.digest()
                    || !Arc::ptr_eq(validation.completion(), &failure)
                    || validation.phase() != &failure_phase.phase
                {
                    return Err(Error::new(
                        "worker_binding",
                        "paired retry validation does not bind actual failed checks",
                    ));
                }
            } else if let Some(source) = &source_validation {
                source.verify_completed_inventory(
                    &inventory,
                    &validation_grants,
                    failure.operation_id,
                )?;
                if source.source_operation() != predecessor_receipt.operation_id
                    || source.source_receipt_digest() != canonical_digest(1, &predecessor_receipt)?
                    || source.phase() != &failure_phase.phase
                    || source.source().digest()? != owned.digest()?
                {
                    return Err(Error::new(
                        "worker_binding",
                        "paired source validation differs from actual failed checks",
                    ));
                }
                source.verify_contract(failure.contract.as_ref().ok_or_else(|| {
                    Error::new("worker_binding", "source validator lost actual contract")
                })?)?;
            } else {
                require_latest_phase(&inventory, predecessor_receipt.runtime_id, &failure_phase)?;
            }
        } else if let Some(review) = &granted_review {
            if !review_round
                || review.outcome().digest() != outcome.digest()
                || !Arc::ptr_eq(review.completion(), &failure)
                || review.phase() != &failure_phase.phase
            {
                return Err(Error::new(
                    "worker_binding",
                    "granted findings differ from actual completed review lineage",
                ));
            }
        } else {
            require_latest_phase(&inventory, predecessor_receipt.runtime_id, &failure_phase)?;
        }
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
        let mut identity = json!({"purpose":"unfinished_child_work","predecessor":predecessor_receipt_digest,
            "failure":failure_receipt_digest,"failure_inventory":failure_phase_inventory_digest,"source":owned.digest()?,"authority":input.admission.gate.digest()?,
            "remaining_task":remaining_task,"progress":progress_digest,"review_round":review_round,"granted_review":granted_review.as_ref().map(CompletedReviewLineage::digest),
            "paired_repair":paired_repair.as_ref().map(|p|p.digest()),
            "granted_validation":granted_validation.as_ref().map(|v|v.digest())});
        if let Some(source) = &source_validation
            && source.has_validation_budget()
        {
            identity["source_validation"] = json!(source.digest());
        }
        if let Some(proof) = &input.paired_completion {
            identity["paired_completion"] = json!(proof.digest());
        }
        let digest = canonical_digest(1, &identity)?;
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
                granted_review,
                paired_repair,
                granted_validation,
                source_validation,
                paired_completion: input.paired_completion,
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
    let expected_cwd = record
        .codex
        .as_ref()
        .and_then(|provenance| provenance.plan.as_ref())
        .and_then(|plan| {
            plan.requirements
                .iter()
                .find(|requirement| requirement.id == record.check_name)
        })
        .map(|requirement| {
            contract.isolation().with_cwd(
                &contract
                    .isolation()
                    .plan()
                    .repository
                    .join(&requirement.cwd),
            )
        })
        .transpose()?
        .map(|isolation| isolation.plan().cwd.clone())
        .unwrap_or_else(|| contract.isolation().plan().repository.clone());
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
        || record.command.cwd != expected_cwd
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
    kegari_git::validate_oid(tree)?;
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
    source_completion: Arc<RunCompletion>,
    granted_review: Option<CompletedReviewLineage>,
    validation_budget: Option<Arc<ValidationBudgetLineage>>,
    paired_completion: Option<Arc<VerifiedCompletedPairedResume>>,
}
impl VerifiedSourceAdvanced {
    /// Observation-only exact closure captured by the actual opaque source.
    /// A caller must still verify current source/grants before capturing it.
    pub(crate) fn retained_receipt_references(&self) -> Result<BTreeMap<Uuid, String>> {
        let mut refs = BTreeMap::new();
        if let Some(proof) = &self.paired_completion {
            refs = proof.retained_receipts()?;
        }
        refs.insert(self.predecessor(), self.predecessor_receipt_digest().into());
        if let Some(old) = refs.insert(self.source_operation(), self.source_receipt_digest().into())
            && old != self.source_receipt_digest()
        {
            return Err(Error::new(
                "review_round_lineage",
                "source ancestry aliases different receipts",
            ));
        }
        if let Some(lineage) = &self.granted_review {
            match lineage {
                CompletedReviewLineage::Round(review) => {
                    for (id, digest) in review.retained_receipt_references()? {
                        if let Some(old) = refs.insert(id, digest.clone())
                            && old != digest
                        {
                            return Err(Error::new(
                                "review_round_lineage",
                                "source ancestry has conflicting exact receipts",
                            ));
                        }
                    }
                }
                #[cfg(test)]
                CompletedReviewLineage::Legacy(_) => {
                    return Err(Error::new(
                        "review_round_lineage",
                        "legacy review lacks typed immutable ancestry",
                    ));
                }
            }
        }
        if let Some(budget) = &self.validation_budget {
            for (id, digest) in budget.retained() {
                if let Some(old) = refs.insert(*id, digest.clone())
                    && old != *digest
                {
                    return Err(Error::new(
                        "validation_source_lineage",
                        "source ancestry aliases different receipts",
                    ));
                }
            }
        }
        Ok(refs)
    }
    pub(crate) fn has_validation_budget(&self) -> bool {
        self.validation_budget.is_some()
    }
    pub(crate) fn verify_joined(
        &self,
        retained: &impl Fn(Uuid) -> Result<Option<Arc<RunCompletion>>>,
    ) -> Result<()> {
        crate::delivery::lineage::verify_joined(self, retained)
    }
    pub(crate) fn retained_validation_grants(&self) -> Vec<crate::decisions::VerifiedRetryGrant> {
        self.validation_budget
            .as_ref()
            .map(|budget| budget.grants().to_vec())
            .unwrap_or_default()
    }
    pub(crate) fn verify_retained_validation_budget(
        &self,
        receipts: &[Receipt],
        grants: &[crate::decisions::VerifiedRetryGrant],
    ) -> Result<()> {
        if let Some(budget) = &self.validation_budget {
            budget.verify_retained(receipts, grants)?;
        }
        Ok(())
    }
    pub(super) fn previous_completion(&self) -> &Arc<RunCompletion> {
        &self._previous_completion
    }
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
    pub(crate) fn requires_review_round(&self) -> bool {
        self.phase().phase == "child_codex_review"
            && matches!(&self.granted_review, Some(CompletedReviewLineage::Round(_)))
    }
    pub(crate) fn has_granted_review(&self) -> bool {
        self.granted_review
            .as_ref()
            .is_some_and(|review| review.phase().retry_grant.is_some())
    }
    pub fn verify_current(
        &self,
        receipts: &[Receipt],
        grants: &[crate::decisions::VerifiedRetryGrant],
    ) -> Result<()> {
        if let Some(proof) = &self.paired_completion {
            proof.verify_immutable(receipts, grants)?;
        }

        self.verify_predecessor_inventory(receipts)?;
        if let Some(review) = &self.granted_review {
            review.verify_current(receipts, grants)?;
        }
        if let Some(budget) = &self.validation_budget {
            budget.verify_current(receipts, grants, false)?;
        }
        self.source().revalidate_filesystem()
    }
    /// Only the authenticated failed-origin retry reader calls this with the
    /// exact prefix it derived from the original model and verified retry chain.
    /// The original model already consumed this source cause; a native retry
    /// grant may supersede its budget without erasing immutable source ancestry.
    pub(super) fn verify_consumed_validation_prefix(
        &self,
        prefix: &[Receipt],
        grants: &[crate::decisions::VerifiedRetryGrant],
        origin: &Receipt,
    ) -> Result<()> {
        self.verify_admission_reference(origin)?;
        self.verify_next_run_header(origin, "failed")?;
        self.verify_predecessor_inventory(prefix)?;
        if let Some(budget) = &self.validation_budget {
            budget.verify_current(prefix, grants, true)?;
        } else if let Some(review) = &self.granted_review {
            review.verify_current(prefix, grants)?;
        }
        self.source().revalidate_filesystem()
    }
    pub(crate) fn verify_completed_inventory(
        &self,
        receipts: &[Receipt],
        grants: &[crate::decisions::VerifiedRetryGrant],
        operation: Uuid,
    ) -> Result<()> {
        let receipt = receipts
            .iter()
            .find(|r| r.operation_id == operation)
            .ok_or_else(|| {
                Error::new(
                    "reconciliation_required",
                    "advanced review admission disappeared",
                )
            })?;
        self.verify_next_run_header(receipt, "completed")?;
        let prior: Vec<_> = receipts
            .iter()
            .filter(|r| r.operation_id != operation)
            .cloned()
            .collect();
        self.verify_current(&prior, grants)
    }
    fn verify_next_run_header(&self, receipt: &Receipt, outcome: &str) -> Result<()> {
        self.verify_admission_reference(receipt)?;
        let run = crate::control::run_reference_from_receipt(receipt)?
            .ok_or_else(|| Error::new("worker_binding", "advanced review admission missing"))?;
        let result = receipt.observed_result.as_ref().ok_or_else(|| {
            Error::new(
                "reconciliation_required",
                "advanced review has no terminal observation",
            )
        })?;
        if receipt.runtime_id != self.predecessor.runtime_id
            || run.workspace != self.reference.workspace
            || run.account != self.reference.account
            || run.phase != self.reference.phase
            || Some(run.attempt) != self.reference.attempt.checked_add(1)
            || run.attempt > run.max_attempts
            || [run.max_attempts, run.max_turns, run.max_review_rounds].contains(&0)
            || run.max_attempts > self.reference.max_attempts
            || run.max_turns > self.reference.max_turns
            || run.max_review_rounds > self.reference.max_review_rounds
            || run.profile_digest != self.reference.profile_digest
            || receipt.phase != ReceiptPhase::Reconciled
            || result["local_execution"] != "exited"
            || result["unresolved_effects"]
                .as_array()
                .is_none_or(|v| !v.is_empty())
            || result["observation"]["outcome"] != outcome
        {
            return Err(Error::new(
                "stale_result",
                "advanced review differs from exact completed next admission",
            ));
        }
        Ok(())
    }
    /// The completed validation is now immutable evidence consumed by a genuine
    /// later paired repair. Only its own exact admitted operation is projected
    /// from the validation inventory; every unrelated/later validator remains.
    pub(super) fn verify_completed_validation_history(
        &self,
        receipts: &[Receipt],
        grants: &[crate::decisions::VerifiedRetryGrant],
        outcome: &VerifiedAuthorizedOutcome,
        completion: &Arc<RunCompletion>,
    ) -> Result<()> {
        let receipt = receipts
            .iter()
            .find(|r| r.operation_id == completion.operation_id)
            .ok_or_else(|| {
                Error::new(
                    "reconciliation_required",
                    "completed source validation disappeared",
                )
            })?;
        self.verify_next_run_header(receipt, "completed")?;
        child::completed_run(outcome, completion)?;
        if receipt.expected != outcome.run_receipt().expected
            || receipt
                .observed_result
                .as_ref()
                .and_then(|r| r.get("observation"))
                != Some(&completion.observation())
        {
            return Err(Error::new(
                "worker_binding",
                "completed source validation differs from actual outcome",
            ));
        }
        self.verify_contract(completion.contract.as_ref().ok_or_else(|| {
            Error::new("worker_binding", "completed validator lost actual contract")
        })?)?;
        let prior = receipts
            .iter()
            .filter(|r| r.operation_id != completion.operation_id)
            .cloned()
            .collect::<Vec<_>>();
        self.verify_predecessor_inventory(&prior)?;
        let budget = self.validation_budget.as_ref().ok_or_else(|| {
            Error::new(
                "validation_source_lineage",
                "completed source lacks granted budget ancestry",
            )
        })?;
        budget.verify_retained(receipts, grants)?;
        self.source().revalidate_filesystem()
    }
    fn verify_admission_reference(&self, receipt: &Receipt) -> Result<()> {
        let expected = crate::control::SourceAdvancedReference {
            predecessor: self.predecessor(),
            predecessor_receipt_digest: self.predecessor_receipt_digest().into(),
            source_operation: self.source_operation(),
            source_receipt_digest: self.source_receipt_digest().into(),
            trigger_digest: self.digest().into(),
            progress_digest: self.progress_digest().into(),
            review_rounds_consumed: self.review_rounds_consumed(),
        };
        if crate::control::SourceAdvancedReference::from_receipt(receipt)?.as_ref()
            != Some(&expected)
        {
            return Err(Error::new(
                "worker_binding",
                "completed review was not admitted by this exact source advancement",
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
        self.source_advanced_inner(request, None, None).await
    }
    pub(crate) async fn source_advanced_retried_review(
        &self,
        request: SourceAdvancedRequest<'_>,
        repair: &UnfinishedChildWork,
        grants: &[crate::decisions::VerifiedRetryGrant],
    ) -> Result<VerifiedSourceAdvanced> {
        let lineage = repair.granted_review().ok_or_else(|| {
            Error::new(
                "worker_binding",
                "repair lacks completed granted review lineage",
            )
        })?;
        lineage.verify_current(&self.host.operations()?, grants)?;
        let source_receipt = self
            .host
            .operation(request.source.original.operation_id())?;
        let continuation_origin = if let Some(proof) = &request.source.paired_completion {
            proof.verify_completion(&request.source.completion)?;
            proof.verify_immutable(&self.host.operations()?, grants)?;
            self.host.operation(proof.origin_operation())?
        } else {
            source_receipt.clone()
        };
        let continuation =
            crate::control::RunContinuationReference::from_receipt(&continuation_origin)?
                .ok_or_else(|| {
                    Error::new(
                        "worker_binding",
                        "changed source lacks its actual scoped repair admission",
                    )
                })?;
        if continuation.predecessor != repair.predecessor()
            || continuation.predecessor_receipt_digest != repair.predecessor_receipt_digest()
            || continuation.failure_operation != repair.failure_operation()
            || continuation.trigger_digest != repair.digest()
            || continuation.progress_digest != repair.progress_digest()
            || continuation.review_rounds_consumed != repair.review_rounds_consumed()
        {
            return Err(Error::new(
                "worker_binding",
                "changed source follows another repair obligation",
            ));
        }
        if request.assignment.phase() != "child_codex_review"
            || request.predecessor.digest() != lineage.outcome().digest()
            || !Arc::ptr_eq(&request.completion, lineage.completion())
        {
            return Err(Error::new(
                "worker_binding",
                "advanced source does not follow its actual granted review",
            ));
        }
        self.source_advanced_inner(request, Some(lineage.clone()), None)
            .await
    }
    /// Validation of a paired source retains the same actual ancestry but does
    /// not allocate a review round. Its own attempt/turn budget stays unchanged.
    pub(crate) async fn source_advanced_paired_validation(
        &self,
        request: SourceAdvancedRequest<'_>,
        repair: &UnfinishedChildWork,
        grants: &[crate::decisions::VerifiedRetryGrant],
    ) -> Result<VerifiedSourceAdvanced> {
        let lineage = repair.granted_review().ok_or_else(|| {
            Error::new(
                "review_round_lineage",
                "paired validation lacks completed review ancestry",
            )
        })?;
        #[cfg(not(test))]
        let CompletedReviewLineage::Round(completed) = lineage;
        #[cfg(test)]
        let CompletedReviewLineage::Round(completed) = lineage else {
            return Err(Error::new(
                "review_round_unavailable",
                "legacy pair has no exact round ancestry",
            ));
        };
        lineage.verify_current(&self.host.operations()?, grants)?;
        let actual = self
            .host
            .operation(request.source.original.operation_id())?;
        let continuation_origin = if let Some(proof) = &request.source.paired_completion {
            proof.verify_completion(&request.source.completion)?;
            proof.verify_immutable(&self.host.operations()?, grants)?;
            self.host.operation(proof.origin_operation())?
        } else {
            actual.clone()
        };
        let link = crate::control::RunContinuationReference::from_receipt(&continuation_origin)?
            .ok_or_else(|| {
                Error::new(
                    "worker_binding",
                    "validation source is not the exact paired repair",
                )
            })?;
        let marker = crate::control::ReviewRoundReference::from_receipt(&actual)?
            .ok_or_else(|| Error::new("review_round_lineage", "paired source marker is missing"))?;
        let findings = completed.findings()?;
        if request.assignment.phase() != "child_local_validation"
            || marker.repair_findings() != Some(&findings)
            || marker.source != completed.plan().reference().source
            || marker.review_phase != *completed.phase()
            || marker.round_limit > completed.plan().reference().round_limit
            || link.predecessor != repair.predecessor()
            || link.predecessor_receipt_digest != repair.predecessor_receipt_digest()
            || link.failure_operation != repair.failure_operation()
            || link.trigger_digest != repair.digest()
            || link.progress_digest != repair.progress_digest()
            || link.review_rounds_consumed != repair.review_rounds_consumed()
        {
            return Err(Error::new(
                "worker_binding",
                "validation source changes its exact pair",
            ));
        }
        self.source_advanced_inner(request, Some(lineage.clone()), None)
            .await
    }
    #[cfg(test)]
    pub(crate) async fn publish_source_advanced_review(
        &self,
        admission: &DeliveryAdmission,
        assignment: &ChildReviewAssignment,
        outcome: &VerifiedAuthorizedOutcome,
        completion: &Arc<RunCompletion>,
        proof: &VerifiedSourceAdvanced,
        current_grant: GateCheck,
    ) -> Result<PublishedReview> {
        child::completed_run(outcome, completion)?;
        proof.verify_contract(
            completion
                .contract
                .as_ref()
                .ok_or_else(|| Error::new("worker_binding", "advanced review contract missing"))?,
        )?;
        let receipt = self.host.operation(completion.operation_id)?;
        proof.verify_admission_reference(&receipt)?;
        let run = crate::control::run_reference_from_receipt(&receipt)?.ok_or_else(|| {
            Error::new("worker_binding", "advanced review lacks actual admission")
        })?;
        if run.phase != *proof.phase()
            || run.instruction_digest != outcome.outcome().instruction_digest
        {
            return Err(Error::new(
                "worker_binding",
                "advanced result differs from its completed admission",
            ));
        }
        let git = self.git.clone();
        let source = proof.source().clone();
        let execution = completion
            .execution
            .clone()
            .ok_or_else(|| Error::new("worker_binding", "advanced review lacks owned source"))?;
        let isolation = completion.contract.as_ref().unwrap().isolation().clone();
        tokio::task::spawn_blocking(move || {
            source.verify_for_clone(&git)?;
            execution.ownership.verify(&execution.binding, &isolation)?;
            if execution.binding.revision != *source.revision() {
                return Err(Error::new("stale_result", "advanced review source differs"));
            }
            git.verify_execution_tree(
                &isolation.plan().repository,
                source.revision(),
                &BTreeSet::new(),
            )
        })
        .await
        .map_err(|_| {
            Error::new(
                "operation_uncertain",
                "advanced review source verification was lost",
            )
        })??;
        self.publish_completed_review_guarded(
            admission,
            assignment,
            outcome,
            completion,
            Some(current_grant),
        )
        .await
    }
    async fn source_advanced_inner(
        &self,
        request: SourceAdvancedRequest<'_>,
        granted_review: Option<CompletedReviewLineage>,
        validation_budget: Option<Arc<ValidationBudgetLineage>>,
    ) -> Result<VerifiedSourceAdvanced> {
        let admission = request.source.admission;
        self.current_child_claim(admission, &request.source.original)?;
        request
            .source
            .verify_paired_resume(&self.host.operations()?)?;
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
        request
            .source
            .verify_paired_resume(&self.host.operations()?)?;
        let source_operation = request.source.original.operation_id();
        let source_receipt = closed_run(&self.host, source_operation)?;
        let reference = crate::control::run_reference_from_receipt(&predecessor)?
            .ok_or_else(|| Error::new("worker_binding", "previous check lacks worker admission"))?;
        let source_run =
            crate::control::run_reference_from_receipt(&source_receipt)?.ok_or_else(|| {
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
            if let Some(run) = crate::control::run_reference_from_receipt(&receipt)? {
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
                    if !matches!(&granted_review, Some(CompletedReviewLineage::Round(_))) {
                        review_rounds_consumed =
                            review_rounds_consumed.checked_add(1).ok_or_else(|| {
                                Error::new("attempts_exhausted", "review round count overflow")
                            })?;
                    }
                }
            }
        }
        let inventory = self.host.operations()?;
        if let Some(CompletedReviewLineage::Round(completed)) = &granted_review {
            review_rounds_consumed = retained_round_scalar_floor(&inventory, &reference)?
                .max(completed.plan().reference().ordinal);
        }
        if let Some(budget) = &validation_budget {
            budget.verify_request(&inventory, &predecessor, &reference, &source_receipt)?;
        } else if granted_review.is_none() || reference.phase.phase != "child_codex_review" {
            require_latest_phase(&inventory, predecessor.runtime_id, &reference)?;
        }
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
        let mut assignment = request.assignment.clone();
        if assignment.phase() == "child_local_validation"
            && let Some(prefix) =
                crate::validation::ValidationPrefix::from_completion(&request.completion)?
        {
            assignment.continue_validation(prefix)?;
        }
        let mut identity = json!({"purpose":"source_advanced","previous":predecessor_digest,"previous_inventory":phase_inventory_digest,"source":source_receipt_digest,
            "source_seal":request.source.source.receipt().payload_digest,"assignment":assignment.digest()?,"progress":progress_digest,"review_rounds_consumed":review_rounds_consumed,"granted_review":granted_review.as_ref().map(CompletedReviewLineage::digest)});
        if let Some(budget) = &validation_budget {
            identity["validation_budget"] = json!(budget.digest());
        }
        if let Some(proof) = &request.source.paired_completion {
            identity["paired_completion"] = json!(proof.digest());
        }
        let digest = canonical_digest(1, &identity)?;
        Ok(VerifiedSourceAdvanced {
            assignment,
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
            source_completion: request.source.completion,
            // The validation budget already flattened this genuine completed
            // round's closure. Keeping it here would recreate an Arc back-chain.
            granted_review: if validation_budget.is_some() {
                None
            } else {
                granted_review
            },
            validation_budget,
            paired_completion: request.source.paired_completion,
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
        if let Some(run) = crate::control::run_reference_from_receipt(receipt)?
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
        if let Some(run) = crate::control::run_reference_from_receipt(receipt)?
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

// Legacy scalar bytes remain a monotone audit floor. Versioned round allocation
// is independently namespaced and never counts transient attempts as new rounds.
fn retained_round_scalar_floor(receipts: &[Receipt], subject: &RunReference) -> Result<u32> {
    let mut floor = 0;
    for receipt in receipts {
        if let Some(run) = crate::control::run_reference_from_receipt(receipt)?
            && run.phase == subject.phase
            && run.workspace == subject.workspace
        {
            if let Some(value) = crate::control::RunContinuationReference::from_receipt(receipt)? {
                floor = floor.max(value.review_rounds_consumed);
            }
            if let Some(value) = crate::control::SourceAdvancedReference::from_receipt(receipt)? {
                floor = floor.max(value.review_rounds_consumed);
            }
            if let Some(value) = crate::control::RunResumeReference::from_receipt(receipt)? {
                floor = floor.max(value.review_rounds_consumed);
            }
        }
    }
    Ok(floor)
}

impl UnfinishedChildWork {
    pub(crate) fn resumed_bridge_inventory(
        &self,
    ) -> Result<BTreeMap<Uuid, Arc<crate::worker_run::AdmittedSessionResume>>> {
        Ok(crate::delivery::lineage::collect(self)?.bridges)
    }
    pub(crate) fn verify_resumed_bridges(
        &self,
        reader: &dyn Fn(Uuid) -> Result<Option<Arc<crate::worker_run::AdmittedSessionResume>>>,
    ) -> Result<()> {
        review_round::verify_resumed_inventory(self.resumed_bridge_inventory()?, reader)
    }
}
impl VerifiedSourceAdvanced {
    pub(crate) fn resumed_bridge_inventory(
        &self,
    ) -> Result<BTreeMap<Uuid, Arc<crate::worker_run::AdmittedSessionResume>>> {
        Ok(crate::delivery::lineage::collect(self)?.bridges)
    }
    pub(crate) fn verify_resumed_bridges(
        &self,
        reader: &dyn Fn(Uuid) -> Result<Option<Arc<crate::worker_run::AdmittedSessionResume>>>,
    ) -> Result<()> {
        review_round::verify_resumed_inventory(self.resumed_bridge_inventory()?, reader)
    }
}

impl RepairSource<'_> {
    fn verify_paired_resume(&self, receipts: &[Receipt]) -> Result<()> {
        let actual = receipts
            .iter()
            .find(|r| r.operation_id == self.completion.operation_id)
            .ok_or_else(|| {
                Error::new(
                    "reconciliation_required",
                    "repair source receipt unavailable",
                )
            })?;
        match (
            &self.paired_completion,
            crate::control::ReviewRoundResumeReference::from_model_receipt(actual)?,
        ) {
            (Some(proof), Some(_)) => {
                proof.verify_completion(&self.completion)?;
                proof.verify_receipts(receipts)
            }
            (None, None) => Ok(()),
            _ => Err(Error::new(
                "paired_recovery_lineage",
                "source omits or invents its completed paired resume capability",
            )),
        }
    }
}

impl VerifiedSourceAdvanced {
    pub(crate) fn retained_native_grants(
        &self,
    ) -> Result<BTreeMap<Uuid, crate::decisions::VerifiedRetryGrant>> {
        crate::delivery::lineage::collect(self)?.into_grants()
    }
}

impl crate::delivery::lineage::Node for UnfinishedChildWork {
    fn key(&self) -> crate::delivery::lineage::Key {
        crate::delivery::lineage::Key::of(self.inner.as_ref())
    }
    fn expand<'a>(
        &'a self,
        children: &mut Vec<&'a dyn crate::delivery::lineage::Node>,
        leaves: &mut crate::delivery::lineage::Inventory,
    ) -> Result<()> {
        leaves.sources([self.source().clone()])?;
        if let Some(p) = &self.inner.paired_completion {
            children.push(p.as_ref());
        }
        if let Some(p) = &self.inner.granted_review {
            children.push(p);
        }
        if let Some(p) = &self.inner.paired_repair {
            children.push(p.as_ref());
        }
        if let Some(p) = &self.inner.granted_validation {
            children.push(p.as_ref());
        }
        if let Some(p) = &self.inner.source_validation {
            children.push(p.as_ref());
        }
        Ok(())
    }
}
impl crate::delivery::lineage::Node for VerifiedSourceAdvanced {
    fn expand<'a>(
        &'a self,
        children: &mut Vec<&'a dyn crate::delivery::lineage::Node>,
        leaves: &mut crate::delivery::lineage::Inventory,
    ) -> Result<()> {
        leaves.sources([self.source().clone()])?;
        leaves.joined(2)?;
        if let Some(p) = &self.paired_completion {
            children.push(p.as_ref());
        }
        if let Some(p) = &self.granted_review {
            children.push(p);
        }
        if let Some(p) = &self.validation_budget {
            children.push(p.as_ref());
        }
        Ok(())
    }
    fn verify_joined_leaf(
        &self,
        reader: &dyn Fn(Uuid) -> Result<Option<Arc<RunCompletion>>>,
    ) -> Result<()> {
        for expected in [&self._previous_completion, &self.source_completion] {
            if reader(expected.operation_id)?.is_none_or(|actual| {
                !Arc::ptr_eq(&actual, expected) || !actual.local_exit_confirmed
            }) {
                return Err(Error::new(
                    "reconciliation_required",
                    "source advancement lost actual joined ancestor",
                ));
            }
        }
        Ok(())
    }
}

impl crate::delivery::lineage_drop::Release for UnfinishedChildWork {
    fn release(self, pending: &mut crate::delivery::lineage_drop::Pending) {
        pending.push(self.inner);
    }
}
impl crate::delivery::lineage_drop::Release for UnfinishedProof {
    fn release(self, pending: &mut crate::delivery::lineage_drop::Pending) {
        if let Some(p) = self.granted_review {
            crate::delivery::lineage_drop::Release::release(p, pending);
        }
        if let Some(p) = self.paired_repair {
            pending.push(p);
        }
        if let Some(p) = self.granted_validation {
            pending.push(p);
        }
        if let Some(p) = self.source_validation {
            pending.push(p);
        }
        // Paired completion and all other fields are flat leaf ownership.
    }
}
impl crate::delivery::lineage_drop::Release for VerifiedSourceAdvanced {
    fn release(self, pending: &mut crate::delivery::lineage_drop::Pending) {
        if let Some(p) = self.granted_review {
            crate::delivery::lineage_drop::Release::release(p, pending);
        }
        // validation_budget/paired_completion contain flat maps, not old plans.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::ExecutionBinding;
    use crate::execution::ExecutionEvidenceClass;
    use crate::execution::SourceSnapshot;
    use kegari_execution::CommandOutcome;
    use kegari_git::Revision;
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
            codex: None,
            discovered: None,
            schema_version: 1,
            correction: None,
            bundle: None,
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
                termination_signal: None,
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
