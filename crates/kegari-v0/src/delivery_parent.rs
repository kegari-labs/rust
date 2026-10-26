//! Complete isolated parent vectors and retained integrated execution evidence.
//! This is a child module of delivery; no public JSON value can construct these
//! capabilities. Provider baselines and execution results are independently sealed.
use super::*;
use crate::config::Config;
use crate::deployment::{ProductionObserver, VerifiedProductionBaseline};
use crate::evidence::{IntegratedRun, ProductionBaseline, ProductionTargetBaseline};
use crate::execution::{ArtifactProof, CheckExecutionRecord};
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;

#[path = "delivery_parent_run_origin.rs"]
mod run_origin;
pub(crate) use run_origin::{
    ParentAssignmentOrigin, ParentOriginRecord, PreparedParentRunOrigin,
    read_origin as read_parent_run_origin,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct VectorRecord {
    schema_version: u32,
    runtime_id: Uuid,
    work: WorkIdentity,
    contract_digest: String,
    slices: Vec<(RepositoryId, Uuid)>,
    baselines: Vec<ProductionBaseline>,
    candidates: Vec<crate::git::CandidateManifest>,
    targets: Vec<ProductionTargetBaseline>,
    prepared_at: DateTime<Utc>,
}
#[derive(Clone, Debug)]
pub struct ParentCandidateVector {
    signed: Signed<VectorRecord>,
    digest: String,
}
impl ParentCandidateVector {
    pub fn digest(&self) -> &str {
        &self.digest
    }
    pub fn baselines(&self) -> &[ProductionBaseline] {
        &self.signed.payload.baselines
    }
    pub fn candidates(&self) -> &[crate::git::CandidateManifest] {
        &self.signed.payload.candidates
    }
    pub fn prepared_at(&self) -> DateTime<Utc> {
        self.signed.payload.prepared_at
    }
}
fn vector_digest(record: &VectorRecord) -> Result<String> {
    canonical_digest(
        1,
        &json!({"baselines":record.baselines,"candidates":record.candidates}),
    )
}
pub(super) fn candidate_binding(
    candidate: &crate::git::CandidateManifest,
) -> Result<RevisionBinding> {
    let result = candidate
        .result
        .as_ref()
        .ok_or_else(|| Error::new("candidate_provenance", "candidate has no completed result"))?;
    Ok(RevisionBinding {
        repository: candidate.repository.clone(),
        head: result.commit.clone(),
        base: candidate.production.commit.clone(),
        tested_commit: result.commit.clone(),
        tested_tree: result.tree.clone(),
    })
}
// A newly authenticated deployment of the same exact source and artifact is
// semantically equivalent. The retained provider record is provenance; every
// reobservation independently verifies the actual current deployment record.
fn target_identity(target: &ProductionTargetBaseline) -> Result<String> {
    canonical_digest(
        1,
        &json!({"provider":target.provider,"target":target.target,
        "repository":target.repository,"source":target.source,
        "artifact_digest":target.artifact_digest}),
    )
}
impl DeliveryCoordinator {
    /// This read-only assembly check is legal while stopped. It never creates a
    /// candidate, executes a check, or grants acceptance/release authority.
    pub async fn prepare_parent_vector(
        &self,
        contract: &ParentContract,
        observer: &ProductionObserver,
        slices: &[&IsolatedSlice],
        baselines: &[&VerifiedProductionBaseline],
    ) -> Result<ParentCandidateVector> {
        (self.current_gate)(&contract.gate)?;
        let mut record = VectorRecord {
            schema_version: 1,
            runtime_id: self.host.snapshot()?.intent.runtime_id,
            work: contract.work().clone(),
            contract_digest: contract.digest()?,
            slices: vec![],
            baselines: vec![],
            candidates: vec![],
            targets: vec![],
            prepared_at: Utc::now(),
        };
        let mut by_repository = BTreeMap::new();
        for baseline in baselines {
            if by_repository
                .insert(baseline.baseline().repository.clone(), *baseline)
                .is_some()
            {
                return Err(Error::new(
                    "candidate_provenance",
                    "duplicate baseline in parent vector",
                ));
            }
        }
        if by_repository.keys().cloned().collect::<BTreeSet<_>>() != *contract.repositories()
            || slices.len() != contract.repositories().len()
        {
            return Err(Error::new(
                "candidate_provenance",
                "parent vector must cover every authorized repository",
            ));
        }
        let mut targets = BTreeSet::new();
        for slice in slices {
            let candidate = slice.manifest();
            let repository = &candidate.repository;
            if !contract.repositories().contains(repository)
                || record.slices.iter().any(|(id, _)| id == repository)
                || candidate.parent_id != contract.work().parent
                || candidate.work_generation != contract.work().generation.definition
                || candidate.correction_generation != contract.work().generation.correction
            {
                return Err(Error::new(
                    "candidate_provenance",
                    "candidate differs from complete parent scope",
                ));
            }
            record
                .slices
                .push((repository.clone(), slice.record.payload.intent.scope_id()));
            let selected_contract = self.contracts.get(repository).ok_or_else(|| {
                Error::new(
                    "scope_violation",
                    "candidate repository is no longer configured",
                )
            })?;
            let baseline = by_repository[repository];
            if baseline.contract_digest() != selected_contract.digest()? {
                return Err(Error::new(
                    "stale_result",
                    "baseline delivery contract changed",
                ));
            }
            let current = observer.reobserve(selected_contract, baseline).await?;
            if current.baseline() != baseline.baseline()
                || current.baseline().production != candidate.production
            {
                return Err(Error::new(
                    "stale_result",
                    "production changed before complete vector preparation",
                ));
            }
            let observation =
                self.observation(contract.gate.clone(), candidate_binding(candidate)?)?;
            let actual = self
                .recover_isolated_slice(&observation, slice.record.payload.intent.scope_id())
                .await?;
            if actual.provenance_digest()? != slice.provenance_digest()? {
                return Err(Error::new(
                    "candidate_provenance",
                    "retained candidate provenance changed",
                ));
            }
            for target in current.targets() {
                if target.repository != *repository || !targets.insert(target.target.clone()) {
                    return Err(Error::new(
                        "production_baseline",
                        "duplicate or misbound target baseline",
                    ));
                }
                record.targets.push(target.clone());
            }
            record.baselines.push(current.baseline().clone());
            record.candidates.push(candidate.clone());
        }
        if targets != contract.production_targets {
            return Err(Error::new(
                "production_baseline",
                "complete deployed target inventory differs from authorized parent",
            ));
        }
        record.slices.sort_by(|a, b| a.0.cmp(&b.0));
        record
            .baselines
            .sort_by(|a, b| a.repository.cmp(&b.repository));
        record
            .candidates
            .sort_by(|a, b| a.repository.cmp(&b.repository));
        record.targets.sort_by(|a, b| a.target.cmp(&b.target));
        (self.current_gate)(&contract.gate)?;
        let digest = vector_digest(&record)?;
        let leaf = format!("parent-vector-{}.json", &digest[7..]);
        let signed = self.key.sign("parent_candidate_vector", &record)?;
        // Timestamp differences do not justify replacing an immutable vector.
        let _lock = self.records.section_lock("parent-vectors.lock")?;
        match self.records.read::<Signed<VectorRecord>>(&leaf) {
            Ok(prior) => {
                self.key.verify("parent_candidate_vector", &prior)?;
                if prior.payload.contract_digest != record.contract_digest
                    || prior.payload.work != record.work
                    || vector_digest(&prior.payload)? != digest
                    || prior.payload.slices != record.slices
                {
                    return Err(Error::new(
                        "operation_conflict",
                        "candidate vector identity has another lineage",
                    ));
                }
                Ok(ParentCandidateVector {
                    signed: prior,
                    digest,
                })
            }
            Err(error) if error.code == "record_missing" => {
                self.records.write(&leaf, &signed)?;
                Ok(ParentCandidateVector { signed, digest })
            }
            Err(error) => Err(error),
        }
    }
    pub async fn reobserve_parent_vector(
        &self,
        contract: &ParentContract,
        observer: &ProductionObserver,
        vector: &ParentCandidateVector,
    ) -> Result<()> {
        self.key.verify("parent_candidate_vector", &vector.signed)?;
        let record = &vector.signed.payload;
        if record.schema_version != 1
            || record.runtime_id != self.host.snapshot()?.intent.runtime_id
            || record.work != *contract.work()
            || record.contract_digest != contract.digest()?
            || vector_digest(record)? != vector.digest
            || record.slices.len() != contract.repositories().len()
            || record
                .slices
                .iter()
                .map(|(repository, _)| repository.clone())
                .collect::<BTreeSet<_>>()
                != *contract.repositories()
            || record.baselines.len() != record.slices.len()
            || record.candidates.len() != record.slices.len()
            || record
                .candidates
                .iter()
                .map(|c| c.repository.clone())
                .collect::<BTreeSet<_>>()
                != *contract.repositories()
            || record
                .baselines
                .iter()
                .map(|b| b.repository.clone())
                .collect::<BTreeSet<_>>()
                != *contract.repositories()
            || record
                .targets
                .iter()
                .map(|t| t.target.clone())
                .collect::<BTreeSet<_>>()
                != contract.production_targets
            || record.targets.len() != contract.production_targets.len()
        {
            return Err(Error::new(
                "stale_result",
                "parent vector is not current exact authorized scope",
            ));
        }
        (self.current_gate)(&contract.gate)?;
        for candidate in &record.candidates {
            let repository = &candidate.repository;
            let selected_contract = self.contracts.get(repository).ok_or_else(|| {
                Error::new(
                    "scope_violation",
                    "candidate repository is no longer configured",
                )
            })?;
            let current = observer.observe(selected_contract).await?;
            if !record
                .baselines
                .iter()
                .any(|b| b.repository == *repository && b == current.baseline())
                || current.baseline().production != candidate.production
            {
                return Err(Error::new(
                    "stale_result",
                    "current production differs from retained candidate vector",
                ));
            }
            let retained_targets = record
                .targets
                .iter()
                .filter(|t| &t.repository == repository)
                .map(target_identity)
                .collect::<Result<BTreeSet<_>>>()?;
            let current_targets = current
                .targets()
                .iter()
                .map(target_identity)
                .collect::<Result<BTreeSet<_>>>()?;
            if retained_targets != current_targets {
                return Err(Error::new(
                    "stale_result",
                    "current deployment identities differ from the retained parent vector",
                ));
            }
            let observation =
                self.observation(contract.gate.clone(), candidate_binding(candidate)?)?;
            let current = self
                .recover_isolated_slice(
                    &observation,
                    record
                        .slices
                        .iter()
                        .find(|(id, _)| id == repository)
                        .ok_or_else(|| {
                            Error::new(
                                "candidate_provenance",
                                "candidate operation lineage missing",
                            )
                        })?
                        .1,
                )
                .await?;
            if current.manifest() != candidate {
                return Err(Error::new(
                    "candidate_provenance",
                    "retained vector candidate changed",
                ));
            }
            self.git.verify_execution_tree(
                current.path(),
                candidate.result.as_ref().expect("binding validated"),
                &BTreeSet::new(),
            )?;
        }
        (self.current_gate)(&contract.gate)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RetainedOutput {
    original: ArtifactProof,
    chunks: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IntegratedRecord {
    schema_version: u32,
    runtime_id: Uuid,
    contract_digest: String,
    vector_digest: String,
    e2e_work: WorkIdentity,
    records: Vec<CheckExecutionRecord>,
    outputs: Vec<RetainedOutput>,
    runs: Vec<IntegratedRun>,
}
#[derive(Clone, Debug)]
pub struct IntegratedEvidence {
    signed: Signed<IntegratedRecord>,
}
impl IntegratedEvidence {
    pub fn runs(&self) -> &[IntegratedRun] {
        &self.signed.payload.runs
    }
    pub fn digest(&self) -> Result<String> {
        canonical_digest(1, &self.signed)
    }
}
fn actual_artifact(artifact: &ArtifactProof) -> Result<Vec<u8>> {
    if artifact.bytes == 0 || artifact.bytes > 256 * 1024 * 1024 {
        return Err(Error::new(
            "bounded_input",
            "integrated artifact exceeds bounds",
        ));
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&artifact.path)
        .map_err(|e| Error::io("open integrated output", e))?;
    let meta = file
        .metadata()
        .map_err(|e| Error::io("inspect integrated output", e))?;
    if !meta.is_file() || meta.len() != artifact.bytes {
        return Err(Error::new(
            "artifact_conflict",
            "integrated output is not the observed regular file",
        ));
    }
    let mut bytes = Vec::new();
    file.take(artifact.bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| Error::io("retain integrated output", e))?;
    if bytes.len() as u64 != artifact.bytes || bytes_digest(&bytes) != artifact.digest {
        return Err(Error::new(
            "artifact_conflict",
            "integrated output bytes changed",
        ));
    }
    Ok(bytes)
}
impl DeliveryCoordinator {
    pub async fn verify_integrated_checks(
        &self,
        contract: &ParentContract,
        e2e_gate: &FreshGate,
        vector: &ParentCandidateVector,
        observer: &ProductionObserver,
        executions: &[&VerifiedCheckExecution],
    ) -> Result<IntegratedEvidence> {
        (self.current_gate)(e2e_gate)?;
        if e2e_gate.work().parent != contract.work().parent
            || e2e_gate.work().linear_workspace != contract.work().linear_workspace
            || e2e_gate.work().linear_endpoint != contract.work().linear_endpoint
            || e2e_gate.work().child != Some(contract.e2e_child)
            || e2e_gate.workflow_digest() != contract.gate.workflow_digest()
            || e2e_gate.config_revision() != contract.gate.config_revision()
            || e2e_gate.required_children() != contract.required_children()
        {
            return Err(Error::new(
                "integrated_gate",
                "integrated validation is not the current required E2E child",
            ));
        }
        self.reobserve_parent_vector(contract, observer, vector)
            .await?;
        let mut record = IntegratedRecord {
            schema_version: 1,
            runtime_id: vector.signed.payload.runtime_id,
            contract_digest: contract.digest()?,
            vector_digest: vector.digest.clone(),
            e2e_work: e2e_gate.work().clone(),
            records: vec![],
            outputs: vec![],
            runs: vec![],
        };
        let mut names = BTreeSet::new();
        let mut outputs = BTreeSet::new();
        for execution in executions {
            let check = execution.production_evidence()?;
            let candidate = vector
                .candidates()
                .iter()
                .find(|c| c.repository == check.binding.repository)
                .ok_or_else(|| {
                    Error::new(
                        "integrated_gate",
                        "check repository is outside complete candidate vector",
                    )
                })?;
            if !names.insert(check.check_name.clone())
                || check.binding.runtime_id != record.runtime_id
                || check.binding.work != *e2e_gate.work()
                || check.binding.candidate_vector_digest.as_deref() != Some(vector.digest())
                || check.kind != CheckKind::Integrated
                || !check.successful
                || check.blocker.is_some()
                || check.before != check.after
                || Some(&check.before.revision) != candidate.result.as_ref()
                || Some(&check.binding.revision) != candidate.result.as_ref()
                || check.workflow_digest != contract.gate.workflow_digest()
                || contract.integrated_commands.get(&check.check_name) != Some(&check.command.argv)
                || check.started_at < vector.prepared_at()
                || check.completed_at < check.started_at
                || check.completed_at > Utc::now()
                || check.artifacts.is_empty()
            {
                return Err(Error::new(
                    "integrated_gate",
                    "check is not actual successful exact-vector execution after staging",
                ));
            }
            let mut artifacts = Vec::new();
            for artifact in &check.artifacts {
                if artifact.validation_operation != check.binding.operation_id
                    || artifact.candidate_vector_digest.as_deref() != Some(vector.digest())
                    || !outputs.insert(artifact.path.clone())
                {
                    return Err(Error::new(
                        "artifact_conflict",
                        "integrated output identity is duplicate or differs from its command",
                    ));
                }
                let bytes = actual_artifact(artifact)?;
                let mut chunks = Vec::new();
                for (index, chunk) in bytes.chunks(4 * 1024 * 1024).enumerate() {
                    let leaf = format!("integrated-output-{}-{index}.bin", &artifact.digest[7..]);
                    match self.records.read_bytes(&leaf, 4 * 1024 * 1024) {
                        Ok(prior) if prior == chunk => {}
                        Ok(_) => {
                            return Err(Error::new(
                                "artifact_conflict",
                                "retained artifact digest names other bytes",
                            ));
                        }
                        Err(error) if error.code == "record_missing" => {
                            self.records.write_bytes(&leaf, chunk)?
                        }
                        Err(error) => return Err(error),
                    }
                    chunks.push(leaf);
                }
                artifacts.push(ArtifactReference {
                    reference: format!("retained-integrated:{}", artifact.digest),
                    digest: artifact.digest.clone(),
                });
                record.outputs.push(RetainedOutput {
                    original: artifact.clone(),
                    chunks,
                });
            }
            record.runs.push(IntegratedRun {
                id: check.check_name.clone(),
                operation_id: check.binding.operation_id,
                e2e_child: contract.e2e_child,
                candidate_vector_digest: vector.digest.clone(),
                command: check.command.argv.clone(),
                test_resources: contract.test_resources.clone(),
                started_at: check.started_at,
                completed_at: check.completed_at,
                conclusion: Conclusion::Success,
                after_staging_integration: true,
                artifacts,
            });
            record.records.push(
                serde_json::from_value(
                    serde_json::to_value(check)
                        .map_err(|_| Error::invalid("encode check evidence"))?,
                )
                .map_err(|_| Error::invalid("retain check evidence"))?,
            );
        }
        if names != contract.integrated_commands.keys().cloned().collect() {
            return Err(Error::new(
                "integrated_gate",
                "every configured integrated check must execute exactly once",
            ));
        }
        record
            .records
            .sort_by(|a, b| a.check_name.cmp(&b.check_name));
        record.runs.sort_by(|a, b| a.id.cmp(&b.id));
        self.reobserve_parent_vector(contract, observer, vector)
            .await?;
        (self.current_gate)(e2e_gate)?;
        let signed = self.key.sign("parent_integrated_execution", &record)?;
        self.records.write(
            &format!(
                "parent-integrated-{}.json",
                &canonical_digest(1, &signed)?[7..]
            ),
            &signed,
        )?;
        Ok(IntegratedEvidence { signed })
    }
    fn reverify_integrated_record(
        &self,
        contract: &ParentContract,
        vector: &ParentCandidateVector,
        e2e_work: &WorkIdentity,
        integrated: &IntegratedEvidence,
    ) -> Result<()> {
        self.key
            .verify("parent_integrated_execution", &integrated.signed)?;
        let record = &integrated.signed.payload;
        if record.schema_version != 1
            || record.runtime_id != vector.signed.payload.runtime_id
            || record.contract_digest != contract.digest()?
            || record.vector_digest != vector.digest
            || &record.e2e_work != e2e_work
            || record.records.len() != contract.integrated_commands.len()
            || record.runs.len() != record.records.len()
        {
            return Err(Error::new(
                "integrated_gate",
                "retained execution evidence is stale or incomplete",
            ));
        }
        for output in &record.outputs {
            if output.chunks.is_empty() || output.chunks.len() > 64 {
                return Err(Error::new(
                    "artifact_conflict",
                    "retained output chunks invalid",
                ));
            }
            let mut bytes = Vec::new();
            let mut seen = BTreeSet::new();
            for chunk in &output.chunks {
                if !seen.insert(chunk) {
                    return Err(Error::new(
                        "artifact_conflict",
                        "duplicate retained output chunk",
                    ));
                }
                bytes.extend(self.records.read_bytes(chunk, 4 * 1024 * 1024)?);
            }
            if bytes.len() as u64 != output.original.bytes
                || bytes_digest(&bytes) != output.original.digest
            {
                return Err(Error::new(
                    "artifact_conflict",
                    "retained integrated artifact bytes changed",
                ));
            }
        }
        // HMAC binds the original private execution capability and all derived
        // result rows; bytes are independently reread on every recovery.
        Ok(())
    }
}
impl DeliveryCoordinator {
    pub async fn hydrate_parent_vector(
        &self,
        contract: &ParentContract,
        observer: &ProductionObserver,
        digest: &str,
    ) -> Result<ParentCandidateVector> {
        evidence::validate_digest(digest)?;
        let signed: Signed<VectorRecord> = self
            .records
            .read(&format!("parent-vector-{}.json", &digest[7..]))?;
        let vector = ParentCandidateVector {
            signed,
            digest: digest.into(),
        };
        self.reobserve_parent_vector(contract, observer, &vector)
            .await?;
        Ok(vector)
    }
    pub async fn reobserve_integrated_checks(
        &self,
        contract: &ParentContract,
        e2e_gate: &FreshGate,
        vector: &ParentCandidateVector,
        observer: &ProductionObserver,
        integrated: &IntegratedEvidence,
    ) -> Result<()> {
        (self.current_gate)(e2e_gate)?;
        if e2e_gate.work().child != Some(contract.e2e_child) {
            return Err(Error::new("integrated_gate", "required E2E child differs"));
        }
        self.reverify_integrated_record(contract, vector, e2e_gate.work(), integrated)?;
        self.reobserve_parent_vector(contract, observer, vector)
            .await?;
        (self.current_gate)(e2e_gate)
    }
    pub async fn hydrate_integrated_checks(
        &self,
        contract: &ParentContract,
        e2e_gate: &FreshGate,
        vector: &ParentCandidateVector,
        observer: &ProductionObserver,
        digest: &str,
    ) -> Result<IntegratedEvidence> {
        evidence::validate_digest(digest)?;
        let signed: Signed<IntegratedRecord> = self
            .records
            .read(&format!("parent-integrated-{}.json", &digest[7..]))?;
        let integrated = IntegratedEvidence { signed };
        if integrated.digest()? != digest {
            return Err(Error::new(
                "integrated_gate",
                "retained integrated identity changed",
            ));
        }
        self.reobserve_integrated_checks(contract, e2e_gate, vector, observer, &integrated)
            .await?;
        Ok(integrated)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParentResultClaim {
    pub schema_version: u32,
    pub executive_summary: String,
    pub touched_components: Vec<String>,
    pub fulfilled_criteria: BTreeSet<String>,
    pub caveats: Vec<String>,
    pub go: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ParentChildRecord {
    child: Uuid,
    publication: Uuid,
    digest: String,
    revision: RevisionBinding,
    criteria: BTreeSet<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ParentResultRecord {
    schema_version: u32,
    runtime_id: Uuid,
    contract_digest: String,
    vector_digest: String,
    integrated_digest: String,
    child_records: Vec<ParentChildRecord>,
    coordinator_claim_digest: String,
    evidence: evidence::ParentEvidence,
}
impl DeliveryCoordinator {
    /// Parse the exact producer schema for native publication selection. This
    /// projection authenticates bytes and identity, not execution or readiness.
    pub(crate) fn decode_parent_publication(
        body: &str,
        key: &AttestationKey,
    ) -> Result<(Uuid, evidence::ParentEvidence)> {
        let signed: Signed<ParentResultRecord> =
            evidence::decode_signed_publication(body, "parent_result", key)?;
        let record = signed.payload;
        if record.schema_version != 1
            || record.runtime_id.is_nil()
            || record.evidence.schema_version != 1
        {
            return Err(Error::new(
                "invalid_attestation",
                "parent publication schema/runtime is invalid",
            ));
        }
        record.evidence.work.validate()?;
        Ok((record.runtime_id, record.evidence))
    }
}

pub struct ParentReviewAssignment {
    contract_digest: String,
    vector: ParentCandidateVector,
    integrated: IntegratedEvidence,
    children: Vec<ParentChildRecord>,
    contributions: BTreeMap<Uuid, BTreeSet<String>>,
    integrations: Vec<crate::git::VerifiedProductionIntegration>,
    context: Value,
}
impl ParentReviewAssignment {
    pub(super) fn retirement_vector(&self) -> &ParentCandidateVector {
        &self.vector
    }
    pub fn vector_digest(&self) -> &str {
        self.vector.digest()
    }
    pub(super) fn matches_worker(&self, contract: &ParentContract) -> Result<()> {
        if self.contract_digest != contract.digest()?
            || self.vector.signed.payload.work != *contract.work()
        {
            return Err(Error::new(
                "stale_result",
                "parent review assignment differs from current scope",
            ));
        }
        Ok(())
    }
    pub fn context(&self) -> &Value {
        &self.context
    }
}
pub struct ParentEvidenceInputs<'a> {
    pub contract: &'a ParentContract,
    pub observer: &'a ProductionObserver,
    pub vector: ParentCandidateVector,
    pub integrated: IntegratedEvidence,
    pub e2e_gate: &'a FreshGate,
    pub child_gates: &'a BTreeMap<Uuid, FreshGate>,
    pub children: &'a [PublishedChild],
    pub workflow: &'a Workflow,
    pub production_integrations: &'a [crate::git::VerifiedProductionIntegration],
}
#[derive(Clone, Debug)]
pub struct PublishedParent {
    verified: evidence::VerifiedParent,
    signed: Signed<ParentResultRecord>,
    publication: Publication,
}
impl PublishedParent {
    pub fn publication_digest(&self) -> Result<String> {
        self.signed.digest()
    }
    pub fn verified(&self) -> &evidence::VerifiedParent {
        &self.verified
    }
    pub fn publication(&self) -> &Publication {
        &self.publication
    }
}
impl DeliveryCoordinator {
    async fn current_parent_children(
        &self,
        contract: &ParentContract,
        workflow: &Workflow,
        gates: &BTreeMap<Uuid, FreshGate>,
        records: &[ParentChildRecord],
    ) -> Result<Vec<PublishedChild>> {
        if records.len() != contract.required_children().len()
            || records.iter().map(|r| r.child).collect::<BTreeSet<_>>()
                != *contract.required_children()
            || gates.keys().copied().collect::<BTreeSet<_>>() != *contract.required_children()
        {
            return Err(Error::new(
                "parent_incomplete",
                "parent observation requires every current child gate and publication",
            ));
        }
        let mut children = Vec::new();
        for record in records {
            let gate = gates
                .get(&record.child)
                .ok_or_else(|| Error::new("parent_incomplete", "child gate missing"))?;
            if gate.work().parent != contract.work().parent
                || gate.work().child != Some(record.child)
                || gate.work().linear_workspace != contract.work().linear_workspace
                || gate.work().linear_endpoint != contract.work().linear_endpoint
                || gate.config_revision() != contract.gate.config_revision()
                || gate.workflow_digest() != contract.gate.workflow_digest()
            {
                return Err(Error::new(
                    "stale_result",
                    "required child gate differs from current parent authority",
                ));
            }
            let observation = self.observation(gate.clone(), record.revision.clone())?;
            let child = self
                .hydrate_child_result(&observation, workflow, &record.criteria, record.publication)
                .await?;
            if child.verified().digest() != record.digest {
                return Err(Error::new("stale_result", "required child result changed"));
            }
            children.push(child);
        }
        (self.current_gate)(&contract.gate)?;
        Ok(children)
    }
    fn parent_contributions(
        &self,
        contract: &ParentContract,
        children: &[PublishedChild],
        integrations: &[crate::git::VerifiedProductionIntegration],
    ) -> Result<BTreeMap<Uuid, BTreeSet<String>>> {
        let mut required: BTreeMap<Uuid, BTreeSet<String>> = BTreeMap::new();
        for child in children {
            if let evidence::ChildOutcome::Changed { delta, .. }
            | evidence::ChildOutcome::Replacement { delta, .. } =
                &child.verified.evidence().outcome
            {
                required
                    .entry(delta.child_id)
                    .or_default()
                    .extend(delta.original_contributions()?);
            }
        }
        for integration in integrations {
            let record = integration.record();
            if record.parent_id != contract.work().parent
                || !contract.repositories().contains(&record.repository)
            {
                return Err(Error::new(
                    "production_provenance",
                    "prior contribution is outside this authorized parent",
                ));
            }
            for digest in record.selected_contributions.values().flatten() {
                let retained: Signed<Value> =
                    self.records.read(&format!("delta-{}.json", &digest[7..]))?;
                self.key.verify("staged_delta", &retained)?;
                let manifest: crate::git::DeltaManifest =
                    serde_json::from_value(retained.payload["manifest"].clone()).map_err(|_| {
                        Error::new("record_corrupt", "original contribution manifest missing")
                    })?;
                if manifest.digest() != Ok(digest.clone())
                    || manifest.parent_id != record.parent_id
                    || manifest.repository != record.repository
                    || !contract.required_children().contains(&manifest.child_id)
                {
                    return Err(Error::new(
                        "production_provenance",
                        "prior contribution lacks original required-child provenance",
                    ));
                }
                self.retained_delta(&manifest)?;
                required
                    .entry(manifest.child_id)
                    .or_default()
                    .insert(digest.clone());
            }
        }
        Ok(required)
    }
    pub async fn prepare_parent_review(
        &self,
        input: ParentEvidenceInputs<'_>,
        base_context: Value,
    ) -> Result<ParentReviewAssignment> {
        let contract = input.contract;
        self.reobserve_integrated_checks(
            contract,
            input.e2e_gate,
            &input.vector,
            input.observer,
            &input.integrated,
        )
        .await?;
        let mut records = Vec::new();
        for child in input.children {
            let e = child.verified.evidence();
            records.push(ParentChildRecord {
                child: e.work.child.ok_or_else(|| {
                    Error::new("parent_incomplete", "parent cannot substitute for child")
                })?,
                publication: Uuid::parse_str(&child.publication.record_id).map_err(|_| {
                    Error::new("evidence_unpublished", "child publication identity invalid")
                })?,
                digest: child.verified.digest().into(),
                revision: e.revision.clone(),
                criteria: e.fulfilled_criteria.clone(),
            });
        }
        records.sort_by_key(|r| r.child);
        let children = self
            .current_parent_children(contract, input.workflow, input.child_gates, &records)
            .await?;
        let contributions =
            self.parent_contributions(contract, &children, input.production_integrations)?;
        let context = json!({"task":base_context,"parent_delivery":{"schema_version":1,"work":contract.work(),
            "required_children":contract.required_children(),"e2e_child":contract.e2e_child,
            "vector_digest":input.vector.digest(),"baselines":input.vector.baselines(),"candidates":input.vector.candidates(),
            "child_results":children.iter().map(|c|c.verified.evidence()).collect::<Vec<_>>(),
            "required_contributions":contributions,"integrated_results":input.integrated.runs(),
            "integrated_digest":input.integrated.digest()?,"test_resources":contract.test_resources,
            "production_targets":contract.production_targets,"visual":contract.visual,
            "required_result_schema":{"schema_version":1,"executive_summary":"outcome and limits","touched_components":[],
                "fulfilled_criteria":[],"caveats":[],"go":true}}});
        if serde_json::to_vec(&context)
            .map_err(|_| Error::invalid("parent review context encoding"))?
            .len()
            > 8 * 1024 * 1024
        {
            return Err(Error::new(
                "bounded_input",
                "parent review context exceeds finite limit",
            ));
        }
        Ok(ParentReviewAssignment {
            contract_digest: contract.digest()?,
            vector: input.vector,
            integrated: input.integrated,
            children: records,
            contributions,
            integrations: input.production_integrations.to_vec(),
            context,
        })
    }
}

/// Issued only after actual upload, association and authenticated playback checks.
/// Raw VideoEvidence cannot enter parent publication through this adapter.
#[derive(Clone, Debug)]
pub struct VerifiedVideoPublication {
    evidence: evidence::VideoEvidence,
}
pub struct ParentPublicationInputs<'a> {
    pub contract: &'a ParentContract,
    pub observer: &'a ProductionObserver,
    pub assignment: &'a ParentReviewAssignment,
    pub child_gates: &'a BTreeMap<Uuid, FreshGate>,
    pub workflow: &'a Workflow,
    pub outcome: &'a crate::runtime::VerifiedAuthorizedOutcome,
    pub completion: &'a Arc<crate::worker_run::RunCompletion>,
    pub video: Option<&'a VerifiedVideoPublication>,
}
pub struct ParentHydrationInputs<'a> {
    pub contract: &'a ParentContract,
    pub observer: &'a ProductionObserver,
    pub child_gates: &'a BTreeMap<Uuid, FreshGate>,
    pub workflow: &'a Workflow,
    pub production_integrations: &'a [crate::git::VerifiedProductionIntegration],
}
impl DeliveryCoordinator {
    fn verify_parent_record(
        &self,
        contract: &ParentContract,
        record: &Signed<ParentResultRecord>,
        children: &[PublishedChild],
        integrations: &[crate::git::VerifiedProductionIntegration],
        publication: &Publication,
    ) -> Result<evidence::VerifiedParent> {
        self.key.verify("parent_result", record)?;
        let r = &record.payload;
        if r.schema_version != 1
            || r.runtime_id != self.host.snapshot()?.intent.runtime_id
            || r.contract_digest != contract.digest()?
            || r.evidence.work != *contract.work()
            || r.vector_digest != r.evidence.candidate_vector_digest()?
            || r.evidence.dependency_snapshot_digest != contract.gate.dependency_digest()
        {
            return Err(Error::new(
                "stale_result",
                "parent publication differs from exact current scope/dependencies",
            ));
        }
        publication.verify(record, "linear", &publication.writer_account_id)?;
        let contributions = self.parent_contributions(contract, children, integrations)?;
        let mut inner = publication.clone();
        inner.payload_digest = canonical_digest(1, &r.evidence)?;
        evidence::verify_parent(
            &r.evidence,
            evidence::ParentRequirements {
                work: contract.work(),
                required_children: contract.required_children(),
                e2e_child: contract.e2e_child,
                criteria: &r.evidence.fulfilled_criteria,
                integrated_commands: &contract.integrated_commands,
                test_resources: &contract.test_resources,
                production_targets: &contract.production_targets,
                visual: contract.visual,
                required_contributions: &contributions,
                production_integrations: integrations,
            },
            &children
                .iter()
                .map(|c| c.verified.clone())
                .collect::<Vec<_>>(),
            &inner,
            &publication.writer_account_id,
        )
    }
    pub async fn publish_parent_result(
        &self,
        admission: &DeliveryAdmission,
        input: ParentPublicationInputs<'_>,
    ) -> Result<PublishedParent> {
        self.publish_parent_result_inner(admission, input, None)
            .await
    }
    pub(crate) async fn publish_reassembled_parent_result(
        &self,
        admission: &DeliveryAdmission,
        input: ParentPublicationInputs<'_>,
        source: &ParentReassemblySource,
        configuration: &Config,
    ) -> Result<PublishedParent> {
        source.refresh_current(configuration).await?;
        if source.vector().digest() != input.assignment.vector.digest()
            || source.original().parent().manifest().work != *input.contract.work()
        {
            return Err(Error::new(
                "acceptance_carry",
                "reassembled publication differs from original exact parent source",
            ));
        }
        self.publish_parent_result_inner(
            admission,
            input,
            Some(source.previous_parent().verified().token().to_owned()),
        )
        .await
    }
    async fn publish_parent_result_inner(
        &self,
        admission: &DeliveryAdmission,
        input: ParentPublicationInputs<'_>,
        supersedes: Option<String>,
    ) -> Result<PublishedParent> {
        let contract = input.contract;
        let assignment = input.assignment;
        self.current(admission, &[DeliveryPhase::ParentEvidence])?;
        super::child::completed_run(input.outcome, input.completion)?;
        let worker = input.outcome.outcome();
        if admission.scope.work != *contract.work()
            || assignment.contract_digest != contract.digest()?
            || admission.scope.candidate_vector_digest.as_deref()
                != Some(assignment.vector.digest())
            || input.outcome.original_authority().work() != contract.work()
            || input.outcome.original_authority().config_revision()
                != contract.gate.config_revision()
            || worker.instructions.role != crate::instructions::AgentRole::Coordination
            || worker.instructions.workflow_digest != contract.gate.workflow_digest()
            || worker.instructions.permissions.repository_code_execution
            || !worker
                .instructions
                .permissions
                .source_write_paths
                .is_empty()
            || !worker
                .instructions
                .permissions
                .resource_write_paths
                .is_empty()
        {
            return Err(Error::new(
                "parent_incomplete",
                "parent synthesis did not review the current complete verified result context",
            ));
        }
        let actual = input.completion.contract.as_ref().ok_or_else(|| {
            Error::new(
                "worker_binding",
                "parent review lacks actual assembled contract",
            )
        })?;
        if actual.instructions().manifest_digest != worker.instruction_digest
            || actual.tools().digest() != worker.tool_contract_digest
        {
            return Err(Error::new(
                "worker_binding",
                "parent review differs from protected contract",
            ));
        }
        assignment::verify_assignment_contract(
            actual,
            &contract.gate,
            "parent_evidence_review",
            &assignment.context,
        )?;
        let e2e = input
            .child_gates
            .get(&contract.e2e_child)
            .ok_or_else(|| Error::new("integrated_gate", "current E2E child gate missing"))?;
        self.reobserve_integrated_checks(
            contract,
            e2e,
            &assignment.vector,
            input.observer,
            &assignment.integrated,
        )
        .await?;
        let children = self
            .current_parent_children(
                contract,
                input.workflow,
                input.child_gates,
                &assignment.children,
            )
            .await?;
        if self.parent_contributions(contract, &children, &assignment.integrations)?
            != assignment.contributions
        {
            return Err(Error::new(
                "candidate_provenance",
                "required contribution inventory changed",
            ));
        }
        let claim: ParentResultClaim = serde_json::from_value(worker.claim.clone())
            .map_err(|_| Error::new("worker_claim", "parent result has no supported schema"))?;
        if claim.schema_version != 1
            || !claim.go
            || claim.executive_summary.trim().is_empty()
            || claim.executive_summary.len() > 65536
            || claim.touched_components.is_empty()
            || claim.fulfilled_criteria.is_empty()
            || claim.fulfilled_criteria.len() > 256
            || claim.touched_components.len() > 256
            || claim.caveats.len() > 256
            || claim
                .touched_components
                .iter()
                .chain(&claim.caveats)
                .chain(&claim.fulfilled_criteria)
                .any(|s| s.trim().is_empty() || s.len() > 8192)
        {
            return Err(Error::new(
                "parent_incomplete",
                "parent result lacks complete bounded product outcome and go/no-go reasoning",
            ));
        }
        if contract.visual != input.video.is_some() {
            return Err(Error::new(
                "video_gate",
                "visual parent requires actual candidate video upload, association and playback proof",
            ));
        }
        let evidence = evidence::ParentEvidence {
            schema_version: 1,
            work: contract.work().clone(),
            required_children: contract.required_children().clone(),
            e2e_child: contract.e2e_child,
            child_results: children
                .iter()
                .map(|c| {
                    (
                        c.verified.evidence().work.child.expect("validated child"),
                        c.verified.digest().into(),
                    )
                })
                .collect(),
            required_contributions: assignment.contributions.clone(),
            executive_summary: claim.executive_summary,
            touched_components: claim.touched_components,
            fulfilled_criteria: claim.fulfilled_criteria,
            caveats: claim.caveats,
            go: claim.go,
            baselines: assignment.vector.baselines().to_vec(),
            candidates: assignment.vector.candidates().to_vec(),
            dependency_snapshot_digest: contract.gate.dependency_digest().into(),
            production_targets: contract.production_targets.clone(),
            integrated_results: assignment.integrated.runs().to_vec(),
            visual: contract.visual,
            video: input.video.map(|v| v.evidence.clone()),
            publication_operation: admission.operation_id(),
            created_at: self.operation_time(admission, "parent-result")?,
            supersedes,
        };
        let signed = self.key.sign(
            "parent_result",
            &ParentResultRecord {
                schema_version: 1,
                runtime_id: admission.scope.runtime_id,
                contract_digest: contract.digest()?,
                vector_digest: assignment.vector.digest().into(),
                integrated_digest: assignment.integrated.digest()?,
                child_records: assignment.children.clone(),
                coordinator_claim_digest: input.outcome.digest().into(),
                evidence,
            },
        )?;
        let receipt = self
            .effect(
                admission,
                "publish-parent-result",
                ProviderMutation::LinearComment {
                    issue: contract.work().parent,
                    body: record_body("parent_result", &signed)?,
                },
            )
            .await?;
        let publication = self
            .linear_publication(
                admission,
                contract.work().parent,
                &remote_id(&receipt, "comment")?,
                "parent_result",
                &signed,
            )
            .await?;
        self.current(admission, &[DeliveryPhase::ParentEvidence])?;
        self.reobserve_integrated_checks(
            contract,
            e2e,
            &assignment.vector,
            input.observer,
            &assignment.integrated,
        )
        .await?;
        let children = self
            .current_parent_children(
                contract,
                input.workflow,
                input.child_gates,
                &assignment.children,
            )
            .await?;
        let verified = self.verify_parent_record(
            contract,
            &signed,
            &children,
            &assignment.integrations,
            &publication,
        )?;
        self.records.write(
            &format!("parent-result-{}.json", &verified.token()[7..]),
            &signed,
        )?;
        Ok(PublishedParent {
            verified,
            signed,
            publication,
        })
    }
    /// Read-only recovery revalidates current children, integrated bytes, exact
    /// candidate Git objects and current production before returning a proof.
    pub async fn hydrate_parent_result(
        &self,
        input: ParentHydrationInputs<'_>,
        record_id: Uuid,
    ) -> Result<PublishedParent> {
        let contract = input.contract;
        (self.current_gate)(&contract.gate)?;
        let linear = self
            .broker
            .linear
            .as_ref()
            .ok_or_else(|| Error::new("prerequisite", "Linear connection unavailable"))?;
        let comment = linear
            .comments(contract.work().parent)
            .await?
            .into_iter()
            .find(|c| c.id == record_id)
            .ok_or_else(|| {
                Error::new(
                    "evidence_unpublished",
                    "parent result publication disappeared",
                )
            })?;
        let signed: Signed<ParentResultRecord> =
            evidence::decode_signed_publication(&comment.body, "parent_result", &self.key)?;
        let vector = self
            .hydrate_parent_vector(contract, input.observer, &signed.payload.vector_digest)
            .await?;
        let e2e = input
            .child_gates
            .get(&contract.e2e_child)
            .ok_or_else(|| Error::new("integrated_gate", "current E2E child gate missing"))?;
        let integrated = self
            .hydrate_integrated_checks(
                contract,
                e2e,
                &vector,
                input.observer,
                &signed.payload.integrated_digest,
            )
            .await?;
        if signed.payload.evidence.baselines != vector.baselines()
            || signed.payload.evidence.candidates != vector.candidates()
            || signed.payload.evidence.integrated_results != integrated.runs()
        {
            return Err(Error::new(
                "stale_result",
                "published parent differs from retained complete vector and actual integrated execution",
            ));
        }
        // Visual publications additionally require the retained asset observation
        // path before recovery can promote them; raw payload booleans are ignored.
        if signed.payload.evidence.visual {
            return Err(Error::new(
                "video_gate",
                "current authenticated video retrieval observation is required",
            ));
        }
        let children = self
            .current_parent_children(
                contract,
                input.workflow,
                input.child_gates,
                &signed.payload.child_records,
            )
            .await?;
        let candidate = vector
            .candidates()
            .first()
            .ok_or_else(|| Error::new("candidate_provenance", "parent vector is empty"))?;
        let observation = self.observation(contract.gate.clone(), candidate_binding(candidate)?)?;
        let publication = self
            .linear_publication(
                &observation,
                contract.work().parent,
                &record_id.to_string(),
                "parent_result",
                &signed,
            )
            .await?;
        let verified = self.verify_parent_record(
            contract,
            &signed,
            &children,
            input.production_integrations,
            &publication,
        )?;
        Ok(PublishedParent {
            verified,
            signed,
            publication,
        })
    }
}

#[derive(Clone, Debug)]
pub struct VerifiedDeliverySnapshot {
    authority: FreshGate,
    work: WorkIdentity,
    delivery: crate::reconcile::ParentDelivery,
    outcomes: crate::reconcile::DependencyOutcomes,
    observed_at: DateTime<Utc>,
    blockers: Vec<String>,
    no_release_link: Option<String>,
}
impl VerifiedDeliverySnapshot {
    #[cfg(test)]
    pub(crate) fn fixture(
        authority: FreshGate,
        parent: evidence::VerifiedParent,
        acceptance: evidence::VerifiedAcceptance,
        children: Vec<evidence::VerifiedChild>,
    ) -> Result<Self> {
        let work = authority.work().clone();
        if parent.manifest().work != work
            || parent.manifest().required_children
                != children
                    .iter()
                    .filter_map(|child| child.evidence().work.child)
                    .collect()
        {
            return Err(Error::new(
                "stale_result",
                "fixture snapshot evidence does not match actual gate",
            ));
        }
        let mut delivery = crate::reconcile::ParentDelivery::default();
        let mut outcomes = crate::reconcile::DependencyOutcomes::default();
        for child in children {
            outcomes.child(&child)?;
            delivery.child(child)?;
        }
        delivery.parent(parent);
        delivery.acceptance(acceptance)?;
        Ok(Self {
            authority,
            work,
            delivery,
            outcomes,
            observed_at: Utc::now(),
            blockers: vec![],
            no_release_link: None,
        })
    }
    pub(crate) fn no_release_required(
        &mut self,
        proof: evidence::VerifiedNoReleaseRequired,
        publication_url: String,
    ) -> Result<()> {
        if proof.record().work != self.work || self.authority.work() != &self.work {
            return Err(Error::new(
                "stale_result",
                "already-production outcome names another snapshot",
            ));
        }
        self.delivery.record_no_release_required(proof.clone())?;
        self.outcomes.no_release_required(&proof)?;
        self.no_release_link = Some(publication_url);
        Ok(())
    }
    pub(crate) fn no_release_link(&self) -> Option<&str> {
        self.no_release_link.as_deref()
    }
    pub fn release(
        &mut self,
        eligibility: crate::release::VerifiedReleaseEligibility,
    ) -> Result<()> {
        self.delivery.release(eligibility)
    }
    pub(crate) fn carried_acceptance(&mut self, carry: PublishedCarry) -> Result<()> {
        self.delivery.carried_acceptance(carry)?;
        self.no_release_link = None;
        Ok(())
    }
    pub fn acceptance(&mut self, acceptance: evidence::VerifiedAcceptance) -> Result<()> {
        self.delivery.acceptance(acceptance)?;
        let mut outcomes = crate::reconcile::DependencyOutcomes::default();
        for child in self.delivery.verified_children().values() {
            outcomes.child(child)?;
        }
        self.outcomes = outcomes;
        self.no_release_link = None;
        Ok(())
    }
    pub fn authority(&self) -> &FreshGate {
        &self.authority
    }
    pub fn work(&self) -> &WorkIdentity {
        &self.work
    }
    pub fn parent_delivery(&self) -> &crate::reconcile::ParentDelivery {
        &self.delivery
    }
    pub fn outcomes(&self) -> &crate::reconcile::DependencyOutcomes {
        &self.outcomes
    }
    pub fn observed_at(&self) -> DateTime<Utc> {
        self.observed_at
    }
    pub fn blockers(&self) -> &[String] {
        &self.blockers
    }
}
impl DeliveryCoordinator {
    /// Discover exact authenticated publications and reobserve every passed gate.
    /// A fresh empty result replaces stale cached readiness; tracker state never
    /// contributes a passed child, parent, acceptance or release gate.
    pub async fn refresh_delivery(
        &self,
        input: ParentHydrationInputs<'_>,
        configuration: &crate::config::Config,
    ) -> Result<VerifiedDeliverySnapshot> {
        let contract = input.contract;
        (self.current_gate)(&contract.gate)?;
        if configuration.revision()? != contract.gate.config_revision() {
            return Err(Error::new(
                "stale_config",
                "delivery inspection needs current configuration",
            ));
        }
        let workspace = configuration
            .workspaces
            .values()
            .find(|w| w.id == contract.gate.workspace_id())
            .ok_or_else(|| Error::new("scope_violation", "delivery workspace is not configured"))?;
        let linear = self
            .broker
            .linear
            .as_ref()
            .ok_or_else(|| Error::new("prerequisite", "Linear connection unavailable"))?;
        let mut snapshot = VerifiedDeliverySnapshot {
            authority: contract.gate.clone(),
            work: contract.work().clone(),
            delivery: Default::default(),
            outcomes: Default::default(),
            observed_at: Utc::now(),
            blockers: vec![],
            no_release_link: None,
        };
        let mut verified_children = BTreeMap::new();
        for child in contract.required_children() {
            let Some(gate) = input.child_gates.get(child) else {
                snapshot
                    .blockers
                    .push(format!("child {child}: current authority unavailable"));
                continue;
            };
            let mut valid = BTreeMap::new();
            for comment in linear.comments(*child).await? {
                if !comment
                    .body
                    .starts_with("<!-- kegari:v0:child_result -->\n")
                {
                    continue;
                }
                let signed = match evidence::decode_signed_publication::<ChildResultRecord>(
                    &comment.body,
                    "child_result",
                    &self.key,
                ) {
                    Ok(record) => record,
                    Err(_) => continue,
                };
                if signed.payload.evidence.work != *gate.work() {
                    continue;
                }
                let observation =
                    self.observation(gate.clone(), signed.payload.evidence.revision.clone())?;
                match self
                    .hydrate_child_result(
                        &observation,
                        input.workflow,
                        &signed.payload.evidence.fulfilled_criteria,
                        comment.id,
                    )
                    .await
                {
                    Ok(child) => {
                        valid.insert(child.verified.digest().to_owned(), child);
                    }
                    Err(error) => snapshot.blockers.push(format!(
                        "child {} publication {}: {}",
                        child, comment.id, error.code
                    )),
                }
            }
            if valid.len() == 1 {
                let child = valid.into_values().next().expect("one verified result");
                let id = child
                    .verified
                    .evidence()
                    .work
                    .child
                    .ok_or_else(|| Error::new("child_incomplete", "child identity missing"))?;
                verified_children.insert(id, child.verified.clone());
                // A source-only E2E result cannot establish a fulfilled
                // candidate-validation dependency on recovery. Until the
                // canonical parent binds its exact vector/check proof, retain
                // this result only as an input to full parent verification.
                if id != contract.e2e_child() {
                    snapshot.outcomes.child(&child.verified)?;
                    snapshot.delivery.child(child.verified)?;
                }
            } else {
                snapshot.blockers.push(format!(
                    "child {child}: {} current authenticated results",
                    valid.len()
                ));
            }
        }
        let comments = linear.comments(contract.work().parent).await?;
        let mut parents = BTreeMap::new();
        for comment in &comments {
            if !comment
                .body
                .starts_with("<!-- kegari:v0:parent_result -->\n")
            {
                continue;
            }
            let signed = match evidence::decode_signed_publication::<ParentResultRecord>(
                &comment.body,
                "parent_result",
                &self.key,
            ) {
                Ok(record) => record,
                Err(_) => continue,
            };
            if signed.payload.evidence.work != *contract.work() {
                continue;
            }
            match self
                .hydrate_parent_result(
                    ParentHydrationInputs {
                        contract,
                        observer: input.observer,
                        child_gates: input.child_gates,
                        workflow: input.workflow,
                        production_integrations: input.production_integrations,
                    },
                    comment.id,
                )
                .await
            {
                Ok(parent) => {
                    parents.insert(parent.verified.token().to_owned(), parent);
                }
                Err(error) => snapshot
                    .blockers
                    .push(format!("parent publication {}: {}", comment.id, error.code)),
            }
        }
        if parents.len() == 1 {
            let parent = parents.into_values().next().expect("one verified result");
            // Every complete parent child must also have an unambiguous current
            // independently hydrated result in this same refresh.
            if verified_children.len() == contract.required_children().len()
                && parent
                    .verified
                    .manifest()
                    .child_results
                    .iter()
                    .all(|(id, digest)| {
                        verified_children
                            .get(id)
                            .is_some_and(|c| c.digest() == digest)
                    })
            {
                let e2e = verified_children
                    .remove(&contract.e2e_child())
                    .ok_or_else(|| {
                        Error::new("integrated_gate", "verified parent omits E2E child")
                    })?;
                snapshot.outcomes.child(&e2e)?;
                snapshot.delivery.child(e2e)?;
                snapshot.delivery.parent(parent.verified.clone());
                let permitted = workspace
                    .actors
                    .acceptors
                    .iter()
                    .map(|id| id.to_string())
                    .collect::<BTreeSet<_>>();
                let mut acceptances = Vec::new();
                let writer = linear.identity().await?.account_id;
                for comment in comments {
                    if comment.bot
                        || comment.delegated
                        || comment.issue_id != contract.work().parent
                        || comment.user_id.map(|id| id.to_string()).as_deref() != Some(&writer)
                        || !comment
                            .body
                            .starts_with("<!-- kegari:v0:human_acceptance -->\n")
                    {
                        continue;
                    }
                    let signed = match evidence::decode_signed_publication::<evidence::Acceptance>(
                        &comment.body,
                        "human_acceptance",
                        &self.key,
                    ) {
                        Ok(record) => record,
                        Err(_) => continue,
                    };
                    if signed.payload.runtime_id != self.host.snapshot()?.intent.runtime_id {
                        continue;
                    }
                    let publication = Publication {
                        provider: "linear".into(),
                        record_id: comment.id.to_string(),
                        writer_account_id: writer.clone(),
                        payload_digest: canonical_digest(1, &signed)?,
                        observed_at: Utc::now(),
                        url: comment.url,
                    };
                    if let Ok(acceptance) = evidence::verify_acceptance(
                        &signed,
                        &publication,
                        &parent.verified,
                        contract.work(),
                        false,
                        &permitted,
                        &self.key,
                    ) {
                        acceptances.push(acceptance);
                    }
                }
                if acceptances.len() == 1 {
                    snapshot.delivery.acceptance(
                        acceptances
                            .into_iter()
                            .next()
                            .expect("one verified acceptance"),
                    )?;
                } else if acceptances.len() > 1 {
                    snapshot
                        .blockers
                        .push("parent has ambiguous current acceptance publications".into());
                }
            } else {
                snapshot
                    .blockers
                    .push("parent child inventory differs from current unambiguous results".into());
            }
        } else {
            snapshot.blockers.push(format!(
                "parent: {} current authenticated results",
                parents.len()
            ));
        }
        (self.current_gate)(&contract.gate)?;
        snapshot.observed_at = Utc::now();
        Ok(snapshot)
    }
}

impl DeliveryCoordinator {
    /// Mint only the read-root capability needed to inspect retained Git bytes.
    /// This authenticates ownership/receipt/marker first and does not itself
    /// assert that the underlying Git objects or delivery gates are valid.
    pub fn retained_candidate_host_root(
        &self,
        observation: &DeliveryObservation,
        operation: Uuid,
    ) -> Result<crate::git::VerifiedHostReadRoot> {
        self.current(
            observation,
            &[
                DeliveryPhase::Recovery,
                DeliveryPhase::ParentEvidence,
                DeliveryPhase::CandidateAssembly,
                DeliveryPhase::Release,
            ],
        )?;
        let record: Signed<SliceRecord> = self
            .records
            .read(&format!("slice-result-{operation}.json"))?;
        self.key.verify("isolated_slice", &record)?;
        let r = &record.payload;
        if r.intent.schema_version != 1
            || r.intent.operation_id != operation
            || r.intent.scope.runtime_id != observation.scope.runtime_id
            || r.intent.scope.work != observation.scope.work
            || r.intent.scope.repository != observation.scope.repository
            || r.intent.scope.contract_digest != observation.scope.contract_digest
            || r.candidate.operation_id != operation
            || r.candidate.repository != observation.scope.repository
        {
            return Err(Error::new(
                "scope_violation",
                "retained read root differs from exact current authorized candidate",
            ));
        }
        let receipt = self.host.operation(r.receipt_id)?;
        let expected = canonical_digest(
            1,
            &json!({"kind":"candidate_assembly","targets":[r.intent.scope.repository.id],
            "expected":{"delivery_scope":r.intent.scope.digest()?,"admitted_scope":r.intent.scope.digest()?,
                "admission_operation":r.intent.operation_id.to_string(),"admitted_work":crate::authority::IssueKey::new(
                    &r.intent.scope.work.linear_endpoint,r.intent.scope.work.linear_workspace,r.intent.scope.work.parent)?.identity()?},
            "payload":r.intent,"principal":null}),
        )?;
        if !matches!(
            receipt.phase,
            crate::control::ReceiptPhase::Acknowledged | crate::control::ReceiptPhase::Reconciled
        ) || receipt.runtime_id != observation.scope.runtime_id
            || receipt.kind != "candidate_assembly"
            || receipt.payload_digest != expected
        {
            return Err(Error::new(
                "operation_uncertain",
                "candidate read root lacks exact acknowledged preparation",
            ));
        }
        crate::git::retained_candidate_read_root(
            observation.scope.repository.clone(),
            &r.intent
                .execution_root
                .join(format!("candidate-{operation}")),
            observation.gate.config_revision().into(),
            record.digest()?,
            &r.candidate,
        )
    }
}

impl DeliveryCoordinator {
    /// Authenticate the complete retained candidate read-root inventory before
    /// a host-only metadata permit is created. This method executes no Git.
    pub fn retained_parent_read_roots(
        &self,
        gate: &FreshGate,
    ) -> Result<Vec<crate::git::VerifiedHostReadRoot>> {
        (self.current_gate)(gate)?;
        let mut roots = BTreeMap::new();
        for leaf in self.records.records_bounded(65_536)? {
            let Some(id) = leaf
                .strip_prefix("slice-result-")
                .and_then(|s| s.strip_suffix(".json"))
            else {
                continue;
            };
            let operation = Uuid::parse_str(id)
                .map_err(|_| Error::new("record_corrupt", "invalid retained slice identity"))?;
            let record: Signed<SliceRecord> = self.records.read(&leaf)?;
            self.key.verify("isolated_slice", &record)?;
            if record.payload.intent.scope.work != *gate.work() {
                continue;
            }
            let observation =
                self.observation(gate.clone(), candidate_binding(&record.payload.candidate)?)?;
            let root = self.retained_candidate_host_root(&observation, operation)?;
            if roots.insert(root.root().to_owned(), root).is_some() {
                return Err(Error::new(
                    "ownership_conflict",
                    "duplicate retained candidate read root",
                ));
            }
            if roots.len() > 48 {
                return Err(Error::new(
                    "bounded_input",
                    "retained parent candidate read roots exceed finite bound",
                ));
            }
        }
        Ok(roots.into_values().collect())
    }
}

#[cfg(test)]
mod target_equivalence_tests {
    use super::*;
    #[test]
    fn authenticated_redeployment_preserves_semantic_vector_but_source_or_artifact_change_does_not()
    {
        let original = ProductionTargetBaseline {
            provider: "github".into(),
            target: "production-web".into(),
            repository: RepositoryId {
                hostname: "github.com".into(),
                id: "42".into(),
            },
            source: crate::git::Revision {
                commit: "a".repeat(40),
                tree: "b".repeat(40),
            },
            artifact_digest: bytes_digest(b"actual artifact"),
            provider_record: "deployment:1/status:1".into(),
            observed_at: Utc::now(),
        };
        let mut current = original.clone();
        current.provider_record = "deployment:2/status:2".into();
        current.observed_at += chrono::Duration::seconds(1);
        assert_eq!(
            target_identity(&original).unwrap(),
            target_identity(&current).unwrap()
        );
        current.artifact_digest = bytes_digest(b"changed artifact");
        assert_ne!(
            target_identity(&original).unwrap(),
            target_identity(&current).unwrap()
        );
        current = original.clone();
        current.source.commit = "c".repeat(40);
        assert_ne!(
            target_identity(&original).unwrap(),
            target_identity(&current).unwrap()
        );
    }
}

#[path = "delivery_parent_history.rs"]
mod history;
pub use history::{HistoricalReleaseParent, RecoveredReleaseParent, VerifiedReleaseOrigin};

#[path = "delivery_parent_origin.rs"]
mod accepted_origin;
pub(crate) use accepted_origin::{AcceptedOriginIdentity, AcceptedOriginRef};

#[path = "delivery_parent_reassembly.rs"]
mod reassembly;
pub(crate) use reassembly::{
    AcceptedParentOrigin, ParentReassemblyPredecessor, ParentReassemblySource,
    ParentReassemblySourceOrigin, ParentReassemblyWork,
};

#[path = "delivery_completed_carry.rs"]
mod completed_carry;
pub(crate) use completed_carry::CompletedCarryObjects;

#[path = "delivery_carry_history.rs"]
mod carry_history;
pub(crate) use carry_history::{CarryNamespace, CarryReferences};

#[path = "delivery_carried_acceptance.rs"]
mod carried_acceptance;
pub(crate) use carried_acceptance::{
    CarriedNoReleaseOrigin, CarriedOriginRef, CompletedParentReassemblyPhases,
    ObservedCurrentCarry, PublishedCarry, ReassembledParent, RetainedCarrySubject,
};

impl DeliveryCoordinator {
    pub(crate) fn reject_superseding_parent_publication(
        key: &evidence::AttestationKey,
        runtime: Uuid,
        comments: &[crate::providers::Comment],
        selected: &crate::providers::Comment,
    ) -> Result<()> {
        let (_, expected) = DeliveryCoordinator::decode_parent_publication(&selected.body, key)?;
        let selected_time = selected.created_at.ok_or_else(|| {
            Error::new(
                "incompletely_observed",
                "parent publication timestamp is unavailable",
            )
        })?;
        for other in comments {
            if other.id == selected.id
                || other.bot
                || other.delegated
                || other.user_id != selected.user_id
                || other.issue_id != selected.issue_id
            {
                continue;
            }
            if let Ok((observed_runtime, evidence)) =
                DeliveryCoordinator::decode_parent_publication(&other.body, key)
                && observed_runtime == runtime
                && evidence.work == expected.work
                && other.created_at.is_none_or(|time| time >= selected_time)
            {
                return Err(Error::new(
                    "stale_result",
                    "a newer or ambiguous parent result supersedes the selected publication",
                ));
            }
        }
        Ok(())
    }
}
