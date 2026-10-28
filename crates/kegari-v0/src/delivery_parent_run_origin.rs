//! Actual assignment origin retained by the two host parent producers. A reader
//! authenticates history only; there is no decoded assignment/run constructor.
use super::carry_history::{CarryNamespace, CarryReferences};
use super::*;
use crate::control::{ParentReassemblyReference, ReceiptPhase};
use crate::worker::{WorkerAuthority, WorkerContract};
const PURPOSE: &str = "parent_run_origin";
const MAX_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Clone)]
pub(crate) struct ParentAssignmentOrigin {
    workspace_records: Arc<PrivateDir>,
    key: Arc<AttestationKey>,
    control: Arc<ControlHost>,
    parent: WorkIdentity,
    workspace: Uuid,
    runtime: Uuid,
    vector: String,
    contract: String,
    refs: CarryReferences,
    files: BTreeMap<String, String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ParentOriginRecord {
    schema_version: u32,
    pub(crate) runtime: Uuid,
    pub(crate) workspace: Uuid,
    pub(crate) operation: Uuid,
    pub(crate) parent: WorkIdentity,
    pub(crate) worker: WorkIdentity,
    pub(crate) phase: String,
    pub(crate) account: String,
    pub(crate) preparation: crate::control::ReviewReceiptReference,
    assignment: String,
    base_assignment: String,
    vector: String,
    contract: String,
    authority: String,
    context: Value,
    prompt_digest: String,
    workflow: Value,
    tools: Vec<Value>,
    manifest: kegari_context::instructions::InstructionManifest,
    source_descriptors: Vec<Value>,
    reassembly: Option<ParentReassemblyReference>,
    workspace_records: PathBuf,
    references: CarryReferences,
    extra_files: BTreeMap<String, String>,
}
#[derive(Clone)]
pub(crate) struct PreparedParentRunOrigin {
    signed: Signed<ParentOriginRecord>,
    sources: Vec<DeliverySource>,
    records: Arc<PrivateDir>,
}
fn invalid(message: &str) -> Error {
    Error::new("parent_retirement", message)
}
fn root_of(records: &PrivateDir, workspace: Uuid) -> Result<PrivateDir> {
    if records.path().file_name().and_then(|n| n.to_str()) != Some(workspace.to_string().as_str()) {
        return Err(invalid(
            "parent history requires its exact workspace namespace",
        ));
    }
    PrivateDir::open(
        records
            .path()
            .parent()
            .ok_or_else(|| invalid("parent history root absent"))?,
    )
}
/// Retain only bytes that reproduce the signed ordered artifact. Hashing a
/// fresh file without this aggregate check could legitimize a pre-capture edit.
fn retain_artifact_chunks(
    records: &PrivateDir,
    chunks: &[String],
    expected_digest: &str,
    expected_bytes: Option<u64>,
    files: &mut BTreeMap<String, String>,
) -> Result<()> {
    const CHUNK_BYTES: usize = 4 * 1024 * 1024;
    const MAX_CHUNKS: usize = 64;
    const MAX_ARTIFACT_BYTES: usize = CHUNK_BYTES * MAX_CHUNKS;
    evidence::validate_digest(expected_digest)?;
    if chunks.len() > MAX_CHUNKS
        || expected_bytes.is_some_and(|bytes| bytes == 0 || bytes > MAX_ARTIFACT_BYTES as u64)
        || expected_bytes.is_some() && chunks.is_empty()
    {
        return Err(invalid(
            "original parent artifact exceeds retained chunk bounds",
        ));
    }
    let mut captured = BTreeMap::new();
    let mut bytes = Vec::new();
    for leaf in chunks {
        if captured.contains_key(leaf) {
            return Err(invalid("original parent artifact repeats a chunk"));
        }
        let chunk = records.read_bytes(leaf, CHUNK_BYTES as u64)?;
        let total = bytes
            .len()
            .checked_add(chunk.len())
            .ok_or_else(|| invalid("parent artifact byte count overflow"))?;
        if total > MAX_ARTIFACT_BYTES
            || expected_bytes.is_some_and(|expected| total as u64 > expected)
        {
            return Err(invalid("original parent artifact byte bound changed"));
        }
        // The retained hash and aggregate input use the exact same read.
        captured.insert(leaf.clone(), bytes_digest(&chunk));
        bytes.extend(chunk);
    }
    if expected_bytes.is_some_and(|expected| bytes.len() as u64 != expected)
        || bytes_digest(&bytes) != expected_digest
    {
        return Err(invalid("original signed parent artifact bytes changed"));
    }
    for (leaf, digest) in &captured {
        if files.get(leaf).is_some_and(|prior| prior != digest) {
            return Err(invalid(
                "parent artifact changed across original references",
            ));
        }
    }
    // Failed validation must not leave partial new reference hashes behind.
    files.extend(captured);
    Ok(())
}

/// Archive resolution authenticates the complete manifest, history and artifact
/// inventory. Keep the hashes of those exact reads in the original assignment,
/// so later retirement cannot accept a compact reference after its bytes vanish.
fn retain_validation_archive(
    records: &PrivateDir,
    key: &AttestationKey,
    workspace: Uuid,
    runtime: Uuid,
    reference: &crate::validation_archive::ValidationArchiveReference,
    files: &mut BTreeMap<String, String>,
) -> Result<()> {
    let archive = crate::validation_archive::read(
        records,
        key,
        &crate::validation_archive::ValidationArchiveNamespace {
            workspace_id: workspace,
            runtime_id: runtime,
        },
        reference,
    )?;
    retain_authenticated_files(files, archive.retained_files())
}

fn retain_authenticated_files(
    files: &mut BTreeMap<String, String>,
    captured: &BTreeMap<String, String>,
) -> Result<()> {
    for (leaf, digest) in captured {
        if files.get(leaf).is_some_and(|prior| prior != digest) {
            return Err(invalid(
                "validation archive changed across original references",
            ));
        }
    }
    files.extend(captured.clone());
    Ok(())
}

impl ParentAssignmentOrigin {
    pub(in crate::delivery) fn capture(
        coordinator: &DeliveryCoordinator,
        contract: &ParentContract,
        vector: &ParentCandidateVector,
        review: Option<&ParentReviewAssignment>,
        integrated: Option<&Signed<IntegratedRecord>>,
    ) -> Result<Self> {
        coordinator
            .key
            .verify("parent_candidate_vector", &vector.signed)?;
        let workspace = contract.gate.workspace_id();
        let records = &coordinator.records;
        let root = root_of(records, workspace)?;
        if vector.signed.payload.work != *contract.work()
            || vector.signed.payload.contract_digest != contract.digest()?
        {
            return Err(invalid(
                "actual parent vector differs from assignment contract",
            ));
        }
        let retained_vector: Signed<VectorRecord> =
            records.read(&format!("parent-vector-{}.json", &vector.digest()[7..]))?;
        coordinator
            .key
            .verify("parent_candidate_vector", &retained_vector)?;
        if retained_vector.digest()? != vector.signed.digest()? {
            return Err(invalid(
                "retained vector differs from actual producer capability",
            ));
        }
        let namespace = CarryNamespace::Workspace { workspace };
        let mut refs = CarryReferences::default();
        let mut files = BTreeMap::new();
        refs.retain_file(
            namespace.clone(),
            records,
            &format!("parent-vector-{}.json", &vector.digest()[7..]),
            "parent_candidate_vector",
            &coordinator.key,
        )?;
        for (repository, operation) in &vector.signed.payload.slices {
            let leaf = format!("slice-result-{operation}.json");
            let signed: Signed<SliceRecord> = records.read(&leaf)?;
            coordinator.key.verify("isolated_slice", &signed)?;
            if signed.payload.candidate.repository != *repository
                || signed.payload.candidate.operation_id != *operation
                || !vector.candidates().contains(&signed.payload.candidate)
            {
                return Err(invalid(
                    "parent candidate closure differs from actual vector",
                ));
            }
            let intent: Signed<SliceIntent> =
                records.read(&format!("slice-intent-{operation}.json"))?;
            coordinator.key.verify("slice_intent", &intent)?;
            if canonical_digest(1, &intent.payload)? != canonical_digest(1, &signed.payload.intent)?
            {
                return Err(invalid("candidate result and original intent differ"));
            }
            for integration in &signed.payload.candidate.production_integrations {
                refs.retain_receipt(&coordinator.host.operation(integration.release_operation)?)?;
            }
            refs.retain_file(
                namespace.clone(),
                records,
                &leaf,
                "isolated_slice",
                &coordinator.key,
            )?;
            refs.retain_file(
                namespace.clone(),
                records,
                &format!("slice-intent-{operation}.json"),
                "slice_intent",
                &coordinator.key,
            )?;
            refs.retain_receipt(&coordinator.host.operation(*operation)?)?;
            refs.retain_receipt(&coordinator.host.operation(signed.payload.receipt_id)?)?;
            for digest in &signed.payload.candidate.ordered_delta_digests {
                evidence::validate_digest(digest)?;
                let leaf = format!("delta-{}.json", &digest[7..]);
                let delta: Signed<Value> = records.read(&leaf)?;
                coordinator.key.verify("staged_delta", &delta)?;
                let manifest: kegari_git::DeltaManifest =
                    serde_json::from_value(delta.payload["manifest"].clone())
                        .map_err(|_| invalid("staged delta manifest invalid"))?;
                if manifest.digest()? != *digest {
                    return Err(invalid("candidate selected delta identity changed"));
                }

                refs.retain_file(
                    namespace.clone(),
                    records,
                    &leaf,
                    "staged_delta",
                    &coordinator.key,
                )?;
                let chunks = delta.payload["patch_chunks"]
                    .as_array()
                    .ok_or_else(|| invalid("staged delta chunks absent"))?
                    .iter()
                    .map(|chunk| {
                        chunk
                            .as_str()
                            .map(str::to_owned)
                            .ok_or_else(|| invalid("staged delta chunk invalid"))
                    })
                    .collect::<Result<Vec<_>>>()?;
                retain_artifact_chunks(records, &chunks, &manifest.patch_digest, None, &mut files)?;
                let merge: Uuid = serde_json::from_value(delta.payload["merge_request"].clone())
                    .map_err(|_| invalid("staged delta merge identity absent"))?;
                refs.retain_receipt(&coordinator.host.operation(merge)?)?;
            }
        }
        if review.is_some() && integrated.is_some() {
            return Err(invalid("parent origin has conflicting integrated inputs"));
        }
        if let Some(review) = review {
            review.matches_worker(contract)?;
        }
        if let Some(integrated) = integrated.or_else(|| review.map(|r| &r.integrated.signed)) {
            coordinator
                .key
                .verify("parent_integrated_execution", integrated)?;
            if integrated.payload.runtime_id != vector.signed.payload.runtime_id
                || integrated.payload.contract_digest != contract.digest()?
                || integrated.payload.vector_digest != vector.digest()
                || integrated.payload.e2e_work.child != Some(contract.e2e_child)
            {
                return Err(invalid(
                    "integrated origin differs from its actual parent vector",
                ));
            }
            let integrated_digest = canonical_digest(1, integrated)?;
            refs.retain_file(
                namespace,
                records,
                &format!("parent-integrated-{}.json", &integrated_digest[7..]),
                "parent_integrated_execution",
                &coordinator.key,
            )?;
            let retained_integrated: Signed<IntegratedRecord> = records.read(&format!(
                "parent-integrated-{}.json",
                &integrated_digest[7..]
            ))?;
            coordinator
                .key
                .verify("parent_integrated_execution", &retained_integrated)?;
            if retained_integrated.digest()? != integrated.digest()? {
                return Err(invalid(
                    "retained integrated record differs from actual review input",
                ));
            }
            for output in &integrated.payload.outputs {
                retain_artifact_chunks(
                    records,
                    &output.chunks,
                    &output.original.digest,
                    Some(output.original.bytes),
                    &mut files,
                )?;
            }
            if let Some(archive) = &integrated.payload.archive {
                retain_validation_archive(
                    records,
                    &coordinator.key,
                    workspace,
                    vector.signed.payload.runtime_id,
                    archive,
                    &mut files,
                )?;
            }
        }
        if let Some(review) = review {
            let receipts = coordinator.host.operations()?;
            for child in &review.children {
                let child_leaf = format!("child-result-{}.json", child.digest.replace(':', "-"));
                let signed_child: Signed<ChildResultRecord> = records.read(&child_leaf)?;
                coordinator.key.verify("child_result", &signed_child)?;
                if canonical_digest(1, &signed_child.payload.evidence)? != child.digest
                    || signed_child.payload.evidence.work.child != Some(child.child)
                {
                    return Err(invalid(
                        "retained child evidence differs from reviewed original",
                    ));
                }
                if let Some(archive) = signed_child
                    .payload
                    .review
                    .payload
                    .review
                    .archive_reference()
                {
                    retain_validation_archive(
                        records,
                        &coordinator.key,
                        workspace,
                        vector.signed.payload.runtime_id,
                        archive,
                        &mut files,
                    )?;
                }
                refs.retain_file(
                    CarryNamespace::Workspace { workspace },
                    records,
                    &child_leaf,
                    "child_result",
                    &coordinator.key,
                )?;
                let body = evidence::encode_signed_publication("child_result", &signed_child)?;
                let expected_body = bytes_digest(body.as_bytes());

                let selected: Vec<_> = receipts
                    .iter()
                    .filter(|r| {
                        r.kind == "linear_comment"
                            && r.runtime_id == vector.signed.payload.runtime_id
                            && r.expected.get("provider_body_digest") == Some(&expected_body)
                            && r.remote_ids.get("comment_id")
                                == Some(&child.publication.to_string())
                            && matches!(
                                r.phase,
                                ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
                            )
                    })
                    .collect();
                let [receipt] = selected.as_slice() else {
                    return Err(invalid(
                        "parent child publication receipt is missing or ambiguous",
                    ));
                };
                refs.retain_receipt(receipt)?;
            }
        }
        refs.validate(&root, &coordinator.host, &coordinator.key)?;
        Ok(Self {
            workspace_records: records.clone(),
            key: coordinator.key.clone(),
            control: coordinator.host.clone(),
            parent: contract.work().clone(),
            workspace,
            runtime: vector.signed.payload.runtime_id,
            vector: vector.digest().into(),
            contract: contract.digest()?,
            refs,
            files,
        })
    }
    pub(crate) fn prepare(
        &self,
        assignment: &DeliveryWorkerAssignment,
        config: &Config,
        contract: &Arc<WorkerContract>,
        preparation: &Receipt,
        records: &PrivateDir,
    ) -> Result<PreparedParentRunOrigin> {
        let manifest = &contract.instructions().manifest;
        let WorkerAuthority::Authorized(gate) = contract.authority() else {
            return Err(invalid("parent origin needs actual authorized worker"));
        };
        assignment.authority().compare_admitted_authority(gate)?;
        super::super::assignment::verify_assignment_contract(
            contract,
            gate,
            assignment.phase(),
            assignment.context(),
        )?;
        let prep = crate::control::PreparationReference::from_receipt(preparation)?
            .ok_or_else(|| invalid("parent preparation typed reference absent"))?;
        if manifest.role != assignment.role()
            || self.workspace != manifest.workspace_id
            || self.runtime != config.runtime.id
            || self.parent.parent != gate.work().parent
            || self.vector != assignment.candidate_vector_digest().unwrap_or("")
            || self.contract != assignment.contract_digest()
            || prep.worker_operation != manifest.operation_id
            || prep.workspace != self.workspace
            || prep.account != contract.account_id()
            || prep.phase.phase != assignment.phase()
            || preparation.phase != ReceiptPhase::Reconciled
            || preparation.observed_result.as_ref().is_none_or(|v| {
                v["local_execution"] != "exited"
                    || v["unresolved_effects"] != json!([])
                    || v["observation"]["phase"] != "worker_preparation"
                    || v["observation"]["result"] != "prepared"
            })
        {
            return Err(invalid(
                "parent origin differs from exact successful preparation",
            ));
        }
        let mut references = self.refs.clone();
        let reassembly = assignment
            .parent_reassembly()
            .map(|proof| proof.reference().clone());
        if let Some(proof) = assignment.parent_reassembly() {
            references.extend(&proof.retirement_history(config)?)?;
        }
        let sources: Vec<_> = assignment
            .source()
            .into_iter()
            .chain(assignment.read_sources())
            .cloned()
            .collect();
        for source in &sources {
            source.revalidate_filesystem()?;
        }
        let record = ParentOriginRecord {
            schema_version: 1,
            runtime: self.runtime,
            workspace: self.workspace,
            operation: manifest.operation_id,
            parent: self.parent.clone(),
            worker: gate.work().clone(),
            phase: assignment.phase().into(),
            account: contract.account_id().into(),
            preparation: crate::control::ReviewReceiptReference {
                operation: preparation.operation_id,
                receipt_digest: canonical_digest(1, preparation)?,
            },
            assignment: assignment.digest()?,
            base_assignment: assignment.base_digest()?,
            vector: self.vector.clone(),
            contract: self.contract.clone(),
            authority: gate.digest()?,
            context: contract.context().clone(),
            prompt_digest: bytes_digest(contract.instructions().prompt.as_bytes()),
            workflow: json!({"digest":contract.workflow().digest,"front_matter":contract.workflow().front_matter,"body":contract.workflow().body}),
            tools: contract.tools().definitions().to_vec(),
            manifest: manifest.clone(),
            source_descriptors: sources
                .iter()
                .map(DeliverySource::retirement_descriptor)
                .collect::<Result<_>>()?,
            reassembly,
            workspace_records: self.workspace_records.path().to_path_buf(),
            references,
            extra_files: self.files.clone(),
        };
        record.validate_history(&self.control, &self.key)?;
        let prompt = contract.instructions().prompt.as_bytes();
        if prompt.len() as u64 > MAX_BYTES {
            return Err(invalid("parent prompt exceeds retained byte bound"));
        }
        match records.read_bytes("parent-origin-prompt.txt", MAX_BYTES) {
            Ok(bytes) if bytes == prompt => {}
            Ok(_) => return Err(invalid("original parent prompt changed")),
            Err(error) if error.code == "record_missing" => {
                records.create_bytes_exclusive("parent-origin-prompt.txt", prompt)?
            }
            Err(error) => return Err(error),
        }

        let signed = self.key.sign(PURPOSE, &record)?;
        let encoded =
            serde_json::to_vec(&signed).map_err(|_| invalid("parent origin encoding failed"))?;
        if encoded.len() as u64 > MAX_BYTES {
            return Err(invalid(
                "parent origin exceeds existing instruction/context bound",
            ));
        }
        match records.read_bytes("parent-origin.json", MAX_BYTES) {
            Ok(old) if old == encoded => {}
            Ok(_) => {
                return Err(invalid(
                    "parent origin already contains different immutable bytes",
                ));
            }
            Err(error) if error.code == "record_missing" => {
                records.create_bytes_exclusive("parent-origin.json", &encoded)?
            }
            Err(error) => return Err(error),
        }
        Ok(PreparedParentRunOrigin {
            signed,
            sources,
            records: Arc::new(PrivateDir::open(records.path())?),
        })
    }
}
impl PreparedParentRunOrigin {
    pub(crate) fn record(&self) -> &ParentOriginRecord {
        &self.signed.payload
    }
    pub(crate) fn digest(&self) -> Result<String> {
        self.signed.digest()
    }
    pub(crate) fn validate(&self, control: &ControlHost, key: &AttestationKey) -> Result<()> {
        let actual = read_origin(&self.records, control, key)?;
        if actual.digest()? != self.signed.digest()? {
            return Err(invalid("registered parent origin changed"));
        }
        for source in &self.sources {
            source.revalidate_filesystem()?;
        }
        Ok(())
    }
}
impl ParentOriginRecord {
    pub(crate) fn grant_commands(&self, control: &ControlHost) -> Result<BTreeMap<Uuid, String>> {
        let mut commands = BTreeMap::new();
        if let Some(reference) = &self.reassembly {
            for grant in &reference.grants {
                let receipt = control.operation(grant.command)?;
                if receipt.runtime_id != self.runtime
                    || receipt.actor != grant.command_actor
                    || receipt.payload_digest != grant.command_payload_digest
                    || receipt.kind != "command/Retry"
                    || !matches!(
                        receipt.phase,
                        ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
                    )
                {
                    return Err(invalid(
                        "parent native grant command has not finalized with its original identity",
                    ));
                }
                commands.insert(grant.command, canonical_digest(1, &receipt)?);
            }
        }
        Ok(commands)
    }
    pub(crate) fn verify_run_cause(&self, model: &Receipt) -> Result<()> {
        if crate::control::ParentReassemblyReference::from_receipt(model)? != self.reassembly {
            return Err(invalid(
                "parent retired run dropped or changed its original reassembly cause",
            ));
        }
        Ok(())
    }
    pub(crate) fn validate_history(
        &self,
        control: &ControlHost,
        key: &AttestationKey,
    ) -> Result<()> {
        if self.schema_version != 1
            || self.operation.is_nil()
            || self.runtime.is_nil()
            || self.manifest.operation_id != self.operation
            || self.manifest.workspace_id != self.workspace
            || self.parent.child.is_some()
            || self.worker.parent != self.parent.parent
            || self.worker.linear_endpoint != self.parent.linear_endpoint
            || self.worker.linear_workspace != self.parent.linear_workspace
            || !matches!(
                self.phase.as_str(),
                "parent_integrated_validation"
                    | "parent_integrated_review"
                    | "parent_evidence_review"
            )
            || matches!(
                self.phase.as_str(),
                "parent_integrated_validation" | "parent_integrated_review"
            ) != self.worker.child.is_some()
            || canonical_digest(1, &self.context)? != self.manifest.context_digest
        {
            return Err(invalid("parent origin immutable identity is invalid"));
        }
        let records = PrivateDir::open(&self.workspace_records)?;
        self.references
            .validate(&root_of(&records, self.workspace)?, control, key)?;
        for (leaf, digest) in &self.extra_files {
            if bytes_digest(&records.read_bytes(leaf, 4 * 1024 * 1024)?) != *digest {
                return Err(invalid("parent source/integrated retained bytes changed"));
            }
        }
        if canonical_digest(1, &control.operation(self.preparation.operation)?)?
            != self.preparation.receipt_digest
        {
            return Err(invalid("parent preparation changed"));
        }
        Ok(())
    }
}
pub(crate) fn read_origin(
    records: &PrivateDir,
    control: &ControlHost,
    key: &AttestationKey,
) -> Result<Signed<ParentOriginRecord>> {
    let signed: Signed<ParentOriginRecord> =
        serde_json::from_slice(&records.read_bytes("parent-origin.json", MAX_BYTES)?)
            .map_err(|_| invalid("parent origin encoding invalid"))?;
    key.verify(PURPOSE, &signed)?;
    if records.path().file_name().and_then(|n| n.to_str())
        != Some(signed.payload.operation.to_string().as_str())
    {
        return Err(invalid(
            "parent origin record is outside its actual worker namespace",
        ));
    }
    signed.payload.validate_history(control, key)?;
    if bytes_digest(&records.read_bytes("parent-origin-prompt.txt", MAX_BYTES)?)
        != signed.payload.prompt_digest
    {
        return Err(invalid("original parent prompt bytes changed"));
    }
    let actual_contract: Value = records.read("contract.json")?;
    if canonical_digest(1, &signed.payload.tools)?
        != actual_contract["tool_contract"].as_str().unwrap_or("")
        || signed.payload.workflow["digest"] != signed.payload.manifest.workflow_digest
    {
        return Err(invalid("original parent tool/workflow contract changed"));
    }

    Ok(signed)
}

#[cfg(test)]
#[path = "delivery_parent_run_origin_tests.rs"]
mod tests;
