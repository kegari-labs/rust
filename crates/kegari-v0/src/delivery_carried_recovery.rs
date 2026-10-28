//! Completed-current recovery never reconstructs a worker, joined completion,
//! execution capability, or an unfinished parent pipeline.
use super::*;
use crate::control::ParentReassemblyReference;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProductionSelection {
    repository: crate::git::RepositoryId,
    owner: ReviewReceiptReference,
    existing: bool,
    proof_digest: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CarryRecoveryBinding {
    schema_version: u32,
    configuration_revision: String,
    vector_digest: String,
    integrated: ReviewReceiptReference,
    review: ReviewReceiptReference,
    production: Vec<ProductionSelection>,
}
impl CarryRecoveryBinding {
    pub(super) fn capture(
        parent: &ReassembledParent,
        production: &[crate::delivery::CarryProduction],
        config: &Config,
        host: &ControlHost,
    ) -> Result<Self> {
        let [integrated, review] = parent.results.as_slice() else {
            return Err(invalid(
                "carry recovery requires both actual final parent results",
            ));
        };
        let exact = |id| -> Result<ReviewReceiptReference> {
            let receipt = host.operation(id)?;
            Ok(ReviewReceiptReference {
                operation: id,
                receipt_digest: canonical_digest(1, &receipt)?,
            })
        };
        let value = Self {
            schema_version: 1,
            configuration_revision: crate::config::config_revision(config)?,
            vector_digest: parent.source.vector().digest().into(),
            integrated: exact(integrated.operation_id)?,
            review: exact(review.operation_id)?,
            production: production
                .iter()
                .map(|p| {
                    Ok(ProductionSelection {
                        repository: p.proof().revision().repository.clone(),
                        owner: exact(p.operation())?,
                        existing: p.existing(),
                        proof_digest: p.proof().digest()?,
                    })
                })
                .collect::<Result<_>>()?,
        };
        value.validate_shape(&parent.parent.verified().manifest().candidates)?;
        Ok(value)
    }
    fn validate_shape(&self, candidates: &[crate::git::CandidateManifest]) -> Result<()> {
        evidence::validate_digest(&self.configuration_revision)?;
        evidence::validate_digest(&self.vector_digest)?;
        let mut operations = BTreeSet::new();
        for reference in [&self.integrated, &self.review] {
            evidence::validate_digest(&reference.receipt_digest)?;
            if reference.operation.is_nil() || !operations.insert(reference.operation) {
                return Err(invalid("carry recovery final phase identities overlap"));
            }
        }
        let mut repositories = BTreeSet::new();
        for selected in &self.production {
            selected.repository.validate()?;
            evidence::validate_digest(&selected.proof_digest)?;
            evidence::validate_digest(&selected.owner.receipt_digest)?;
            if selected.owner.operation.is_nil()
                || !operations.insert(selected.owner.operation)
                || !repositories.insert(selected.repository.clone())
                || !candidates.iter().any(|candidate| {
                    candidate.repository == selected.repository
                        && selected.existing == candidate.ordered_delta_digests.is_empty()
                })
            {
                return Err(invalid(
                    "carry recovery production selections are duplicate or differ from the vector",
                ));
            }
        }
        if self.schema_version != 1
            || repositories.is_empty()
            || repositories != candidates.iter().map(|c| c.repository.clone()).collect()
            || self.production.len() != candidates.len()
        {
            return Err(invalid(
                "carry recovery omits the complete production vector",
            ));
        }
        Ok(())
    }
}

/// A fully authenticated historical subject; its fields have no public or
/// serde constructor. It is not current acceptance, delivery or worker authority.
#[derive(Clone)]
pub(crate) struct RetainedCarrySubject {
    saved: Signed<CarryPublicationRecord>,
    records: Arc<PrivateDir>,
    root: Arc<PrivateDir>,
    host: Arc<ControlHost>,
    key: Arc<AttestationKey>,
}
impl RetainedCarrySubject {
    pub(crate) fn digest(&self) -> Result<String> {
        self.saved.digest()
    }
    fn record(&self) -> &CarryRecord {
        &self.saved.payload.signed.payload
    }
    fn binding(&self) -> Result<&CarryRecoveryBinding> {
        self.record().recovery.as_ref().ok_or_else(|| {
            Error::new(
                "carry_recovery_unavailable",
                "legacy carry has no producer-written exact recovery selection",
            )
        })
    }
    pub(crate) fn original_acceptance(&self) -> &Signed<evidence::Acceptance> {
        &self.record().original_acceptance
    }
    pub(crate) fn current_evidence(&self) -> &evidence::ParentEvidence {
        &self.record().current_parent.payload.evidence
    }
    pub(crate) fn carry_record(&self) -> &CarriedAcceptance {
        &self.record().carry
    }
    pub(crate) fn validate_retained(&self) -> Result<()> {
        self.validate()
    }
    fn validate(&self) -> Result<()> {
        let actual: Signed<CarryPublicationRecord> = self
            .records
            .read(&leaf(&self.record().carry.current_evidence_token)?)?;
        self.key.verify(SAVED_PURPOSE, &actual)?;
        if actual.digest()? != self.saved.digest()? {
            return Err(invalid("retained current carry changed"));
        }
        validate_saved(&actual, &self.root, &self.records, &self.host, &self.key)?;
        closed(&self.host.operation(self.record().publication_operation)?)?;
        self.binding()?
            .validate_shape(&self.current_evidence().candidates)?;
        if self.binding()?.vector_digest != self.record().current_parent.payload.vector_digest {
            return Err(invalid("carry recovery vector selection differs"));
        }
        let receipts = self.host.operations()?;
        for selected in &self.binding()?.production {
            let owner = self.host.operation(selected.owner.operation)?;
            if canonical_digest(1, &owner)? != selected.owner.receipt_digest {
                return Err(invalid("carry recovery production owner changed"));
            }
            closed(&owner)?;
            self.record().references.require_receipt(&owner)?;
        }
        for (reference, phase) in [
            (&self.binding()?.integrated, "parent_integrated_validation"),
            (&self.binding()?.review, "parent_evidence_review"),
        ] {
            let actual = exact_phase(&receipts, reference, phase, self.record())?;
            self.record().references.require_receipt(actual)?;
        }
        Ok(())
    }
}

fn exact_phase<'a>(
    receipts: &'a [Receipt],
    selected: &ReviewReceiptReference,
    phase: &str,
    carry: &CarryRecord,
) -> Result<&'a Receipt> {
    let matches = receipts
        .iter()
        .filter(|r| r.operation_id == selected.operation)
        .collect::<Vec<_>>();
    let [receipt] = matches.as_slice() else {
        return Err(invalid(
            "carry recovery final worker is missing or duplicated",
        ));
    };
    if canonical_digest(1, *receipt)? != selected.receipt_digest {
        return Err(invalid("carry recovery final worker changed"));
    }
    closed(receipt)?;
    let reference = ParentReassemblyReference::from_receipt(receipt)?
        .ok_or_else(|| invalid("carry recovery final worker lost parent marker"))?;
    let run = crate::control::run_reference_from_receipt(receipt)?
        .ok_or_else(|| invalid("carry recovery final worker lost run"))?;
    let result = receipt
        .observed_result
        .as_ref()
        .ok_or_else(|| invalid("carry recovery worker has no terminal observation"))?;
    if !reference.matches_run(&run)
        || run.phase.phase != phase
        || reference.source != carry.source
        || reference.vector_digest != carry.current_parent.payload.vector_digest
        || reference.baseline_digest != carry.baseline_digest
        || reference.original_token != carry.carry.original_reviewed_token
        || reference.parent_work != carry.current_parent.payload.evidence.work
        || (phase == "parent_integrated_validation"
            && (reference.worker_work.child
                != Some(carry.current_parent.payload.evidence.e2e_child)
                || !carry
                    .current_parent
                    .payload
                    .evidence
                    .required_children
                    .contains(&carry.current_parent.payload.evidence.e2e_child)))
        || reference.workspace != carry.workspace
        || reference.runtime != carry.runtime
        || result["observation"]["operation_id"] != receipt.operation_id.to_string()
        || result["observation"]["outcome"] != "completed"
        || !result["observation"]["error"].is_null()
        || result["observation"]["local_exit_confirmed"] != true
        || result["observation"]["session"]["terminal"] != "completed"
        || !result["observation"]["session"]["turn_id"].is_null()
        || !result["observation"]["session"]["unresolved_turn"].is_null()
        || result["observation"]["session"]["input_required"] != false
        || !result["observation"]["session"]["intervention"].is_null()
        || !result["observation"]["hook_error"].is_null()
    {
        return Err(invalid(
            "carry recovery selected worker is not the exact completed parent result",
        ));
    }
    let prep: ReviewReceiptReference = serde_json::from_str(
        receipt
            .expected
            .get("parent_reassembly_preparation")
            .ok_or_else(|| invalid("carry recovery preparation absent"))?,
    )
    .map_err(|_| invalid("carry recovery preparation identity invalid"))?;
    let actual = receipts
        .iter()
        .find(|r| r.operation_id == prep.operation)
        .ok_or_else(|| invalid("carry recovery preparation missing"))?;
    crate::control::require_successful_parent_preparation(actual)?;
    if canonical_digest(1, actual)? != prep.receipt_digest
        || actual.actor != receipt.actor
        || actual.owner_generation != receipt.owner_generation
        || ParentReassemblyReference::from_receipt(actual)?.as_ref() != Some(&reference)
    {
        return Err(invalid("carry recovery preparation changed"));
    }
    carry.references.require_receipt(actual)?;
    let prior = receipts
        .iter()
        .filter(|r| r.operation_id != receipt.operation_id)
        .cloned()
        .collect::<Vec<_>>();
    if crate::control::parent_reassembly_inventory(&prior, &reference)?
        != reference.inventory_digest
    {
        return Err(invalid(
            "carry recovery worker was superseded or its complete phase history changed",
        ));
    }
    for (operation, digest) in &reference.history {
        let actual = receipts
            .iter()
            .find(|r| r.operation_id == *operation)
            .ok_or_else(|| invalid("carry recovery consumed history missing"))?;
        if canonical_digest(1, actual)? != *digest {
            return Err(invalid("carry recovery consumed history changed"));
        }
        carry.references.require_receipt(actual)?;
    }
    Ok(receipt)
}

#[derive(Clone)]
pub(super) struct ReobservedCarry {
    pub(super) parent: PublishedParent,
    pub(super) acceptance: VerifiedAcceptance,
    subject: RetainedCarrySubject,
    configuration_revision: String,
    phase_gates: Vec<FreshGate>,
    integrations: Option<crate::delivery::RetainedCarryIntegrations>,
    pub(super) current_grants: ParentGrantObserver,
    read: Option<ReviewReceiptReference>,
}
impl ReobservedCarry {
    pub(super) fn validate(
        &self,
        config: &Config,
        host: &ControlHost,
        grants: &dyn Fn(&FreshGate) -> Result<Vec<crate::decisions::VerifiedRetryGrant>>,
    ) -> Result<()> {
        if crate::config::config_revision(config)? != self.configuration_revision {
            return Err(Error::new(
                "stale_config",
                "recovered current carry configuration changed",
            ));
        }
        self.subject.validate()?;
        if let Some(integrations) = &self.integrations {
            integrations.validate_retained(&self.subject)?;
        }
        let read = self
            .read
            .as_ref()
            .ok_or_else(|| invalid("carry recovery owned reads have not retired"))?;
        let receipt = host.operation(read.operation)?;
        if canonical_digest(1, &receipt)? != read.receipt_digest {
            return Err(invalid("carry recovery closed read receipt changed"));
        }
        let receipts = host.operations()?;
        let binding = self.subject.binding()?;
        if self.phase_gates.len() != 2 {
            return Err(invalid("carry recovery omitted a final phase authority"));
        }
        for (reference, gate) in [&binding.integrated, &binding.review]
            .into_iter()
            .zip(&self.phase_gates)
        {
            let receipt = host.operation(reference.operation)?;
            let marker = ParentReassemblyReference::from_receipt(&receipt)?
                .ok_or_else(|| invalid("carry recovery marker absent"))?;
            if gate.work() != &marker.worker_work || gate.workspace_id() != marker.workspace {
                return Err(invalid("carry recovery phase gate changed subject"));
            }
            super::super::reassembly::verify_retained_parent_grants(
                &marker,
                &receipts,
                &grants(gate)?,
            )?;
        }
        let workspace = config
            .workspaces
            .values()
            .find(|w| w.id == self.subject.record().workspace)
            .ok_or_else(|| invalid("recovered carry workspace removed"))?;
        if !workspace
            .actors
            .acceptors
            .iter()
            .any(|id| id.to_string() == self.acceptance.acceptance().linear_actor)
        {
            return Err(Error::new(
                "access_denied",
                "recovered carry acceptor removed",
            ));
        }
        Ok(())
    }
}

/// Fresh scoped reads are complete, but the actual host owner must close before
/// this object can yield PublishedCarry. It cannot enter a worker/run cache.
pub(crate) struct ObservedCurrentCarry {
    carry: PublishedCarry,
    read: crate::host_process::HostProcessIdentity,
}
impl ObservedCurrentCarry {
    pub(crate) fn finish(mut self, receipt: &Receipt) -> Result<PublishedCarry> {
        let digest = self.carry.saved.digest()?;
        if receipt.operation_id != self.read.operation_id
            || receipt.runtime_id != self.read.runtime_id
            || receipt.kind != "host_metadata"
            || receipt.phase != ReceiptPhase::Acknowledged
            || receipt.canonical_targets != [self.read.work_key.clone()]
            || receipt.expected.get("scope") != Some(&self.read.scope_digest)
            || receipt.expected.get("host_identity") != Some(&canonical_digest(1, &self.read)?)
            || receipt.observed_result.as_ref().is_none_or(|r| {
                r["owned_handles_joined"] != true
                    || r["metadata_observation"]["carry_recovery"] != digest
                    || r["metadata_observation"]["verified"] != true
            })
        {
            return Err(invalid(
                "carry recovery requires its actual joined scoped read owner",
            ));
        }
        let actual = self.carry.host.operation(receipt.operation_id)?;
        if canonical_digest(1, &actual)? != canonical_digest(1, receipt)? {
            return Err(invalid(
                "carry recovery supplied read receipt differs from Control",
            ));
        }
        let CarryBasis::Reobserved(value) = &mut self.carry.basis else {
            return Err(invalid("carry recovery basis differs"));
        };
        value.subject.validate()?;
        if let Some(integrations) = &value.integrations {
            integrations.validate_retained(&value.subject)?;
        }
        value.read = Some(ReviewReceiptReference {
            operation: receipt.operation_id,
            receipt_digest: canonical_digest(1, receipt)?,
        });
        Ok(self.carry)
    }
}

impl DeliveryCoordinator {
    /// No native reads and no capability from a caller-supplied signed value.
    pub(crate) fn retained_current_carry_subject(
        &self,
        gate: &FreshGate,
        token: &str,
        root: Arc<PrivateDir>,
    ) -> Result<Option<RetainedCarrySubject>> {
        (self.current_gate)(gate)?;
        let Some(saved) = read_retained_carry_record(&self.records, &self.key, token)? else {
            return Ok(None);
        };
        let subject = RetainedCarrySubject {
            saved,
            records: self.records.clone(),
            root,
            host: self.host.clone(),
            key: self.key.clone(),
        };
        subject.validate()?;
        if subject.record().workspace != gate.workspace_id()
            || &subject.current_evidence().work != gate.work()
            || subject.carry_record().current_evidence_token != token
        {
            return Err(invalid(
                "retained current carry differs from actual current subject",
            ));
        }
        Ok(Some(subject))
    }
    pub(crate) async fn reobserve_current_carry(
        &self,
        subject: RetainedCarrySubject,
        input: ParentHydrationInputs<'_>,
        config: &Config,
        read: &crate::host_process::LocalProcessPermit,
        current_grants: ParentGrantObserver,
        integration_history: Option<&crate::delivery::RetainedCarryIntegrations>,
    ) -> Result<ObservedCurrentCarry> {
        subject.validate()?;
        match integration_history {
            Some(history) => {
                history.validate_retained(&subject)?;
                history.require_input(input.production_integrations)?;
            }
            None if input.production_integrations.is_empty()
                && subject
                    .current_evidence()
                    .candidates
                    .iter()
                    .all(|candidate| candidate.production_integrations.is_empty()) => {}
            None => {
                return Err(invalid(
                    "carry integrations require their genuine retained source history",
                ));
            }
        }
        let gate = input.contract.gate.clone();
        if crate::config::config_revision(config)? != subject.binding()?.configuration_revision
            || crate::config::config_revision(config)? != gate.config_revision()
            || read.identity().runtime_id != config.runtime.id
            || read.identity().work_key != gate.work_key()?
            || read.identity().config_revision != crate::config::config_revision(config)?
        {
            return Err(Error::new(
                "stale_config",
                "carry recovery needs exact current configured read authority",
            ));
        }
        let parent = self
            .hydrate_parent_result(
                ParentHydrationInputs {
                    contract: input.contract,
                    observer: input.observer,
                    child_gates: input.child_gates,
                    workflow: input.workflow,
                    production_integrations: input.production_integrations,
                },
                subject
                    .record()
                    .current_publication
                    .record_id
                    .parse()
                    .map_err(|_| invalid("carry current parent publication is invalid"))?,
            )
            .await?;
        if parent.signed.digest()? != subject.record().current_parent.digest()? {
            return Err(invalid("fresh parent differs from saved current carry"));
        }
        let mut production = Vec::new();
        for selected in &subject.binding()?.production {
            let candidate = parent
                .verified()
                .manifest()
                .candidates
                .iter()
                .find(|c| c.repository == selected.repository)
                .ok_or_else(|| invalid("carry production repository omitted"))?;
            let observation = self.observation(gate.clone(), candidate_binding(candidate)?)?;
            let proof = self
                .reobserve_retained_carry_production(
                    crate::delivery::RetainedCarryProductionInput {
                        observation: &observation,
                        parent: parent.verified(),
                        integrations: input.production_integrations,
                        observer: input.observer,
                        operation: selected.owner.operation,
                        existing: selected.existing,
                    },
                )
                .await?;
            if proof.proof().digest()? != selected.proof_digest {
                return Err(invalid(
                    "carry production proof differs from actual original selection",
                ));
            }
            production.push(proof);
        }
        let mut phase_gates = Vec::new();
        let receipts = self.host.operations()?;
        for reference in [&subject.binding()?.integrated, &subject.binding()?.review] {
            let receipt = self.host.operation(reference.operation)?;
            let marker = ParentReassemblyReference::from_receipt(&receipt)?
                .ok_or_else(|| invalid("carry phase marker missing"))?;
            let actual = match marker.worker_work.child {
                Some(child) => input
                    .child_gates
                    .get(&child)
                    .ok_or_else(|| invalid("carry current required child gate missing"))?,
                None => &gate,
            };
            if actual.work() != &marker.worker_work || actual.workspace_id() != marker.workspace {
                return Err(invalid(
                    "carry actual phase gate differs from original work",
                ));
            }
            (self.current_gate)(actual)?;
            super::super::reassembly::verify_retained_parent_grants(
                &marker,
                &receipts,
                &current_grants(actual.work())?,
            )?;
            phase_gates.push(actual.clone());
        }
        let linear = self
            .broker
            .linear
            .as_ref()
            .ok_or_else(|| invalid("carry native reader unavailable"))?;
        let identity = linear.identity().await?;
        let comments = linear.comments(gate.work().parent).await?;
        if identity.account_id != subject.original_acceptance().payload.provider_writer {
            return Err(invalid("carry native writer changed"));
        }
        let workspace = config
            .workspaces
            .values()
            .find(|w| w.id == gate.workspace_id())
            .ok_or_else(|| invalid("carry workspace removed"))?;
        let permitted = workspace
            .actors
            .acceptors
            .iter()
            .map(|id| id.to_string())
            .collect();
        let (acceptance, verified) = evidence::recover_current_carry_semantics(
            &subject,
            parent.verified(),
            &production
                .iter()
                .map(|p| p.proof().clone())
                .collect::<Vec<_>>(),
            &permitted,
        )?;
        let carry = PublishedCarry {
            basis: CarryBasis::Reobserved(Box::new(ReobservedCarry {
                parent,
                acceptance,
                subject: subject.clone(),
                configuration_revision: crate::config::config_revision(config)?,
                phase_gates,
                integrations: integration_history.cloned(),
                current_grants,
                read: None,
            })),
            verified,
            saved: subject.saved.clone(),
            records: subject.records.clone(),
            root: subject.root.clone(),
            host: subject.host.clone(),
            key: subject.key.clone(),
        };
        carry.verify_native_comments(&comments)?;
        subject.validate()?;
        if let Some(history) = integration_history {
            history.validate_retained(&subject)?;
        }
        (self.current_gate)(&gate)?;
        Ok(ObservedCurrentCarry {
            carry,
            read: read.identity().clone(),
        })
    }
}

fn read_retained_carry_record(
    records: &PrivateDir,
    key: &AttestationKey,
    token: &str,
) -> Result<Option<Signed<CarryPublicationRecord>>> {
    let saved: Signed<CarryPublicationRecord> = match records.read(&leaf(token)?) {
        Ok(value) => value,
        Err(error) if error.code == "record_missing" => return Ok(None),
        Err(error) => return Err(error),
    };
    key.verify(SAVED_PURPOSE, &saved)?;
    Ok(Some(saved))
}

#[cfg(test)]
#[path = "delivery_carried_recovery_tests.rs"]
mod tests;
