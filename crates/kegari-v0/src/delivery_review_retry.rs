//! A new operator-granted review budget preserves the original exact source and
//! host assignment. A failed session is never promoted to a completed review.
use super::*;
use crate::{
    control::{ReceiptPhase, RunReference},
    decisions::VerifiedRetryGrant,
    instructions::AgentRole,
    scheduler::PhaseKey,
    worker::{WorkerAuthority, WorkerContract},
    worker_run::{RunCompletion, RunOutcome},
};

#[derive(Clone)]
pub struct ReviewRetryWork {
    assignment: DeliveryWorkerAssignment,
    failed: Arc<RunCompletion>,
    reference: RunReference,
    origin_reference: RunReference,
    transient_failure: Option<crate::codex::TransientTurnFailure>,
    receipt: Receipt,
    receipt_digest: String,
    inventory_digest: String,
    grant: VerifiedRetryGrant,
    phase: PhaseKey,
    digest: String,
}
impl ReviewRetryWork {
    pub fn assignment(&self) -> &DeliveryWorkerAssignment {
        &self.assignment
    }
    pub fn authority(&self) -> &FreshGate {
        self.assignment.authority()
    }
    pub fn source(&self) -> &DeliverySource {
        self.assignment.source().expect("verified review source")
    }
    pub fn phase(&self) -> &PhaseKey {
        &self.phase
    }
    pub fn predecessor(&self) -> Uuid {
        self.failed.operation_id
    }
    pub fn predecessor_receipt_digest(&self) -> &str {
        &self.receipt_digest
    }
    pub fn grant(&self) -> &VerifiedRetryGrant {
        &self.grant
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
    pub(crate) fn next_attempt(&self) -> Result<u32> {
        if self.reference.phase == self.phase {
            self.reference
                .attempt
                .checked_add(1)
                .ok_or_else(|| Error::new("attempts_exhausted", "review attempt ordinal overflow"))
        } else {
            Ok(1)
        }
    }
    pub fn verify_completion(&self, actual: &Arc<RunCompletion>) -> Result<()> {
        if !Arc::ptr_eq(actual, &self.failed) {
            return Err(Error::new(
                "worker_binding",
                "review retry lost its actual joined failed completion",
            ));
        }
        failed_review(actual)?;
        self.verify_retry_cause()
    }
    pub(crate) fn verify_retry_cause(&self) -> Result<()> {
        if self.reference.phase == self.phase
            && (self.transient_failure.is_none()
                || self.failed.state.terminal_transient_failure()? != self.transient_failure)
        {
            return Err(Error::new(
                "persistent_blocker",
                "automatic review retry requires an observed transient terminal cause",
            ));
        }
        Ok(())
    }
    pub fn verify_assignment(&self, assignment: &DeliveryWorkerAssignment) -> Result<()> {
        if assignment.digest()? != self.assignment.digest()? {
            return Err(Error::new(
                "worker_binding",
                "review retry cannot replace the sealed assignment or source",
            ));
        }
        Ok(())
    }
    pub fn verify_contract(&self, contract: &WorkerContract) -> Result<()> {
        review_contract(&self.assignment, contract)
    }
    pub fn verify_inventory(&self, receipts: &[Receipt]) -> Result<()> {
        if review_inventory(receipts, self.receipt.runtime_id, &self.reference)?
            != self.inventory_digest
        {
            return Err(Error::new(
                "stale_result",
                "review admission inventory changed after retry preparation",
            ));
        }
        let actual = receipts
            .iter()
            .find(|r| r.operation_id == self.predecessor())
            .ok_or_else(|| {
                Error::new(
                    "reconciliation_required",
                    "failed review receipt disappeared",
                )
            })?;
        if canonical_digest(1, actual)? != self.receipt_digest {
            return Err(Error::new("stale_result", "failed review receipt changed"));
        }
        closed_failed_review(actual)?;
        self.source().revalidate_filesystem()
    }
    pub fn verify_completed_inventory(
        &self,
        receipts: &[Receipt],
        grants: &[VerifiedRetryGrant],
        operation: Uuid,
    ) -> Result<()> {
        self.verify_terminal_inventory(receipts, grants, operation, "completed")
    }
    fn verify_terminal_inventory(
        &self,
        receipts: &[Receipt],
        grants: &[VerifiedRetryGrant],
        operation: Uuid,
        outcome: &str,
    ) -> Result<()> {
        let receipt = receipts
            .iter()
            .find(|r| r.operation_id == operation)
            .ok_or_else(|| {
                Error::new(
                    "reconciliation_required",
                    "retried review admission missing",
                )
            })?;
        let run = RunReference::from_receipt(receipt)?
            .ok_or_else(|| Error::new("worker_binding", "retried review lacks actual run"))?;
        let result = receipt.observed_result.as_ref();
        if receipt.runtime_id != self.receipt.runtime_id
            || run.phase != self.phase
            || run.workspace != self.reference.workspace
            || run.account != self.reference.account
            || run.attempt != self.next_attempt()?
            || run.attempt > run.max_attempts
            || [run.max_attempts, run.max_turns, run.max_review_rounds].contains(&0)
            || run.profile_digest != self.reference.profile_digest
            || run.max_attempts > self.grant.new_budget().max_attempts
            || run.max_turns > self.grant.new_budget().max_turns
            || run.max_review_rounds > self.grant.new_budget().max_review_rounds
            || self.reference.phase == self.phase
                && (run.max_attempts > self.reference.max_attempts
                    || run.max_turns > self.reference.max_turns
                    || run.max_review_rounds > self.reference.max_review_rounds)
            || receipt.phase != ReceiptPhase::Reconciled
            || result
                .and_then(|v| v.get("local_execution"))
                .and_then(Value::as_str)
                != Some("exited")
            || result
                .and_then(|v| v.get("unresolved_effects"))
                .and_then(Value::as_array)
                .is_none_or(|v| !v.is_empty())
            || result
                .and_then(|v| v.pointer("/observation/outcome"))
                .and_then(Value::as_str)
                != Some(outcome)
        {
            return Err(Error::new(
                "reconciliation_required",
                "retried review is not a closed terminal exact granted run",
            ));
        }
        let prior: Vec<_> = receipts
            .iter()
            .filter(|r| r.operation_id != operation)
            .cloned()
            .collect();
        self.verify_current(&prior, grants)
    }
    pub fn verify_current(
        &self,
        receipts: &[Receipt],
        grants: &[VerifiedRetryGrant],
    ) -> Result<()> {
        self.verify_inventory(receipts)?;
        self.verify_retry_cause()?;
        if grants.iter().any(|g| g.phase() == &self.phase) {
            return Err(Error::new(
                "stale_result",
                "review retry grant was superseded",
            ));
        }
        let mut ancestral = Vec::new();
        let mut granted = Vec::new();
        for receipt in receipts {
            match RunReference::from_receipt(receipt)? {
                Some(run) if run.phase == self.phase => granted.push((receipt, run)),
                _ => ancestral.push(receipt.clone()),
            }
        }
        verify_lineage(
            &self.grant,
            self.authority(),
            &self.origin_reference,
            self.receipt.runtime_id,
            &ancestral,
            grants,
        )?;
        granted.sort_by_key(|(_, run)| run.attempt);
        if granted.len() != (self.next_attempt()? - 1) as usize {
            return Err(Error::new(
                "stale_result",
                "review retry no longer binds the latest granted attempt",
            ));
        }
        let mut caps = (
            self.grant.new_budget().max_attempts,
            self.grant.new_budget().max_turns,
            self.grant.new_budget().max_review_rounds,
        );
        for (index, (receipt, run)) in granted.iter().enumerate() {
            closed_failed_review(receipt)?;
            if !same_group(self.receipt.runtime_id, &self.reference, receipt, run)
                || run.account != self.reference.account
                || run.profile_digest != self.reference.profile_digest
                || run.attempt as usize != index + 1
                || run.attempt > run.max_attempts
                || [run.max_attempts, run.max_turns, run.max_review_rounds].contains(&0)
                || run.max_attempts > caps.0
                || run.max_turns > caps.1
                || run.max_review_rounds > caps.2
            {
                return Err(Error::new(
                    "stale_result",
                    "granted review history changes identity, ordinal or captured bounds",
                ));
            }
            caps = (run.max_attempts, run.max_turns, run.max_review_rounds);
        }
        if let Some((latest, _)) = granted.last()
            && latest.operation_id != self.predecessor()
        {
            return Err(Error::new(
                "stale_result",
                "review retry predecessor is no longer latest",
            ));
        }
        Ok(())
    }
}
pub struct ReviewRetryRequest<'a> {
    pub admission: &'a DeliveryAdmission,
    pub assignment: &'a DeliveryWorkerAssignment,
    pub failed: Arc<RunCompletion>,
    pub grant: VerifiedRetryGrant,
    pub current_grants: &'a [VerifiedRetryGrant],
}
impl DeliveryCoordinator {
    pub async fn publish_retried_review(
        &self,
        admission: &DeliveryAdmission,
        assignment: &ChildReviewAssignment,
        outcome: &crate::runtime::VerifiedAuthorizedOutcome,
        completion: &Arc<RunCompletion>,
        proof: &ReviewRetryWork,
        current_grant: GateCheck,
    ) -> Result<PublishedReview> {
        proof.verify_contract(completion.contract.as_ref().ok_or_else(|| {
            Error::new("worker_binding", "retried review contract unavailable")
        })?)?;
        child::completed_run(outcome, completion)?;
        let run = RunReference::from_receipt(&self.host.operation(completion.operation_id)?)?
            .ok_or_else(|| Error::new("worker_binding", "review run missing"))?;
        if run.phase != *proof.phase()
            || run.instruction_digest != outcome.outcome().instruction_digest
            || run.profile_digest != canonical_digest(1, &outcome.outcome().instructions.profile)?
        {
            return Err(Error::new(
                "worker_binding",
                "retried result differs from actual granted admission",
            ));
        }
        let source = proof.source().clone();
        let git = self.git.clone();
        let execution = completion.execution.clone().ok_or_else(|| {
            Error::new("worker_binding", "retried review lacks actual owned source")
        })?;
        let isolation = completion.contract.as_ref().unwrap().isolation().clone();
        let revision = proof.source().revision().clone();
        tokio::task::spawn_blocking(move || {
            source.verify_for_clone(&git)?;
            execution.ownership.verify(&execution.binding, &isolation)?;
            if execution.binding.revision != revision {
                return Err(Error::new(
                    "stale_result",
                    "retried review checked another source",
                ));
            }
            git.verify_execution_tree(&isolation.plan().repository, &revision, &BTreeSet::new())
        })
        .await
        .map_err(|_| {
            Error::new(
                "operation_uncertain",
                "retried review source validation was lost",
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
    pub async fn retry_failed_review(
        &self,
        request: ReviewRetryRequest<'_>,
    ) -> Result<ReviewRetryWork> {
        self.prepare_review_retry(request, None).await
    }
    /// A later failed session consumes the same granted budget. Its exact
    /// completed effect replaces only the retry cause, never the grant/source.
    pub async fn continue_failed_review(
        &self,
        admission: &DeliveryAdmission,
        prior: &ReviewRetryWork,
        failed: Arc<RunCompletion>,
        current_grants: &[VerifiedRetryGrant],
    ) -> Result<ReviewRetryWork> {
        self.prepare_review_retry(
            ReviewRetryRequest {
                admission,
                assignment: prior.assignment(),
                failed,
                grant: prior.grant().clone(),
                current_grants,
            },
            Some(prior),
        )
        .await
    }
    async fn prepare_review_retry(
        &self,
        request: ReviewRetryRequest<'_>,
        prior: Option<&ReviewRetryWork>,
    ) -> Result<ReviewRetryWork> {
        self.current(
            request.admission,
            &[DeliveryPhase::ReviewPublication, DeliveryPhase::NoChange],
        )?;
        request
            .assignment
            .verify_binding(request.admission.fresh_gate(), "child_codex_review")?;
        failed_review(&request.failed)?;
        let contract = request
            .failed
            .contract
            .as_ref()
            .ok_or_else(|| Error::new("worker_binding", "failed review contract missing"))?;
        review_contract(request.assignment, contract)?;
        let source = request
            .assignment
            .source()
            .ok_or_else(|| Error::new("worker_binding", "review source missing"))?;
        if source.revision().commit != request.admission.scope.revision.head
            || source.revision().tree != request.admission.scope.revision.tested_tree
        {
            return Err(Error::new(
                "stale_result",
                "review retry source differs from current exact delivery head",
            ));
        }
        let checked = source.clone();
        let git = self.git.clone();
        tokio::task::spawn_blocking(move || checked.verify_for_clone(&git))
            .await
            .map_err(|_| {
                Error::new(
                    "operation_uncertain",
                    "review retry source verification lost",
                )
            })??;
        let receipts = self.host.operations()?;
        let receipt = self.host.operation(request.failed.operation_id)?;
        closed_failed_review(&receipt)?;
        if receipt
            .observed_result
            .as_ref()
            .and_then(|v| v.get("observation"))
            != Some(&request.failed.observation())
        {
            return Err(Error::new(
                "worker_binding",
                "joined failed review differs from protected terminal observation",
            ));
        }
        let reference = RunReference::from_receipt(&receipt)?
            .ok_or_else(|| Error::new("worker_binding", "failed review lacks admission"))?;
        let manifest = &contract.instructions().manifest;
        if receipt.runtime_id != request.admission.scope.runtime_id
            || reference.workspace != request.assignment.authority().workspace_id()
            || reference.phase.work != request.assignment.authority().work_key()?
            || reference.phase.generation
                != canonical_digest(1, &request.assignment.authority().work().generation)?
            || reference.phase.phase != "child_codex_review"
            || reference.account != contract.account_id()
            || reference.instruction_digest != contract.instructions().manifest_digest
            || reference.profile_digest != canonical_digest(1, &manifest.profile)?
            || manifest.operation_id != request.failed.operation_id
            || request.failed.state.binding.operation_id != request.failed.operation_id.to_string()
            || request.failed.state.binding.instruction_digest
                != contract.instructions().manifest_digest
            || request.failed.state.binding.work_identity != reference.phase.work
            || request.failed.state.binding.generation != manifest.work_generation
            || request.failed.state.binding.cwd != manifest.cwd
            || request.failed.state.binding.isolation_digest
                != manifest.permissions.enforcement_profile
            || request.failed.state.binding.account_id != reference.account
        {
            return Err(Error::new(
                "worker_binding",
                "failed review differs from original admitted contract",
            ));
        }
        let transient_failure = if let Some(prior) = prior {
            prior.verify_terminal_inventory(
                &receipts,
                request.current_grants,
                request.failed.operation_id,
                "failed",
            )?;
            Some(
                request
                    .failed
                    .state
                    .terminal_transient_failure()?
                    .ok_or_else(|| {
                        Error::new(
                            "persistent_blocker",
                            "known review failure is not a supported transient provider failure",
                        )
                    })?,
            )
        } else {
            None
        };
        let receipt_digest = canonical_digest(1, &receipt)?;
        let inventory_digest = review_inventory(&receipts, receipt.runtime_id, &reference)?;
        let digest = canonical_digest(
            1,
            &json!({"purpose":"exact_source_review_retry","assignment":request.assignment.digest()?,"failed":receipt_digest,"inventory":inventory_digest,"grant":request.grant.digest(),"previous":prior.map(ReviewRetryWork::digest),"transient_failure":transient_failure}),
        )?;
        let result = ReviewRetryWork {
            assignment: request.assignment.clone(),
            failed: request.failed,
            transient_failure,
            origin_reference: prior
                .map_or_else(|| reference.clone(), |p| p.origin_reference.clone()),
            reference,
            receipt,
            receipt_digest,
            inventory_digest,
            phase: request.grant.phase_key(),
            grant: request.grant,
            digest,
        };
        result.verify_current(&receipts, request.current_grants)?;
        Ok(result)
    }
}
fn failed_review(completion: &RunCompletion) -> Result<()> {
    if !completion.local_exit_confirmed
        || !matches!(completion.outcome, RunOutcome::Failed)
        || completion.hook_error.is_some()
        || completion.error.as_ref().is_none_or(|error| {
            matches!(
                error.code.as_str(),
                "operation_uncertain" | "provider_intervention" | "input_required" | "interrupted"
            )
        })
        || completion.state.turn_id.is_some()
        || completion.state.unresolved_turn.is_some()
        || completion.state.intervention.is_some()
        || completion.state.input_required
        || completion.state.terminal.as_deref() != Some("failed")
        || !completion.checks.is_empty()
        || !completion.failed_checks.is_empty()
    {
        return Err(Error::new(
            "reconciliation_required",
            "review retry requires actual known failed terminal session and joined exit",
        ));
    }
    Ok(())
}
fn review_contract(assignment: &DeliveryWorkerAssignment, contract: &WorkerContract) -> Result<()> {
    let manifest = &contract.instructions().manifest;
    let WorkerAuthority::Authorized(gate) = contract.authority() else {
        return Err(Error::new(
            "worker_binding",
            "review must have child authority",
        ));
    };
    // These are immutable admission records, potentially retained long after
    // the review finished. Fresh authority is checked separately at producer,
    // preparation, final model admission and publication gates.
    if assignment.authority().digest()? != gate.digest()? {
        return Err(Error::new(
            "worker_binding",
            "review contract changed its admitted authority",
        ));
    }
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
        return Err(Error::new(
            "worker_binding",
            "review retry contract changed role, source, or permissions",
        ));
    }
    assignment::verify_assignment_contract(
        contract,
        assignment.authority(),
        assignment.phase(),
        assignment.context(),
    )
}
fn closed_failed_review(receipt: &Receipt) -> Result<()> {
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
        || result
            .and_then(|v| v.pointer("/observation/outcome"))
            .and_then(Value::as_str)
            != Some("failed")
    {
        return Err(Error::new(
            "reconciliation_required",
            "failed review effects are not exactly retired",
        ));
    }
    Ok(())
}
fn same_group(
    runtime: Uuid,
    reference: &RunReference,
    receipt: &Receipt,
    run: &RunReference,
) -> bool {
    receipt.runtime_id == runtime
        && run.workspace == reference.workspace
        && run.phase.work == reference.phase.work
        && run.phase.generation == reference.phase.generation
        && run.phase.phase == reference.phase.phase
}
fn review_inventory(
    receipts: &[Receipt],
    runtime: Uuid,
    reference: &RunReference,
) -> Result<String> {
    let mut inventory = BTreeMap::new();
    for receipt in receipts {
        if let Some(run) = RunReference::from_receipt(receipt)?
            && same_group(runtime, reference, receipt, &run)
            && inventory.insert(receipt.operation_id, receipt).is_some()
        {
            return Err(Error::new("record_corrupt", "duplicate review admission"));
        }
    }
    canonical_digest(1, &inventory)
}
fn verify_lineage(
    grant: &VerifiedRetryGrant,
    gate: &FreshGate,
    reference: &RunReference,
    runtime: Uuid,
    receipts: &[Receipt],
    grants: &[VerifiedRetryGrant],
) -> Result<()> {
    crate::runtime_workflow::require_current_grant(grant, grants)?;
    if grant.phase() != &reference.phase
        || grant.work() != gate.work()
        || grant.record().workspace_id != gate.workspace_id()
        || grant.record().runtime_id != runtime
    {
        return Err(Error::new(
            "stale_result",
            "retry grant does not replace exact failed review budget",
        ));
    }
    let mut cursor = grant;
    let mut phases = BTreeSet::new();
    for _ in 0..1024 {
        let phase = cursor.phase();
        if cursor.work() != gate.work()
            || cursor.record().runtime_id != runtime
            || cursor.record().workspace_id != gate.workspace_id()
        {
            return Err(Error::new(
                "identity_mismatch",
                "review grant lineage changes work or workspace",
            ));
        }
        if !phases.insert(canonical_digest(1, phase)?)
            || grants.iter().filter(|g| g.phase() == phase).count() != 1
        {
            return Err(Error::new(
                "stale_result",
                "review grant lineage is cyclic or forked",
            ));
        }
        let mut prior = BTreeMap::new();
        for receipt in receipts {
            if let Some(run) = RunReference::from_receipt(receipt)?
                && run.phase == *phase
                && same_group(runtime, reference, receipt, &run)
            {
                if run.account != reference.account {
                    return Err(Error::new(
                        "identity_mismatch",
                        "review lineage changes account",
                    ));
                }
                let observed = receipt.observed_result.as_ref();
                if receipt.phase != ReceiptPhase::Reconciled
                    || observed
                        .and_then(|v| v.get("local_execution"))
                        .and_then(Value::as_str)
                        != Some("exited")
                    || observed
                        .and_then(|v| v.get("unresolved_effects"))
                        .and_then(Value::as_array)
                        .is_none_or(|v| !v.is_empty())
                {
                    return Err(Error::new(
                        "reconciliation_required",
                        "review lineage retains uncertain ownership",
                    ));
                }
                prior.insert(receipt.operation_id, receipt.payload_digest.clone());
            }
        }
        if prior != cursor.record().prior_admissions {
            return Err(Error::new("stale_result", "review grant inventory changed"));
        }
        match phase.retry_grant {
            Some(id) => {
                cursor = grants
                    .iter()
                    .find(|g| g.record().operation_id == id && g.phase_key() == *phase)
                    .ok_or_else(|| {
                        Error::new("stale_result", "review grant predecessor unavailable")
                    })?;
                crate::runtime_workflow::require_current_grant(cursor, grants)?;
            }
            None => {
                for receipt in receipts {
                    if let Some(run) = RunReference::from_receipt(receipt)?
                        && same_group(runtime, reference, receipt, &run)
                        && (!phases.contains(&canonical_digest(1, &run.phase)?)
                            || run.phase == reference.phase && run.attempt > reference.attempt)
                    {
                        return Err(Error::new(
                            "stale_result",
                            "review has later or unrelated granted history",
                        ));
                    }
                }
                return Ok(());
            }
        }
    }
    Err(Error::new(
        "bounded_input",
        "review grant lineage exceeds bound",
    ))
}
