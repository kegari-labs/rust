//! Exact-source validation retry lineage and current-grant verification.
//! An explicit grant retains the original source and host assignment. A failed
//! session does not supply passed check evidence; preparation, model admission
//! and result consumption all use this opaque cause and actual joined handles.
//! This path neither allocates a review round nor creates a review/repair marker.
use super::*;
#[path = "delivery_validation_retry_source.rs"]
mod source_lineage;
use source_lineage::ValidationSourceLineage;
#[path = "delivery_validation_passed.rs"]
mod passed;
pub(crate) use passed::{ConsumedValidationSuccessor, VerifiedPassedValidation};

// Resource ceiling aligned with the protected Control/review receipt inventory.
// This is not a retry budget: actual admission still applies original, current
// and explicitly granted configured caps before every additional session.
const MAX_VALIDATION_HISTORY_RECEIPTS: usize = 65_536;
use crate::{
    control::ReceiptPhase,
    decisions::VerifiedRetryGrant,
    worker::{WorkerAuthority, WorkerContract},
    worker_run::{RunCompletion, RunOutcome},
};
use kegari_context::instructions::AgentRole;
use kegari_types::run::PhaseKey;
use kegari_types::run::RunReference;

#[derive(Clone)]
pub struct ValidationRetryWork {
    assignment: DeliveryWorkerAssignment,
    source_lineage: Option<Arc<ValidationSourceLineage>>,
    previous: Option<Arc<ValidationRetryWork>>,
    failed: Arc<RunCompletion>,
    reference: RunReference,
    origin_reference: RunReference,
    transient_failure: Option<kegari_codex::TransientTurnFailure>,
    receipt: Receipt,
    receipt_digest: String,
    inventory_digest: String,
    grant: VerifiedRetryGrant,
    phase: PhaseKey,
    digest: String,
}
// Release a uniquely owned Arc-linked history without recursive field drop.
// Arc::into_inner consumes this owner's reference atomically: if another strong
// owner remains, that owner's eventual destructor continues the same process.
// Unlike a failed try_unwrap followed by dropping its returned Arc, this cannot
// accidentally become the last drop during a concurrent shared-owner release.
fn drop_arc_history<T>(
    mut previous: Option<Arc<T>>,
    mut take_previous: impl FnMut(&mut T) -> Option<Arc<T>>,
) {
    while let Some(link) = previous {
        let Some(mut unique) = Arc::into_inner(link) else {
            break;
        };
        previous = take_previous(&mut unique);
        // The recursive edge is gone before any remaining field is destroyed.
        drop(unique);
    }
}

impl Drop for ValidationRetryWork {
    fn drop(&mut self) {
        drop_arc_history(self.previous.take(), |previous| previous.previous.take());
    }
}

// These maps retain only actual already-admitted bridges. Neither merging a
// map nor reading a missing entry can construct native session authority.
fn verify_resumed_bridge_inventory(
    inventory: BTreeMap<Uuid, Arc<crate::worker_run::AdmittedSessionResume>>,
    reader: &dyn Fn(Uuid) -> Result<Option<Arc<crate::worker_run::AdmittedSessionResume>>>,
) -> Result<()> {
    if inventory.len() > MAX_VALIDATION_HISTORY_RECEIPTS {
        return Err(Error::new(
            "bounded_input",
            "retained bridge inventory exceeds protected bound",
        ));
    }
    for (operation, expected) in inventory {
        let actual = reader(operation)?.ok_or_else(|| {
            Error::new(
                "paired_recovery_lineage",
                "actual registered resume bridge is unavailable",
            )
        })?;
        if operation != expected.bridge().next_operation() || !Arc::ptr_eq(&actual, &expected) {
            return Err(Error::new(
                "paired_recovery_lineage",
                "actual registered resume bridge was substituted",
            ));
        }
    }
    Ok(())
}

impl ValidationRetryWork {
    pub(crate) fn resumed_bridge_inventory(
        &self,
    ) -> Result<BTreeMap<Uuid, Arc<crate::worker_run::AdmittedSessionResume>>> {
        Ok(crate::delivery::lineage::collect(self)?.bridges)
    }
    pub(crate) fn verify_resumed_bridges(
        &self,
        reader: &dyn Fn(Uuid) -> Result<Option<Arc<crate::worker_run::AdmittedSessionResume>>>,
    ) -> Result<()> {
        verify_resumed_bridge_inventory(self.resumed_bridge_inventory()?, reader)
    }
    /// Data-only capacity check before retaining one more validation operation.
    /// Passing this resource check grants no session or retry authority.
    pub(crate) fn require_history_capacity(retained: usize) -> Result<()> {
        if retained >= MAX_VALIDATION_HISTORY_RECEIPTS {
            return Err(Error::new(
                "bounded_input",
                "validation retry history exceeds protected receipt inventory bound",
            ));
        }
        Ok(())
    }
    pub fn assignment(&self) -> &DeliveryWorkerAssignment {
        &self.assignment
    }
    pub fn authority(&self) -> &FreshGate {
        self.assignment.authority()
    }
    pub fn source(&self) -> &DeliverySource {
        self.assignment
            .source()
            .expect("verified validation source")
    }
    /// Original source advancement remains distinct from this granted run.
    pub(crate) fn source_advanced(&self) -> Option<&Arc<VerifiedSourceAdvanced>> {
        self.source_lineage
            .as_ref()
            .map(|lineage| lineage.advanced())
    }
    pub(crate) fn original_source_admission(&self) -> Option<&Receipt> {
        self.source_lineage.as_ref().map(|lineage| lineage.origin())
    }
    /// Called with the runtime's actual joined-completion reader at preparation,
    /// final admission, and result consumption; no receipt recreates a completion.
    pub(crate) fn verify_joined(
        &self,
        retained: &impl Fn(Uuid) -> Result<Option<Arc<RunCompletion>>>,
    ) -> Result<()> {
        crate::delivery::lineage::verify_joined(self, retained)
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
    fn require_bounded_chain(&self) -> Result<()> {
        let mut cursor = Some(self);
        let mut seen = BTreeSet::new();
        while let Some(proof) = cursor {
            if seen.len() >= MAX_VALIDATION_HISTORY_RECEIPTS || !seen.insert(proof.digest()) {
                return Err(Error::new(
                    "bounded_input",
                    "validation retry lineage is cyclic or oversized",
                ));
            }
            cursor = proof.previous.as_deref();
        }
        Ok(())
    }
    pub(crate) fn next_attempt(&self) -> Result<u32> {
        if self.reference.phase == self.phase {
            self.reference.attempt.checked_add(1).ok_or_else(|| {
                Error::new("attempts_exhausted", "validation attempt ordinal overflow")
            })
        } else {
            Ok(1)
        }
    }
    pub fn verify_completion(&self, actual: &Arc<RunCompletion>) -> Result<()> {
        if !Arc::ptr_eq(actual, &self.failed) {
            return Err(Error::new(
                "worker_binding",
                "validation retry lost its actual joined failed completion",
            ));
        }
        failed_validation(actual)?;
        self.verify_retry_cause()
    }
    pub(crate) fn verify_retry_cause(&self) -> Result<()> {
        if self.reference.phase == self.phase
            && (self.transient_failure.is_none()
                || self.failed.state.terminal_transient_failure()? != self.transient_failure)
        {
            return Err(Error::new(
                "persistent_blocker",
                "automatic validation retry requires an observed transient terminal cause",
            ));
        }
        Ok(())
    }
    pub fn verify_assignment(&self, assignment: &DeliveryWorkerAssignment) -> Result<()> {
        if assignment.digest()? != self.assignment.digest()? {
            return Err(Error::new(
                "worker_binding",
                "validation retry cannot replace the sealed assignment or source",
            ));
        }
        Ok(())
    }
    pub fn verify_contract(&self, contract: &WorkerContract) -> Result<()> {
        validation_contract(&self.assignment, contract)
    }
    pub fn verify_inventory(&self, receipts: &[Receipt]) -> Result<()> {
        if validation_inventory(receipts, self.receipt.runtime_id, &self.reference)?
            != self.inventory_digest
        {
            return Err(Error::new(
                "stale_result",
                "validation admission inventory changed after retry preparation",
            ));
        }
        let actual = receipts
            .iter()
            .find(|r| r.operation_id == self.predecessor())
            .ok_or_else(|| {
                Error::new(
                    "reconciliation_required",
                    "failed validation receipt disappeared",
                )
            })?;
        if canonical_digest(1, actual)? != self.receipt_digest {
            return Err(Error::new(
                "stale_result",
                "failed validation receipt changed",
            ));
        }
        closed_failed_validation(actual)?;
        self.source().revalidate_filesystem()
    }
    /// Observe an already admitted next attempt without replaying its launch.
    /// Callers supply actual Control records and still require retained live/joined
    /// execution ownership; this check never reconstructs such a capability.
    pub(crate) fn verify_admitted_inventory(
        &self,
        receipts: &[Receipt],
        grants: &[VerifiedRetryGrant],
        operation: Uuid,
    ) -> Result<()> {
        let receipt = unique_receipt(receipts, operation)?;
        source_lineage::require_plain_retry(receipt)?;
        let run = crate::control::run_reference_from_receipt(receipt)?.ok_or_else(|| {
            Error::new(
                "worker_binding",
                "validation retry launch is not a model admission",
            )
        })?;
        self.verify_admitted_header(receipt, &run)?;
        let prior: Vec<_> = receipts
            .iter()
            .filter(|r| r.operation_id != operation)
            .cloned()
            .collect();
        self.verify_current(&prior, grants)
    }
    fn verify_admitted_header(&self, receipt: &Receipt, run: &RunReference) -> Result<()> {
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
        {
            return Err(Error::new(
                "worker_binding",
                "validation retry admission changed bounded identity",
            ));
        }
        Ok(())
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
        self.verify_terminal_inventory_inner(receipts, grants, operation, outcome, None)
    }
    fn verify_terminal_inventory_inner(
        &self,
        receipts: &[Receipt],
        grants: &[VerifiedRetryGrant],
        operation: Uuid,
        outcome: &str,
        replacing: Option<&VerifiedRetryGrant>,
    ) -> Result<()> {
        let receipt = unique_receipt(receipts, operation)?;
        source_lineage::require_plain_retry(receipt)?;
        let run = crate::control::run_reference_from_receipt(receipt)?
            .ok_or_else(|| Error::new("worker_binding", "retried validation lacks actual run"))?;
        let result = receipt.observed_result.as_ref();
        self.verify_admitted_header(receipt, &run)?;
        if receipt.phase != ReceiptPhase::Reconciled
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
                "retried validation is not a closed terminal exact granted run",
            ));
        }
        let prior: Vec<_> = receipts
            .iter()
            .filter(|r| r.operation_id != operation)
            .cloned()
            .collect();
        self.verify_current_inner(&prior, grants, replacing)
    }
    pub fn verify_current(
        &self,
        receipts: &[Receipt],
        grants: &[VerifiedRetryGrant],
    ) -> Result<()> {
        self.verify_current_inner(receipts, grants, None)
    }
    fn verify_current_inner(
        &self,
        receipts: &[Receipt],
        grants: &[VerifiedRetryGrant],
        replacing: Option<&VerifiedRetryGrant>,
    ) -> Result<()> {
        self.require_bounded_chain()?;
        self.verify_inventory(receipts)?;
        self.verify_retry_cause()?;
        if let Some(grant) = replacing {
            crate::runtime_workflow::require_current_grant(grant, grants)?;
            if grant.phase() != &self.phase {
                return Err(Error::new(
                    "stale_result",
                    "new validation grant replaces another budget",
                ));
            }
        }
        if grants.iter().any(|g| {
            g.phase() == &self.phase
                && replacing.is_none_or(|expected| g.digest() != expected.digest())
        }) {
            return Err(Error::new(
                "stale_result",
                "validation retry grant was superseded",
            ));
        }
        let mut ancestral = Vec::new();
        let mut granted = Vec::new();
        for receipt in receipts {
            match crate::control::run_reference_from_receipt(receipt)? {
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
                "validation retry no longer binds the latest granted attempt",
            ));
        }
        let mut caps = (
            self.grant.new_budget().max_attempts,
            self.grant.new_budget().max_turns,
            self.grant.new_budget().max_review_rounds,
        );
        for (index, (receipt, run)) in granted.iter().enumerate() {
            closed_failed_validation(receipt)?;
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
                    "granted validation history changes identity, ordinal or captured bounds",
                ));
            }
            caps = (run.max_attempts, run.max_turns, run.max_review_rounds);
        }
        if let Some((latest, _)) = granted.last()
            && latest.operation_id != self.predecessor()
        {
            return Err(Error::new(
                "stale_result",
                "validation retry predecessor is no longer latest",
            ));
        }
        if let Some(lineage) = &self.source_lineage {
            // Whole exact validation inventory and current grant ancestry above
            // must pass before projecting the immutable source-origin prefix.
            lineage.verify_prefix(receipts, grants)?;
        }
        Ok(())
    }
}
/// A caller supplies an actual retained cause, never a marker copied into a new
/// run. Omitting SourceAdvanced for an admission carrying that marker is refused.
pub enum ValidationRetryOrigin {
    Original {
        source_advanced: Option<Arc<VerifiedSourceAdvanced>>,
    },
    Retried(Arc<ValidationRetryWork>),
}
pub struct ValidationRetryRequest<'a> {
    pub admission: &'a DeliveryAdmission,
    pub assignment: &'a DeliveryWorkerAssignment,
    pub origin: ValidationRetryOrigin,
    pub failed: Arc<RunCompletion>,
    pub grant: VerifiedRetryGrant,
    pub current_grants: &'a [VerifiedRetryGrant],
}

/// A completed exact-source validation session within a verified grant.
/// This authenticates the session and its new check handles, not successful
/// checks. `advance_child(ValidateLocal)` must still validate the genuine checks,
/// and its existing failed-check repair path is unchanged.
#[derive(Clone)]
pub(crate) struct CompletedGrantedValidation {
    basis: Arc<ValidationRetryWork>,
    outcome: crate::runtime::VerifiedAuthorizedOutcome,
    completion: Arc<RunCompletion>,
    reference: RunReference,
    receipt_digest: String,
    inventory_digest: String,
    digest: String,
}
impl CompletedGrantedValidation {
    pub(crate) fn resumed_bridge_inventory(
        &self,
    ) -> Result<BTreeMap<Uuid, Arc<crate::worker_run::AdmittedSessionResume>>> {
        Ok(crate::delivery::lineage::collect(self)?.bridges)
    }
    pub(crate) fn verify_resumed_bridges(
        &self,
        reader: &dyn Fn(Uuid) -> Result<Option<Arc<crate::worker_run::AdmittedSessionResume>>>,
    ) -> Result<()> {
        self.basis.verify_resumed_bridges(reader)
    }
    pub(crate) fn authority(&self) -> &FreshGate {
        self.basis.authority()
    }
    pub(crate) fn digest(&self) -> &str {
        &self.digest
    }
    pub(crate) fn outcome(&self) -> &crate::runtime::VerifiedAuthorizedOutcome {
        &self.outcome
    }
    pub(crate) fn completion(&self) -> &Arc<RunCompletion> {
        &self.completion
    }
    pub(crate) fn phase(&self) -> &PhaseKey {
        &self.reference.phase
    }
    pub(crate) fn source(&self) -> &DeliverySource {
        self.basis.source()
    }
    /// Exact captured current validation inventory only; the new typed source
    /// successor separately proves how the old source/review was consumed.
    pub(crate) fn verify_validation_inventory(&self, receipts: &[Receipt]) -> Result<()> {
        if validation_inventory(receipts, self.basis.receipt.runtime_id, &self.reference)?
            != self.inventory_digest
        {
            return Err(Error::new(
                "stale_result",
                "completed validation inventory changed",
            ));
        }
        let actual = unique_receipt(receipts, self.completion.operation_id)?;
        if canonical_digest(1, actual)? != self.receipt_digest {
            return Err(Error::new(
                "stale_result",
                "completed validation receipt changed",
            ));
        }
        child::completed_run(&self.outcome, &self.completion)?;
        bound_completion(
            self.basis.assignment(),
            &self.completion,
            actual,
            &self.reference,
        )
    }
    pub(crate) fn retained_receipt_references(
        &self,
        receipts: &[Receipt],
        grants: &[VerifiedRetryGrant],
    ) -> Result<BTreeMap<Uuid, String>> {
        self.verify_validation_inventory(receipts)?;
        Ok(passed::passed_receipts(self, receipts, grants)?.0)
    }
    pub(crate) fn source_advanced(&self) -> Option<&Arc<VerifiedSourceAdvanced>> {
        self.basis.source_advanced()
    }
    pub(crate) fn original_source_admission(&self) -> Option<&Receipt> {
        self.basis.original_source_admission()
    }
    pub(crate) fn verify_joined(
        &self,
        retained: &impl Fn(Uuid) -> Result<Option<Arc<RunCompletion>>>,
    ) -> Result<()> {
        crate::delivery::lineage::verify_joined(self, retained)
    }
    pub(crate) fn verify_current(
        &self,
        receipts: &[Receipt],
        grants: &[VerifiedRetryGrant],
    ) -> Result<()> {
        if validation_inventory(receipts, self.basis.receipt.runtime_id, &self.reference)?
            != self.inventory_digest
        {
            return Err(Error::new(
                "stale_result",
                "completed granted validation inventory changed",
            ));
        }
        let receipt = receipts
            .iter()
            .find(|r| r.operation_id == self.completion.operation_id)
            .ok_or_else(|| {
                Error::new(
                    "reconciliation_required",
                    "completed granted validation receipt disappeared",
                )
            })?;
        if canonical_digest(1, receipt)? != self.receipt_digest {
            return Err(Error::new(
                "stale_result",
                "completed granted validation receipt changed",
            ));
        }
        self.basis
            .verify_completed_inventory(receipts, grants, self.completion.operation_id)?;
        child::completed_run(&self.outcome, &self.completion)?;
        bound_completion(
            self.basis.assignment(),
            &self.completion,
            receipt,
            &self.reference,
        )
    }
}
impl DeliveryCoordinator {
    pub(crate) async fn completed_granted_validation(
        &self,
        admission: &DeliveryAdmission,
        outcome: &crate::runtime::VerifiedAuthorizedOutcome,
        completion: &Arc<RunCompletion>,
        proof: &ValidationRetryWork,
        grants: &[VerifiedRetryGrant],
    ) -> Result<CompletedGrantedValidation> {
        self.current(admission, &[DeliveryPhase::LocalValidation])?;
        proof
            .assignment()
            .verify_binding(admission.fresh_gate(), "child_local_validation")?;
        child::completed_run(outcome, completion)?;
        proof
            .authority()
            .compare_admitted_authority(outcome.original_authority())?;
        if outcome.outcome().instructions.role != AgentRole::Validation {
            return Err(Error::new(
                "worker_binding",
                "granted outcome is not a validation session",
            ));
        }
        self.verify_validation_retry_source(admission, proof.source())
            .await?;
        let receipts = self.host.operations()?;
        proof.verify_completed_inventory(&receipts, grants, completion.operation_id)?;
        let receipt = self.host.operation(completion.operation_id)?;
        let reference = crate::control::run_reference_from_receipt(&receipt)?.ok_or_else(|| {
            Error::new("worker_binding", "completed validation admission missing")
        })?;
        bound_completion(proof.assignment(), completion, &receipt, &reference)?;
        let receipt_digest = canonical_digest(1, &receipt)?;
        let inventory_digest = validation_inventory(&receipts, receipt.runtime_id, &reference)?;
        let digest = canonical_digest(
            1,
            &json!({
                "purpose":"completed_granted_validation", "basis":proof.digest(),
                "outcome":outcome.digest(), "receipt":receipt_digest, "inventory":inventory_digest
            }),
        )?;
        let result = CompletedGrantedValidation {
            basis: Arc::new(proof.clone()),
            outcome: outcome.clone(),
            completion: completion.clone(),
            reference,
            receipt_digest,
            inventory_digest,
            digest,
        };
        result.verify_current(&receipts, grants)?;
        self.current(admission, &[DeliveryPhase::LocalValidation])?;
        Ok(result)
    }
    async fn verify_validation_retry_source(
        &self,
        admission: &DeliveryAdmission,
        source: &DeliverySource,
    ) -> Result<()> {
        self.current(admission, &[DeliveryPhase::LocalValidation])?;
        if source.repository() != &admission.scope.repository
            || source.revision().commit != admission.scope.revision.head
            || source.revision().tree != admission.scope.revision.tested_tree
        {
            return Err(Error::new(
                "stale_result",
                "validation retry source differs from admitted revision",
            ));
        }
        let checked = source.clone();
        let git = self.git.clone();
        tokio::task::spawn_blocking(move || checked.verify_for_clone(&git))
            .await
            .map_err(|_| {
                Error::new("operation_uncertain", "validation source verification lost")
            })??;
        self.current(admission, &[DeliveryPhase::LocalValidation])
    }
    pub async fn retry_failed_validation(
        &self,
        request: ValidationRetryRequest<'_>,
    ) -> Result<ValidationRetryWork> {
        self.prepare_validation_retry(request).await
    }
    /// A later failed session consumes the same granted budget. Its exact
    /// completed effect replaces only the retry cause, never the grant/source.
    pub async fn continue_failed_validation(
        &self,
        admission: &DeliveryAdmission,
        prior: &ValidationRetryWork,
        failed: Arc<RunCompletion>,
        current_grants: &[VerifiedRetryGrant],
    ) -> Result<ValidationRetryWork> {
        self.prepare_validation_retry(ValidationRetryRequest {
            admission,
            assignment: prior.assignment(),
            origin: ValidationRetryOrigin::Retried(Arc::new(prior.clone())),
            failed,
            grant: prior.grant().clone(),
            current_grants,
        })
        .await
    }
    async fn prepare_validation_retry(
        &self,
        request: ValidationRetryRequest<'_>,
    ) -> Result<ValidationRetryWork> {
        self.current(request.admission, &[DeliveryPhase::LocalValidation])?;
        request
            .assignment
            .verify_binding(request.admission.fresh_gate(), "child_local_validation")?;
        failed_validation(&request.failed)?;
        let contract =
            request.failed.contract.as_ref().ok_or_else(|| {
                Error::new("worker_binding", "failed validation contract missing")
            })?;
        validation_contract(request.assignment, contract)?;
        let source = request
            .assignment
            .source()
            .ok_or_else(|| Error::new("worker_binding", "validation source missing"))?;
        self.verify_validation_retry_source(request.admission, source)
            .await?;
        let receipts = self.host.operations()?;
        let receipt = self.host.operation(request.failed.operation_id)?;
        closed_failed_validation(&receipt)?;
        if receipt
            .observed_result
            .as_ref()
            .and_then(|v| v.get("observation"))
            != Some(&request.failed.observation())
        {
            return Err(Error::new(
                "worker_binding",
                "joined failed validation differs from protected terminal observation",
            ));
        }
        let reference = crate::control::run_reference_from_receipt(&receipt)?
            .ok_or_else(|| Error::new("worker_binding", "failed validation lacks admission"))?;
        if receipt.runtime_id != request.admission.scope.runtime_id {
            return Err(Error::new("worker_binding", "validation runtime differs"));
        }
        bound_completion(request.assignment, &request.failed, &receipt, &reference)?;
        let (previous, source_lineage, same_budget) = match request.origin {
            ValidationRetryOrigin::Original { source_advanced } => {
                let lineage = ValidationSourceLineage::from_original(
                    request.assignment,
                    &request.failed,
                    &receipt,
                    &reference,
                    source_advanced,
                    &receipts,
                    request.current_grants,
                )?;
                (None, lineage.map(Arc::new), false)
            }
            ValidationRetryOrigin::Retried(prior) => {
                prior.verify_assignment(request.assignment)?;
                let same_budget = prior.phase() == &request.grant.phase_key();
                if same_budget && prior.grant().digest() != request.grant.digest() {
                    return Err(Error::new(
                        "stale_result",
                        "validation budget changed its grant",
                    ));
                }
                prior.verify_terminal_inventory_inner(
                    &receipts,
                    request.current_grants,
                    request.failed.operation_id,
                    "failed",
                    (!same_budget).then_some(&request.grant),
                )?;
                let lineage = prior.source_lineage.clone();
                (Some(prior), lineage, same_budget)
            }
        };
        let transient_failure = if same_budget {
            Some(request.failed.state.terminal_transient_failure()?.ok_or_else(|| {
                Error::new("persistent_blocker", "known validation failure is not a supported transient provider failure")
            })?)
        } else {
            None
        };
        let receipt_digest = canonical_digest(1, &receipt)?;
        let inventory_digest = validation_inventory(&receipts, receipt.runtime_id, &reference)?;
        let digest = canonical_digest(
            1,
            &json!({"purpose":"exact_source_validation_retry","assignment":request.assignment.digest()?,"failed":receipt_digest,"inventory":inventory_digest,"grant":request.grant.digest(),"previous":previous.as_deref().map(ValidationRetryWork::digest),"source_lineage":source_lineage.as_ref().map(|lineage| lineage.digest()),"transient_failure":transient_failure}),
        )?;
        let result = ValidationRetryWork {
            assignment: request.assignment.clone(),
            failed: request.failed,
            transient_failure,
            origin_reference: if same_budget {
                previous
                    .as_ref()
                    .expect("verified previous budget")
                    .origin_reference
                    .clone()
            } else {
                reference.clone()
            },
            source_lineage,
            previous,
            reference,
            receipt,
            receipt_digest,
            inventory_digest,
            phase: request.grant.phase_key(),
            grant: request.grant,
            digest,
        };
        result.verify_current(&receipts, request.current_grants)?;
        self.current(request.admission, &[DeliveryPhase::LocalValidation])?;
        Ok(result)
    }
}
fn failed_validation(completion: &RunCompletion) -> Result<()> {
    super::discovered_validation::require_legacy_continuation(completion)?;
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
        || completion
            .checks
            .iter()
            .any(|check| check.record().binding.operation_id != completion.operation_id)
        || completion
            .failed_checks
            .iter()
            .any(|check| check.record().binding.operation_id != completion.operation_id)
    {
        return Err(Error::new(
            "reconciliation_required",
            "validation retry requires actual known failed terminal session and joined exit",
        ));
    }
    Ok(())
}
/// The original closed receipt and host-owned contract must agree with the
/// joined session. This does not deserialize a worker claim or mint a check.
fn bound_completion(
    assignment: &DeliveryWorkerAssignment,
    completion: &RunCompletion,
    receipt: &Receipt,
    reference: &RunReference,
) -> Result<()> {
    let contract = completion
        .contract
        .as_ref()
        .ok_or_else(|| Error::new("worker_binding", "validation contract missing"))?;
    validation_contract(assignment, contract)?;
    let manifest = &contract.instructions().manifest;
    if receipt.operation_id != completion.operation_id
        || receipt
            .observed_result
            .as_ref()
            .and_then(|v| v.get("observation"))
            != Some(&completion.observation())
        || reference.workspace != assignment.authority().workspace_id()
        || reference.phase.work != assignment.authority().work_key()?
        || reference.phase.generation
            != canonical_digest(1, &assignment.authority().work().generation)?
        || reference.phase.phase != "child_local_validation"
        || reference.account != contract.account_id()
        || reference.instruction_digest != contract.instructions().manifest_digest
        || reference.profile_digest != canonical_digest(1, &manifest.profile)?
        || manifest.operation_id != completion.operation_id
        || completion.state.binding.operation_id != completion.operation_id.to_string()
        || completion.state.binding.instruction_digest != contract.instructions().manifest_digest
        || completion.state.binding.work_identity != reference.phase.work
        || completion.state.binding.generation != manifest.work_generation
        || completion.state.binding.cwd != manifest.cwd
        || completion.state.binding.isolation_digest != manifest.permissions.enforcement_profile
        || completion.state.binding.account_id != reference.account
    {
        return Err(Error::new(
            "worker_binding",
            "validation session differs from exact admitted contract",
        ));
    }
    Ok(())
}
// Validation permits configured check execution/disposable resources. Do not
// copy the Review role's code-execution prohibition. WorkerContract is already
// host-built; preparation/final admission must repeat its actual isolation and
// resource checks, and retry cannot change the exact assignment or source.
fn validation_contract(
    assignment: &DeliveryWorkerAssignment,
    contract: &WorkerContract,
) -> Result<()> {
    let manifest = &contract.instructions().manifest;
    let WorkerAuthority::Authorized(gate) = contract.authority() else {
        return Err(Error::new(
            "worker_binding",
            "validation must have child authority",
        ));
    };
    // These are immutable admission records, potentially retained long after
    // the validation finished. Fresh authority is checked separately at producer,
    // preparation, final model admission and publication gates.
    assignment.authority().compare_admitted_authority(gate)?;
    if assignment.role() != AgentRole::Validation
        || assignment.phase() != "child_local_validation"
        || !assignment.read_sources().is_empty()
        || assignment.candidate_vector_digest().is_some()
        || manifest.role != AgentRole::Validation
        || !manifest.permissions.source_write_paths.is_empty()
        || manifest.repository_revision.as_deref()
            != assignment.source().map(|s| s.revision().commit.as_str())
    {
        return Err(Error::new(
            "worker_binding",
            "validation retry contract changed role, source, or permissions",
        ));
    }
    assignment::verify_assignment_contract(
        contract,
        assignment.authority(),
        assignment.phase(),
        assignment.context(),
    )
}
fn closed_failed_validation(receipt: &Receipt) -> Result<()> {
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
            "failed validation effects are not exactly retired",
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
fn validation_inventory(
    receipts: &[Receipt],
    runtime: Uuid,
    reference: &RunReference,
) -> Result<String> {
    let mut inventory = BTreeMap::new();
    for receipt in receipts {
        if let Some(run) = crate::control::run_reference_from_receipt(receipt)?
            && same_group(runtime, reference, receipt, &run)
            && inventory.insert(receipt.operation_id, receipt).is_some()
        {
            return Err(Error::new(
                "record_corrupt",
                "duplicate validation admission",
            ));
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
            "retry grant does not replace exact failed validation budget",
        ));
    }
    let mut cursor = grant;
    let mut phases = BTreeSet::new();
    for _ in 0..MAX_VALIDATION_HISTORY_RECEIPTS {
        let phase = cursor.phase();
        if cursor.work() != gate.work()
            || cursor.record().runtime_id != runtime
            || cursor.record().workspace_id != gate.workspace_id()
        {
            return Err(Error::new(
                "identity_mismatch",
                "validation grant lineage changes work or workspace",
            ));
        }
        if !phases.insert(canonical_digest(1, phase)?)
            || grants.iter().filter(|g| g.phase() == phase).count() != 1
        {
            return Err(Error::new(
                "stale_result",
                "validation grant lineage is cyclic or forked",
            ));
        }
        let mut prior = BTreeMap::new();
        for receipt in receipts {
            if let Some(run) = crate::control::run_reference_from_receipt(receipt)?
                && run.phase == *phase
                && same_group(runtime, reference, receipt, &run)
            {
                if run.account != reference.account {
                    return Err(Error::new(
                        "identity_mismatch",
                        "validation lineage changes account",
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
                        "validation lineage retains uncertain ownership",
                    ));
                }
                prior.insert(receipt.operation_id, receipt.payload_digest.clone());
            }
        }
        if prior != cursor.record().prior_admissions {
            return Err(Error::new(
                "stale_result",
                "validation grant inventory changed",
            ));
        }
        match phase.retry_grant {
            Some(id) => {
                cursor = grants
                    .iter()
                    .find(|g| g.record().operation_id == id && g.phase_key() == *phase)
                    .ok_or_else(|| {
                        Error::new("stale_result", "validation grant predecessor unavailable")
                    })?;
                crate::runtime_workflow::require_current_grant(cursor, grants)?;
            }
            None => {
                for receipt in receipts {
                    if let Some(run) = crate::control::run_reference_from_receipt(receipt)?
                        && same_group(runtime, reference, receipt, &run)
                        && (!phases.contains(&canonical_digest(1, &run.phase)?)
                            || run.phase == reference.phase && run.attempt > reference.attempt)
                    {
                        return Err(Error::new(
                            "stale_result",
                            "validation has later or unrelated granted history",
                        ));
                    }
                }
                return Ok(());
            }
        }
    }
    Err(Error::new(
        "bounded_input",
        "validation grant lineage exceeds bound",
    ))
}

fn unique_receipt(receipts: &[Receipt], operation: Uuid) -> Result<&Receipt> {
    let mut found = receipts
        .iter()
        .filter(|receipt| receipt.operation_id == operation);
    let receipt = found.next().ok_or_else(|| {
        Error::new(
            "reconciliation_required",
            "validation lineage receipt is missing",
        )
    })?;
    if found.next().is_some() {
        return Err(Error::new(
            "record_corrupt",
            "validation lineage has duplicate receipt identities",
        ));
    }
    Ok(receipt)
}

#[cfg(test)]
mod history_bound_tests {
    use super::*;

    // No opaque retry/check capability is constructed: this is the exact
    // data-count predicate used before child launch and history insertion.
    #[test]
    fn validation_history_capacity_uses_protected_receipt_bound() {
        ValidationRetryWork::require_history_capacity(1024).unwrap();
        ValidationRetryWork::require_history_capacity(4096).unwrap();
        ValidationRetryWork::require_history_capacity(MAX_VALIDATION_HISTORY_RECEIPTS - 1).unwrap();
        for retained in [MAX_VALIDATION_HISTORY_RECEIPTS, usize::MAX] {
            assert_eq!(
                ValidationRetryWork::require_history_capacity(retained)
                    .unwrap_err()
                    .code,
                "bounded_input"
            );
        }
    }
}

#[cfg(test)]
mod history_drop_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct Counters {
        dropped: AtomicUsize,
        active: AtomicUsize,
        maximum_depth: AtomicUsize,
    }

    // Plain nodes exercise the same private destructor helper. They are not
    // validation capabilities and carry no worker, source, grant or check data.
    struct Node {
        previous: Option<Arc<Node>>,
        counters: Arc<Counters>,
    }
    impl Drop for Node {
        fn drop(&mut self) {
            let active = self.counters.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.counters
                .maximum_depth
                .fetch_max(active, Ordering::SeqCst);
            drop_arc_history(self.previous.take(), |node| node.previous.take());
            self.counters.dropped.fetch_add(1, Ordering::SeqCst);
            self.counters.active.fetch_sub(1, Ordering::SeqCst);
        }
    }
    fn chain(count: usize, mut previous: Option<Arc<Node>>, counters: &Arc<Counters>) -> Arc<Node> {
        for _ in 0..count {
            previous = Some(Arc::new(Node {
                previous,
                counters: counters.clone(),
            }));
        }
        previous.expect("nonempty data-only history")
    }

    #[test]
    fn long_unique_retry_history_drops_once_at_constant_depth() {
        let counters = Arc::new(Counters::default());
        let head = chain(MAX_VALIDATION_HISTORY_RECEIPTS, None, &counters);
        let weak = Arc::downgrade(&head);
        drop(head);
        assert!(weak.upgrade().is_none());
        assert_eq!(
            counters.dropped.load(Ordering::SeqCst),
            MAX_VALIDATION_HISTORY_RECEIPTS
        );
        assert_eq!(counters.active.load(Ordering::SeqCst), 0);
        assert!(counters.maximum_depth.load(Ordering::SeqCst) <= 2);
    }

    #[test]
    fn shared_retry_tail_survives_until_its_last_owner_and_drops_once() {
        let counters = Arc::new(Counters::default());
        let tail = chain(MAX_VALIDATION_HISTORY_RECEIPTS, None, &counters);
        let weak = Arc::downgrade(&tail);
        let first = chain(17, Some(tail.clone()), &counters);
        let second = chain(23, Some(tail.clone()), &counters);
        drop(first);
        assert_eq!(counters.dropped.load(Ordering::SeqCst), 17);
        assert!(weak.upgrade().is_some());
        drop(second);
        assert_eq!(counters.dropped.load(Ordering::SeqCst), 40);
        assert!(weak.upgrade().is_some());
        drop(tail);
        assert!(weak.upgrade().is_none());
        assert_eq!(
            counters.dropped.load(Ordering::SeqCst),
            MAX_VALIDATION_HISTORY_RECEIPTS + 40
        );
        assert_eq!(counters.active.load(Ordering::SeqCst), 0);
        assert!(counters.maximum_depth.load(Ordering::SeqCst) <= 2);
    }
}

impl CompletedGrantedValidation {
    pub(crate) fn retained_native_grants(&self) -> Result<BTreeMap<Uuid, VerifiedRetryGrant>> {
        crate::delivery::lineage::collect(self)?.into_grants()
    }
}

impl crate::delivery::lineage::Node for ValidationRetryWork {
    fn expand<'a>(
        &'a self,
        children: &mut Vec<&'a dyn crate::delivery::lineage::Node>,
        leaves: &mut crate::delivery::lineage::Inventory,
    ) -> Result<()> {
        leaves.sources([self.source().clone()])?;
        leaves.grants([self.grant.clone()])?;
        leaves.joined(1)?;
        if let Some(p) = &self.previous {
            children.push(p.as_ref());
        }
        if let Some(p) = &self.source_lineage {
            children.push(p.as_ref());
        }
        Ok(())
    }
    fn verify_joined_leaf(
        &self,
        reader: &dyn Fn(Uuid) -> Result<Option<Arc<RunCompletion>>>,
    ) -> Result<()> {
        let actual = reader(self.predecessor())?.ok_or_else(|| {
            Error::new(
                "reconciliation_required",
                "validation retry lost a joined predecessor",
            )
        })?;
        self.verify_completion(&actual)
    }
}
impl crate::delivery::lineage::Node for CompletedGrantedValidation {
    fn expand<'a>(
        &'a self,
        children: &mut Vec<&'a dyn crate::delivery::lineage::Node>,
        leaves: &mut crate::delivery::lineage::Inventory,
    ) -> Result<()> {
        children.push(self.basis.as_ref());
        leaves.joined(1)
    }
    fn verify_joined_leaf(
        &self,
        reader: &dyn Fn(Uuid) -> Result<Option<Arc<RunCompletion>>>,
    ) -> Result<()> {
        source_lineage::require_joined(&self.completion, &|id| reader(id))
    }
}

impl crate::delivery::lineage_drop::Release for ValidationRetryWork {
    fn release(mut self, pending: &mut crate::delivery::lineage_drop::Pending) {
        if let Some(p) = self.previous.take() {
            pending.push(p);
        }
        if let Some(p) = self.source_lineage.take() {
            pending.push(p);
        }
        // The existing iterative Drop sees no remaining previous edge.
    }
}
impl crate::delivery::lineage_drop::Release for CompletedGrantedValidation {
    fn release(self, pending: &mut crate::delivery::lineage_drop::Pending) {
        pending.push(self.basis);
    }
}
