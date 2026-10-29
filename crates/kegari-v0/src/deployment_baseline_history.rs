//! Immutable references for a baseline already authenticated through the owned
//! target producer. This does not attest externally deployed production.
use super::*;

const MAX_REFERENCES: usize = 4096;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReceiptReference {
    operation: Uuid,
    digest: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileReference {
    leaf: String,
    purpose: String,
    digest: String,
}

/// Private reference data, not a proof constructor. Only an actual current
/// opaque baseline may produce it. A signed enclosing seal must retain it.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OwnedTargetObservationRef {
    runtime: Uuid,
    contract_digest: String,
    target: ProductionTargetBaseline,
    record_digest: String,
    proof_leaf: String,
    files: Vec<FileReference>,
    receipts: Vec<ReceiptReference>,
}
impl OwnedTargetObservationRef {
    pub(crate) fn target(&self) -> &ProductionTargetBaseline {
        &self.target
    }
    pub(crate) fn runtime_id(&self) -> Uuid {
        self.runtime
    }
    pub(crate) fn contract_digest(&self) -> &str {
        &self.contract_digest
    }
    /// Existing exact workspace/deployment namespace only; later main or target
    /// movement is irrelevant to this immutable observation's validity.
    pub(crate) fn validate_retained(
        &self,
        records: &PrivateDir,
        control: &ControlHost,
        key: &AttestationKey,
    ) -> Result<()> {
        if self.runtime.is_nil()
            || self.files.is_empty()
            || self.files.len() > MAX_REFERENCES
            || self.receipts.is_empty()
            || self.receipts.len() > MAX_REFERENCES
        {
            return Err(blocked(
                "owned baseline references have invalid bounded identity",
            ));
        }
        self.target.source.validate()?;
        crate::evidence::validate_digest(&self.target.artifact_digest)?;
        crate::evidence::validate_digest(&self.contract_digest)?;
        crate::evidence::validate_digest(&self.record_digest)?;
        let signed: Signed<TargetRecord> = records.read_bounded(&self.proof_leaf, 1_048_576)?;
        key.verify("target_deployment", &signed)?;
        let record = &signed.payload;
        if self.proof_leaf
            != format!(
                "target-proof-{}.json",
                signed.digest()?.rsplit(':').next().unwrap()
            )
            || canonical_digest(1, record)? != self.record_digest
            || record.challenge.runtime != self.runtime
            || record.challenge.contract_digest != self.contract_digest
            || record.target.repository != self.target.repository
            || !same_target(&record.target, &self.target)
        {
            return Err(blocked(
                "owned baseline proof differs from retained target identity",
            ));
        }
        let mut files = BTreeSet::new();
        let mut has_proof = false;
        let mut has_challenge = false;
        for reference in &self.files {
            if !files.insert(reference.leaf.clone()) {
                return Err(blocked(
                    "owned baseline contains duplicate immutable file references",
                ));
            }
            crate::evidence::validate_digest(&reference.digest)?;
            match reference.purpose.as_str() {
                "target_deployment" if reference.leaf == self.proof_leaf => {
                    if signed.digest()? != reference.digest {
                        return Err(blocked("owned baseline immutable proof changed"));
                    }
                    has_proof = true;
                }
                "deployment_challenge"
                    if reference.leaf == challenge_file(record.challenge.challenge) =>
                {
                    let challenge: Signed<ChallengeRecord> = records.read(&reference.leaf)?;
                    key.verify("deployment_challenge", &challenge)?;
                    if challenge.digest()? != reference.digest
                        || challenge.payload != record.challenge
                    {
                        return Err(blocked("owned baseline immutable challenge changed"));
                    }
                    has_challenge = true;
                }
                "target_deployment_index" => {
                    let allowed = [
                        format!("target-dispatch-{}.json", record.challenge.dispatch),
                        format!(
                            "target-digest-{}.json",
                            self.record_digest.rsplit(':').next().unwrap()
                        ),
                    ];
                    if !allowed.contains(&reference.leaf) {
                        return Err(blocked(
                            "mutable or unrelated target locator is not historical evidence",
                        ));
                    }
                    let index: Signed<TargetIndex> = records.read(&reference.leaf)?;
                    key.verify("target_deployment_index", &index)?;
                    if index.digest()? != reference.digest
                        || index.payload.proof_file != self.proof_leaf
                        || index.payload.proof_digest != signed.digest()?
                        || index.payload.contract_digest != self.contract_digest
                        || index.payload.step != record.challenge.step
                    {
                        return Err(blocked("owned baseline immutable locator changed"));
                    }
                }
                _ => return Err(blocked("owned baseline reference purpose differs")),
            }
        }
        let mut receipts = BTreeSet::new();
        for reference in &self.receipts {
            let actual = control.operation(reference.operation)?;
            if !receipts.insert(reference.operation)
                || reference.operation.is_nil()
                || actual.operation_id != reference.operation
                || actual.request_id != reference.operation
                || actual.runtime_id != self.runtime
                || canonical_digest(1, &actual)? != reference.digest
            {
                return Err(blocked("owned baseline immutable receipt changed"));
            }
        }
        if !has_proof
            || !has_challenge
            || !receipts.contains(&record.challenge.operation)
            || !receipts.contains(&record.challenge.dispatch)
        {
            return Err(blocked(
                "owned baseline omitted original proof or ownership references",
            ));
        }
        Ok(())
    }
}
pub(super) fn same_target(
    left: &ProductionTargetBaseline,
    right: &ProductionTargetBaseline,
) -> bool {
    let mut normalized = left.clone();
    normalized.observed_at = right.observed_at;
    normalized == *right
}

impl ProductionObserver {
    /// Consume only target proofs retained by the actual complete baseline read.
    /// A current target may have an old owned deployment; no new run is minted.
    pub(crate) fn owned_baseline_references(
        &self,
        contract: &RepositoryContract,
        baseline: &VerifiedProductionBaseline,
    ) -> Result<Vec<OwnedTargetObservationRef>> {
        if baseline.contract_digest != contract.digest()?
            || baseline.baseline.repository != contract.repository
            || baseline.baseline.successor_recovery_operation.is_some()
            || Utc::now() - baseline.observed_at > chrono::Duration::seconds(30)
            || baseline.observed_at > Utc::now() + chrono::Duration::seconds(5)
        {
            return Err(blocked(
                "owned baseline reference requires exact recent ordinary observation",
            ));
        }
        if baseline.baseline.deployment_free {
            if !baseline.targets.is_empty()
                || !baseline.owned_targets.is_empty()
                || !baseline.external_targets.is_empty()
            {
                return Err(blocked(
                    "deployment-free baseline unexpectedly contains target proofs",
                ));
            }
            return Ok(vec![]);
        }
        let Deployment::Steps { steps } = contract.configuration.deployment_policy()? else {
            return Err(blocked(
                "owned target baseline lacks configured deployment inventory",
            ));
        };
        if steps.len() != baseline.targets.len()
            || steps.len() != baseline.owned_targets.len() + baseline.external_targets.len()
        {
            return Err(blocked(
                "owned target baseline provenance inventory is incomplete",
            ));
        }
        let mut seen = BTreeSet::new();
        let mut output = Vec::new();
        for proof in &baseline.owned_targets {
            let matching: Vec<_> = baseline
                .targets
                .iter()
                .filter(|target| target.target == proof.target().target)
                .collect();
            let [target] = matching.as_slice() else {
                return Err(blocked("owned baseline target is missing or duplicated"));
            };
            if !seen.insert(target.target.clone())
                || !same_target(target, proof.target())
                || !steps.iter().any(|step| {
                    step.id == proof.record.challenge.step && step.target_id == target.target
                })
                || target.source != baseline.baseline.production
                || baseline.baseline.artifact_for_target(&target.target)
                    != Some(target.artifact_digest.as_str())
                || baseline.baseline.source_for_target(&target.target)
                    != Some(target.source.commit.as_str())
            {
                return Err(blocked(
                    "owned baseline target differs from exact complete source/artifact inventory",
                ));
            }
            let refs = self.current_owned_references(contract, proof)?;
            let mut receipts = BTreeMap::new();
            for receipt in refs.receipts {
                let digest = canonical_digest(1, &receipt)?;
                if receipts
                    .insert(receipt.operation_id, digest.clone())
                    .is_some_and(|prior| prior != digest)
                {
                    return Err(blocked("owned baseline receipt ancestry conflicts"));
                }
            }
            let signed = self.key.sign("target_deployment", &proof.record)?;
            let reference = OwnedTargetObservationRef {
                runtime: proof.record.challenge.runtime,
                contract_digest: baseline.contract_digest.clone(),
                target: (**target).clone(),
                record_digest: proof.digest()?,
                proof_leaf: format!(
                    "target-proof-{}.json",
                    signed.digest()?.rsplit(':').next().unwrap()
                ),
                files: refs
                    .files
                    .into_iter()
                    .map(|(leaf, purpose, digest)| FileReference {
                        leaf,
                        purpose,
                        digest,
                    })
                    .collect(),
                receipts: receipts
                    .into_iter()
                    .map(|(operation, digest)| ReceiptReference { operation, digest })
                    .collect(),
            };
            reference.validate_retained(&self.records, &self.control, &self.key)?;
            output.push(reference);
        }
        output.sort_by(|a, b| a.target.target.cmp(&b.target.target));
        Ok(output)
    }
}
