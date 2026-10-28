//! Bounded, authenticated descriptions of the complete validation history.
//!
//! This module never reconstructs execution, enrollment or native capabilities.
//! Producers supply original retained artifact bytes; current scratch paths are
//! deliberately not read. A reference becomes usable only after full readback.
use crate::evidence::validation::MAX_VALIDATION_EVIDENCE_BYTES;
use crate::evidence::{CheckKind, RevisionBinding, ValidationEvidence};
use crate::execution::{ArtifactProof, ExecutionBinding};
use kegari_digest::{bytes_digest, canonical_digest};
use kegari_secrets::{AttestationKey, Signed};
use kegari_storage::PrivateDir;
use kegari_types::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::sync::Arc;
use uuid::Uuid;

const PURPOSE: &str = "kegari/validation-archive/v1";
const CHUNK_BYTES: usize = 4 * 1024 * 1024;
const MANIFEST_BYTES: usize = 256 * 1024;
const ARTIFACT_BYTES: u64 = 256 * 1024 * 1024;
const ARTIFACT_COUNT: usize = 4096;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationArchiveNamespace {
    pub workspace_id: Uuid,
    pub runtime_id: Uuid,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationArchiveReference {
    pub schema_version: u32,
    pub namespace: ValidationArchiveNamespace,
    pub manifest_digest: String,
    pub evidence_digest: String,
}

impl ValidationArchiveReference {
    pub fn validate_shape(&self) -> Result<()> {
        validate_namespace(&self.namespace)?;
        if self.schema_version != 1 {
            return Err(invalid("unsupported validation archive reference"));
        }
        raw_suffix(&self.manifest_digest)?;
        canonical_suffix(&self.evidence_digest)?;
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) struct ResolvedValidationArchive {
    reference: ValidationArchiveReference,
    evidence: Arc<ValidationEvidence>,
    artifacts: Arc<BTreeMap<String, (ArtifactProof, Vec<u8>)>>,
    retained_files: BTreeMap<String, String>,
}

impl std::fmt::Debug for ResolvedValidationArchive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedValidationArchive")
            .field("reference", &self.reference)
            .field("artifact_count", &self.artifacts.len())
            .field("retained_files", &self.retained_files)
            .finish_non_exhaustive()
    }
}

impl ResolvedValidationArchive {
    pub(crate) fn reference(&self) -> &ValidationArchiveReference {
        &self.reference
    }
    pub(crate) fn evidence(&self) -> &ValidationEvidence {
        &self.evidence
    }
    /// Leaf digests from the exact successful reads that authenticated this value.
    /// Includes the signed manifest and every history/artifact chunk; no reread.
    pub(crate) fn retained_files(&self) -> &BTreeMap<String, String> {
        &self.retained_files
    }
    pub(crate) fn read_artifact(&self, proof: &ArtifactProof) -> Result<Vec<u8>> {
        let key = canonical_digest(1, proof)?;
        match self.artifacts.get(&key) {
            Some((original, bytes)) if original == proof => Ok(bytes.clone()),
            _ => Err(invalid(
                "artifact is not in the authenticated complete inventory",
            )),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Chunk {
    ordinal: u32,
    digest: String,
    bytes: u64,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Blob {
    digest: String,
    bytes: u64,
    chunks: Vec<Chunk>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArchivedArtifact {
    identity: String,
    proof: ArtifactProof,
    blob: Blob,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    namespace: ValidationArchiveNamespace,
    original_binding: ExecutionBinding,
    binding: ExecutionBinding,
    source_digest: String,
    plan_digest: String,
    evidence_digest: String,
    history_count: u32,
    attempt_count: u32,
    evidence: Blob,
    artifacts: Vec<ArchivedArtifact>,
    artifact_bytes: u64,
}

/// Writes only exclusive content-addressed records. Existing exact bytes may be
/// reused, but changed/partial records are never repaired or overwritten. Any
/// failure leaves its owned partial files for reconciliation, with no reference.
pub(crate) fn retain(
    records: &PrivateDir,
    key: &AttestationKey,
    namespace: ValidationArchiveNamespace,
    evidence: &ValidationEvidence,
    mut artifact_read: impl FnMut(&ArtifactProof) -> Result<Vec<u8>>,
) -> Result<ResolvedValidationArchive> {
    let evidence_bytes = encode(evidence, MAX_VALIDATION_EVIDENCE_BYTES)?;
    validate_evidence(&namespace, evidence)?;
    let proofs = artifact_inventory(evidence)?;
    let artifact_bytes = proofs.values().map(|proof| proof.bytes).sum();
    let current = evidence.current()?;
    let mut manifest = Manifest {
        schema_version: 1,
        namespace: namespace.clone(),
        original_binding: current.original_binding.clone(),
        binding: current.binding.clone(),
        source_digest: current.source_digest.clone(),
        plan_digest: evidence.plan_digest.clone(),
        evidence_digest: evidence.digest()?,
        history_count: evidence.history.len() as u32,
        attempt_count: evidence.attempts.len() as u32,
        evidence: describe(&evidence_bytes),
        artifacts: Vec::with_capacity(proofs.len()),
        artifact_bytes,
    };
    // Preflight the exact metadata size before callbacks or writes. Digests have
    // fixed lengths; placeholders are never stored or returned as evidence.
    for (identity, proof) in &proofs {
        manifest.artifacts.push(ArchivedArtifact {
            identity: identity.clone(),
            proof: proof.clone(),
            blob: placeholder_blob(proof),
        });
    }
    encode(&key.sign(PURPOSE, &manifest)?, MANIFEST_BYTES)?;
    store_blob(records, &evidence_bytes)?;
    for artifact in &mut manifest.artifacts {
        let bytes = artifact_read(&artifact.proof)?;
        if bytes.len() as u64 != artifact.proof.bytes
            || bytes_digest(&bytes) != artifact.proof.digest
        {
            return Err(invalid(
                "original retained artifact bytes differ from their proof",
            ));
        }
        artifact.blob = describe(&bytes);
        store_blob(records, &bytes)?;
    }
    let signed = key.sign(PURPOSE, &manifest)?;
    let bytes = encode(&signed, MANIFEST_BYTES)?;
    let reference = ValidationArchiveReference {
        schema_version: 1,
        namespace: namespace.clone(),
        manifest_digest: bytes_digest(&bytes),
        evidence_digest: manifest.evidence_digest,
    };
    store_exact(records, &manifest_leaf(&reference.manifest_digest)?, &bytes)?;
    read(records, key, &namespace, &reference)
}

/// Expected identity must be supplied from the actual runtime/workspace owner,
/// never copied from the public reference. Fully validates data and all original
/// artifact content before making any getter available.
pub(crate) fn read(
    records: &PrivateDir,
    key: &AttestationKey,
    expected: &ValidationArchiveNamespace,
    reference: &ValidationArchiveReference,
) -> Result<ResolvedValidationArchive> {
    reference.validate_shape()?;
    validate_namespace(expected)?;
    if &reference.namespace != expected {
        return Err(invalid(
            "validation archive namespace differs from its actual owner",
        ));
    }
    let leaf = manifest_leaf(&reference.manifest_digest)?;
    let bytes = records.read_bytes(&leaf, MANIFEST_BYTES as u64)?;
    if bytes_digest(&bytes) != reference.manifest_digest {
        return Err(invalid("validation archive manifest bytes changed"));
    }
    let signed: Signed<Manifest> = serde_json::from_slice(&bytes)
        .map_err(|_| invalid("invalid validation archive manifest"))?;
    key.verify(PURPOSE, &signed)?;
    let manifest = &signed.payload;
    if encode(&signed, MANIFEST_BYTES)? != bytes
        || manifest.schema_version != 1
        || &manifest.namespace != expected
        || manifest.evidence_digest != reference.evidence_digest
    {
        return Err(invalid("validation archive manifest identity differs"));
    }
    validate_blob(&manifest.evidence, MAX_VALIDATION_EVIDENCE_BYTES as u64)?;
    validate_manifest_inventory(manifest)?;
    let mut retained_files = BTreeMap::from([(leaf, reference.manifest_digest.clone())]);
    let evidence_bytes = read_blob(records, &manifest.evidence, &mut retained_files)?;
    let evidence: ValidationEvidence = serde_json::from_slice(&evidence_bytes)
        .map_err(|_| invalid("invalid complete validation evidence"))?;
    if encode(&evidence, MAX_VALIDATION_EVIDENCE_BYTES)? != evidence_bytes {
        return Err(invalid(
            "validation evidence encoding differs from its signed producer",
        ));
    }
    validate_evidence(expected, &evidence)?;
    let current = evidence.current()?;
    if evidence.digest()? != manifest.evidence_digest
        || evidence.plan_digest != manifest.plan_digest
        || current.original_binding != manifest.original_binding
        || current.binding != manifest.binding
        || current.source_digest != manifest.source_digest
        || evidence.history.len() != manifest.history_count as usize
        || evidence.attempts.len() != manifest.attempt_count as usize
    {
        return Err(invalid(
            "validation manifest differs from complete history or source",
        ));
    }
    let proofs = artifact_inventory(&evidence)?;
    if proofs.len() != manifest.artifacts.len()
        || proofs
            .iter()
            .zip(&manifest.artifacts)
            .any(|((id, proof), artifact)| id != &artifact.identity || proof != &artifact.proof)
    {
        return Err(invalid(
            "validation archive omits or substitutes an attempt artifact",
        ));
    }
    let mut artifacts = BTreeMap::new();
    for artifact in &manifest.artifacts {
        let bytes = read_blob(records, &artifact.blob, &mut retained_files)?;
        artifacts.insert(artifact.identity.clone(), (artifact.proof.clone(), bytes));
    }
    Ok(ResolvedValidationArchive {
        reference: reference.clone(),
        evidence: Arc::new(evidence),
        artifacts: Arc::new(artifacts),
        retained_files,
    })
}

fn validate_evidence(
    namespace: &ValidationArchiveNamespace,
    evidence: &ValidationEvidence,
) -> Result<()> {
    validate_namespace(namespace)?;
    let current = evidence.current()?;
    if current.binding.runtime_id != namespace.runtime_id
        || current.original_binding.runtime_id != namespace.runtime_id
    {
        return Err(invalid(
            "validation archive runtime differs from its original owner",
        ));
    }
    let revision = RevisionBinding {
        repository: current.binding.repository.clone(),
        head: current.binding.revision.commit.clone(),
        base: current.binding.revision.commit.clone(),
        tested_commit: current.binding.revision.commit.clone(),
        tested_tree: current.binding.revision.tree.clone(),
    };
    match current.kind {
        CheckKind::Local => evidence.validate(&current.binding.work, &revision),
        CheckKind::Integrated => evidence.validate_integrated(
            &current.binding.work,
            &revision,
            current
                .binding
                .candidate_vector_digest
                .as_deref()
                .ok_or_else(|| invalid("integrated archive lacks its candidate vector"))?,
        ),
        _ => Err(invalid("unsupported validation archive kind")),
    }
}

fn artifact_inventory(evidence: &ValidationEvidence) -> Result<BTreeMap<String, ArtifactProof>> {
    let mut proofs = BTreeMap::new();
    let mut total = 0u64;
    for proof in evidence
        .attempts
        .iter()
        .flat_map(|attempt| &attempt.artifacts)
    {
        raw_suffix(&proof.digest)?;
        let identity = canonical_digest(1, proof)?;
        if let Some(previous) = proofs.get(&identity) {
            if previous != proof {
                return Err(invalid("conflicting artifact identity"));
            }
            continue;
        }
        total = total.checked_add(proof.bytes).ok_or_else(limit)?;
        if proof.bytes > ARTIFACT_BYTES || total > ARTIFACT_BYTES || proofs.len() >= ARTIFACT_COUNT
        {
            return Err(limit());
        }
        proofs.insert(identity, proof.clone());
    }
    Ok(proofs)
}

fn validate_manifest_inventory(manifest: &Manifest) -> Result<()> {
    if manifest.history_count == 0
        || manifest.history_count > 32
        || manifest.attempt_count == 0
        || manifest.attempt_count > 2048
        || manifest.artifacts.len() > ARTIFACT_COUNT
        || manifest.artifact_bytes > ARTIFACT_BYTES
    {
        return Err(limit());
    }
    canonical_suffix(&manifest.evidence_digest)?;
    canonical_suffix(&manifest.plan_digest)?;
    let mut last: Option<&str> = None;
    let mut total = 0u64;
    for artifact in &manifest.artifacts {
        if last.is_some_and(|previous| previous >= artifact.identity.as_str())
            || canonical_digest(1, &artifact.proof)? != artifact.identity
            || artifact.blob.bytes != artifact.proof.bytes
            || artifact.blob.digest != artifact.proof.digest
        {
            return Err(invalid(
                "validation artifact order or proof identity differs",
            ));
        }
        validate_blob(&artifact.blob, ARTIFACT_BYTES)?;
        total = total.checked_add(artifact.proof.bytes).ok_or_else(limit)?;
        if total > ARTIFACT_BYTES {
            return Err(limit());
        }
        last = Some(&artifact.identity);
    }
    if total != manifest.artifact_bytes {
        return Err(invalid("validation artifact aggregate differs"));
    }
    Ok(())
}

fn validate_blob(blob: &Blob, maximum: u64) -> Result<()> {
    raw_suffix(&blob.digest)?;
    if blob.bytes > maximum || blob.chunks.len() != blob.bytes.div_ceil(CHUNK_BYTES as u64) as usize
    {
        return Err(limit());
    }
    let mut remaining = blob.bytes;
    for (index, chunk) in blob.chunks.iter().enumerate() {
        raw_suffix(&chunk.digest)?;
        let size = remaining.min(CHUNK_BYTES as u64);
        if chunk.ordinal as usize != index || chunk.bytes != size || size == 0 {
            return Err(invalid("validation chunk order or size differs"));
        }
        remaining -= size;
    }
    if blob.bytes == 0 && blob.digest != bytes_digest(&[]) {
        return Err(invalid("empty artifact digest differs"));
    }
    Ok(())
}

fn describe(bytes: &[u8]) -> Blob {
    Blob {
        digest: bytes_digest(bytes),
        bytes: bytes.len() as u64,
        chunks: bytes
            .chunks(CHUNK_BYTES)
            .enumerate()
            .map(|(index, chunk)| Chunk {
                ordinal: index as u32,
                digest: bytes_digest(chunk),
                bytes: chunk.len() as u64,
            })
            .collect(),
    }
}
fn placeholder_blob(proof: &ArtifactProof) -> Blob {
    let mut remaining = proof.bytes;
    let mut chunks = Vec::new();
    while remaining > 0 {
        let bytes = remaining.min(CHUNK_BYTES as u64);
        chunks.push(Chunk {
            ordinal: chunks.len() as u32,
            digest: bytes_digest(b"placeholder"),
            bytes,
        });
        remaining -= bytes;
    }
    Blob {
        digest: proof.digest.clone(),
        bytes: proof.bytes,
        chunks,
    }
}
fn store_blob(records: &PrivateDir, bytes: &[u8]) -> Result<()> {
    for chunk in bytes.chunks(CHUNK_BYTES) {
        store_exact(records, &chunk_leaf(&bytes_digest(chunk))?, chunk)?;
    }
    Ok(())
}
fn store_exact(records: &PrivateDir, leaf: &str, bytes: &[u8]) -> Result<()> {
    match records.read_bytes(leaf, CHUNK_BYTES as u64) {
        Ok(existing) if existing == bytes => Ok(()),
        Ok(_) => Err(invalid(
            "immutable validation archive leaf already contains different bytes",
        )),
        Err(error) if error.code == "record_missing" => {
            records.create_bytes_exclusive(leaf, bytes)?;
            if records.read_bytes(leaf, CHUNK_BYTES as u64)? != bytes {
                return Err(invalid(
                    "new immutable validation archive leaf failed readback",
                ));
            }
            Ok(())
        }
        Err(error) => Err(error),
    }
}
fn read_blob(
    records: &PrivateDir,
    blob: &Blob,
    files: &mut BTreeMap<String, String>,
) -> Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(blob.bytes as usize);
    for chunk in &blob.chunks {
        let leaf = chunk_leaf(&chunk.digest)?;
        let part = records.read_bytes(&leaf, chunk.bytes)?;
        let actual = bytes_digest(&part);
        if part.len() as u64 != chunk.bytes || actual != chunk.digest {
            return Err(invalid(
                "retained validation chunk changed or is incomplete",
            ));
        }
        if files.get(&leaf).is_some_and(|previous| previous != &actual) {
            return Err(invalid("conflicting same-read archive identity"));
        }
        files.insert(leaf, actual);
        bytes.extend(part);
    }
    if bytes.len() as u64 != blob.bytes || bytes_digest(&bytes) != blob.digest {
        return Err(invalid("complete retained validation stream differs"));
    }
    Ok(bytes)
}
fn validate_namespace(namespace: &ValidationArchiveNamespace) -> Result<()> {
    if namespace.workspace_id.is_nil() || namespace.runtime_id.is_nil() {
        return Err(invalid(
            "validation archive requires actual workspace and runtime identities",
        ));
    }
    Ok(())
}
fn raw_suffix(digest: &str) -> Result<&str> {
    suffix(digest, "sha256:")
}
fn canonical_suffix(digest: &str) -> Result<&str> {
    suffix(digest, "sha256:v1:")
}
fn suffix<'a>(digest: &'a str, prefix: &str) -> Result<&'a str> {
    let value = digest
        .strip_prefix(prefix)
        .ok_or_else(|| invalid("invalid archive digest schema"))?;
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(invalid("invalid archive digest bytes"));
    }
    Ok(value)
}
fn chunk_leaf(digest: &str) -> Result<String> {
    Ok(format!("validation-chunk-{}.bin", raw_suffix(digest)?))
}
fn manifest_leaf(digest: &str) -> Result<String> {
    Ok(format!("validation-archive-{}.json", raw_suffix(digest)?))
}
fn encode(value: &impl Serialize, maximum: usize) -> Result<Vec<u8>> {
    struct Bounded {
        bytes: Vec<u8>,
        maximum: usize,
    }
    impl Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > self.maximum.saturating_sub(self.bytes.len()) {
                return Err(std::io::Error::other("validation archive size bound"));
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = Bounded {
        bytes: Vec::new(),
        maximum,
    };
    serde_json::to_writer(&mut writer, value).map_err(|_| limit())?;
    Ok(writer.bytes)
}
fn invalid(message: &str) -> Error {
    Error::new("validation_archive", message)
}
fn limit() -> Error {
    Error::new(
        "validation_archive_limit",
        "complete validation archive exceeds its bounded storage inventory",
    )
}

#[cfg(test)]
#[path = "validation_archive_tests.rs"]
pub(crate) mod tests;
