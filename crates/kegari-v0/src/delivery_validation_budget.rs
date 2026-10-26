//! Opaque changed-source continuation of an already genuine validation budget.
//! References remain data; actual joined completions are captured from the same
//! runtime owner after the passed/failed-check/paired capabilities validate.
use super::*;
use crate::{decisions::VerifiedRetryGrant, runtime::RuntimeCoordinator};
const MAX_VALIDATION_HISTORY: usize = 65_536;

#[derive(Clone)]
pub(crate) enum ValidationAdvanceBasis {
    Passed(Arc<VerifiedPassedValidation>),
    FailedChecks(Arc<CompletedGrantedValidation>),
    Advanced {
        previous: Arc<VerifiedSourceAdvanced>,
        outcome: Box<VerifiedAuthorizedOutcome>,
        completion: Arc<RunCompletion>,
    },
}
#[derive(Clone)]
pub(super) struct ValidationBudgetLineage {
    runtime: Uuid,
    evidence_digest: String,
    passed_checks: BTreeMap<Uuid, Vec<String>>,
    phase: PhaseKey,
    reference: RunReference,
    retained: BTreeMap<Uuid, String>,
    joined: BTreeMap<Uuid, Arc<RunCompletion>>,
    bridges: BTreeMap<Uuid, Arc<crate::worker_run::AdmittedSessionResume>>,
    sources: Vec<DeliverySource>,
    grants: Vec<VerifiedRetryGrant>,
    implementation_inventory: String,
    validation_inventory: String,
    digest: String,
}
impl ValidationBudgetLineage {
    pub(super) fn digest(&self) -> &str {
        &self.digest
    }
    pub(super) fn retained(&self) -> &BTreeMap<Uuid, String> {
        &self.retained
    }
    pub(super) fn grants(&self) -> &[VerifiedRetryGrant] {
        &self.grants
    }
    pub(super) fn verify_request(
        &self,
        receipts: &[Receipt],
        predecessor: &Receipt,
        reference: &RunReference,
        source: &Receipt,
    ) -> Result<()> {
        if predecessor.runtime_id != self.runtime
            || canonical_digest(1, reference)? != canonical_digest(1, &self.reference)?
            || self.retained.get(&predecessor.operation_id)
                != Some(&canonical_digest(1, predecessor)?)
            || self.retained.get(&source.operation_id) != Some(&canonical_digest(1, source)?)
            || phase_inventory(receipts, self.runtime, reference)? != self.validation_inventory
            || implementation_inventory(receipts, self.runtime, reference)?
                != self.implementation_inventory
        {
            return Err(invalid(
                "validation advancement changed its actual predecessor or paired source",
            ));
        }
        Ok(())
    }
    pub(super) fn verify_retained(
        &self,
        receipts: &[Receipt],
        grants: &[VerifiedRetryGrant],
    ) -> Result<()> {
        if receipts.len() > MAX_VALIDATION_HISTORY {
            return Err(limit());
        }
        for (id, expected) in &self.retained {
            if canonical_digest(1, actual(receipts, *id)?)? != *expected {
                return Err(invalid("validation budget ancestry changed or disappeared"));
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
                return Err(invalid(
                    "validation budget native grant namespace is ambiguous",
                ));
            }
        }
        for (operation, expected) in &self.passed_checks {
            let completion = self
                .joined
                .get(operation)
                .ok_or_else(|| invalid("passed ancestor lost its actual completion"))?;
            let actual = completion
                .checks
                .iter()
                .map(|check| {
                    check.production_evidence()?;
                    check.digest()
                })
                .collect::<Result<Vec<_>>>()?;
            if &actual != expected {
                return Err(invalid("passed ancestor check handles changed"));
            }
        }
        for source in &self.sources {
            source.revalidate_filesystem()?;
        }
        // Leaf execution/contract capabilities retain every ancestral owned
        // checkout without retaining any prior round/VSA/retry proof graph.
        for completion in self.joined.values() {
            if let Some(execution) = &completion.execution {
                let contract = completion
                    .contract
                    .as_ref()
                    .ok_or_else(|| invalid("owned ancestor lost actual contract"))?;
                execution
                    .ownership
                    .verify(&execution.binding, contract.isolation())?;
            }
        }
        Ok(())
    }
    pub(super) fn verify_current(
        &self,
        receipts: &[Receipt],
        grants: &[VerifiedRetryGrant],
        consumed_origin: bool,
    ) -> Result<()> {
        self.verify_retained(receipts, grants)?;
        if phase_inventory(receipts, self.runtime, &self.reference)? != self.validation_inventory
            || implementation_inventory(receipts, self.runtime, &self.reference)?
                != self.implementation_inventory
        {
            return Err(invalid(
                "a later implementation superseded validation source",
            ));
        }
        if !consumed_origin && grants.iter().any(|grant| grant.phase() == &self.phase) {
            return Err(invalid("validation source budget has been superseded"));
        }
        Ok(())
    }
    pub(super) fn verify_joined(
        &self,
        retained: &impl Fn(Uuid) -> Result<Option<Arc<RunCompletion>>>,
    ) -> Result<()> {
        for (operation, expected) in &self.joined {
            let actual = retained(*operation)?
                .ok_or_else(|| invalid("validation source lost actual joined ancestor"))?;
            if !Arc::ptr_eq(&actual, expected) || !actual.local_exit_confirmed {
                return Err(invalid("validation source joined ancestor was substituted"));
            }
        }
        Ok(())
    }
}

impl DeliveryCoordinator {
    pub(crate) async fn source_advanced_validated_repair(
        &self,
        request: SourceAdvancedRequest<'_>,
        paired: Arc<VerifiedReviewRoundPlan>,
        basis: ValidationAdvanceBasis,
        grants: &[VerifiedRetryGrant],
        runtime: &RuntimeCoordinator,
    ) -> Result<VerifiedSourceAdvanced> {
        if !Arc::ptr_eq(&self.host, &runtime.control)
            || request.assignment.phase() != "child_local_validation"
        {
            return Err(invalid(
                "validation advancement requires the actual local runtime owner",
            ));
        }
        self.current_child_claim(request.source.admission, &request.source.original)?;
        let receipts = self.host.operations()?;
        let findings = paired.completed_repair_for_validation(
            &receipts,
            grants,
            &request.source.completion,
            request.source.paired_completion.as_ref(),
        )?;
        paired.verify_joined(&|id| runtime.retained_completion(id))?;
        require_joined(&request.source.completion, runtime)?;
        child::completed_run(request.predecessor, &request.completion)?;
        require_joined(&request.completion, runtime)?;
        let predecessor = actual(&receipts, request.completion.operation_id)?;
        let reference = RunReference::from_receipt(predecessor)?
            .ok_or_else(|| invalid("validation predecessor is not an actual run"))?;
        if reference.phase.phase != "child_local_validation"
            || reference.phase.retry_grant.is_none()
        {
            return Err(invalid(
                "changed-source validation has no existing granted budget",
            ));
        }
        let mut bridges = paired.resumed_bridge_inventory()?;
        if let Some(proof) = &request.source.paired_completion {
            review_round::merge_resumed_bridges(&mut bridges, proof.resumed_bridge_inventory()?)?;
        }
        let prior_bridges = match &basis {
            ValidationAdvanceBasis::Passed(p) => p.resumed_bridge_inventory()?,
            ValidationAdvanceBasis::FailedChecks(p) => p.resumed_bridge_inventory()?,
            ValidationAdvanceBasis::Advanced { previous, .. } => {
                previous.resumed_bridge_inventory()?
            }
        };
        review_round::merge_resumed_bridges(&mut bridges, prior_bridges)?;
        review_round::verify_resumed_inventory(bridges.clone(), &|id| {
            runtime.retained_session_resume(id)
        })?;
        let mut native = paired.retained_native_grants()?;
        if let Some(proof) = &request.source.paired_completion {
            review_round::merge_native_grants(
                &mut native,
                proof.retained_native_grants()?.into_values(),
            )?;
        }
        let prior_native = match &basis {
            ValidationAdvanceBasis::Passed(p) => p.retained_native_grants()?,
            ValidationAdvanceBasis::FailedChecks(p) => p.retained_native_grants()?,
            ValidationAdvanceBasis::Advanced { previous, .. } => {
                previous.retained_native_grants()?
            }
        };
        review_round::merge_native_grants(&mut native, prior_native.into_values())?;
        let (evidence_digest, passed_checks, mut retained, mut sources) = match basis {
            ValidationAdvanceBasis::Passed(passed) => {
                if paired.reference().passed_validation.as_ref() != Some(passed.reference())
                    || passed.validation_outcome().digest() != request.predecessor.digest()
                    || !Arc::ptr_eq(passed.validation_completion(), &request.completion)
                    || paired.reference().dispatch_source().source_digest
                        != passed.source().digest()?
                {
                    return Err(invalid(
                        "paired repair does not consume this exact passed validation",
                    ));
                }
                passed.verify_retained(&receipts, grants)?;
                passed.verify_validation_inventory(&receipts)?;
                passed.verify_joined(&|id| runtime.retained_completion(id))?;
                let checks = passed
                    .validation_completion()
                    .checks
                    .iter()
                    .map(|check| {
                        check.production_evidence()?;
                        check.digest()
                    })
                    .collect::<Result<Vec<_>>>()?;
                let previous = passed
                    .validation_source()
                    .and_then(|source| source.validation_budget.as_ref());
                let mut passed_checks = previous
                    .map(|budget| budget.passed_checks.clone())
                    .unwrap_or_default();
                passed_checks.insert(passed.validation_completion().operation_id, checks);
                let mut sources = previous
                    .map(|budget| budget.sources.clone())
                    .unwrap_or_default();
                sources.push(passed.source().clone());
                (
                    passed.digest().into(),
                    passed_checks,
                    passed.reference().retained_receipts.clone(),
                    sources,
                )
            }
            ValidationAdvanceBasis::FailedChecks(completed) => {
                let crate::control::ReviewRoundCause::RepairContinuation { validation, .. } =
                    &paired.reference().cause
                else {
                    return Err(invalid(
                        "failed checks did not produce this exact continued pair",
                    ));
                };
                let crate::control::RepairValidationLineage::Retried {
                    completed_retry_digest,
                    ..
                } = &validation.lineage
                else {
                    return Err(invalid("continued pair lost its actual retried validation"));
                };
                if completed_retry_digest != completed.digest()
                    || validation.validation.operation != completed.completion().operation_id
                    || completed.outcome().digest() != request.predecessor.digest()
                    || !Arc::ptr_eq(completed.completion(), &request.completion)
                {
                    return Err(invalid(
                        "failed-check successor changes its actual completion",
                    ));
                }
                // The exact paired implementation already consumed this check.
                // Its typed plan verified the pre-implementation current prefix.
                completed.verify_validation_inventory(&receipts)?;
                completed.verify_joined(&|id| runtime.retained_completion(id))?;
                let retained = completed.retained_receipt_references(&receipts, grants)?;
                let previous = completed
                    .source_advanced()
                    .and_then(|source| source.validation_budget.as_ref());
                let passed_checks = previous
                    .map(|budget| budget.passed_checks.clone())
                    .unwrap_or_default();
                let mut sources = previous
                    .map(|budget| budget.sources.clone())
                    .unwrap_or_default();
                sources.push(completed.source().clone());
                (completed.digest().into(), passed_checks, retained, sources)
            }
            ValidationAdvanceBasis::Advanced {
                previous,
                outcome,
                completion,
            } => {
                let prior = previous
                    .validation_budget
                    .as_ref()
                    .ok_or_else(|| invalid("validation source omitted its granted ancestry"))?;
                if outcome.digest() != request.predecessor.digest()
                    || !Arc::ptr_eq(&completion, &request.completion)
                    || paired.reference().dispatch_source().source_digest
                        != previous.source().digest()?
                {
                    return Err(invalid(
                        "validation successor does not follow the exact previous source",
                    ));
                }
                previous.verify_completed_validation_history(
                    &receipts,
                    grants,
                    &outcome,
                    &completion,
                )?;
                previous.verify_joined(&|id| runtime.retained_completion(id))?;
                let mut retained = previous.retained_receipt_references()?;
                retain(&mut retained, predecessor)?;
                retain_preparation(&mut retained, &receipts, predecessor.operation_id)?;
                let mut sources = prior.sources.clone();
                sources.push(previous.source().clone());
                (
                    prior.evidence_digest.clone(),
                    prior.passed_checks.clone(),
                    retained,
                    sources,
                )
            }
        };
        // The latest completed validation namespace is not inferred from labels.
        let current_grant = grants
            .iter()
            .find(|grant| grant.phase_key() == reference.phase)
            .ok_or_else(|| invalid("validation source lost its current native grant"))?;
        crate::runtime_workflow::require_current_grant(current_grant, grants)?;
        if current_grant.work() != request.source.admission.fresh_gate().work()
            || current_grant.record().runtime_id != predecessor.runtime_id
            || current_grant.record().workspace_id != reference.workspace
            || grants
                .iter()
                .filter(|grant| grant.phase() == current_grant.phase())
                .count()
                != 1
            || grants.iter().any(|grant| grant.phase() == &reference.phase)
        {
            return Err(invalid("validation source native budget changed or forked"));
        }
        retain(
            &mut retained,
            actual(&receipts, current_grant.record().operation_id)?,
        )?;
        for (id, digest) in paired.retained_repair_receipts(
            &receipts,
            request.source.completion.operation_id,
            request.source.paired_completion.as_ref(),
        )? {
            insert(&mut retained, id, digest)?;
        }
        let seal = actual(&receipts, request.source.source.receipt().operation_id)?;
        if canonical_digest(1, seal)? != canonical_digest(1, request.source.source.receipt())? {
            return Err(invalid(
                "new source seal differs from actual protected receipt",
            ));
        }
        retain(&mut retained, seal)?;
        retain(&mut retained, predecessor)?;
        retain_preparation(&mut retained, &receipts, predecessor.operation_id)?;
        verify_accounted_validation_history(
            &receipts,
            predecessor.runtime_id,
            &reference,
            &retained,
        )?;
        review_round::merge_native_grants(&mut native, [current_grant.clone()])?;
        let native = review_round::close_native_grants(native, grants)?;
        for id in native.keys() {
            retain(&mut retained, actual(&receipts, *id)?)?;
        }
        let native = native.into_values().collect::<Vec<_>>();
        let mut joined = BTreeMap::new();
        for id in retained.keys() {
            let receipt = actual(&receipts, *id)?;
            if RunReference::from_receipt(receipt)?.is_some() {
                let completion = runtime
                    .retained_completion(*id)?
                    .ok_or_else(|| invalid("validation ancestry has no genuine joined run"))?;
                if !completion.local_exit_confirmed
                    || receipt
                        .observed_result
                        .as_ref()
                        .and_then(|r| r.get("observation"))
                        != Some(&completion.observation())
                {
                    return Err(invalid(
                        "joined ancestor differs from its exact protected observation",
                    ));
                }
                joined.insert(*id, completion);
            }
        }
        let mut source_keys = BTreeSet::new();
        let mut unique_sources = Vec::new();
        for source in sources.drain(..) {
            if source_keys.insert(source.digest()?) {
                unique_sources.push(source);
            }
        }
        let sources = unique_sources;
        let implementation_inventory =
            implementation_inventory(&receipts, predecessor.runtime_id, &reference)?;
        let validation_inventory = phase_inventory(&receipts, predecessor.runtime_id, &reference)?;
        let digest = canonical_digest(
            1,
            &json!({"purpose":"validation_budget_source_successor","phase":reference.phase,"evidence":evidence_digest,"passed_checks":passed_checks,"latest":canonical_digest(1, predecessor)?,"retained":retained,"source_inventory":implementation_inventory,"validation_inventory":validation_inventory,"grants":native.iter().map(VerifiedRetryGrant::digest).collect::<Vec<_>>()}),
        )?;
        let lineage = Arc::new(ValidationBudgetLineage {
            runtime: predecessor.runtime_id,
            evidence_digest,
            passed_checks,
            phase: reference.phase.clone(),
            reference,
            retained,
            joined,
            bridges,
            sources,
            grants: native,
            implementation_inventory,
            validation_inventory,
            digest,
        });
        lineage.verify_current(&receipts, grants, false)?;
        lineage.verify_resumed_bridges(&|id| runtime.retained_session_resume(id))?;
        let proof = self
            .source_advanced_inner(
                request,
                Some(CompletedReviewLineage::Round(findings)),
                Some(lineage),
            )
            .await?;
        let fresh_receipts = self.host.operations()?;
        let fresh_grants = crate::runtime_workflow::current_retry_grants(
            &runtime.configuration.snapshot()?,
            &runtime.observations.admission_snapshot()?,
            proof.authority().work(),
            &fresh_receipts,
            &runtime.services.key,
        )?;
        (self.current_gate)(proof.authority())?;
        proof.verify_current(&fresh_receipts, &fresh_grants)?;
        proof.verify_joined(&|id| runtime.retained_completion(id))?;
        proof.verify_resumed_bridges(&|id| runtime.retained_session_resume(id))?;
        Ok(proof)
    }
}
fn invalid(message: &str) -> Error {
    Error::new("validation_source_lineage", message)
}
fn limit() -> Error {
    Error::new(
        "history_limit",
        "validation source ancestry exceeds protected receipt history bound",
    )
}
fn actual(receipts: &[Receipt], operation: Uuid) -> Result<&Receipt> {
    let mut found = receipts
        .iter()
        .filter(|receipt| receipt.operation_id == operation);
    let first = found
        .next()
        .ok_or_else(|| invalid("validation ancestry receipt is missing"))?;
    if found.next().is_some() {
        return Err(invalid("duplicate validation ancestry receipt"));
    }
    Ok(first)
}
fn insert(map: &mut BTreeMap<Uuid, String>, id: Uuid, digest: String) -> Result<()> {
    if let Some(old) = map.insert(id, digest.clone())
        && old != digest
    {
        return Err(invalid("validation ancestry aliases different receipts"));
    }
    if map.len() > MAX_VALIDATION_HISTORY {
        return Err(limit());
    }
    Ok(())
}
fn retain(map: &mut BTreeMap<Uuid, String>, receipt: &Receipt) -> Result<()> {
    insert(map, receipt.operation_id, canonical_digest(1, receipt)?)
}
fn retain_preparation(
    map: &mut BTreeMap<Uuid, String>,
    receipts: &[Receipt],
    operation: Uuid,
) -> Result<()> {
    let model = actual(receipts, operation)?;
    let run = RunReference::from_receipt(model)?
        .ok_or_else(|| invalid("validation ancestor is not a model run"))?;
    let prep = actual(receipts, crate::control::preparation_operation(operation)?)?;
    let reference = crate::control::PreparationReference::from_receipt(prep)?
        .ok_or_else(|| invalid("validation ancestor preparation is absent"))?;
    let result = prep
        .observed_result
        .as_ref()
        .ok_or_else(|| invalid("validation ancestor preparation has no exit"))?;
    if reference.worker_operation != operation
        || reference.phase != run.phase
        || reference.workspace != run.workspace
        || reference.account != run.account
        || prep.phase != ReceiptPhase::Reconciled
        || prep.runtime_id != model.runtime_id
        || prep.actor != model.actor
        || prep.owner_generation != model.owner_generation
        || result["local_execution"] != "exited"
        || result["unresolved_effects"]
            .as_array()
            .is_none_or(|v| !v.is_empty())
        || result["observation"]["phase"] != "worker_preparation"
        || result["observation"]["result"] != "prepared"
    {
        return Err(invalid("validation ancestor preparation is not closed"));
    }
    retain(map, prep)
}
fn implementation_inventory(
    receipts: &[Receipt],
    runtime: Uuid,
    reference: &RunReference,
) -> Result<String> {
    let mut selected = BTreeMap::new();
    for receipt in receipts {
        if let Some(run) = RunReference::from_receipt(receipt)?
            && receipt.runtime_id == runtime
            && run.workspace == reference.workspace
            && run.phase.work == reference.phase.work
            && run.phase.generation == reference.phase.generation
            && run.phase.phase == "implementation"
        {
            insert(
                &mut selected,
                receipt.operation_id,
                canonical_digest(1, receipt)?,
            )?;
        }
    }
    canonical_digest(1, &selected)
}
fn require_joined(expected: &Arc<RunCompletion>, runtime: &RuntimeCoordinator) -> Result<()> {
    let actual = runtime
        .retained_completion(expected.operation_id)?
        .ok_or_else(|| invalid("actual joined validation ancestor is unavailable"))?;
    if !Arc::ptr_eq(&actual, expected) || !actual.local_exit_confirmed {
        return Err(invalid("validation ancestor was not genuinely joined"));
    }
    Ok(())
}

/// Data-only cross-namespace closure check shared with independent regressions.
/// The private producer has already authenticated each retained capability.
fn verify_accounted_validation_history(
    receipts: &[Receipt],
    runtime: Uuid,
    expected: &RunReference,
    retained: &BTreeMap<Uuid, String>,
) -> Result<()> {
    if receipts.len() > MAX_VALIDATION_HISTORY {
        return Err(limit());
    }
    let mut seen = BTreeSet::new();
    for receipt in receipts {
        if let Some(run) = RunReference::from_receipt(receipt)?
            && same_phase_group(runtime, expected, receipt, &run)
            && (!seen.insert(receipt.operation_id)
                || retained.get(&receipt.operation_id) != Some(&canonical_digest(1, receipt)?))
        {
            return Err(invalid(
                "unaccounted validation history prevents source advancement",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "delivery_validation_budget_tests.rs"]
mod tests;

impl crate::delivery::lineage::Node for ValidationBudgetLineage {
    fn expand<'a>(
        &'a self,
        _children: &mut Vec<&'a dyn crate::delivery::lineage::Node>,
        leaves: &mut crate::delivery::lineage::Inventory,
    ) -> Result<()> {
        leaves.sources(self.sources.iter().cloned())?;
        leaves.grants(self.grants.iter().cloned())?;
        leaves.bridges(&self.bridges)?;
        leaves.joined(self.joined.len())
    }
    fn verify_joined_leaf(
        &self,
        reader: &dyn Fn(Uuid) -> Result<Option<Arc<RunCompletion>>>,
    ) -> Result<()> {
        // This capability is deliberately flat: this method reads no other node.
        self.verify_joined(&|id| reader(id))
    }
}

impl ValidationBudgetLineage {
    pub(super) fn verify_resumed_bridges(
        &self,
        reader: &dyn Fn(Uuid) -> Result<Option<Arc<crate::worker_run::AdmittedSessionResume>>>,
    ) -> Result<()> {
        review_round::verify_resumed_inventory(self.bridges.clone(), reader)
    }
}
