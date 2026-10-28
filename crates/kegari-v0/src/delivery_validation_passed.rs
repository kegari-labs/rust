//! Genuine passed-validation provenance, transferred only to an observed round.
//! Serialized references describe retained facts and never recreate these caps.
use super::*;
use crate::control::{PassedValidationReference, PreparationReference, ReviewReceiptReference};
use crate::delivery::{PassedValidationSuccessor, verify_validation_successor_admission};
use crate::runtime::RuntimeCoordinator;
use chrono::Duration;

#[path = "delivery_validation_passed_basis.rs"]
mod basis;
use basis::PassedValidationBasis;

#[derive(Clone)]
pub(crate) struct VerifiedPassedValidation {
    reference: PassedValidationReference,
    completed: PassedValidationBasis,
    local: LocalPassed,
    check_digests: Vec<String>,
    grants: Vec<VerifiedRetryGrant>,
    source_completion: Arc<RunCompletion>,
    digest: String,
    control: Arc<crate::control::ControlHost>,
}
impl VerifiedPassedValidation {
    pub(crate) fn resumed_bridge_inventory(
        &self,
    ) -> Result<BTreeMap<Uuid, Arc<crate::worker_run::AdmittedSessionResume>>> {
        Ok(crate::delivery::lineage::collect(self)?.bridges)
    }
    pub(crate) fn verify_resumed_bridges(
        &self,
        reader: &dyn Fn(Uuid) -> Result<Option<Arc<crate::worker_run::AdmittedSessionResume>>>,
    ) -> Result<()> {
        self.completed.verify_resumed_bridges(reader)
    }
    pub(crate) fn reference(&self) -> &PassedValidationReference {
        &self.reference
    }
    pub(crate) fn digest(&self) -> &str {
        &self.digest
    }
    pub(crate) fn authority(&self) -> &FreshGate {
        self.completed.authority()
    }
    pub(crate) fn source(&self) -> &DeliverySource {
        self.completed.source()
    }
    pub(crate) fn validation_outcome(&self) -> &crate::runtime::VerifiedAuthorizedOutcome {
        self.completed.outcome()
    }
    pub(crate) fn validation_completion(&self) -> &Arc<RunCompletion> {
        self.completed.completion()
    }
    pub(crate) fn verify_validation_inventory(&self, receipts: &[Receipt]) -> Result<()> {
        self.completed.verify_validation_inventory(receipts)
    }
    pub(crate) fn validation_source(&self) -> Option<&Arc<VerifiedSourceAdvanced>> {
        self.completed.source_advanced()
    }
    pub(crate) fn verify_current(
        &self,
        receipts: &[Receipt],
        grants: &[VerifiedRetryGrant],
    ) -> Result<()> {
        self.completed.verify_current(receipts, grants)?;
        self.verify_retained(receipts, grants)
    }
    /// Immutable evidence only. A caller must separately prove the exact actual
    /// successor and its current namespace before this replaces verify_current.
    /// Later legitimately admitted source/validation generations do not erase
    /// this captured closure; full current inventory remains verify_current's job.
    pub(crate) fn verify_retained(
        &self,
        receipts: &[Receipt],
        grants: &[VerifiedRetryGrant],
    ) -> Result<()> {
        if self.reference.schema_version != 1
            || self.reference.completed_validation_digest != self.completed.digest()
            || self.reference.local_pass_digest != local_digest(&self.local)?
            || self.reference.source.source_digest != self.source().digest()?
            || self.digest != canonical_digest(1, &self.reference)?
        {
            return Err(passed_error("passed validation immutable identity changed"));
        }
        if self.reference.retained_receipts.len() > MAX_VALIDATION_HISTORY_RECEIPTS {
            return Err(Error::new(
                "bounded_input",
                "passed validation references exceed bound",
            ));
        }
        for (id, expected) in &self.reference.retained_receipts {
            if canonical_digest(1, unique_receipt(receipts, *id)?)? != *expected {
                return Err(passed_error("passed validation retained receipt changed"));
            }
        }
        for grant in &self.grants {
            crate::runtime_workflow::require_current_grant(grant, grants)?;
            if grants
                .iter()
                .filter(|current| current.phase() == grant.phase())
                .count()
                != 1
            {
                return Err(passed_error(
                    "passed validation retained grant namespace is ambiguous",
                ));
            }
        }
        let completion = self.completed.completion();
        child::completed_run(self.completed.outcome(), completion)?;
        bound_completion(
            self.completed.assignment(),
            completion,
            unique_receipt(receipts, completion.operation_id)?,
            self.completed.reference(),
        )?;
        let actual = completion
            .checks
            .iter()
            .map(|check| {
                check.production_evidence()?;
                check.digest()
            })
            .collect::<Result<Vec<_>>>()?;
        if actual != self.check_digests {
            return Err(passed_error(
                "passed validation genuine check inventory changed",
            ));
        }
        self.source().revalidate_filesystem()
    }
    pub(crate) fn verify_joined(
        &self,
        retained: &impl Fn(Uuid) -> Result<Option<Arc<RunCompletion>>>,
    ) -> Result<()> {
        crate::delivery::lineage::verify_joined(self, retained)
    }
}

/// The original genuine pass plus an actual model/preparation observation.
/// The model's request is immutable while its lifecycle remains live. No claim
/// of terminal completion is made by observing a Prepared admission.
#[derive(Clone)]
pub(crate) struct ConsumedValidationSuccessor {
    passed: Arc<VerifiedPassedValidation>,
    round: Arc<VerifiedReviewRoundPlan>,
    observed: PassedValidationSuccessor,
    digest: String,
}
impl ConsumedValidationSuccessor {
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
    pub(crate) fn digest(&self) -> &str {
        &self.digest
    }
    pub(crate) fn authority(&self) -> &FreshGate {
        self.round.authority()
    }
    pub(crate) fn worker_operation(&self) -> Uuid {
        self.observed.operation()
    }
    /// Only the explicit RetryReview metadata phase may use this transition.
    /// It preserves immutable passed evidence and consumes no model budget.
    pub(crate) fn verify_retry_preparation(
        &self,
        receipts: &[Receipt],
        grants: &[VerifiedRetryGrant],
        grant: &VerifiedRetryGrant,
        failed: &Arc<RunCompletion>,
    ) -> Result<()> {
        self.round.verify_validation_successor_retry(
            receipts,
            grants,
            &self.observed,
            grant,
            failed,
        )?;
        self.passed.verify_retained(receipts, grants)
    }
    pub(crate) fn verify_current(
        &self,
        receipts: &[Receipt],
        grants: &[VerifiedRetryGrant],
    ) -> Result<()> {
        let observed = verify_validation_successor_admission(
            receipts,
            self.observed.operation(),
            self.passed.reference(),
        )?;
        if observed.operation() != self.observed.operation()
            || observed.round() != self.observed.round()
            || observed.preparation() != self.observed.preparation()
            || observed.request_digest() != self.observed.request_digest()
        {
            return Err(passed_error(
                "validation successor immutable admission changed",
            ));
        }
        self.round
            .verify_validation_successor_current(receipts, grants, &observed)?;
        self.passed.verify_retained(receipts, grants)
    }
    pub(crate) fn verify_joined(
        &self,
        retained: &impl Fn(Uuid) -> Result<Option<Arc<RunCompletion>>>,
    ) -> Result<()> {
        crate::delivery::lineage::verify_joined(self, retained)
    }
}

impl DeliveryCoordinator {
    pub(crate) async fn passed_granted_validation(
        &self,
        source: RepairSource<'_>,
        workflow: &Workflow,
        completed: Arc<CompletedGrantedValidation>,
        local: &LocalPassed,
        runtime: &RuntimeCoordinator,
    ) -> Result<VerifiedPassedValidation> {
        self.passed_validation(
            source,
            workflow,
            PassedValidationBasis::Retried(completed),
            local,
            runtime,
        )
        .await
    }
    async fn passed_validation(
        &self,
        source: RepairSource<'_>,
        workflow: &Workflow,
        completed: PassedValidationBasis,
        local: &LocalPassed,
        runtime: &RuntimeCoordinator,
    ) -> Result<VerifiedPassedValidation> {
        if !Arc::ptr_eq(&self.host, &runtime.control) {
            return Err(passed_error(
                "passed validation uses another actual control host",
            ));
        }
        super::super::discovered_validation::require_legacy_continuation(completed.completion())?;
        self.current(source.admission, &[DeliveryPhase::LocalValidation])?;
        local.matches(source.admission)?;
        completed
            .assignment()
            .verify_binding(source.admission.fresh_gate(), "child_local_validation")?;
        let (receipts, grants) = current_inputs(runtime, completed.authority())?;
        completed.verify_current(&receipts, &grants)?;
        completed.verify_joined(&|id| runtime.retained_completion(id))?;
        completed.verify_resumed_bridges(&|id| runtime.retained_session_resume(id))?;
        source_lineage::require_joined(&source.completion, &|id| runtime.retained_completion(id))?;
        // This helper performs the same actual source/claim/seal/Git observation
        // as round construction; it cannot convert a raw source reference to proof.
        let source_reference = self.reviewed_repair_source_reference(&source).await?;
        if source_reference.source_digest != completed.source().digest()? {
            return Err(passed_error(
                "passed validation belongs to another implementation source",
            ));
        }
        let required = local_requirements(self.contract(source.admission)?, workflow)?;
        let completion = completed.completion();
        let checks: Vec<_> = completion
            .checks
            .iter()
            .map(|c| (c.record(), false))
            .chain(completion.failed_checks.iter().map(|c| (c.record(), true)))
            .collect();
        match super::super::continuation::latest_required_failures(&required, &checks) {
            Ok(_) => {
                return Err(Error::new(
                    "validation_failed",
                    "latest required check remains failed",
                ));
            }
            Err(error) if error.code == "no_work_trigger" => {}
            Err(error) => return Err(error),
        }
        let actual_local = self.verify_local_checks(
            source.admission,
            workflow,
            &completion.checks.iter().collect::<Vec<_>>(),
        )?;
        if local_digest(&actual_local)? != local_digest(local)? {
            return Err(passed_error(
                "LocalPassed does not match this actual completed retry",
            ));
        }
        // Re-read after the asynchronous source observation, before retaining any
        // historical closure. Final native admission still repeats verify_current.
        let (receipts, current_grants) = current_inputs(runtime, completed.authority())?;
        completed.verify_current(&receipts, &current_grants)?;
        completed.verify_joined(&|id| runtime.retained_completion(id))?;
        completed.verify_resumed_bridges(&|id| runtime.retained_session_resume(id))?;
        self.current(source.admission, &[DeliveryPhase::LocalValidation])?;
        let (retained_receipts, retained_grants) =
            completed.retained_receipts(&receipts, &current_grants)?;
        let mut retained_receipts = retained_receipts;
        for reference in [&source_reference.implementation, &source_reference.seal] {
            if canonical_digest(1, unique_receipt(&receipts, reference.operation)?)?
                != reference.receipt_digest
            {
                return Err(passed_error(
                    "reviewed implementation receipt changed during pass capture",
                ));
            }
            retain_reference(
                &mut retained_receipts,
                reference.operation,
                reference.receipt_digest.clone(),
            )?;
        }
        let reference = PassedValidationReference {
            schema_version: 1,
            source: source_reference,
            validation: ReviewReceiptReference {
                operation: completion.operation_id,
                receipt_digest: completed.receipt_digest().into(),
            },
            completed_validation_digest: completed.digest().into(),
            local_pass_digest: local_digest(&actual_local)?,
            retained_receipts,
        };
        let check_digests = completion
            .checks
            .iter()
            .map(|check| check.digest())
            .collect::<Result<_>>()?;
        let proof = VerifiedPassedValidation {
            digest: canonical_digest(1, &reference)?,
            reference,
            completed,
            local: actual_local,
            check_digests,
            grants: retained_grants,
            source_completion: source.completion,
            control: runtime.control.clone(),
        };
        proof.verify_current(&receipts, &current_grants)?;
        proof.verify_joined(&|id| runtime.retained_completion(id))?;
        proof.verify_resumed_bridges(&|id| runtime.retained_session_resume(id))?;
        Ok(proof)
    }
}
impl VerifiedPassedValidation {
    pub(crate) fn consume_successor(
        self: &Arc<Self>,
        round: Arc<VerifiedReviewRoundPlan>,
        worker_operation: Uuid,
        runtime: &RuntimeCoordinator,
    ) -> Result<ConsumedValidationSuccessor> {
        if !Arc::ptr_eq(&self.control, &runtime.control) {
            return Err(passed_error(
                "validation handoff uses another actual control host",
            ));
        }
        self.authority()
            .compare_admitted_authority(round.authority())?;
        let passed = self.clone();
        let (receipts, grants) = current_inputs(runtime, round.authority())?;
        let observed =
            verify_validation_successor_admission(&receipts, worker_operation, passed.reference())?;
        let digest = canonical_digest(
            1,
            &json!({
                "purpose":"consumed_passed_validation", "passed":passed.digest(),
                "round":round.digest(), "operation":observed.operation(),
                "preparation":observed.preparation(), "request":observed.request_digest(),
            }),
        )?;
        let proof = ConsumedValidationSuccessor {
            passed,
            round,
            observed,
            digest,
        };
        proof.verify_current(&receipts, &grants)?;
        proof.verify_joined(&|id| runtime.retained_completion(id))?;
        proof.verify_resumed_bridges(&|id| runtime.retained_session_resume(id))?;
        Ok(proof)
    }
}

fn current_inputs(
    runtime: &RuntimeCoordinator,
    gate: &FreshGate,
) -> Result<(Vec<Receipt>, Vec<VerifiedRetryGrant>)> {
    runtime
        .configuration
        .authority_fences()?
        .check_workspace(gate.workspace_id())?;
    if crate::config::config_revision(&runtime.configuration.snapshot()?)? != gate.config_revision()
    {
        return Err(Error::new(
            "stale_config",
            "validation successor configuration changed",
        ));
    }
    let projection = runtime.observations.admission_snapshot()?;
    let current = projection
        .parent(gate.workspace_id(), gate.work().parent)?
        .fresh_gate(
            gate.work().child,
            gate.purpose(),
            &runtime.dependency_outcomes()?,
            Utc::now(),
            Duration::seconds(30),
        )?;
    gate.compare_current(&current, Utc::now(), Duration::seconds(30))?;
    let receipts = runtime.control.operations()?;
    let grants = crate::runtime_workflow::current_retry_grants(
        &runtime.configuration.snapshot()?,
        &runtime.observations.admission_snapshot()?,
        gate.work(),
        &receipts,
        &runtime.services.key,
    )?;
    Ok((receipts, grants))
}
fn local_digest(local: &LocalPassed) -> Result<String> {
    canonical_digest(
        1,
        &json!({"work":local.work, "revision":local.revision,
        "workflow":local.workflow_digest, "checks":local.checks}),
    )
}
pub(super) fn passed_receipts(
    completed: &CompletedGrantedValidation,
    receipts: &[Receipt],
    grants: &[VerifiedRetryGrant],
) -> Result<(BTreeMap<Uuid, String>, Vec<VerifiedRetryGrant>)> {
    let mut retained = BTreeMap::new();
    let mut runs = BTreeSet::new();
    for receipt in receipts {
        if let Some(run) = RunReference::from_receipt(receipt)?
            && same_group(
                completed.basis.receipt.runtime_id,
                &completed.reference,
                receipt,
                &run,
            )
        {
            retain_reference(
                &mut retained,
                receipt.operation_id,
                canonical_digest(1, receipt)?,
            )?;
            runs.insert(receipt.operation_id);
        }
    }
    for receipt in receipts {
        if let Some(preparation) = PreparationReference::from_receipt(receipt)?
            && runs.contains(&preparation.worker_operation)
        {
            if receipt.phase != ReceiptPhase::Reconciled {
                return Err(passed_error(
                    "completed validation preparation remains uncertain",
                ));
            }
            retain_reference(
                &mut retained,
                receipt.operation_id,
                canonical_digest(1, receipt)?,
            )?;
        }
    }
    let mut retained_grants = Vec::new();
    let mut grant = &completed.basis.grant;
    let mut seen = BTreeSet::new();
    loop {
        if !seen.insert(grant.record().operation_id) || seen.len() > MAX_VALIDATION_HISTORY_RECEIPTS
        {
            return Err(Error::new(
                "bounded_input",
                "passed validation grant chain is cyclic or oversized",
            ));
        }
        crate::runtime_workflow::require_current_grant(grant, grants)?;
        let receipt = unique_receipt(receipts, grant.record().operation_id)?;
        retain_reference(
            &mut retained,
            receipt.operation_id,
            canonical_digest(1, receipt)?,
        )?;
        retained_grants.push(grant.clone());
        match grant.phase().retry_grant {
            None => break,
            Some(id) => {
                grant = grants
                    .iter()
                    .find(|g| g.record().operation_id == id && g.phase_key() == *grant.phase())
                    .ok_or_else(|| passed_error("passed validation original grant is missing"))?;
            }
        }
    }
    if let Some(advanced) = completed.source_advanced() {
        for (id, digest) in advanced.retained_receipt_references()? {
            if canonical_digest(1, unique_receipt(receipts, id)?)? != digest {
                return Err(passed_error("passed validation source ancestry changed"));
            }
            retain_reference(&mut retained, id, digest)?;
        }
    }
    // The opaque source closure can itself depend on a granted review or
    // repair budget. Retain only native grants named by that exact closure.
    for grant in grants {
        if retained.contains_key(&grant.record().operation_id)
            && !retained_grants
                .iter()
                .any(|old| old.record().operation_id == grant.record().operation_id)
        {
            crate::runtime_workflow::require_current_grant(grant, grants)?;
            retained_grants.push(grant.clone());
        }
    }
    Ok((retained, retained_grants))
}
fn retain_reference(retained: &mut BTreeMap<Uuid, String>, id: Uuid, digest: String) -> Result<()> {
    if let Some(previous) = retained.insert(id, digest.clone())
        && previous != digest
    {
        return Err(passed_error("passed validation retained receipt conflicts"));
    }
    if retained.len() > MAX_VALIDATION_HISTORY_RECEIPTS {
        return Err(Error::new(
            "bounded_input",
            "passed validation retained inventory is oversized",
        ));
    }
    Ok(())
}
fn passed_error(message: &str) -> Error {
    Error::new("validation_successor", message)
}

impl VerifiedPassedValidation {
    pub(crate) fn retained_native_grants(&self) -> Result<BTreeMap<Uuid, VerifiedRetryGrant>> {
        crate::delivery::lineage::collect(self)?.into_grants()
    }
}

impl crate::delivery::lineage::Node for VerifiedPassedValidation {
    fn expand<'a>(
        &'a self,
        children: &mut Vec<&'a dyn crate::delivery::lineage::Node>,
        leaves: &mut crate::delivery::lineage::Inventory,
    ) -> Result<()> {
        children.push(&self.completed);
        leaves.sources([self.source().clone()])?;
        leaves.grants(self.grants.iter().cloned())?;
        leaves.joined(1)
    }
    fn verify_joined_leaf(
        &self,
        reader: &dyn Fn(Uuid) -> Result<Option<Arc<RunCompletion>>>,
    ) -> Result<()> {
        source_lineage::require_joined(&self.source_completion, &|id| reader(id))
    }
}
impl crate::delivery::lineage::Node for ConsumedValidationSuccessor {
    fn expand<'a>(
        &'a self,
        children: &mut Vec<&'a dyn crate::delivery::lineage::Node>,
        _leaves: &mut crate::delivery::lineage::Inventory,
    ) -> Result<()> {
        children.push(self.passed.as_ref());
        children.push(self.round.as_ref());
        Ok(())
    }
}

impl crate::delivery::lineage_drop::Release for VerifiedPassedValidation {
    fn release(self, pending: &mut crate::delivery::lineage_drop::Pending) {
        crate::delivery::lineage_drop::Release::release(self.completed, pending);
    }
}

#[cfg(test)]
mod history_bound_tests {
    use super::*;

    // These are plain retained-reference data. This does not construct genuine
    // check evidence, a passed capability or a serializable round admission.
    #[test]
    fn retained_reference_bound_counts_unique_receipts_without_a_retry_cap() {
        let mut refs = BTreeMap::new();
        let digest = "a".repeat(64);
        for id in 1..=MAX_VALIDATION_HISTORY_RECEIPTS {
            retain_reference(&mut refs, Uuid::from_u128(id as u128), digest.clone()).unwrap();
        }
        assert_eq!(refs.len(), MAX_VALIDATION_HISTORY_RECEIPTS);
        // Re-reading one exact immutable reference does not consume capacity.
        retain_reference(&mut refs, Uuid::from_u128(1), digest.clone()).unwrap();
        assert_eq!(refs.len(), MAX_VALIDATION_HISTORY_RECEIPTS);
        assert_eq!(
            retain_reference(&mut refs, Uuid::from_u128(1), "b".repeat(64))
                .unwrap_err()
                .code,
            "validation_successor"
        );
        assert_eq!(
            retain_reference(
                &mut refs,
                Uuid::from_u128(MAX_VALIDATION_HISTORY_RECEIPTS as u128 + 1),
                digest,
            )
            .unwrap_err()
            .code,
            "bounded_input"
        );
    }
}
