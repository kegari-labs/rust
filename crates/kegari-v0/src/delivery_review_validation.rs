//! Paired validation result lineage. Direct source advancement and the
//! original source of a granted retry are distinct opaque paths. Neither a
//! serialized retry reference nor a claim can construct this enum.
use super::*;
use crate::control::{RepairValidationLineage, SourceAdvancedReference};
use kegari_types::run::RunReference;

#[derive(Clone)]
pub(super) enum ValidationEvidence {
    Direct(Arc<VerifiedSourceAdvanced>),
    Retried(Arc<CompletedGrantedValidation>),
}
impl ValidationEvidence {
    pub(super) fn verify_current(
        &self,
        receipts: &[Receipt],
        grants: &[VerifiedRetryGrant],
        completion: &Arc<RunCompletion>,
    ) -> Result<()> {
        match self {
            Self::Direct(source) => {
                source.verify_completed_inventory(receipts, grants, completion.operation_id)
            }
            Self::Retried(proof) => {
                if !Arc::ptr_eq(proof.completion(), completion) {
                    return Err(blocked("retried validation lost actual completed run"));
                }
                proof.verify_current(receipts, grants)
            }
        }
    }
    pub(super) fn source_receipts(&self) -> Result<BTreeMap<Uuid, String>> {
        match self {
            Self::Direct(source) => source.retained_receipt_references(),
            Self::Retried(completed) => completed
                .source_advanced()
                .map(|source| source.retained_receipt_references())
                .transpose()
                .map(|value| value.unwrap_or_default()),
        }
    }
    pub(super) fn reference_lineage(
        &self,
        receipts: &[Receipt],
        grants: &[VerifiedRetryGrant],
        completion: &Arc<RunCompletion>,
    ) -> Result<RepairValidationLineage> {
        self.verify_current(receipts, grants, completion)?;
        let latest = actual_receipt(receipts, completion.operation_id)?;
        let latest_run = closed_run(latest, "completed")?;
        require_recorded_session(latest, completion)?;
        match self {
            Self::Direct(source) => {
                if !source.has_validation_budget() {
                    return Ok(RepairValidationLineage::Direct {});
                }
                let actual = SourceAdvancedReference::from_receipt(latest)?.ok_or_else(|| {
                    blocked("advanced validator lost its actual source admission")
                })?;
                if actual != source_reference(source) || latest_run.phase != *source.phase() {
                    return Err(blocked(
                        "advanced validator changed its exact source budget",
                    ));
                }
                let inventory = validation_inventory(receipts, latest.runtime_id, &latest_run)?;
                let (native_target, publications) =
                    validation_publications(receipts, grants, &inventory)?;
                Ok(RepairValidationLineage::SourceAdvanced {
                    source_budget_digest: source.digest().into(),
                    native_target,
                    inventory,
                    grants: publications,
                })
            }
            Self::Retried(proof) => {
                let origin = proof.original_source_admission().ok_or_else(|| {
                    blocked("retried paired validation has no original source admission")
                })?;
                let actual = actual_receipt(receipts, origin.operation_id)?;
                if canonical_digest(1, actual)? != canonical_digest(1, origin)? {
                    return Err(blocked("original validation source receipt changed"));
                }
                let origin_run = closed_run(actual, "failed")?;
                if origin_run.phase.phase != "child_local_validation"
                    || origin_run.phase.work != latest_run.phase.work
                    || origin_run.phase.generation != latest_run.phase.generation
                    || origin_run.workspace != latest_run.workspace
                    || origin_run.account != latest_run.account
                {
                    return Err(blocked(
                        "retried validator changes original source workspace or phase",
                    ));
                }
                let inventory = validation_inventory(receipts, latest.runtime_id, &latest_run)?;
                let (native_target, publications) =
                    validation_publications(receipts, grants, &inventory)?;
                Ok(RepairValidationLineage::Retried {
                    origin: Box::new(receipt_reference(actual)?),
                    origin_phase: origin_run.phase,
                    completed_retry_digest: proof.digest().into(),
                    native_target,
                    inventory,
                    grants: publications,
                })
            }
        }
    }
    pub(super) fn retained_grants(
        &self,
        receipts: &[Receipt],
        grants: &[VerifiedRetryGrant],
        completion: &Arc<RunCompletion>,
    ) -> Result<Vec<VerifiedRetryGrant>> {
        if let Self::Direct(source) = self {
            source.verify_retained_validation_budget(receipts, grants)?;
            return Ok(source.retained_validation_grants());
        }
        let RepairValidationLineage::Retried {
            grants: references, ..
        } = self.reference_lineage(receipts, grants, completion)?
        else {
            return Err(blocked("retried validation lost its typed lineage"));
        };
        references
            .keys()
            .map(|id| {
                grants
                    .iter()
                    .find(|g| g.record().operation_id == *id)
                    .cloned()
                    .ok_or_else(|| blocked("validation grant disappeared during plan capture"))
            })
            .collect()
    }
}
fn validation_publications(
    receipts: &[Receipt],
    grants: &[VerifiedRetryGrant],
    inventory: &BTreeMap<Uuid, String>,
) -> Result<(String, BTreeMap<Uuid, String>)> {
    let mut publications = BTreeMap::new();
    let mut native_target: Option<String> = None;
    for receipt in receipts {
        if !inventory.contains_key(&receipt.operation_id) {
            continue;
        }
        let run = crate::control::run_reference_from_receipt(receipt)?
            .ok_or_else(|| blocked("validation inventory contains non-model record"))?;
        if let Some(id) = run.phase.retry_grant {
            let grant = grants
                .iter()
                .find(|g| g.record().operation_id == id)
                .ok_or_else(|| blocked("validation prefix lost current native grant"))?;
            crate::runtime_workflow::require_current_grant(grant, grants)?;
            if grant.phase_key() != run.phase
                || grant.record().runtime_id != receipt.runtime_id
                || grant.record().workspace_id != run.workspace
                || grants
                    .iter()
                    .filter(|current| current.phase() == grant.phase())
                    .count()
                    != 1
            {
                return Err(blocked(
                    "validation grant no longer uniquely names its actual budget",
                ));
            }
            let work = grant.work();
            let target = format!(
                "linear:{}:{}:{}",
                work.linear_endpoint,
                work.linear_workspace,
                work.child.unwrap_or(work.parent)
            );
            if native_target
                .as_ref()
                .is_some_and(|expected| expected != &target)
            {
                return Err(blocked(
                    "validation grants name different actual provider targets",
                ));
            }
            native_target = Some(target);
            let actual = actual_receipt(receipts, id)?;
            publications.insert(id, canonical_digest(1, actual)?);
        }
    }
    Ok((
        native_target.ok_or_else(|| blocked("granted validation lost actual native target"))?,
        publications,
    ))
}
fn validation_inventory(
    receipts: &[Receipt],
    runtime: Uuid,
    expected: &RunReference,
) -> Result<BTreeMap<Uuid, String>> {
    if receipts.len() > MAX_RECEIPTS {
        return Err(history_limit());
    }
    let mut result = BTreeMap::new();
    for receipt in receipts {
        if let Some(run) = crate::control::run_reference_from_receipt(receipt)?
            && receipt.runtime_id == runtime
            && run.workspace == expected.workspace
            && run.phase.work == expected.phase.work
            && run.phase.generation == expected.phase.generation
            && run.phase.phase == expected.phase.phase
            && result
                .insert(receipt.operation_id, canonical_digest(1, receipt)?)
                .is_some()
        {
            return Err(blocked("duplicate retried validation prefix"));
        }
    }
    Ok(result)
}
impl DeliveryCoordinator {
    /// Actual completed granted validation may retain failed required checks.
    /// This reuses the original Findings round while retaining its true source
    /// admission separately from the completed retry. No retry is relabelled as
    /// SourceAdvanced; actual qualified failed-check production remains required.
    pub(crate) async fn continue_paired_repair_after_granted_validation(
        &self,
        request: ValidationRepairRequest<'_>,
        previous: Arc<VerifiedReviewRoundPlan>,
        validation: Arc<CompletedGrantedValidation>,
        config: &Config,
        grants: &[VerifiedRetryGrant],
    ) -> Result<(Arc<UnfinishedChildWork>, VerifiedReviewRoundPlan)> {
        validation.verify_current(&self.host.operations()?, grants)?;
        if validation.outcome().digest() != request.outcome.digest()
            || !Arc::ptr_eq(validation.completion(), &request.completion)
        {
            return Err(blocked(
                "repair request differs from actual completed granted validator",
            ));
        }
        let source = self.repair_round_source(&request.source).await?;
        // The async actual source read cannot extend the lifetime of the grant,
        // current admission or latest validation inventory.
        self.current(request.source.admission, &[DeliveryPhase::LocalValidation])?;
        let receipts = self.host.operations()?;
        validation.verify_current(&receipts, grants)?;
        let actual_repair = actual_receipt(&receipts, source.reference.implementation.operation)?;
        source
            .paired_completion
            .verify(&previous, &receipts, grants, actual_repair)?;
        let advanced = validation.source_advanced().ok_or_else(|| {
            blocked("paired validation retry omitted original SourceAdvanced capability")
        })?;
        if advanced.source_operation() != source.reference.implementation.operation
            || advanced.source_receipt_digest() != source.reference.implementation.receipt_digest
            || advanced.source().digest()? != source.reference.source_digest
        {
            return Err(blocked("retried validation checks another repaired source"));
        }
        let origin = validation
            .original_source_admission()
            .ok_or_else(|| blocked("paired validation retry omitted original admission"))?;
        let source_advanced = SourceAdvancedReference::from_receipt(origin)?
            .ok_or_else(|| blocked("original validation lacks SourceAdvanced marker"))?;
        if source_advanced != source_reference(advanced) {
            return Err(blocked(
                "retried validation source marker differs from retained capability",
            ));
        }
        let validation_outcome = Box::new(request.outcome.clone());
        let validation_completion = request.completion.clone();
        let actual = actual_receipt(&receipts, validation_completion.operation_id)?;
        let run = closed_run(actual, "completed")?;
        require_recorded_session(actual, &validation_completion)?;
        let evidence = ValidationEvidence::Retried(validation.clone());
        let lineage = evidence.reference_lineage(&receipts, grants, &validation_completion)?;
        let unfinished = Arc::new(
            self.unfinished_validation_after_granted_round(
                request,
                previous.clone(),
                validation,
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
                validation: receipt_reference(actual)?,
                phase: run.phase,
                source_advanced,
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
}
fn source_reference(source: &VerifiedSourceAdvanced) -> SourceAdvancedReference {
    SourceAdvancedReference {
        predecessor: source.predecessor(),
        predecessor_receipt_digest: source.predecessor_receipt_digest().into(),
        source_operation: source.source_operation(),
        source_receipt_digest: source.source_receipt_digest().into(),
        trigger_digest: source.digest().into(),
        progress_digest: source.progress_digest().into(),
        review_rounds_consumed: source.review_rounds_consumed(),
    }
}

impl crate::delivery::lineage::Node for ValidationEvidence {
    fn expand<'a>(
        &'a self,
        children: &mut Vec<&'a dyn crate::delivery::lineage::Node>,
        _leaves: &mut crate::delivery::lineage::Inventory,
    ) -> Result<()> {
        match self {
            Self::Direct(p) => children.push(p.as_ref()),
            Self::Retried(p) => children.push(p.as_ref()),
        }
        Ok(())
    }
}

impl crate::delivery::lineage_drop::Release for ValidationEvidence {
    fn release(self, pending: &mut crate::delivery::lineage_drop::Pending) {
        match self {
            Self::Direct(p) => pending.push(p),
            Self::Retried(p) => pending.push(p),
        }
    }
}
