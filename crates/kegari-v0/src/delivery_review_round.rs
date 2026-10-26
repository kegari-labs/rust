//! Child review-round producers, not admission authority.
//! All constructors are crate-only and consume existing opaque source/cause
//! capabilities. The native consumer must recheck actual joined Arc identity.
//! No serde implementation can recreate any type in this module.
use super::*;
use crate::{
    config::Config,
    control::{
        FindingsReference, PreparationReference, ReceiptPhase, RepairValidationReference,
        ReviewReceiptReference, ReviewRoundCause, ReviewRoundReference, ReviewedSourceReference,
        RunReference, TransientFailureCode,
    },
    decisions::VerifiedRetryGrant,
    instructions::AgentRole,
    runtime::VerifiedAuthorizedOutcome,
    scheduler::PhaseKey,
    worker::{WorkerAuthority, WorkerContract},
    worker_run::{RunCompletion, RunOutcome},
};

#[path = "delivery_review_validation.rs"]
mod validation_lineage;
use validation_lineage::ValidationEvidence;
#[path = "delivery_paired_recovery.rs"]
mod paired_recovery;
use paired_recovery::PairedCompletionEvidence;
pub(crate) use paired_recovery::{
    PairedRecoveryInput, PairedRecoveryReaders, VerifiedCompletedPairedResume,
    VerifiedPairedRecoveryCandidate,
};
#[path = "delivery_review_passed.rs"]
mod passed_lineage;
pub(crate) use passed_lineage::{PassedValidationSuccessor, verify_validation_successor_admission};

const MAX_RECEIPTS: usize = 65_536;

/// Inputs already obtained through the real child/source pipeline. The source
/// seal is borrowed for observation; the resulting proof retains its whole
/// actual receipt and the original actual source/claim/completion capabilities.
pub(crate) struct ReviewRoundSource<'a> {
    pub admission: &'a DeliveryAdmission,
    pub claim: Arc<CompletedChildClaim>,
    pub source: &'a SealedChildSource,
    pub completion: Arc<RunCompletion>,
    pub assignment: &'a ChildReviewAssignment,
    pub validated_retry: Option<Arc<VerifiedPassedValidation>>,
    pub paired_completion: Option<Arc<VerifiedCompletedPairedResume>>,
}
#[derive(Clone)]
struct SourceEvidence {
    reference: ReviewedSourceReference,
    source: DeliverySource,
    claim: Arc<CompletedChildClaim>,
    completion: Arc<RunCompletion>,
    paired_completion: PairedCompletionEvidence,
}
#[derive(Clone)]
enum RoundBasis {
    Initial,
    /// First session of a genuine new operator-granted review budget. This is
    /// distinct from an automatically retryable failure within the same budget.
    ExplicitGrant(Arc<ReviewRetryWork>),
    Transient {
        previous: Arc<VerifiedReviewRoundPlan>,
        failed: Arc<RunCompletion>,
        receipt: Receipt,
    },
    PairedRepair {
        findings: Arc<CompletedReviewRound>,
        unfinished: Arc<UnfinishedChildWork>,
    },
    RepairContinuation {
        previous: Arc<VerifiedReviewRoundPlan>,
        unfinished: Arc<UnfinishedChildWork>,
        validation_source: ValidationEvidence,
        validation_outcome: Box<VerifiedAuthorizedOutcome>,
        validation_completion: Arc<RunCompletion>,
    },
    AfterRepair {
        paired: Arc<VerifiedReviewRoundPlan>,
        repair: Receipt,
        advanced: Arc<VerifiedSourceAdvanced>,
    },
}

/// Orthogonal to the existing run cause. A clone only retains the exact sealed
/// predecessor/source; it cannot select a new phase, task or permission set.
#[derive(Clone)]
pub struct VerifiedReviewRoundPlan {
    reference: ReviewRoundReference,
    assignment: DeliveryWorkerAssignment,
    source: Arc<SourceEvidence>,
    basis: RoundBasis,
    inventory_digest: String,
    source_inventory_digest: String,
    retained: BTreeMap<Uuid, String>,
    grant: Option<VerifiedRetryGrant>,
    // Retained native identities are checked without reusing an old current
    // validation inventory after its exact paired successor has been admitted.
    validation_grants: BTreeMap<Uuid, VerifiedRetryGrant>,
    passed_validation: Option<Arc<VerifiedPassedValidation>>,
    digest: String,
}
impl VerifiedReviewRoundPlan {
    pub(crate) fn reference(&self) -> &ReviewRoundReference {
        &self.reference
    }
    pub(crate) fn digest(&self) -> &str {
        &self.digest
    }
    pub(crate) fn assignment(&self) -> &DeliveryWorkerAssignment {
        &self.assignment
    }
    pub(crate) fn authority(&self) -> &FreshGate {
        self.assignment.authority()
    }
    pub(crate) fn subject_phase(&self) -> &PhaseKey {
        self.reference.subject_phase()
    }
    pub(crate) fn verify_assignment(&self, assignment: &DeliveryWorkerAssignment) -> Result<()> {
        if assignment.digest()? != self.assignment.digest()? {
            return Err(blocked("round assignment or source changed"));
        }
        self.source.source.revalidate_filesystem()
    }
    pub(crate) fn verify_contract(&self, contract: &WorkerContract) -> Result<()> {
        if let RoundBasis::PairedRepair { unfinished, .. }
        | RoundBasis::RepairContinuation { unfinished, .. } = &self.basis
        {
            unfinished.verify_contract(contract)?;
        } else {
            verify_review_contract(&self.assignment, contract)?;
        }
        if contract.instructions().manifest.phase != self.subject_phase().phase {
            return Err(blocked("round subject differs from assembled phase"));
        }
        Ok(())
    }
    /// Final callback receives actual protected receipts and current verified
    /// native grants; neither argument is reconstructed from model context.
    pub(crate) fn verify_current(
        &self,
        receipts: &[Receipt],
        grants: &[VerifiedRetryGrant],
    ) -> Result<()> {
        self.reference.validate()?;
        self.source
            .paired_completion
            .verify_immutable(receipts, grants)?;
        self.verify_passed_current(receipts, grants)?;
        if inventory(receipts, &self.reference)? != self.inventory_digest {
            return Err(blocked("review/source admission inventory changed"));
        }
        if implementation_inventory(receipts, &self.reference)? != self.source_inventory_digest {
            return Err(blocked(
                "implementation history changed after round preparation",
            ));
        }
        self.verify_retained_history(receipts, grants)?;
        require_latest_source(receipts, &self.reference, grants)?;
        match &self.basis {
            RoundBasis::Initial => {
                for receipt in receipts {
                    if RunReference::from_receipt(receipt)?
                        .is_some_and(|run| run.phase == self.reference.review_phase)
                    {
                        return Err(blocked("initial round already has a review session"));
                    }
                }
            }
            RoundBasis::ExplicitGrant(proof) => proof.verify_current(receipts, grants)?,
            RoundBasis::Transient {
                previous,
                failed,
                receipt,
            } => {
                require_failed(failed)?;
                previous.verify_terminal_header(receipts, receipt, "failed")?;
                previous.verify_retained_history(receipts, grants)?;
                require_recorded_session(receipt, failed)?;
                let expected = transient_cause(receipt, failed, previous.reference())?;
                if expected != self.reference.cause {
                    return Err(blocked(
                        "transient round lost its actual terminal observation",
                    ));
                }
            }
            RoundBasis::PairedRepair {
                findings,
                unfinished,
            } => {
                findings.verify_current(receipts, grants)?;
                unfinished.verify_current(receipts, grants)?;
                if unfinished.failure_operation() != findings.completion.operation_id
                    || unfinished.failure_receipt_digest() != findings.receipt_digest
                    || unfinished.predecessor() != self.reference.source.implementation.operation
                    || unfinished.predecessor_receipt_digest()
                        != self.reference.source.implementation.receipt_digest
                {
                    return Err(blocked(
                        "paired repair changed Findings or implementation predecessor",
                    ));
                }
            }
            RoundBasis::RepairContinuation {
                previous,
                unfinished,
                validation_source,
                validation_outcome,
                validation_completion,
            } => {
                let repair =
                    actual_receipt(receipts, self.source.reference.implementation.operation)?;
                self.source
                    .paired_completion
                    .verify(previous, receipts, grants, repair)?;
                unfinished.verify_current(receipts, grants)?;
                validation_source.verify_current(receipts, grants, validation_completion)?;
                let validation = actual_receipt(receipts, validation_completion.operation_id)?;
                require_recorded_session(validation, validation_completion)?;
                let ReviewRoundCause::RepairContinuation {
                    findings,
                    previous_round_reference_digest,
                    current_source,
                    validation: reference,
                } = &self.reference.cause
                else {
                    return Err(blocked("continued repair marker is missing"));
                };
                if previous.reference.repair_findings() != Some(findings)
                    || *previous_round_reference_digest != canonical_digest(1, &previous.reference)?
                    || current_source.as_ref() != &self.source.reference
                    || unfinished.predecessor() != current_source.implementation.operation
                    || unfinished.predecessor_receipt_digest()
                        != current_source.implementation.receipt_digest
                    || unfinished.failure_operation() != validation.operation_id
                    || unfinished.failure_receipt_digest() != canonical_digest(1, validation)?
                    || reference.validation != receipt_reference(validation)?
                    || reference.outcome_proposal != validation_outcome.outcome().proposal_id
                    || reference.outcome_claim_digest != validation_outcome.digest()
                    || reference.failure_inventory_digest
                        != unfinished.failure_phase_inventory_digest()
                    || validation_source.reference_lineage(
                        receipts,
                        grants,
                        validation_completion,
                    )? != reference.lineage
                    || crate::control::SourceAdvancedReference::from_receipt(actual_receipt(
                        receipts,
                        reference.source_admission().operation,
                    )?)?
                    .as_ref()
                        != Some(&reference.source_advanced)
                {
                    return Err(blocked(
                        "continued pair differs from actual repair or failed validation",
                    ));
                }
            }
            RoundBasis::AfterRepair {
                paired,
                repair,
                advanced,
            } => {
                self.source
                    .paired_completion
                    .verify(paired, receipts, grants, repair)?;
                advanced.verify_current(receipts, grants)?;
                if advanced.source_operation() != self.reference.source.implementation.operation
                    || advanced.source_receipt_digest()
                        != self.reference.source.implementation.receipt_digest
                    || advanced.phase() != &self.reference.review_phase
                    || advanced.source().digest()? != self.reference.source.source_digest
                {
                    return Err(blocked(
                        "changed-source review differs from the actual paired repair",
                    ));
                }
            }
        }
        self.source.source.revalidate_filesystem()
    }
    fn verify_retained_history(
        &self,
        receipts: &[Receipt],
        grants: &[VerifiedRetryGrant],
    ) -> Result<()> {
        require_source_receipts(&self.source, receipts)?;
        if let Some(passed) = &self.passed_validation {
            passed.verify_retained(receipts, grants)?;
        }
        for (id, digest) in &self.retained {
            if canonical_digest(1, actual_receipt(receipts, *id)?)? != *digest {
                return Err(blocked("retained round ancestry changed or disappeared"));
            }
        }
        for grant in self.validation_grants.values() {
            crate::runtime_workflow::require_current_grant(grant, grants)?;
            if grants
                .iter()
                .filter(|current| current.phase() == grant.phase())
                .count()
                != 1
            {
                return Err(blocked("retained validation grant namespace is ambiguous"));
            }
        }
        require_current_namespace(
            &self.reference,
            self.authority(),
            self.grant.as_ref(),
            grants,
        )?;
        self.source.source.revalidate_filesystem()
    }
    fn verify_terminal_header(
        &self,
        receipts: &[Receipt],
        expected: &Receipt,
        outcome: &str,
    ) -> Result<()> {
        let actual = actual_receipt(receipts, expected.operation_id)?;
        if canonical_digest(1, actual)? != canonical_digest(1, expected)? {
            return Err(blocked("completed round receipt changed"));
        }
        let run = closed_run(actual, outcome)?;
        self.verify_run_cause(actual)?;
        self.verify_run_bounds(&run)?;
        if ReviewRoundReference::from_receipt(actual)?.as_ref() != Some(&self.reference)
            || run.phase != *self.subject_phase()
            || run.workspace != self.reference.workspace
            || run.account != self.reference.account
        {
            return Err(blocked(
                "terminal worker differs from the exact admitted round proposal",
            ));
        }
        let prep_id = crate::control::preparation_operation(actual.operation_id)?;
        let prep = actual_receipt(receipts, prep_id)?;
        ReviewReceiptReference::preparation_for(actual)?.matches_observation(prep)?;
        let prep_ref = PreparationReference::from_receipt(prep)?
            .ok_or_else(|| blocked("round has no typed preparation"))?;
        if prep_ref.worker_operation != actual.operation_id
            || prep.actor != actual.actor
            || prep.owner_generation != actual.owner_generation
            || prep.phase != ReceiptPhase::Reconciled
            || ReviewRoundReference::from_receipt(prep)?.as_ref() != Some(&self.reference)
        {
            return Err(blocked("preparation and model round proposals disagree"));
        }
        Ok(())
    }
    fn verify_terminal(
        &self,
        receipts: &[Receipt],
        grants: &[VerifiedRetryGrant],
        expected: &Receipt,
        outcome: &str,
    ) -> Result<()> {
        self.verify_terminal_header(receipts, expected, outcome)?;
        let prep_id = crate::control::preparation_operation(expected.operation_id)?;
        let before: Vec<_> = receipts
            .iter()
            .filter(|r| r.operation_id != expected.operation_id && r.operation_id != prep_id)
            .cloned()
            .collect();
        self.verify_current(&before, grants)
    }
    pub(crate) fn completed_repair_for_validation(
        &self,
        receipts: &[Receipt],
        grants: &[VerifiedRetryGrant],
        completion: &Arc<RunCompletion>,
        resumed: Option<&Arc<VerifiedCompletedPairedResume>>,
    ) -> Result<Arc<CompletedReviewRound>> {
        if !self.reference.is_repair() {
            return Err(blocked(
                "validation advancement requires an actual paired implementation",
            ));
        }
        let receipt = actual_receipt(receipts, completion.operation_id)?;
        if let Some(proof) = resumed {
            proof.verify_completion(completion)?;
            proof.verify_retained(self, receipts, grants)?;
            return self.completed_findings();
        }
        self.verify_terminal(receipts, grants, receipt, "completed")?;
        require_recorded_session(receipt, completion)?;
        self.verify_contract(
            completion
                .contract
                .as_ref()
                .ok_or_else(|| blocked("paired implementation lost contract"))?,
        )?;
        self.completed_findings()
    }
    pub(crate) fn retained_repair_receipts(
        &self,
        receipts: &[Receipt],
        operation: Uuid,
        resumed: Option<&Arc<VerifiedCompletedPairedResume>>,
    ) -> Result<BTreeMap<Uuid, String>> {
        if let Some(proof) = resumed {
            if proof.reference().terminal.operation != operation
                || proof.reference().round_plan_digest != self.digest()
            {
                return Err(blocked(
                    "resumed repair receipt closure changes original plan",
                ));
            }
            proof.verify_receipts(receipts)?;
            return proof.retained_receipts();
        }
        let actual = actual_receipt(receipts, operation)?;
        self.verify_terminal_header(receipts, actual, "completed")?;
        let mut retained = self.retained.clone();
        retain_terminal(&mut retained, receipts, actual)?;
        Ok(retained)
    }

    fn verify_run_bounds(&self, run: &RunReference) -> Result<()> {
        let previous = match &self.basis {
            RoundBasis::Initial => {
                if run.attempt != 1 {
                    return Err(blocked(
                        "initial round is not the first actual review attempt",
                    ));
                }
                None
            }
            RoundBasis::ExplicitGrant(proof) => {
                if run.attempt != 1
                    || run.max_attempts > proof.grant().new_budget().max_attempts
                    || run.max_turns > proof.grant().new_budget().max_turns
                    || run.max_review_rounds > proof.grant().new_budget().max_review_rounds
                {
                    return Err(blocked(
                        "initial granted review exceeds its actual new budget",
                    ));
                }
                None
            }
            RoundBasis::Transient { receipt, .. } => Some(closed_run(receipt, "failed")?),
            RoundBasis::PairedRepair { unfinished, .. }
            | RoundBasis::RepairContinuation { unfinished, .. } => {
                Some(unfinished.reference().clone())
            }
            RoundBasis::AfterRepair { advanced, .. } => Some(advanced.reference().clone()),
        };
        if let Some(previous) = previous
            && (run.phase != previous.phase
                || run.workspace != previous.workspace
                || run.account != previous.account
                || run.profile_digest != previous.profile_digest
                || Some(run.attempt) != previous.attempt.checked_add(1)
                || run.max_attempts > previous.max_attempts
                || run.max_turns > previous.max_turns
                || run.max_review_rounds > previous.max_review_rounds)
        {
            return Err(blocked(
                "new session changes its exact predecessor or retained phase caps",
            ));
        }
        Ok(())
    }
    fn verify_run_cause(&self, receipt: &Receipt) -> Result<()> {
        match &self.basis {
            RoundBasis::PairedRepair { unfinished, .. }
            | RoundBasis::RepairContinuation { unfinished, .. } => {
                let expected = crate::control::RunContinuationReference {
                    predecessor: unfinished.predecessor(),
                    predecessor_receipt_digest: unfinished.predecessor_receipt_digest().into(),
                    trigger_digest: unfinished.digest().into(),
                    progress_digest: unfinished.progress_digest().into(),
                    failure_operation: unfinished.failure_operation(),
                    review_rounds_consumed: unfinished.review_rounds_consumed(),
                };
                // Data shape only: Control's round-aware validator must verify
                // the same reference atomically without legacy scalar inference.
                let actual: crate::control::RunContinuationReference = serde_json::from_str(
                    receipt
                        .expected
                        .get("continuation")
                        .ok_or_else(|| blocked("paired run lacks exact continuation"))?,
                )
                .map_err(|_| blocked("paired continuation is malformed"))?;
                if actual != expected {
                    return Err(blocked(
                        "paired run consumes a different unfinished trigger",
                    ));
                }
            }
            RoundBasis::AfterRepair { advanced, .. } => {
                let expected = crate::control::SourceAdvancedReference {
                    predecessor: advanced.predecessor(),
                    predecessor_receipt_digest: advanced.predecessor_receipt_digest().into(),
                    source_operation: advanced.source_operation(),
                    source_receipt_digest: advanced.source_receipt_digest().into(),
                    trigger_digest: advanced.digest().into(),
                    progress_digest: advanced.progress_digest().into(),
                    review_rounds_consumed: advanced.review_rounds_consumed(),
                };
                let actual: crate::control::SourceAdvancedReference = serde_json::from_str(
                    receipt
                        .expected
                        .get("source_advanced")
                        .ok_or_else(|| blocked("new-source run lacks exact advancement"))?,
                )
                .map_err(|_| blocked("source advancement is malformed"))?;
                if actual != expected {
                    return Err(blocked("review consumes another source-advanced trigger"));
                }
            }
            _ => {
                if ["continuation", "source_advanced", "session_resume"]
                    .iter()
                    .any(|key| receipt.expected.contains_key(*key))
                {
                    return Err(blocked(
                        "initial or transient review has a conflicting run cause",
                    ));
                }
            }
        }
        Ok(())
    }
    pub(super) fn verify_completed_repair(
        &self,
        receipts: &[Receipt],
        grants: &[VerifiedRetryGrant],
        actual: &Receipt,
    ) -> Result<()> {
        if !self.reference.is_repair() {
            return Err(blocked(
                "completed implementation lacks its actual paired plan",
            ));
        }
        self.verify_terminal_header(receipts, actual, "completed")?;
        self.verify_retained_history(receipts, grants)
    }
    /// Verify the actual completed implementation consumed by another scoped
    /// fix. A resumed implementation retains its real terminal/metadata chain;
    /// it must not be reinterpreted as the original ordinary paired admission.
    pub(crate) fn verify_completed_repair_result(
        &self,
        receipts: &[Receipt],
        grants: &[VerifiedRetryGrant],
        actual: &Receipt,
        resumed: Option<&Arc<VerifiedCompletedPairedResume>>,
    ) -> Result<()> {
        if let Some(proof) = resumed {
            proof.reference().terminal.matches_observation(actual)?;
            proof.verify_retained(self, receipts, grants)
        } else if actual.expected.contains_key("review_round_resume") {
            Err(Error::new(
                "reconciliation_required",
                "resumed repair requires its actual retained completed capability",
            ))
        } else {
            self.verify_completed_repair(receipts, grants, actual)
        }
    }
    pub(super) fn completed_findings(&self) -> Result<Arc<CompletedReviewRound>> {
        let mut current = self;
        let mut seen = BTreeSet::new();
        loop {
            if !seen.insert(current.digest()) || seen.len() > MAX_RECEIPTS {
                return Err(history_limit());
            }
            match &current.basis {
                RoundBasis::PairedRepair { findings, .. } => return Ok(findings.clone()),
                RoundBasis::RepairContinuation { previous, .. } => current = previous,
                _ => {
                    return Err(blocked(
                        "implementation is not part of a retained Findings pair",
                    ));
                }
            }
        }
    }
    /// The runtime must use its genuine joined cache. Public RunCompletion
    /// fields or an arbitrary map of decoded observations are insufficient.
    pub(crate) fn verify_joined(
        &self,
        retained: &impl Fn(Uuid) -> Result<Option<Arc<RunCompletion>>>,
    ) -> Result<()> {
        crate::delivery::lineage::verify_joined(self, retained)
    }
}

/// Completed initial, transient-retried, or changed-source review using one
/// common immutable basis. Never recreated from a claim/receipt DTO.
#[derive(Clone)]
pub(crate) struct CompletedReviewRound {
    plan: Arc<VerifiedReviewRoundPlan>,
    outcome: VerifiedAuthorizedOutcome,
    completion: Arc<RunCompletion>,
    claim: ReviewResultClaim,
    receipt: Receipt,
    receipt_digest: String,
    preparation_digest: String,
    inventory_digest: String,
    digest: String,
}
impl CompletedReviewRound {
    pub(crate) fn retained_receipt_references(&self) -> Result<BTreeMap<Uuid, String>> {
        let mut refs = self.plan.retained.clone();
        for (id, digest) in [
            (self.receipt.operation_id, self.receipt_digest.clone()),
            (
                crate::control::preparation_operation(self.receipt.operation_id)?,
                self.preparation_digest.clone(),
            ),
        ] {
            retain_exact(&mut refs, id, digest)?;
        }
        Ok(refs)
    }
    pub(crate) fn phase(&self) -> &PhaseKey {
        &self.plan.reference.review_phase
    }
    pub(crate) fn digest(&self) -> &str {
        &self.digest
    }
    pub(crate) fn plan(&self) -> &VerifiedReviewRoundPlan {
        &self.plan
    }
    pub(crate) fn outcome(&self) -> &VerifiedAuthorizedOutcome {
        &self.outcome
    }
    pub(crate) fn completion(&self) -> &Arc<RunCompletion> {
        &self.completion
    }
    pub(crate) fn verify_current(
        &self,
        receipts: &[Receipt],
        grants: &[VerifiedRetryGrant],
    ) -> Result<()> {
        if inventory(receipts, self.plan.reference())? != self.inventory_digest {
            return Err(blocked(
                "completed review is no longer the exact latest source/round",
            ));
        }
        self.plan.verify_retained_history(receipts, grants)?;
        let current = actual_receipt(receipts, self.receipt.operation_id)?;
        if canonical_digest(1, current)? != self.receipt_digest
            || ReviewRoundReference::from_receipt(current)?.as_ref() != Some(self.plan.reference())
        {
            return Err(blocked(
                "completed review admission or round marker changed",
            ));
        }
        closed_run(current, "completed")?;
        self.plan.verify_run_cause(current)?;
        let prep = actual_receipt(
            receipts,
            crate::control::preparation_operation(current.operation_id)?,
        )?;
        if canonical_digest(1, prep)? != self.preparation_digest {
            return Err(blocked("completed review preparation provenance changed"));
        }
        child::completed_run(&self.outcome, &self.completion)?;
        self.plan.verify_contract(
            self.completion
                .contract
                .as_ref()
                .ok_or_else(|| blocked("completed round lost its actual contract"))?,
        )
    }
    pub(super) fn findings(&self) -> Result<FindingsReference> {
        if self.claim.verdict != evidence::Verdict::Findings
            || self.claim.unresolved_findings.is_empty()
        {
            return Err(Error::new(
                "no_work_trigger",
                "round has no concrete unresolved Findings",
            ));
        }
        Ok(FindingsReference {
            review: receipt_reference(&self.receipt)?,
            round_id: self.plan.reference.round_id.clone(),
            round_ordinal: self.plan.reference.ordinal,
            round_reference_digest: canonical_digest(1, &self.plan.reference)?,
            outcome_proposal: self.outcome.outcome().proposal_id,
            outcome_claim_digest: self.outcome.digest().into(),
            reviewed_source_digest: self.plan.reference.source.source_digest.clone(),
        })
    }
}

impl DeliveryCoordinator {
    /// Referenced facts from the actual claim, owned source and current scoped
    /// Git observation. This return value itself is not a source capability.
    pub(crate) async fn reviewed_repair_source_reference(
        &self,
        input: &RepairSource<'_>,
    ) -> Result<ReviewedSourceReference> {
        Ok(self.repair_round_source(input).await?.reference.clone())
    }
    pub(crate) async fn initial_review_round(
        &self,
        input: ReviewRoundSource<'_>,
        config: &Config,
        grants: &[VerifiedRetryGrant],
    ) -> Result<VerifiedReviewRoundPlan> {
        let (source, assignment) = self.round_source(&input).await?;
        let phase = PhaseKey {
            work: input.admission.gate.work_key()?,
            generation: canonical_digest(1, &input.admission.gate.work().generation)?,
            phase: "child_codex_review".into(),
            retry_grant: None,
        };
        let mut reference = initial_reference(
            input.admission,
            &source,
            phase,
            current_round_limit(config, &input.admission.gate)?,
        )?;
        reference.passed_validation = input
            .validated_retry
            .as_ref()
            .map(|p| p.reference().clone());
        self.finish_round_plan(
            reference,
            source,
            assignment,
            input.validated_retry.clone(),
            RoundBasis::Initial,
            grants,
        )
    }
    pub(crate) async fn granted_review_round(
        &self,
        input: ReviewRoundSource<'_>,
        config: &Config,
        proof: Arc<ReviewRetryWork>,
        grants: &[VerifiedRetryGrant],
    ) -> Result<VerifiedReviewRoundPlan> {
        let (source, observed_assignment) = self.round_source(&input).await?;
        proof
            .authority()
            .compare_admitted_authority(observed_assignment.authority())?;
        let assignment = proof.assignment().clone();
        if assignment.context() != observed_assignment.context()
            || assignment.phase() != observed_assignment.phase()
            || assignment.role() != observed_assignment.role()
            || assignment.contract_digest() != observed_assignment.contract_digest()
            || !assignment.read_sources().is_empty()
            || assignment.candidate_vector_digest().is_some()
        {
            return Err(blocked(
                "granted round changes immutable review assignment semantics",
            ));
        }
        if proof.next_attempt()? != 1 || proof.source().digest()? != source.reference.source_digest
        {
            return Err(blocked(
                "new grant round is not the first session for this exact source",
            ));
        }
        let limit = current_round_limit(config, &input.admission.gate)?
            .min(proof.grant().new_budget().max_review_rounds);
        let mut reference =
            initial_reference(input.admission, &source, proof.phase().clone(), limit)?;
        reference.passed_validation = input
            .validated_retry
            .as_ref()
            .map(|p| p.reference().clone());
        self.finish_round_plan(
            reference,
            source,
            assignment,
            input.validated_retry.clone(),
            RoundBasis::ExplicitGrant(proof),
            grants,
        )
    }
    pub(crate) fn transient_review_round(
        &self,
        previous: Arc<VerifiedReviewRoundPlan>,
        failed: Arc<RunCompletion>,
        config: &Config,
        grants: &[VerifiedRetryGrant],
    ) -> Result<VerifiedReviewRoundPlan> {
        (self.current_gate)(previous.authority())?;
        if previous.reference.is_repair() {
            return Err(blocked(
                "review transient route cannot retry an implementation session",
            ));
        }
        require_failed(&failed)?;
        previous.verify_contract(
            failed
                .contract
                .as_ref()
                .ok_or_else(|| blocked("failed review contract disappeared"))?,
        )?;
        let receipt = self.host.operation(failed.operation_id)?;
        previous.verify_terminal(&self.host.operations()?, grants, &receipt, "failed")?;
        require_recorded_session(&receipt, &failed)?;
        let mut reference = previous.reference.clone();
        reference.round_limit = reference
            .round_limit
            .min(current_round_limit(config, previous.authority())?);
        reference.cause = transient_cause(&receipt, &failed, &previous.reference)?;
        self.finish_round_plan(
            reference,
            previous.source.clone(),
            previous.assignment.clone(),
            previous.passed_validation.clone(),
            RoundBasis::Transient {
                previous,
                failed,
                receipt,
            },
            grants,
        )
    }
    pub(crate) async fn completed_review_round(
        &self,
        admission: &DeliveryAdmission,
        assignment: &ChildReviewAssignment,
        plan: Arc<VerifiedReviewRoundPlan>,
        outcome: VerifiedAuthorizedOutcome,
        completion: Arc<RunCompletion>,
        grants: &[VerifiedRetryGrant],
    ) -> Result<CompletedReviewRound> {
        self.current(
            admission,
            &[DeliveryPhase::ReviewPublication, DeliveryPhase::NoChange],
        )?;
        plan.authority()
            .compare_admitted_authority(&admission.gate)?;
        if plan.reference.is_repair() {
            return Err(blocked("implementation pair is not a review outcome"));
        }
        let claim = Self::completed_review_claim(admission, assignment, &outcome, &completion)?;
        plan.verify_contract(
            completion
                .contract
                .as_ref()
                .ok_or_else(|| blocked("review contract disappeared"))?,
        )?;
        let receipt = self.host.operation(completion.operation_id)?;
        let run =
            RunReference::from_receipt(&receipt)?.ok_or_else(|| blocked("review run missing"))?;
        if receipt.expected != outcome.run_receipt().expected
            || run.instruction_digest != outcome.outcome().instruction_digest
        {
            return Err(blocked(
                "protected review outcome differs from actual admission",
            ));
        }
        let receipts = self.host.operations()?;
        plan.verify_terminal(&receipts, grants, &receipt, "completed")?;
        require_recorded_session(&receipt, &completion)?;
        self.round_review_source(&plan.source.source, &completion)
            .await?;
        // Actual native/current gate must survive the awaited Git verification.
        self.current(
            admission,
            &[DeliveryPhase::ReviewPublication, DeliveryPhase::NoChange],
        )?;
        let receipts = self.host.operations()?;
        plan.verify_terminal(&receipts, grants, &receipt, "completed")?;
        let receipt_digest = canonical_digest(1, &receipt)?;
        let preparation_digest = canonical_digest(
            1,
            actual_receipt(
                &receipts,
                crate::control::preparation_operation(receipt.operation_id)?,
            )?,
        )?;
        let inventory_digest = inventory(&receipts, plan.reference())?;
        let digest = canonical_digest(
            1,
            &json!({"purpose":"completed_review_round",
            "plan":plan.digest(),"outcome":outcome.digest(),"receipt":receipt_digest,
            "preparation":preparation_digest,"inventory":inventory_digest}),
        )?;
        Ok(CompletedReviewRound {
            plan,
            outcome,
            completion,
            claim,
            receipt,
            receipt_digest,
            preparation_digest,
            inventory_digest,
            digest,
        })
    }
    pub(crate) async fn publish_completed_round(
        &self,
        admission: &DeliveryAdmission,
        assignment: &ChildReviewAssignment,
        completed: &Arc<CompletedReviewRound>,
        current: GateCheck,
    ) -> Result<PublishedReview> {
        (current)(&admission.gate)?;
        self.publish_completed_review_guarded(
            admission,
            assignment,
            completed.outcome(),
            completed.completion(),
            Some(current),
        )
        .await
    }
    pub(crate) fn paired_review_repair(
        &self,
        findings: Arc<CompletedReviewRound>,
        unfinished: Arc<UnfinishedChildWork>,
        config: &Config,
        grants: &[VerifiedRetryGrant],
    ) -> Result<VerifiedReviewRoundPlan> {
        (self.current_gate)(unfinished.authority())?;
        let receipts = self.host.operations()?;
        findings.verify_current(&receipts, grants)?;
        if implementation_inventory(&receipts, findings.plan.reference())?
            != findings.plan.source_inventory_digest
        {
            return Err(blocked(
                "new implementation history superseded the reviewed source",
            ));
        }
        unfinished.verify_current(&receipts, grants)?;
        if unfinished.phase() != &findings.plan.reference.source.implementation_phase
            || unfinished.source().digest()? != findings.plan.reference.source.source_digest
        {
            return Err(blocked(
                "Findings repair cannot renew implementation phase or replace source",
            ));
        }
        let mut reference = findings.plan.reference.clone();
        reference.round_limit = reference
            .round_limit
            .min(current_round_limit(config, unfinished.authority())?);
        reference.cause = ReviewRoundCause::PairedRepair {
            findings: findings.findings()?,
        };
        // Equal ordinal and cap are intentional. The actual implementation
        // attempt/turn bounds remain independently enforced by runtime/Control.
        let assignment = repair_worker_assignment(&unfinished)?;
        let source = findings.plan.source.clone();
        self.finish_round_plan(
            reference,
            source,
            assignment,
            findings.plan.passed_validation.clone(),
            RoundBasis::PairedRepair {
                findings,
                unfinished,
            },
            grants,
        )
    }
    /// Continue an actual scoped fix after qualified failed validation. This
    /// retains the original Findings round while using the latest repaired source.
    pub(crate) async fn continue_paired_repair(
        &self,
        request: ValidationRepairRequest<'_>,
        previous: Arc<VerifiedReviewRoundPlan>,
        validation_source: Arc<VerifiedSourceAdvanced>,
        config: &Config,
        grants: &[VerifiedRetryGrant],
    ) -> Result<(Arc<UnfinishedChildWork>, VerifiedReviewRoundPlan)> {
        let source = self.repair_round_source(&request.source).await?;
        let receipts = self.host.operations()?;
        let actual_repair = actual_receipt(&receipts, source.reference.implementation.operation)?;
        source
            .paired_completion
            .verify(&previous, &receipts, grants, actual_repair)?;
        validation_source.verify_completed_inventory(
            &receipts,
            grants,
            request.completion.operation_id,
        )?;
        if validation_source.source_operation() != source.reference.implementation.operation
            || validation_source.source_receipt_digest()
                != source.reference.implementation.receipt_digest
            || validation_source.source().digest()? != source.reference.source_digest
        {
            return Err(blocked(
                "continued validation did not check the actual repaired source",
            ));
        }
        let validation_outcome = Box::new(request.outcome.clone());
        let validation_completion = request.completion.clone();
        let validation = actual_receipt(&receipts, validation_completion.operation_id)?;
        let validation_run = closed_run(validation, "completed")?;
        require_recorded_session(validation, &validation_completion)?;
        let advanced = crate::control::SourceAdvancedReference::from_receipt(validation)?
            .ok_or_else(|| blocked("continued validation lacks its exact source advancement"))?;
        let evidence = ValidationEvidence::Direct(validation_source.clone());
        let lineage = evidence.reference_lineage(&receipts, grants, &validation_completion)?;
        let unfinished = Arc::new(
            self.unfinished_validation_in_round(
                request,
                previous.clone(),
                validation_source.clone(),
                grants,
            )
            .await?,
        );
        let mut reference = previous.reference.clone();
        reference.round_limit = reference
            .round_limit
            .min(current_round_limit(config, unfinished.authority())?);
        reference.cause = ReviewRoundCause::RepairContinuation {
            findings: previous.completed_findings()?.findings()?,
            previous_round_reference_digest: canonical_digest(1, &previous.reference)?,
            current_source: Box::new(source.reference.clone()),
            validation: Box::new(RepairValidationReference {
                validation: receipt_reference(validation)?,
                phase: validation_run.phase,
                source_advanced: advanced,
                lineage,
                outcome_proposal: validation_outcome.outcome().proposal_id,
                outcome_claim_digest: validation_outcome.digest().into(),
                failure_inventory_digest: unfinished.failure_phase_inventory_digest().into(),
            }),
        };
        let assignment = repair_worker_assignment(&unfinished)?;
        let plan = self.finish_round_plan(
            reference,
            source,
            assignment,
            previous.passed_validation.clone(),
            RoundBasis::RepairContinuation {
                previous,
                unfinished: unfinished.clone(),
                validation_source: evidence,
                validation_outcome,
                validation_completion,
            },
            grants,
        )?;
        Ok((unfinished, plan))
    }
    async fn repair_round_source(&self, input: &RepairSource<'_>) -> Result<Arc<SourceEvidence>> {
        self.current_child_claim(input.admission, &input.original)?;
        if input
            .original
            .outcome()
            .correction_transition_digest()
            .is_some()
        {
            return Err(blocked("corrected pair source is outside this increment"));
        }
        let source = assignment::child_source(
            input.admission,
            &input.original,
            input.source,
            &input.completion,
        )?;
        let implementation = self.host.operation(input.original.operation_id())?;
        let run = closed_run(&implementation, "completed")?;
        let seal = self.host.operation(input.source.receipt().operation_id)?;
        if canonical_digest(1, &seal)? != canonical_digest(1, input.source.receipt())?
            || run.phase.phase != "implementation"
            || run.workspace != input.admission.gate.workspace_id()
            || implementation.expected != input.original.outcome().run_receipt().expected
            || !matches!(
                seal.phase,
                ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
            )
        {
            return Err(blocked("latest repair differs from actual source receipts"));
        }
        let paired_completion = PairedCompletionEvidence::from_actual(
            input.paired_completion.as_ref(),
            &input.completion,
            &self.host.operations()?,
        )?;
        let reference = ReviewedSourceReference {
            paired_resume: paired_completion.reference(),
            implementation_phase: run.phase,
            implementation: receipt_reference(&implementation)?,
            claim_digest: input.original.digest().into(),
            seal: receipt_reference(&seal)?,
            source_digest: source.digest()?,
            repository_hostname: source.repository().hostname.clone(),
            repository_id: source.repository().id.clone(),
            commit: source.revision().commit.clone(),
            tree: source.revision().tree.clone(),
        };
        let checked = source.clone();
        let git = self.git.clone();
        tokio::task::spawn_blocking(move || checked.verify_for_clone(&git))
            .await
            .map_err(|_| {
                Error::new("operation_uncertain", "repair source observation was lost")
            })??;
        self.current_child_claim(input.admission, &input.original)?;
        Ok(Arc::new(SourceEvidence {
            reference,
            source,
            claim: input.original.clone(),
            completion: input.completion.clone(),
            paired_completion,
        }))
    }
    pub(crate) async fn after_repair_review_round(
        &self,
        input: ReviewRoundSource<'_>,
        paired: Arc<VerifiedReviewRoundPlan>,
        advanced: Arc<VerifiedSourceAdvanced>,
        config: &Config,
        grants: &[VerifiedRetryGrant],
    ) -> Result<VerifiedReviewRoundPlan> {
        let findings = paired
            .reference
            .repair_findings()
            .ok_or_else(|| blocked("changed-source review lacks a sealed paired repair"))?;
        let (source, assignment) = self.round_source(&input).await?;
        let repair = self.host.operation(input.claim.operation_id())?;
        // Completed repair is immutable history; later successful validation
        // must not invalidate the earlier failed-validation obligation already
        // consumed by that run. The new source/advanced proof supplies current
        // dispatch freshness, with joined identity retained through the chain.
        source
            .paired_completion
            .verify(&paired, &self.host.operations()?, grants, &repair)?;
        advanced.verify_assignment(&assignment)?;
        if source.reference.implementation_phase != paired.reference.source.implementation_phase
            || source.reference.tree == paired.reference.source.tree
            || advanced.phase() != &paired.reference.review_phase
            || advanced.predecessor() != findings.review.operation
            || advanced.predecessor_receipt_digest() != findings.review.receipt_digest
        {
            return Err(blocked(
                "next review changes review budget, paired origin, or lacks changed source",
            ));
        }
        let mut reference = paired.reference.clone();
        reference.ordinal = reference
            .ordinal
            .checked_add(1)
            .ok_or_else(|| blocked("review ordinal overflow"))?;
        reference.round_limit = reference
            .round_limit
            .min(current_round_limit(config, &input.admission.gate)?);
        reference.source = source.reference.clone();
        reference.passed_validation = input
            .validated_retry
            .as_ref()
            .map(|p| p.reference().clone());
        reference.cause = ReviewRoundCause::AfterRepair {
            findings: findings.clone(),
            repair: receipt_reference(&repair)?,
            paired_round_reference_digest: canonical_digest(1, &paired.reference)?,
        };
        reference.round_id = reference.expected_round_id()?;
        self.finish_round_plan(
            reference,
            source,
            assignment,
            input.validated_retry.clone(),
            RoundBasis::AfterRepair {
                paired,
                repair,
                advanced,
            },
            grants,
        )
    }
    async fn round_source(
        &self,
        input: &ReviewRoundSource<'_>,
    ) -> Result<(Arc<SourceEvidence>, DeliveryWorkerAssignment)> {
        self.current_child_claim(input.admission, &input.claim)?;
        if input
            .claim
            .outcome()
            .correction_transition_digest()
            .is_some()
        {
            return Err(blocked(
                "corrected-generation round lineage is outside this increment",
            ));
        }
        let assignment = self.child_review_worker_assignment(
            input.admission,
            &input.claim,
            input.source,
            &input.completion,
            input.assignment,
        )?;
        let source = assignment
            .source()
            .ok_or_else(|| blocked("review source missing"))?
            .clone();
        let implementation = self.host.operation(input.claim.operation_id())?;
        let run = closed_run(&implementation, "completed")?;
        let seal = self.host.operation(input.source.receipt().operation_id)?;
        if canonical_digest(1, &seal)? != canonical_digest(1, input.source.receipt())?
            || run.phase.phase != "implementation"
            || run.workspace != input.admission.gate.workspace_id()
            || implementation.expected != input.claim.outcome().run_receipt().expected
            || !matches!(
                seal.phase,
                ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
            )
        {
            return Err(blocked(
                "source claim/seal differs from actual protected receipts",
            ));
        }
        let paired_completion = PairedCompletionEvidence::from_actual(
            input.paired_completion.as_ref(),
            &input.completion,
            &self.host.operations()?,
        )?;
        let reference = ReviewedSourceReference {
            paired_resume: paired_completion.reference(),
            implementation_phase: run.phase,
            implementation: receipt_reference(&implementation)?,
            claim_digest: input.claim.digest().into(),
            seal: receipt_reference(&seal)?,
            source_digest: source.digest()?,
            repository_hostname: source.repository().hostname.clone(),
            repository_id: source.repository().id.clone(),
            commit: source.revision().commit.clone(),
            tree: source.revision().tree.clone(),
        };
        let git = self.git.clone();
        let checked = source.clone();
        tokio::task::spawn_blocking(move || checked.verify_for_clone(&git))
            .await
            .map_err(|_| {
                Error::new("operation_uncertain", "round source observation was lost")
            })??;
        self.current_child_claim(input.admission, &input.claim)?;
        Ok((
            Arc::new(SourceEvidence {
                reference,
                source,
                claim: input.claim.clone(),
                completion: input.completion.clone(),
                paired_completion,
            }),
            assignment,
        ))
    }
    async fn round_review_source(
        &self,
        source: &DeliverySource,
        completion: &RunCompletion,
    ) -> Result<()> {
        let git = self.git.clone();
        let source = source.clone();
        let execution = completion
            .execution
            .clone()
            .ok_or_else(|| blocked("review ownership missing"))?;
        let isolation = completion
            .contract
            .as_ref()
            .ok_or_else(|| blocked("review contract missing"))?
            .isolation()
            .clone();
        tokio::task::spawn_blocking(move || {
            source.verify_for_clone(&git)?;
            execution.ownership.verify(&execution.binding, &isolation)?;
            if execution.binding.revision != *source.revision() {
                return Err(blocked("review checked another source revision"));
            }
            git.verify_execution_tree(
                &isolation.plan().repository,
                source.revision(),
                &BTreeSet::new(),
            )
        })
        .await
        .map_err(|_| Error::new("operation_uncertain", "review source observation was lost"))?
    }
    fn finish_round_plan(
        &self,
        reference: ReviewRoundReference,
        source: Arc<SourceEvidence>,
        assignment: DeliveryWorkerAssignment,
        passed_validation: Option<Arc<VerifiedPassedValidation>>,
        basis: RoundBasis,
        grants: &[VerifiedRetryGrant],
    ) -> Result<VerifiedReviewRoundPlan> {
        if reference.ordinal > reference.round_limit {
            return Err(Error::new(
                "review_rounds_exhausted",
                "no further review round remains in this exact budget",
            ));
        }
        reference.validate()?;
        let receipts = self.host.operations()?;
        let inventory_digest = inventory(&receipts, &reference)?;
        let source_inventory_digest = implementation_inventory(&receipts, &reference)?;
        let mut validation_grants = match &basis {
            RoundBasis::Initial | RoundBasis::ExplicitGrant(_) => BTreeMap::new(),
            RoundBasis::Transient { previous, .. }
            | RoundBasis::RepairContinuation { previous, .. } => previous.validation_grants.clone(),
            RoundBasis::PairedRepair { findings, .. } => findings.plan.validation_grants.clone(),
            RoundBasis::AfterRepair { paired, .. } => paired.validation_grants.clone(),
        };
        if let RoundBasis::RepairContinuation {
            validation_source,
            validation_completion,
            ..
        } = &basis
        {
            for grant in
                validation_source.retained_grants(&receipts, grants, validation_completion)?
            {
                if let Some(old) =
                    validation_grants.insert(grant.record().operation_id, grant.clone())
                    && old.digest() != grant.digest()
                {
                    return Err(blocked("validation native grant identity changed"));
                }
            }
        }
        let (mut retained, grant) = match &basis {
            RoundBasis::Initial => (BTreeMap::new(), None),
            RoundBasis::ExplicitGrant(proof) => (BTreeMap::new(), Some(proof.grant().clone())),
            RoundBasis::Transient {
                previous, receipt, ..
            } => {
                let mut retained = previous.retained.clone();
                retain_terminal(&mut retained, &receipts, receipt)?;
                (retained, previous.grant.clone())
            }
            RoundBasis::PairedRepair { findings, .. } => {
                let mut retained = findings.plan.retained.clone();
                retain_terminal(&mut retained, &receipts, &findings.receipt)?;
                (retained, findings.plan.grant.clone())
            }
            RoundBasis::RepairContinuation {
                previous,
                validation_completion,
                validation_source,
                ..
            } => {
                let mut retained = previous.retained.clone();
                for (id, digest) in validation_source.source_receipts()? {
                    if canonical_digest(1, actual_receipt(&receipts, id)?)? != digest {
                        return Err(blocked("continued validation source ancestry changed"));
                    }
                    retain_exact(&mut retained, id, digest)?;
                }
                source.paired_completion.retain(
                    &mut retained,
                    &receipts,
                    actual_receipt(&receipts, source.reference.implementation.operation)?,
                )?;
                let validation_receipt =
                    actual_receipt(&receipts, validation_completion.operation_id)?;
                retain_terminal(&mut retained, &receipts, validation_receipt)?;
                let validation_run = closed_run(validation_receipt, "completed")?;
                // Keep the complete exact inventory consumed by this failure,
                // so later immutable completion/publication cannot lose a link
                // merely because current validation has legitimately advanced.
                for observed in &receipts {
                    if let Some(run) = RunReference::from_receipt(observed)?
                        && observed.runtime_id == reference.runtime
                        && run.workspace == reference.workspace
                        && run.phase.work == validation_run.phase.work
                        && run.phase.generation == validation_run.phase.generation
                        && run.phase.phase == validation_run.phase.phase
                    {
                        retain_exact(
                            &mut retained,
                            observed.operation_id,
                            canonical_digest(1, observed)?,
                        )?;
                    }
                }
                (retained, previous.grant.clone())
            }
            RoundBasis::AfterRepair { paired, repair, .. } => {
                let mut retained = paired.retained.clone();
                source
                    .paired_completion
                    .retain(&mut retained, &receipts, repair)?;
                (retained, paired.grant.clone())
            }
        };
        if let Some(passed) = &passed_validation {
            if reference.passed_validation.as_ref() != Some(passed.reference()) {
                return Err(blocked(
                    "passed capability differs from exact planned round reference",
                ));
            }
            for (id, digest) in &passed.reference().retained_receipts {
                if canonical_digest(1, actual_receipt(&receipts, *id)?)? != *digest {
                    return Err(blocked(
                        "passed validation ancestry changed before round capture",
                    ));
                }
                retain_exact(&mut retained, *id, digest.clone())?;
            }
        }
        if let Some(grant) = &grant {
            let receipt = actual_receipt(&receipts, grant.record().operation_id)?;
            retain_exact(
                &mut retained,
                receipt.operation_id,
                canonical_digest(1, receipt)?,
            )?;
        }
        for grant in validation_grants.values() {
            let receipt = actual_receipt(&receipts, grant.record().operation_id)?;
            retain_exact(
                &mut retained,
                receipt.operation_id,
                canonical_digest(1, receipt)?,
            )?;
        }
        for entry in [
            &reference.source.implementation,
            &reference.source.seal,
            &source.reference.implementation,
            &source.reference.seal,
        ] {
            retain_exact(&mut retained, entry.operation, entry.receipt_digest.clone())?;
        }
        let digest = canonical_digest(
            1,
            &json!({"purpose":"verified_review_round_plan",
            "reference":reference,"assignment":assignment.digest()?,"inventory":inventory_digest,
            "source_inventory":source_inventory_digest,"retained":retained,"grant":grant.as_ref().map(VerifiedRetryGrant::digest),
            "validation_grants":validation_grants.iter().map(|(id,g)|(*id,g.digest())).collect::<BTreeMap<_,_>>()}),
        )?;
        let plan = VerifiedReviewRoundPlan {
            reference,
            assignment,
            source,
            basis,
            inventory_digest,
            source_inventory_digest,
            retained,
            grant,
            validation_grants,
            passed_validation,
            digest,
        };
        plan.verify_current(&receipts, grants)?;
        Ok(plan)
    }
}

fn blocked(message: &str) -> Error {
    Error::new("review_round_lineage", message)
}
fn history_limit() -> Error {
    Error::new(
        "review_round_history_limit",
        "round provenance exceeds the finite protected receipt inventory; preserve history and report the resource prerequisite",
    )
}
fn receipt_reference(receipt: &Receipt) -> Result<ReviewReceiptReference> {
    Ok(ReviewReceiptReference {
        operation: receipt.operation_id,
        receipt_digest: canonical_digest(1, receipt)?,
    })
}
fn actual_receipt(receipts: &[Receipt], id: Uuid) -> Result<&Receipt> {
    let mut found = receipts.iter().filter(|r| r.operation_id == id);
    let value = found
        .next()
        .ok_or_else(|| blocked("required protected receipt missing"))?;
    if found.next().is_some() {
        return Err(blocked("duplicate protected receipt identity"));
    }
    Ok(value)
}
fn closed_run(receipt: &Receipt, outcome: &str) -> Result<RunReference> {
    let run =
        RunReference::from_receipt(receipt)?.ok_or_else(|| blocked("worker admission missing"))?;
    let observed = receipt
        .observed_result
        .as_ref()
        .ok_or_else(|| blocked("terminal observation missing"))?;
    if receipt.phase != ReceiptPhase::Reconciled
        || observed["local_execution"] != "exited"
        || observed["unresolved_effects"]
            .as_array()
            .is_none_or(|v| !v.is_empty())
        || observed["observation"]["outcome"] != outcome
        || observed["observation"]["local_exit_confirmed"] != true
        || !observed["observation"]["hook_error"].is_null()
        || !observed["observation"]["session"]["intervention"].is_null()
        || observed["observation"]["session"]["input_required"] != false
    {
        return Err(blocked(
            "worker has no exact resolved, policy-free terminal exit",
        ));
    }
    Ok(run)
}
fn require_recorded_session(receipt: &Receipt, completion: &RunCompletion) -> Result<()> {
    let session = receipt
        .observed_result
        .as_ref()
        .and_then(|r| r.pointer("/observation/session"))
        .ok_or_else(|| blocked("actual terminal session observation missing"))?;
    if receipt.operation_id != completion.operation_id
        || canonical_digest(1, session)? != canonical_digest(1, &completion.state)?
    {
        return Err(blocked(
            "joined session differs from protected terminal observation",
        ));
    }
    Ok(())
}
fn require_joined(
    expected: &Arc<RunCompletion>,
    retained: &impl Fn(Uuid) -> Result<Option<Arc<RunCompletion>>>,
) -> Result<()> {
    if retained(expected.operation_id)?
        .as_ref()
        .is_none_or(|actual| !Arc::ptr_eq(actual, expected))
    {
        return Err(blocked("actual same-run joined capability is unavailable"));
    }
    Ok(())
}
fn require_source_receipts(source: &SourceEvidence, receipts: &[Receipt]) -> Result<()> {
    source
        .reference
        .implementation
        .matches_observation(actual_receipt(
            receipts,
            source.reference.implementation.operation,
        )?)?;
    source
        .reference
        .seal
        .matches_observation(actual_receipt(receipts, source.reference.seal.operation)?)?;
    if source.claim.digest() != source.reference.claim_digest
        || source.source.digest()? != source.reference.source_digest
    {
        return Err(blocked("source proof or protected claim changed"));
    }
    Ok(())
}
fn inventory(receipts: &[Receipt], reference: &ReviewRoundReference) -> Result<String> {
    if receipts.len() > MAX_RECEIPTS {
        return Err(history_limit());
    }
    let mut selected = BTreeMap::new();
    for receipt in receipts {
        if let Some(run) = RunReference::from_receipt(receipt)?
            && receipt.runtime_id == reference.runtime
            && run.workspace == reference.workspace
            && run.phase.work == reference.review_phase.work
            && run.phase.generation == reference.review_phase.generation
            && run.phase.phase == "child_codex_review"
            && selected.insert(receipt.operation_id, receipt).is_some()
        {
            return Err(blocked("duplicate source/review receipt in inventory"));
        }
    }
    canonical_digest(1, &selected)
}
fn implementation_inventory(
    receipts: &[Receipt],
    reference: &ReviewRoundReference,
) -> Result<String> {
    if receipts.len() > MAX_RECEIPTS {
        return Err(history_limit());
    }
    let mut selected = BTreeMap::new();
    for receipt in receipts {
        if let Some(run) = RunReference::from_receipt(receipt)?
            && receipt.runtime_id == reference.runtime
            && run.workspace == reference.workspace
            && run.phase.work == reference.review_phase.work
            && run.phase.generation == reference.review_phase.generation
            && run.phase.phase == "implementation"
            && selected.insert(receipt.operation_id, receipt).is_some()
        {
            return Err(blocked("duplicate implementation receipt in inventory"));
        }
    }
    canonical_digest(1, &selected)
}
fn current_round_limit(config: &Config, gate: &FreshGate) -> Result<u32> {
    if config.revision()? != gate.config_revision() || config.runtime.max_review_rounds == 0 {
        return Err(blocked(
            "round planning configuration differs from current pinned authority",
        ));
    }
    Ok(config.runtime.max_review_rounds)
}
fn initial_reference(
    admission: &DeliveryAdmission,
    source: &SourceEvidence,
    phase: PhaseKey,
    limit: u32,
) -> Result<ReviewRoundReference> {
    let run = RunReference::from_receipt(source.claim.outcome().run_receipt())?
        .ok_or_else(|| blocked("source admitted account is missing"))?;
    let mut reference = ReviewRoundReference {
        schema_version: 1,
        runtime: admission.scope.runtime_id,
        workspace: admission.gate.workspace_id(),
        account: run.account,
        review_phase: phase,
        round_id: String::new(),
        ordinal: 1,
        round_limit: limit,
        source: source.reference.clone(),
        passed_validation: None,
        cause: ReviewRoundCause::Initial {},
    };
    reference.round_id = reference.expected_round_id()?;
    reference.validate()?;
    Ok(reference)
}
fn require_current_namespace(
    reference: &ReviewRoundReference,
    gate: &FreshGate,
    expected: Option<&VerifiedRetryGrant>,
    grants: &[VerifiedRetryGrant],
) -> Result<()> {
    if grants.iter().any(|g| g.phase() == &reference.review_phase) {
        return Err(blocked(
            "review budget was superseded by a new native grant",
        ));
    }
    if let Some(id) = reference.review_phase.retry_grant {
        let grant =
            expected.ok_or_else(|| blocked("granted round lacks its opaque native grant"))?;
        crate::runtime_workflow::require_current_grant(grant, grants)?;
        if grant.record().operation_id != id
            || grant.phase_key() != reference.review_phase
            || grant.work() != gate.work()
            || grant.record().runtime_id != reference.runtime
            || grant.record().workspace_id != reference.workspace
            || reference.round_limit > grant.new_budget().max_review_rounds
        {
            return Err(blocked(
                "round differs from its current native granted budget",
            ));
        }
    }
    Ok(())
}
fn require_failed(completion: &RunCompletion) -> Result<()> {
    if !completion.local_exit_confirmed
        || !matches!(completion.outcome, RunOutcome::Failed)
        || completion.hook_error.is_some()
        || completion.error.as_ref().is_none_or(|e| {
            matches!(
                e.code.as_str(),
                "operation_uncertain" | "provider_intervention" | "input_required" | "interrupted"
            )
        })
        || completion.state.terminal_transient_failure()?.is_none()
        || !completion.checks.is_empty()
        || !completion.failed_checks.is_empty()
    {
        return Err(blocked(
            "automatic review retry lacks actual supported transient terminal evidence",
        ));
    }
    Ok(())
}
fn transient_cause(
    receipt: &Receipt,
    failed: &RunCompletion,
    previous: &ReviewRoundReference,
) -> Result<ReviewRoundCause> {
    let classification = match failed.state.terminal_transient_failure()? {
        Some(crate::codex::TransientTurnFailure::ServerOverloaded) => {
            TransientFailureCode::ServerOverloaded
        }
        Some(crate::codex::TransientTurnFailure::InternalServerError) => {
            TransientFailureCode::InternalServerError
        }
        None => return Err(blocked("terminal review failure is not transient")),
    };
    // This names the exact complete retained session, not a claimed per-turn
    // digest. The typed getter independently validates its final full turn.
    let terminal_state_digest = canonical_digest(1, &failed.state)?;
    Ok(ReviewRoundCause::TransientRetry {
        predecessor: receipt_reference(receipt)?,
        prior_round_reference_digest: canonical_digest(1, previous)?,
        classified_failure_digest: canonical_digest(
            1,
            &json!({
            "state":terminal_state_digest,"classification":classification}),
        )?,
        terminal_state_digest,
        classification,
    })
}
fn verify_review_contract(
    assignment: &DeliveryWorkerAssignment,
    contract: &WorkerContract,
) -> Result<()> {
    let WorkerAuthority::Authorized(gate) = contract.authority() else {
        return Err(blocked(
            "child review needs actual authorized worker authority",
        ));
    };
    assignment.authority().compare_admitted_authority(gate)?;
    let manifest = &contract.instructions().manifest;
    if assignment.role() != AgentRole::Review
        || assignment.phase() != "child_codex_review"
        || !assignment.read_sources().is_empty()
        || assignment.candidate_vector_digest().is_some()
        || manifest.role != AgentRole::Review
        || manifest.permissions.repository_code_execution
        || !manifest.permissions.source_write_paths.is_empty()
        || !manifest.permissions.resource_write_paths.is_empty()
        || manifest.repository_revision.as_deref()
            != assignment.source().map(|s| s.revision().commit.as_str())
    {
        return Err(blocked(
            "review round contract changed source, role or permissions",
        ));
    }
    assignment::verify_assignment_contract(
        contract,
        assignment.authority(),
        assignment.phase(),
        assignment.context(),
    )
}

fn retain_exact(retained: &mut BTreeMap<Uuid, String>, id: Uuid, digest: String) -> Result<()> {
    if !retained.contains_key(&id) && retained.len() >= MAX_RECEIPTS {
        return Err(history_limit());
    }
    if retained.get(&id).is_some_and(|old| old != &digest) {
        return Err(blocked("conflicting retained round ancestry"));
    }
    retained.insert(id, digest);
    Ok(())
}
fn retain_terminal(
    retained: &mut BTreeMap<Uuid, String>,
    receipts: &[Receipt],
    receipt: &Receipt,
) -> Result<()> {
    retain_exact(
        retained,
        receipt.operation_id,
        canonical_digest(1, receipt)?,
    )?;
    let prep = actual_receipt(
        receipts,
        crate::control::preparation_operation(receipt.operation_id)?,
    )?;
    retain_exact(retained, prep.operation_id, canonical_digest(1, prep)?)
}
fn require_latest_source(
    receipts: &[Receipt],
    reference: &ReviewRoundReference,
    grants: &[VerifiedRetryGrant],
) -> Result<()> {
    let current = actual_receipt(
        receipts,
        reference.dispatch_source().implementation.operation,
    )?;
    let source = RunReference::from_receipt(current)?
        .ok_or_else(|| blocked("source implementation admission missing"))?;
    if grants.iter().any(|grant| grant.phase() == &source.phase) {
        return Err(blocked("source implementation budget was superseded"));
    }
    for receipt in receipts {
        if let Some(run) = RunReference::from_receipt(receipt)?
            && receipt.runtime_id == reference.runtime
            && run.workspace == reference.workspace
            && run.phase.work == source.phase.work
            && run.phase.generation == source.phase.generation
            && run.phase.phase == "implementation"
            && (run.phase != source.phase || run.attempt >= source.attempt)
            && receipt.operation_id != current.operation_id
        {
            return Err(blocked(
                "newer or unaccounted granted implementation history supersedes this source",
            ));
        }
    }
    Ok(())
}

/// Production retains common completed-round proof. Test-only legacy variants
/// preserve the earlier granted-review compatibility regressions without
/// supplying a production path around the current round admission marker.
#[derive(Clone)]
pub(crate) enum CompletedReviewLineage {
    #[cfg(test)]
    Legacy(Box<CompletedGrantedReview>),
    Round(Arc<CompletedReviewRound>),
}
impl CompletedReviewLineage {
    pub(crate) fn digest(&self) -> &str {
        match self {
            #[cfg(test)]
            Self::Legacy(p) => p.digest(),
            Self::Round(p) => p.digest(),
        }
    }
    pub(crate) fn outcome(&self) -> &VerifiedAuthorizedOutcome {
        match self {
            #[cfg(test)]
            Self::Legacy(p) => p.outcome(),
            Self::Round(p) => p.outcome(),
        }
    }
    pub(crate) fn completion(&self) -> &Arc<RunCompletion> {
        match self {
            #[cfg(test)]
            Self::Legacy(p) => p.completion(),
            Self::Round(p) => p.completion(),
        }
    }
    pub(crate) fn phase(&self) -> &PhaseKey {
        match self {
            #[cfg(test)]
            Self::Legacy(p) => p.phase(),
            Self::Round(p) => p.phase(),
        }
    }
    pub(crate) fn verify_current(
        &self,
        receipts: &[Receipt],
        grants: &[VerifiedRetryGrant],
    ) -> Result<()> {
        match self {
            #[cfg(test)]
            Self::Legacy(p) => p.verify_current(receipts, grants),
            Self::Round(p) => p.verify_current(receipts, grants),
        }
    }
}

impl CompletedReviewRound {
    pub(crate) fn verify_resumed_bridges(
        &self,
        reader: &dyn Fn(Uuid) -> Result<Option<Arc<crate::worker_run::AdmittedSessionResume>>>,
    ) -> Result<()> {
        self.plan.verify_resumed_bridges(reader)
    }
}
pub(super) fn merge_resumed_bridges(
    result: &mut BTreeMap<Uuid, Arc<crate::worker_run::AdmittedSessionResume>>,
    values: BTreeMap<Uuid, Arc<crate::worker_run::AdmittedSessionResume>>,
) -> Result<()> {
    for (id, value) in values {
        if id.is_nil() || id != value.bridge().next_operation() {
            return Err(blocked("resumed bridge operation changed"));
        }
        if let Some(old) = result.get(&id) {
            if !Arc::ptr_eq(old, &value) {
                return Err(blocked(
                    "resumed bridge identity conflicts across actual ancestors",
                ));
            }
        } else {
            result.insert(id, value);
        }
        if result.len() > MAX_RECEIPTS {
            return Err(history_limit());
        }
    }
    Ok(())
}
pub(super) fn verify_resumed_inventory(
    values: BTreeMap<Uuid, Arc<crate::worker_run::AdmittedSessionResume>>,
    reader: &dyn Fn(Uuid) -> Result<Option<Arc<crate::worker_run::AdmittedSessionResume>>>,
) -> Result<()> {
    for (id, expected) in values {
        if reader(id)?
            .as_ref()
            .is_none_or(|actual| !Arc::ptr_eq(actual, &expected))
        {
            return Err(blocked("actual admitted resumed bridge is unavailable"));
        }
    }
    Ok(())
}

impl VerifiedReviewRoundPlan {
    pub(crate) fn retained_source_inventory(&self) -> Result<Vec<DeliverySource>> {
        Ok(crate::delivery::lineage::collect(self)?
            .sources
            .into_values()
            .collect())
    }
}
pub(super) fn merge_source_inventory(
    into: &mut BTreeMap<String, DeliverySource>,
    sources: Vec<DeliverySource>,
) -> Result<()> {
    for source in sources {
        into.entry(source.digest()?).or_insert(source);
        if into.len() > MAX_RECEIPTS {
            return Err(history_limit());
        }
    }
    Ok(())
}

// Native grants are actual retained capabilities. Whole local receipt hashes
// alone cannot preserve their current native publication and unique namespace.
pub(super) fn merge_native_grants(
    into: &mut BTreeMap<Uuid, VerifiedRetryGrant>,
    values: impl IntoIterator<Item = VerifiedRetryGrant>,
) -> Result<()> {
    for grant in values {
        let operation = grant.record().operation_id;
        if operation.is_nil() {
            return Err(blocked("retained native grant has no operation"));
        }
        if let Some(prior) = into.get(&operation) {
            if native_grant_identity(prior)? != native_grant_identity(&grant)? {
                return Err(blocked(
                    "one native grant operation has conflicting retained identities",
                ));
            }
        } else {
            into.insert(operation, grant);
            if into.len() > MAX_RECEIPTS {
                return Err(history_limit());
            }
        }
    }
    Ok(())
}
fn native_grant_identity(grant: &VerifiedRetryGrant) -> Result<String> {
    let publication = grant.provenance();
    canonical_digest(
        1,
        &json!({"record":grant.record(),"signed_digest":grant.digest(),
        "provider":publication.provider,"record_id":publication.record_id,
        "writer":publication.writer_account_id,"payload":publication.payload_digest}),
    )
}
/// Complete only actual predecessor namespaces named by authentic opaque seeds.
/// This captures prior explicit grants rather than silently trusting the latest
/// grant's reference to an operation after the original plan is flattened.
pub(super) fn close_native_grants(
    seeds: BTreeMap<Uuid, VerifiedRetryGrant>,
    current: &[VerifiedRetryGrant],
) -> Result<BTreeMap<Uuid, VerifiedRetryGrant>> {
    if current.len() > MAX_RECEIPTS || seeds.len() > MAX_RECEIPTS {
        return Err(history_limit());
    }
    let mut result = seeds;
    let mut pending = result.keys().copied().collect::<Vec<_>>();
    let mut seen = BTreeSet::new();
    let mut namespaces = BTreeMap::<PhaseKey, usize>::new();
    let mut operations = BTreeMap::<Uuid, Vec<&VerifiedRetryGrant>>::new();
    for grant in current {
        *namespaces.entry(grant.phase().clone()).or_default() += 1;
        operations
            .entry(grant.record().operation_id)
            .or_default()
            .push(grant);
    }
    while let Some(operation) = pending.pop() {
        if !seen.insert(operation) {
            continue;
        }
        if seen.len() > MAX_RECEIPTS {
            return Err(history_limit());
        }
        let grant = result.get(&operation).expect("queued actual grant").clone();
        let observed = operations
            .get(&operation)
            .filter(|values| values.len() == 1)
            .and_then(|values| values.first())
            .copied()
            .ok_or_else(|| blocked("retained native grant publication is missing or ambiguous"))?;
        crate::runtime_workflow::require_current_grant(&grant, std::slice::from_ref(observed))?;
        if native_grant_identity(&grant)? != native_grant_identity(observed)? {
            return Err(blocked(
                "retained native grant publication changed its exact identity",
            ));
        }
        if namespaces.get(grant.phase()) != Some(&1) {
            return Err(blocked("retained native grant namespace is ambiguous"));
        }
        let Some(prior_operation) = grant.phase().retry_grant else {
            continue;
        };
        let prior = operations
            .get(&prior_operation)
            .filter(|values| values.len() == 1)
            .and_then(|values| values.first())
            .copied()
            .ok_or_else(|| blocked("retained native grant predecessor is missing or ambiguous"))?;
        if prior.phase_key() != *grant.phase()
            || prior.work() != grant.work()
            || prior.record().runtime_id != grant.record().runtime_id
            || prior.record().workspace_id != grant.record().workspace_id
        {
            return Err(blocked(
                "retained native grants cross exact work or predecessor namespace",
            ));
        }
        let new = !result.contains_key(&prior_operation);
        merge_native_grants(&mut result, [prior.clone()])?;
        if new {
            pending.push(prior_operation);
        }
    }
    validate_native_grant_edges(
        &result
            .iter()
            .map(|(id, grant)| (*id, grant.phase().retry_grant))
            .collect(),
    )?;
    Ok(result)
}

/// Data-only graph validation shared with the actual opaque collector. Complete
/// paths are memoized, so shared tails and long histories remain iterative.
fn validate_native_grant_edges(edges: &BTreeMap<Uuid, Option<Uuid>>) -> Result<()> {
    if edges.len() > MAX_RECEIPTS {
        return Err(history_limit());
    }
    let mut settled = BTreeSet::new();
    for start in edges.keys() {
        if settled.contains(start) {
            continue;
        }
        let mut path = BTreeSet::new();
        let mut cursor = Some(*start);
        while let Some(operation) = cursor {
            if settled.contains(&operation) {
                break;
            }
            if !path.insert(operation) {
                return Err(blocked("retained native grant ancestry cycles"));
            }
            cursor = *edges.get(&operation).ok_or_else(|| {
                blocked("retained native grant ancestry omits a referenced operation")
            })?;
        }
        settled.extend(path);
    }
    Ok(())
}

impl crate::delivery::lineage::Node for VerifiedReviewRoundPlan {
    fn expand<'a>(
        &'a self,
        children: &mut Vec<&'a dyn crate::delivery::lineage::Node>,
        leaves: &mut crate::delivery::lineage::Inventory,
    ) -> Result<()> {
        leaves.sources([self.source.source.clone()])?;
        leaves.grants(
            self.grant
                .iter()
                .cloned()
                .chain(self.validation_grants.values().cloned()),
        )?;
        leaves.joined(1)?;
        children.push(&self.source.paired_completion);
        if let Some(passed) = &self.passed_validation {
            children.push(passed.as_ref());
        }
        match &self.basis {
            RoundBasis::Initial => (),
            RoundBasis::ExplicitGrant(proof) => {
                leaves.grants([proof.grant().clone()])?;
                leaves.joined(1)?;
            }
            RoundBasis::Transient { previous, .. } => {
                children.push(previous.as_ref());
                leaves.joined(1)?;
            }
            RoundBasis::PairedRepair {
                findings,
                unfinished,
            } => {
                children.push(findings.as_ref());
                children.push(unfinished.as_ref());
            }
            RoundBasis::RepairContinuation {
                previous,
                unfinished,
                validation_source,
                ..
            } => {
                children.push(previous.as_ref());
                children.push(unfinished.as_ref());
                children.push(validation_source);
                leaves.joined(1)?;
            }
            RoundBasis::AfterRepair {
                paired, advanced, ..
            } => {
                children.push(paired.as_ref());
                children.push(advanced.as_ref());
            }
        }
        Ok(())
    }
    fn verify_joined_leaf(
        &self,
        reader: &dyn Fn(Uuid) -> Result<Option<Arc<RunCompletion>>>,
    ) -> Result<()> {
        require_joined(&self.source.completion, &|id| reader(id))?;
        match &self.basis {
            RoundBasis::ExplicitGrant(proof) => {
                let actual = reader(proof.predecessor())?
                    .ok_or_else(|| blocked("grant predecessor is absent from joined cache"))?;
                proof.verify_completion(&actual)?;
            }
            RoundBasis::Transient { failed, .. } => require_joined(failed, &|id| reader(id))?,
            RoundBasis::RepairContinuation {
                validation_completion,
                ..
            } => require_joined(validation_completion, &|id| reader(id))?,
            _ => (),
        }
        Ok(())
    }
}
impl crate::delivery::lineage::Node for CompletedReviewRound {
    fn expand<'a>(
        &'a self,
        children: &mut Vec<&'a dyn crate::delivery::lineage::Node>,
        leaves: &mut crate::delivery::lineage::Inventory,
    ) -> Result<()> {
        children.push(self.plan.as_ref());
        leaves.joined(1)
    }
    fn verify_joined_leaf(
        &self,
        reader: &dyn Fn(Uuid) -> Result<Option<Arc<RunCompletion>>>,
    ) -> Result<()> {
        require_joined(&self.completion, &|id| reader(id))
    }
}
impl crate::delivery::lineage::Node for CompletedReviewLineage {
    fn expand<'a>(
        &'a self,
        children: &mut Vec<&'a dyn crate::delivery::lineage::Node>,
        _leaves: &mut crate::delivery::lineage::Inventory,
    ) -> Result<()> {
        match self {
            Self::Round(p) => children.push(p.as_ref()),
            #[cfg(test)]
            Self::Legacy(_) => _leaves.legacy_grant_missing = true,
        }
        Ok(())
    }
}

impl VerifiedReviewRoundPlan {
    pub(crate) fn resumed_bridge_inventory(
        &self,
    ) -> Result<BTreeMap<Uuid, Arc<crate::worker_run::AdmittedSessionResume>>> {
        Ok(crate::delivery::lineage::collect(self)?.bridges)
    }
    pub(crate) fn verify_resumed_bridges(
        &self,
        reader: &dyn Fn(Uuid) -> Result<Option<Arc<crate::worker_run::AdmittedSessionResume>>>,
    ) -> Result<()> {
        verify_resumed_inventory(self.resumed_bridge_inventory()?, reader)
    }
    pub(crate) fn retained_native_grants(&self) -> Result<BTreeMap<Uuid, VerifiedRetryGrant>> {
        crate::delivery::lineage::collect(self)?.into_grants()
    }
}
impl CompletedReviewRound {
    pub(crate) fn verify_joined(
        &self,
        retained: &impl Fn(Uuid) -> Result<Option<Arc<RunCompletion>>>,
    ) -> Result<()> {
        crate::delivery::lineage::verify_joined(self, retained)
    }
}

impl VerifiedReviewRoundPlan {
    fn detach_lineage_for_drop(&mut self, pending: &mut crate::delivery::lineage_drop::Pending) {
        if let Some(passed) = self.passed_validation.take() {
            pending.push(passed);
        }
        // Inert only during destruction; no caller can observe this replacement
        // or obtain a newly usable Initial capability from it.
        match std::mem::replace(&mut self.basis, RoundBasis::Initial) {
            RoundBasis::Initial | RoundBasis::ExplicitGrant(_) => (),
            RoundBasis::Transient { previous, .. } => pending.push(previous),
            RoundBasis::PairedRepair {
                findings,
                unfinished,
            } => {
                pending.push(findings);
                pending.push(unfinished);
            }
            RoundBasis::RepairContinuation {
                previous,
                unfinished,
                validation_source,
                ..
            } => {
                pending.push(previous);
                pending.push(unfinished);
                crate::delivery::lineage_drop::Release::release(validation_source, pending);
            }
            RoundBasis::AfterRepair {
                paired, advanced, ..
            } => {
                pending.push(paired);
                pending.push(advanced);
            }
        }
    }
}
impl Drop for VerifiedReviewRoundPlan {
    fn drop(&mut self) {
        crate::delivery::lineage_drop::drain(|pending| self.detach_lineage_for_drop(pending));
    }
}
impl crate::delivery::lineage_drop::Release for VerifiedReviewRoundPlan {
    fn release(mut self, pending: &mut crate::delivery::lineage_drop::Pending) {
        self.detach_lineage_for_drop(pending);
        // Drop now observes only the inert basis and detached optional edge.
    }
}
impl crate::delivery::lineage_drop::Release for CompletedReviewRound {
    fn release(self, pending: &mut crate::delivery::lineage_drop::Pending) {
        pending.push(self.plan);
    }
}
impl crate::delivery::lineage_drop::Release for CompletedReviewLineage {
    fn release(self, pending: &mut crate::delivery::lineage_drop::Pending) {
        match self {
            Self::Round(p) => pending.push(p),
            #[cfg(test)]
            Self::Legacy(_) => (),
        }
    }
}

#[cfg(test)]
mod retained_grant_edges_tests {
    use super::*;
    #[test]
    fn retained_native_grant_edges_are_bounded_iterative_and_complete() {
        // Plain dependency edges only; this creates no VerifiedRetryGrant.
        let mut edges = BTreeMap::new();
        for ordinal in 1..=MAX_RECEIPTS {
            edges.insert(
                Uuid::from_u128(ordinal as u128),
                (ordinal > 1).then(|| Uuid::from_u128((ordinal - 1) as u128)),
            );
        }
        validate_native_grant_edges(&edges).unwrap();
        edges.insert(Uuid::from_u128((MAX_RECEIPTS + 1) as u128), None);
        assert!(validate_native_grant_edges(&edges).is_err());
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        let c = Uuid::from_u128(3);
        validate_native_grant_edges(&BTreeMap::from([(a, None), (b, Some(a)), (c, Some(a))]))
            .unwrap();
        assert!(
            validate_native_grant_edges(&BTreeMap::from([(a, Some(b)), (b, Some(a))])).is_err()
        );
        assert!(validate_native_grant_edges(&BTreeMap::from([(a, Some(b))])).is_err());
    }
}
