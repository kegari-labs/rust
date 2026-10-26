//! Flat actual resumed-repair result lineage. Neither serialized markers nor a
//! copied RunCompletion can construct this evidence. Native readers are required
//! at observation and every joined verification boundary.
use super::*;

#[derive(Clone)]
struct ResumeLink {
    receipt: Receipt,
    metadata: Receipt,
    completion: Arc<RunCompletion>,
    admitted: Arc<AdmittedSessionResume>,
    reference: ReviewRoundResumeReference,
}
#[derive(Clone)]
pub(super) struct PairedResumeChain {
    pub(super) origin: Receipt,
    pub(super) preparation: Receipt,
    original: Arc<RunCompletion>,
    links: BTreeMap<u32, ResumeLink>,
    implementation_grant: Option<VerifiedRetryGrant>,
    grant_receipt: Option<Receipt>,
}
impl PairedResumeChain {
    pub(super) fn is_resumed(&self) -> bool {
        !self.links.is_empty()
    }
    pub(super) fn observe(
        plan: &VerifiedReviewRoundPlan,
        receipts: &[Receipt],
        grants: &[VerifiedRetryGrant],
        terminal: &Receipt,
        outcome: &str,
        readers: &PairedRecoveryReaders<'_>,
    ) -> Result<Self> {
        if receipts.len() > MAX_RECEIPTS || !plan.reference().is_repair() {
            return Err(recovery_blocked(
                "paired resume inventory exceeds its protected bound",
            ));
        }
        let mut current = terminal;
        let mut links = BTreeMap::new();
        let mut seen = BTreeSet::new();
        while let Some(reference) = ReviewRoundResumeReference::from_model_receipt(current)? {
            if !seen.insert(current.operation_id) || seen.len() > MAX_RECEIPTS {
                return Err(recovery_blocked(
                    "paired resume ancestry has a cycle or exceeds its bound",
                ));
            }
            let run = closed_run(
                current,
                if current.operation_id == terminal.operation_id {
                    outcome
                } else {
                    "failed"
                },
            )?;
            let completion = (readers.joined)(current.operation_id)?
                .ok_or_else(|| recovery_blocked("paired resume lost an actual joined result"))?;
            let admitted = (readers.resumed)(current.operation_id)?
                .ok_or_else(|| recovery_blocked("paired resume lost its actual admitted bridge"))?;
            admitted.verify_completed_record(current, &completion)?;
            let metadata = actual_receipt(receipts, reference.metadata.operation)?;
            reference.metadata.matches_observation(metadata)?;
            verify_link_metadata(plan, current, metadata, &reference, &admitted)?;
            let previous = actual_receipt(receipts, reference.recovery.predecessor.operation)?;
            reference
                .recovery
                .predecessor
                .matches_observation(previous)?;
            let previous_run = closed_run(previous, "failed")?;
            let previous_completion =
                (readers.joined)(previous.operation_id)?.ok_or_else(|| {
                    recovery_blocked("paired resume lost its actual predecessor completion")
                })?;
            admitted.bridge().verify_completion(&previous_completion)?;
            let previous_link = ReviewRoundResumeReference::from_model_receipt(previous)?;
            if run.phase != previous_run.phase
                || run.workspace != previous_run.workspace
                || run.account != previous_run.account
                || run.profile_digest != previous_run.profile_digest
                || Some(run.attempt) != previous_run.attempt.checked_add(1)
                || run.max_attempts > previous_run.max_attempts
                || run.max_turns > previous_run.max_turns
                || run.max_review_rounds > previous_run.max_review_rounds
                || current.actor != previous.actor
                || current.owner_generation != previous.owner_generation
                || current.runtime_id != previous.runtime_id
                || reference.recovery.previous_resume_digest
                    != previous_link
                        .as_ref()
                        .map(|v| canonical_digest(1, v))
                        .transpose()?
                || links
                    .insert(
                        run.attempt,
                        ResumeLink {
                            receipt: current.clone(),
                            metadata: metadata.clone(),
                            completion,
                            admitted,
                            reference,
                        },
                    )
                    .is_some()
            {
                return Err(recovery_blocked(
                    "paired resume skipped its actual predecessor or raised retained bounds",
                ));
            }
            current = previous;
        }
        let origin = current.clone();
        let original = (readers.joined)(origin.operation_id)?
            .ok_or_else(|| recovery_blocked("paired origin lost its joined failure"))?;
        require_recorded_session(&origin, &original)?;
        let preparation = actual_receipt(
            receipts,
            crate::control::preparation_operation(origin.operation_id)?,
        )?
        .clone();
        let origin_ref = receipt_reference(&origin)?;
        let preparation_ref = receipt_reference(&preparation)?;
        for link in links.values() {
            if link.reference.recovery.origin != origin_ref
                || link.reference.recovery.origin_preparation != preparation_ref
                || link.reference.recovery.round_plan_digest != plan.digest()
                || link.reference.recovery.round_reference_digest
                    != canonical_digest(1, plan.reference())?
            {
                return Err(recovery_blocked(
                    "paired resume replaced its exact original admission",
                ));
            }
        }
        let run = closed_run(&origin, "failed")?;
        let implementation_grant = run
            .phase
            .retry_grant
            .map(|id| {
                grants
                    .iter()
                    .find(|g| g.record().operation_id == id && g.phase_key() == run.phase)
                    .cloned()
                    .ok_or_else(|| recovery_blocked("original implementation grant disappeared"))
            })
            .transpose()?;
        let grant_receipt = implementation_grant
            .as_ref()
            .map(|g| actual_receipt(receipts, g.record().operation_id).cloned())
            .transpose()?;
        let result = Self {
            grant_receipt,
            origin,
            preparation,
            original,
            links,
            implementation_grant,
        };
        result.verify_current(plan, receipts, grants, outcome, readers)?;
        Ok(result)
    }
    pub(super) fn verify_joined(&self, readers: &PairedRecoveryReaders<'_>) -> Result<()> {
        require_joined(&self.original, &|id| (readers.joined)(id))?;
        for link in self.links.values() {
            require_joined(&link.completion, &|id| (readers.joined)(id))?;
            if (readers.resumed)(link.receipt.operation_id)?
                .as_ref()
                .is_none_or(|actual| !Arc::ptr_eq(actual, &link.admitted))
            {
                return Err(recovery_blocked(
                    "paired resume admitted bridge identity changed",
                ));
            }
        }
        Ok(())
    }
    fn verify_retained(
        &self,
        plan: &VerifiedReviewRoundPlan,
        receipts: &[Receipt],
        grants: &[VerifiedRetryGrant],
    ) -> Result<()> {
        receipt_reference(&self.origin)?
            .matches_observation(actual_receipt(receipts, self.origin.operation_id)?)?;
        receipt_reference(&self.preparation)?
            .matches_observation(actual_receipt(receipts, self.preparation.operation_id)?)?;
        plan.verify_terminal_header(
            &[self.origin.clone(), self.preparation.clone()],
            &self.origin,
            "failed",
        )?;
        plan.verify_retained_history(receipts, grants)?;
        if let Some(receipt) = &self.grant_receipt {
            receipt_reference(receipt)?
                .matches_observation(actual_receipt(receipts, receipt.operation_id)?)?;
        }
        if let Some(grant) = &self.implementation_grant {
            crate::runtime_workflow::require_current_grant(grant, grants)?;
            if grants.iter().filter(|g| g.phase() == grant.phase()).count() != 1 {
                return Err(recovery_blocked(
                    "retained implementation grant is ambiguous",
                ));
            }
        }
        let mut previous = &self.original;
        for link in self.links.values() {
            let actual = actual_receipt(receipts, link.receipt.operation_id)?;
            receipt_reference(&link.receipt)?.matches_observation(actual)?;
            receipt_reference(&link.metadata)?
                .matches_observation(actual_receipt(receipts, link.metadata.operation_id)?)?;
            link.admitted
                .verify_completed_record(actual, &link.completion)?;
            link.admitted.bridge().verify_completion(previous)?;
            verify_link_metadata(
                plan,
                actual,
                &link.metadata,
                &link.reference,
                &link.admitted,
            )?;
            previous = &link.completion;
        }
        Ok(())
    }
    pub(super) fn verify_current(
        &self,
        plan: &VerifiedReviewRoundPlan,
        receipts: &[Receipt],
        grants: &[VerifiedRetryGrant],
        outcome: &str,
        readers: &PairedRecoveryReaders<'_>,
    ) -> Result<()> {
        self.verify_joined(readers)?;
        self.verify_retained(plan, receipts, grants)?;
        let terminal = self
            .links
            .last_key_value()
            .map(|(_, l)| &l.receipt)
            .unwrap_or(&self.origin);
        closed_run(terminal, outcome)?;
        // Only models whose actual admitted bridges above were verified may be
        // projected out. Every unrelated/new model remains in the current check.
        let consumed: BTreeSet<_> = self
            .links
            .values()
            .map(|l| l.receipt.operation_id)
            .collect();
        let before: Vec<_> = receipts
            .iter()
            .filter(|r| !consumed.contains(&r.operation_id))
            .cloned()
            .collect();
        plan.verify_terminal(&before, grants, &self.origin, "failed")
    }
    fn references(&self) -> Result<BTreeMap<Uuid, String>> {
        let mut refs = BTreeMap::new();
        for receipt in std::iter::once(&self.origin)
            .chain(std::iter::once(&self.preparation))
            .chain(
                self.links
                    .values()
                    .flat_map(|link| [&link.receipt, &link.metadata]),
            )
        {
            let hash = canonical_digest(1, receipt)?;
            if refs
                .insert(receipt.operation_id, hash.clone())
                .is_some_and(|old| old != hash)
            {
                return Err(recovery_blocked(
                    "paired resume has conflicting receipt identity",
                ));
            }
        }
        Ok(refs)
    }
    pub(super) fn digest(&self) -> Result<String> {
        canonical_digest(1, &self.references()?)
    }
}
fn verify_link_metadata(
    plan: &VerifiedReviewRoundPlan,
    model: &Receipt,
    metadata: &Receipt,
    reference: &ReviewRoundResumeReference,
    admitted: &AdmittedSessionResume,
) -> Result<()> {
    verify_link_metadata_reference(plan.reference(), model, metadata, reference, admitted)
}
fn verify_link_metadata_reference(
    round: &ReviewRoundReference,
    model: &Receipt,
    metadata: &Receipt,
    reference: &ReviewRoundResumeReference,
    admitted: &AdmittedSessionResume,
) -> Result<()> {
    let bridge = admitted.bridge();
    let provider = bridge.recovery();
    let retained = provider.observation().retained();
    let observed = metadata.observed_result.as_ref();
    if ReviewRoundReference::from_receipt(model)?.as_ref() != Some(round)
        || ReviewRoundReference::from_receipt(metadata)?.as_ref() != Some(round)
        || ReviewRoundRecoveryReference::from_metadata_receipt(metadata)?.as_ref()
            != Some(&reference.recovery)
        || canonical_digest(1, metadata)? != canonical_digest(1, provider.preparation())?
        || metadata.phase != ReceiptPhase::Reconciled
        || metadata.actor != model.actor
        || metadata.owner_generation != model.owner_generation
        || metadata.runtime_id != model.runtime_id
        || observed.is_none_or(|r| {
            r["local_execution"] != "exited"
                || r["unresolved_effects"]
                    .as_array()
                    .is_none_or(|a| !a.is_empty())
                || r["observation"]["metadata_recovery"] != "verified"
                || r["observation"]["local_exit_confirmed"] != true
                || r["observation"]["proof_digest"] != provider.observation().digest()
        })
        || reference.provider_recovery_digest != provider.digest()
        || reference.instruction_compatibility_digest != bridge.instructions().digest()
        || reference.recovery.protected_candidate_digest != retained.digest()
        || reference.recovery.predecessor.receipt_digest != retained.admission_digest()
        || reference.recovery.terminal_state_digest != canonical_digest(1, retained.state())?
        || reference.recovery.classification
            != classification(provider.observation().transient_failure())
    {
        return Err(recovery_blocked(
            "paired resume changed its actual provider readback or admitted metadata",
        ));
    }
    Ok(())
}

/// A genuine completed resumed repair, separately from a directly prepared
/// paired run. Flat bridge entries retain actual registry identities; the cap
/// does not retain a previous candidate/cap chain or derive authority from JSON.
#[derive(Clone)]
pub(crate) struct VerifiedCompletedPairedResume {
    plan_digest: String,
    chain: PairedResumeChain,
    reference: PairedResumeResultReference,
    round: ReviewRoundReference,
    retained: BTreeMap<Uuid, String>,
    joined: BTreeMap<Uuid, Arc<RunCompletion>>,
    bridges: BTreeMap<Uuid, Arc<AdmittedSessionResume>>,
    grants: BTreeMap<Uuid, VerifiedRetryGrant>,
    sources: Vec<DeliverySource>,
    digest: String,
}
impl VerifiedCompletedPairedResume {
    pub(crate) fn observe(
        control: &ControlHost,
        plan: &Arc<VerifiedReviewRoundPlan>,
        completion: &Arc<RunCompletion>,
        current: &FreshGate,
        config: &Config,
        grants: &[VerifiedRetryGrant],
        readers: &PairedRecoveryReaders<'_>,
    ) -> Result<Self> {
        (readers.current_gate)(current)?;
        plan.authority().compare_admitted_authority(current)?;
        plan.verify_joined(&|id| (readers.joined)(id))?;
        require_joined(completion, &|id| (readers.joined)(id))?;
        if config.revision()? != current.config_revision()
            || !matches!(completion.outcome, RunOutcome::Completed)
        {
            return Err(recovery_blocked(
                "resumed repair result has no exact completed current subject",
            ));
        }
        let receipts = control.operations()?;
        let terminal = actual_receipt(&receipts, completion.operation_id)?;
        let chain =
            PairedResumeChain::observe(plan, &receipts, grants, terminal, "completed", readers)?;
        let latest = chain
            .links
            .last_key_value()
            .map(|(_, l)| l)
            .ok_or_else(|| recovery_blocked("completed resumed repair lacks an admitted bridge"))?;
        if !Arc::ptr_eq(completion, &latest.completion) {
            return Err(recovery_blocked(
                "completed repair substituted its joined result",
            ));
        }
        let reference = PairedResumeResultReference {
            schema_version: 1,
            round_reference_digest: canonical_digest(1, plan.reference())?,
            round_plan_digest: plan.digest().into(),
            origin: receipt_reference(&chain.origin)?,
            origin_preparation: receipt_reference(&chain.preparation)?,
            terminal: receipt_reference(terminal)?,
            resume_reference_digest: canonical_digest(1, &latest.reference)?,
            history_digest: chain.digest()?,
        };
        reference.validate()?;
        let mut retained = plan.retained.clone();
        for (id, hash) in chain.references()? {
            retain_exact(&mut retained, id, hash)?;
        }
        let mut native = plan.retained_native_grants()?;
        merge_native_grants(&mut native, chain.implementation_grant.iter().cloned())?;
        let native = close_native_grants(native, grants)?;
        for id in native.keys() {
            retain_exact(
                &mut retained,
                *id,
                canonical_digest(1, actual_receipt(&receipts, *id)?)?,
            )?;
        }
        let mut joined = BTreeMap::new();
        for id in retained.keys() {
            let actual = actual_receipt(&receipts, *id)?;
            if RunReference::from_receipt(actual)?.is_some() {
                let completion = (readers.joined)(*id)?.ok_or_else(|| {
                    recovery_blocked("completed pair lost joined original closure")
                })?;
                require_recorded_session(actual, &completion)?;
                joined.insert(*id, completion);
            }
        }
        let mut bridges = plan.resumed_bridge_inventory()?;
        for link in chain.links.values() {
            merge_resumed_bridges(
                &mut bridges,
                [(link.receipt.operation_id, link.admitted.clone())].into(),
            )?;
        }
        let digest = canonical_digest(1, &json!({"reference":reference,"retained":retained}))?;
        (readers.current_gate)(current)?;
        let result = Self {
            plan_digest: plan.digest().into(),
            chain,
            reference,
            round: plan.reference().clone(),
            retained,
            joined,
            bridges,
            grants: native,
            sources: plan.retained_source_inventory()?,
            digest,
        };
        result.verify_joined(readers)?;
        Ok(result)
    }

    pub(crate) fn reference(&self) -> &PairedResumeResultReference {
        &self.reference
    }
    pub(crate) fn digest(&self) -> &str {
        &self.digest
    }
    pub(crate) fn completion(&self) -> &Arc<RunCompletion> {
        // Constructor proves at least one actual admitted resume.
        &self
            .chain
            .links
            .last_key_value()
            .expect("verified nonempty resume chain")
            .1
            .completion
    }
    pub(crate) fn verify_joined(&self, readers: &PairedRecoveryReaders<'_>) -> Result<()> {
        self.verify_joined_completions(&|id| (readers.joined)(id))?;
        self.verify_resumed_bridges(readers.resumed)
    }
    pub(crate) fn verify_joined_completions(
        &self,
        reader: &impl Fn(Uuid) -> Result<Option<Arc<RunCompletion>>>,
    ) -> Result<()> {
        for completion in self.joined.values() {
            require_joined(completion, reader)?;
        }
        Ok(())
    }
    pub(crate) fn resumed_bridge_inventory(
        &self,
    ) -> Result<BTreeMap<Uuid, Arc<AdmittedSessionResume>>> {
        Ok(self.bridges.clone())
    }
    pub(crate) fn verify_resumed_bridges(
        &self,
        reader: &dyn Fn(Uuid) -> Result<Option<Arc<AdmittedSessionResume>>>,
    ) -> Result<()> {
        for (id, expected) in self.resumed_bridge_inventory()? {
            if reader(id)?
                .as_ref()
                .is_none_or(|actual| !Arc::ptr_eq(actual, &expected))
            {
                return Err(recovery_blocked(
                    "completed pair lost its actual admitted bridge",
                ));
            }
        }
        Ok(())
    }
    pub(crate) fn origin_operation(&self) -> Uuid {
        self.reference.origin.operation
    }
    pub(crate) fn verify_completion(&self, completion: &Arc<RunCompletion>) -> Result<()> {
        if !Arc::ptr_eq(self.completion(), completion) {
            return Err(recovery_blocked(
                "paired source changed its actual joined resumed result",
            ));
        }
        Ok(())
    }
    pub(crate) fn verify_receipts(&self, receipts: &[Receipt]) -> Result<()> {
        for (id, hash) in &self.retained {
            if canonical_digest(1, actual_receipt(receipts, *id)?)? != *hash {
                return Err(recovery_blocked(
                    "completed resumed pair lost original immutable closure",
                ));
            }
        }
        for link in self.chain.links.values() {
            link.admitted.verify_completed_record(
                actual_receipt(receipts, link.receipt.operation_id)?,
                &link.completion,
            )?;
            verify_link_metadata_reference(
                &self.round,
                &link.receipt,
                &link.metadata,
                &link.reference,
                &link.admitted,
            )?;
        }
        for source in &self.sources {
            source.revalidate_filesystem()?;
        }
        for completion in self.joined.values() {
            if let Some(execution) = &completion.execution {
                let contract = completion.contract.as_ref().ok_or_else(|| {
                    recovery_blocked("retained joined ancestor lost its contract")
                })?;
                execution
                    .ownership
                    .verify(&execution.binding, contract.isolation())?;
            }
        }
        Ok(())
    }
    pub(crate) fn verify_immutable(
        &self,
        receipts: &[Receipt],
        grants: &[VerifiedRetryGrant],
    ) -> Result<()> {
        self.verify_receipts(receipts)?;
        let closure = close_native_grants(self.grants.clone(), grants)?;
        if closure.len() != self.grants.len() {
            return Err(recovery_blocked(
                "completed resumed pair lost captured native grant ancestry",
            ));
        }
        Ok(())
    }
    pub(crate) fn verify_retained(
        &self,
        plan: &VerifiedReviewRoundPlan,
        receipts: &[Receipt],
        grants: &[VerifiedRetryGrant],
    ) -> Result<()> {
        if plan.digest() != self.plan_digest
            || canonical_digest(1, plan.reference())? != self.reference.round_reference_digest
            || self.chain.digest()? != self.reference.history_digest
        {
            return Err(recovery_blocked(
                "completed resumed repair changed its original plan or closure",
            ));
        }
        self.verify_immutable(receipts, grants)?;
        plan.verify_retained_history(receipts, grants)?;
        closed_run(
            actual_receipt(receipts, self.reference.terminal.operation)?,
            "completed",
        )?;
        Ok(())
    }
    pub(crate) fn retained_receipts(&self) -> Result<BTreeMap<Uuid, String>> {
        Ok(self.retained.clone())
    }
}
#[derive(Clone, Default)]
pub(crate) enum PairedCompletionEvidence {
    #[default]
    Direct,
    Resumed(Arc<VerifiedCompletedPairedResume>),
}
impl PairedCompletionEvidence {
    pub(crate) fn from_actual(
        proof: Option<&Arc<VerifiedCompletedPairedResume>>,
        completion: &Arc<RunCompletion>,
        receipts: &[Receipt],
    ) -> Result<Self> {
        match proof {
            Some(proof) => {
                proof.verify_completion(completion)?;
                proof.verify_receipts(receipts)?;
                Ok(Self::Resumed(proof.clone()))
            }
            None => {
                if ReviewRoundResumeReference::from_model_receipt(actual_receipt(
                    receipts,
                    completion.operation_id,
                )?)?
                .is_some()
                {
                    return Err(recovery_blocked(
                        "paired source omitted its actual completed resume capability",
                    ));
                }
                Ok(Self::Direct)
            }
        }
    }
    pub(crate) fn verify_immutable(
        &self,
        receipts: &[Receipt],
        grants: &[VerifiedRetryGrant],
    ) -> Result<()> {
        match self {
            Self::Direct => Ok(()),
            Self::Resumed(p) => p.verify_immutable(receipts, grants),
        }
    }
    pub(crate) fn retain(
        &self,
        retained: &mut BTreeMap<Uuid, String>,
        receipts: &[Receipt],
        actual: &Receipt,
    ) -> Result<()> {
        match self {
            Self::Direct => retain_terminal(retained, receipts, actual),
            Self::Resumed(p) => {
                p.reference.terminal.matches_observation(actual)?;
                p.verify_receipts(receipts)?;
                for (id, hash) in p.retained_receipts()? {
                    retain_exact(retained, id, hash)?;
                }
                Ok(())
            }
        }
    }

    pub(crate) fn verify(
        &self,
        plan: &VerifiedReviewRoundPlan,
        receipts: &[Receipt],
        grants: &[VerifiedRetryGrant],
        actual: &Receipt,
    ) -> Result<()> {
        match self {
            Self::Direct => plan.verify_completed_repair(receipts, grants, actual),
            Self::Resumed(proof) => {
                proof.reference.terminal.matches_observation(actual)?;
                proof.verify_retained(plan, receipts, grants)
            }
        }
    }
    pub(crate) fn reference(&self) -> Option<PairedResumeResultReference> {
        match self {
            Self::Direct => None,
            Self::Resumed(proof) => Some(proof.reference.clone()),
        }
    }
}

impl crate::delivery::lineage::Node for VerifiedCompletedPairedResume {
    fn expand<'a>(
        &'a self,
        _children: &mut Vec<&'a dyn crate::delivery::lineage::Node>,
        leaves: &mut crate::delivery::lineage::Inventory,
    ) -> Result<()> {
        leaves.sources(self.sources.iter().cloned())?;
        leaves.grants(self.grants.values().cloned())?;
        leaves.bridges(&self.bridges)?;
        leaves.joined(self.joined.len())
    }
    fn verify_joined_leaf(
        &self,
        reader: &dyn Fn(Uuid) -> Result<Option<Arc<RunCompletion>>>,
    ) -> Result<()> {
        // The sealed result holds flat leaf maps, never an older round/candidate.
        self.verify_joined_completions(&|id| reader(id))
    }
}
impl crate::delivery::lineage::Node for PairedCompletionEvidence {
    fn expand<'a>(
        &'a self,
        children: &mut Vec<&'a dyn crate::delivery::lineage::Node>,
        _leaves: &mut crate::delivery::lineage::Inventory,
    ) -> Result<()> {
        if let Self::Resumed(p) = self {
            children.push(p.as_ref());
        }
        Ok(())
    }
}

impl VerifiedCompletedPairedResume {
    pub(crate) fn retained_native_grants(&self) -> Result<BTreeMap<Uuid, VerifiedRetryGrant>> {
        Ok(self.grants.clone())
    }
}
