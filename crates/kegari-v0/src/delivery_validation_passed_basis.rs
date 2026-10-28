//! Private authentic completed-validation bases for the existing passed marker.
//! A SourceAdvanced run is never relabelled as a ValidationRetryWork.
use super::*;
use kegari_types::run::RunReference;

#[derive(Clone)]
pub(super) enum PassedValidationBasis {
    Retried(Arc<CompletedGrantedValidation>),
    SourceAdvanced(Arc<CompletedSourceValidation>),
}
#[derive(Clone)]
pub(super) struct CompletedSourceValidation {
    advanced: Arc<VerifiedSourceAdvanced>,
    outcome: crate::runtime::VerifiedAuthorizedOutcome,
    completion: Arc<RunCompletion>,
    reference: RunReference,
    receipt_digest: String,
    inventory_digest: String,
    digest: String,
}
impl PassedValidationBasis {
    pub(super) fn resumed_bridge_inventory(
        &self,
    ) -> Result<BTreeMap<Uuid, Arc<crate::worker_run::AdmittedSessionResume>>> {
        Ok(crate::delivery::lineage::collect(self)?.bridges)
    }
    pub(super) fn verify_resumed_bridges(
        &self,
        reader: &dyn Fn(Uuid) -> Result<Option<Arc<crate::worker_run::AdmittedSessionResume>>>,
    ) -> Result<()> {
        verify_resumed_bridge_inventory(self.resumed_bridge_inventory()?, reader)
    }
    pub(super) fn digest(&self) -> &str {
        match self {
            Self::Retried(p) => p.digest(),
            Self::SourceAdvanced(p) => &p.digest,
        }
    }
    pub(super) fn authority(&self) -> &FreshGate {
        self.assignment().authority()
    }
    pub(super) fn assignment(&self) -> &DeliveryWorkerAssignment {
        match self {
            Self::Retried(p) => p.basis.assignment(),
            Self::SourceAdvanced(p) => p.advanced.assignment(),
        }
    }
    pub(super) fn source(&self) -> &DeliverySource {
        self.assignment()
            .source()
            .expect("genuine completed validation source")
    }
    pub(super) fn outcome(&self) -> &crate::runtime::VerifiedAuthorizedOutcome {
        match self {
            Self::Retried(p) => p.outcome(),
            Self::SourceAdvanced(p) => &p.outcome,
        }
    }
    pub(super) fn completion(&self) -> &Arc<RunCompletion> {
        match self {
            Self::Retried(p) => p.completion(),
            Self::SourceAdvanced(p) => &p.completion,
        }
    }
    pub(super) fn reference(&self) -> &RunReference {
        match self {
            Self::Retried(p) => &p.reference,
            Self::SourceAdvanced(p) => &p.reference,
        }
    }
    pub(super) fn receipt_digest(&self) -> &str {
        match self {
            Self::Retried(p) => &p.receipt_digest,
            Self::SourceAdvanced(p) => &p.receipt_digest,
        }
    }
    pub(super) fn source_advanced(&self) -> Option<&Arc<VerifiedSourceAdvanced>> {
        match self {
            Self::Retried(p) => p.source_advanced(),
            Self::SourceAdvanced(p) => Some(&p.advanced),
        }
    }
    pub(super) fn verify_identity(&self, receipts: &[Receipt]) -> Result<()> {
        let receipt = unique_receipt(receipts, self.completion().operation_id)?;
        if canonical_digest(1, receipt)? != self.receipt_digest() {
            return Err(passed_error("completed validation receipt changed"));
        }
        child::completed_run(self.outcome(), self.completion())?;
        bound_completion(
            self.assignment(),
            self.completion(),
            receipt,
            self.reference(),
        )
    }
    pub(super) fn verify_validation_inventory(&self, receipts: &[Receipt]) -> Result<()> {
        match self {
            Self::Retried(p) => p.verify_validation_inventory(receipts),
            Self::SourceAdvanced(p) => {
                self.verify_identity(receipts)?;
                let receipt = unique_receipt(receipts, p.completion.operation_id)?;
                if validation_inventory(receipts, receipt.runtime_id, &p.reference)?
                    != p.inventory_digest
                {
                    return Err(passed_error(
                        "completed source validation inventory changed",
                    ));
                }
                Ok(())
            }
        }
    }
    pub(super) fn verify_current(
        &self,
        receipts: &[Receipt],
        grants: &[VerifiedRetryGrant],
    ) -> Result<()> {
        match self {
            Self::Retried(p) => p.verify_current(receipts, grants),
            Self::SourceAdvanced(p) => {
                self.verify_validation_inventory(receipts)?;
                p.advanced
                    .verify_completed_inventory(receipts, grants, p.completion.operation_id)
            }
        }
    }
    pub(super) fn verify_joined(
        &self,
        retained: &impl Fn(Uuid) -> Result<Option<Arc<RunCompletion>>>,
    ) -> Result<()> {
        crate::delivery::lineage::verify_joined(self, retained)
    }
    pub(super) fn retained_receipts(
        &self,
        receipts: &[Receipt],
        grants: &[VerifiedRetryGrant],
    ) -> Result<(BTreeMap<Uuid, String>, Vec<VerifiedRetryGrant>)> {
        match self {
            Self::Retried(p) => passed_receipts(p, receipts, grants),
            Self::SourceAdvanced(p) => {
                self.verify_current(receipts, grants)?;
                let mut refs = p.advanced.retained_receipt_references()?;
                let model = unique_receipt(receipts, p.completion.operation_id)?;
                retain_reference(&mut refs, model.operation_id, canonical_digest(1, model)?)?;
                let prep = unique_receipt(
                    receipts,
                    crate::control::preparation_operation(model.operation_id)?,
                )?;
                let header = PreparationReference::from_receipt(prep)?
                    .ok_or_else(|| passed_error("source validation lacks exact preparation"))?;
                let result = prep
                    .observed_result
                    .as_ref()
                    .ok_or_else(|| passed_error("source validation preparation has no exit"))?;
                if header.worker_operation != model.operation_id
                    || header.phase != p.reference.phase
                    || header.workspace != p.reference.workspace
                    || header.account != p.reference.account
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
                    return Err(passed_error("source validation preparation changed"));
                }
                retain_reference(&mut refs, prep.operation_id, canonical_digest(1, prep)?)?;
                let native = p.advanced.retained_validation_grants();
                for grant in &native {
                    crate::runtime_workflow::require_current_grant(grant, grants)?;
                    let publication = unique_receipt(receipts, grant.record().operation_id)?;
                    retain_reference(
                        &mut refs,
                        publication.operation_id,
                        canonical_digest(1, publication)?,
                    )?;
                }
                Ok((refs, native))
            }
        }
    }
}
impl DeliveryCoordinator {
    pub(crate) async fn passed_source_validation(
        &self,
        source: RepairSource<'_>,
        workflow: &Workflow,
        advanced: Arc<VerifiedSourceAdvanced>,
        result: (
            &crate::runtime::VerifiedAuthorizedOutcome,
            Arc<RunCompletion>,
        ),
        local: &LocalPassed,
        runtime: &RuntimeCoordinator,
    ) -> Result<VerifiedPassedValidation> {
        if !Arc::ptr_eq(&self.host, &runtime.control) || !advanced.has_validation_budget() {
            return Err(passed_error(
                "completed source validation has no actual granted ancestry",
            ));
        }
        self.current(source.admission, &[DeliveryPhase::LocalValidation])?;
        let (outcome, completion) = result;
        child::completed_run(outcome, &completion)?;
        advanced.verify_contract(
            completion
                .contract
                .as_ref()
                .ok_or_else(|| passed_error("source validator lost actual contract"))?,
        )?;
        let (receipts, grants) = current_inputs(runtime, advanced.authority())?;
        advanced.verify_completed_inventory(&receipts, &grants, completion.operation_id)?;
        advanced.verify_joined(&|id| runtime.retained_completion(id))?;
        advanced.verify_resumed_bridges(&|id| runtime.retained_session_resume(id))?;
        source_lineage::require_joined(&completion, &|id| runtime.retained_completion(id))?;
        let receipt = unique_receipt(&receipts, completion.operation_id)?;
        let reference = crate::control::run_reference_from_receipt(receipt)?
            .ok_or_else(|| passed_error("completed source validator lacks actual run"))?;
        if receipt.expected != outcome.run_receipt().expected
            || receipt
                .observed_result
                .as_ref()
                .and_then(|value| value.get("observation"))
                != Some(&completion.observation())
        {
            return Err(passed_error("completed source validation outcome changed"));
        }
        let receipt_digest = canonical_digest(1, receipt)?;
        let inventory_digest = validation_inventory(&receipts, receipt.runtime_id, &reference)?;
        let digest = canonical_digest(
            1,
            &json!({"purpose":"completed_source_validation","source_advanced":advanced.digest(),"outcome":outcome.digest(),"receipt":receipt_digest,"inventory":inventory_digest}),
        )?;
        let basis = PassedValidationBasis::SourceAdvanced(Arc::new(CompletedSourceValidation {
            advanced,
            outcome: outcome.clone(),
            completion,
            reference,
            receipt_digest,
            inventory_digest,
            digest,
        }));
        self.passed_validation(source, workflow, basis, local, runtime)
            .await
    }
}

impl crate::delivery::lineage::Node for PassedValidationBasis {
    fn expand<'a>(
        &'a self,
        children: &mut Vec<&'a dyn crate::delivery::lineage::Node>,
        _leaves: &mut crate::delivery::lineage::Inventory,
    ) -> Result<()> {
        match self {
            Self::Retried(p) => children.push(p.as_ref()),
            Self::SourceAdvanced(p) => children.push(p.as_ref()),
        }
        Ok(())
    }
}
impl crate::delivery::lineage::Node for CompletedSourceValidation {
    fn expand<'a>(
        &'a self,
        children: &mut Vec<&'a dyn crate::delivery::lineage::Node>,
        leaves: &mut crate::delivery::lineage::Inventory,
    ) -> Result<()> {
        children.push(self.advanced.as_ref());
        leaves.joined(1)
    }
    fn verify_joined_leaf(
        &self,
        reader: &dyn Fn(Uuid) -> Result<Option<Arc<RunCompletion>>>,
    ) -> Result<()> {
        source_lineage::require_joined(&self.completion, &|id| reader(id))
    }
}

impl crate::delivery::lineage_drop::Release for PassedValidationBasis {
    fn release(self, pending: &mut crate::delivery::lineage_drop::Pending) {
        match self {
            Self::Retried(p) => pending.push(p),
            Self::SourceAdvanced(p) => pending.push(p),
        }
    }
}
impl crate::delivery::lineage_drop::Release for CompletedSourceValidation {
    fn release(self, pending: &mut crate::delivery::lineage_drop::Pending) {
        pending.push(self.advanced);
    }
}
