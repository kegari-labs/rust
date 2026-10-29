//! Workflow execution evidence and owned repository identity.
//! Codex owns repository tools; this module accepts results and protects artifacts.
use crate::evidence::{ArtifactReference, CheckKind, WorkIdentity, validate_digest};
use crate::git::Git;
use chrono::{DateTime, Utc};
use kegari_context::workflow::Workflow;
use kegari_digest::{bytes_digest, canonical_digest};
use kegari_execution::CommandOutcome;
use kegari_execution::{ExecutionRole, ValidatedIsolation};
use kegari_git::{RepositoryId, Revision};
use kegari_types::artifact::ArtifactProof;
use kegari_types::error::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[path = "execution_corrections.rs"]
mod corrections;
pub use corrections::{CorrectionExecutionProvenance, EffectiveExecutionBinding};
#[path = "execution_check_bundle.rs"]
mod check_bundle;
#[path = "execution_codex.rs"]
mod codex;
#[path = "execution_discovered.rs"]
mod discovered;
#[path = "execution_record_validation.rs"]
mod record_validation;
pub use check_bundle::{
    CheckBundleExecutionRecord, CheckBundleMemberResult, ServiceExecutionRecord,
};
pub use codex::CodexCheckProvenance;
pub(crate) use codex::{
    CodexArtifactSnapshot, CodexCheckInput, accept_codex_check, capture_codex_artifacts,
    observe_codex_source,
};
pub use discovered::{DiscoveredCheckProvenance, DiscoveredServiceObservation};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionEvidenceClass {
    ConfinedExecution,
    CodexExecution,
    DeterministicFixture,
}

pub trait CommandCancellation: Send + Sync {
    /// Caller durably fences authority before requesting operator cancellation.
    fn cancel(&self) -> Result<()>;
    /// Only an actual retained child handle may establish exit. Unknown
    /// adapters conservatively retain their registry entry.
    fn exited(&self) -> Result<bool> {
        Ok(false)
    }
    fn service_observation(&self) -> Result<kegari_execution::ServiceObservation> {
        Err(Error::new(
            "service_observation_unavailable",
            "boundary has no original service leader observation",
        ))
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionBinding {
    pub runtime_id: Uuid,
    pub operation_id: Uuid,
    pub work: WorkIdentity,
    pub repository: RepositoryId,
    pub revision: Revision,
    pub candidate_vector_digest: Option<String>,
    pub isolation_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectoryIdentity {
    pub schema_version: u32,
    pub runtime_id: Uuid,
    pub directory_id: Uuid,
    pub repository: RepositoryId,
    pub work: WorkIdentity,
    pub repository_path: PathBuf,
    pub operator_checkout: PathBuf,
}

#[derive(Debug)]
pub struct OwnedDirectory {
    identity: DirectoryIdentity,
    marker: PathBuf,
    digest: String,
}
impl OwnedDirectory {
    fn validate_identity(
        identity: &DirectoryIdentity,
        boundary: &ValidatedIsolation,
    ) -> Result<PathBuf> {
        boundary.revalidate()?;
        identity.work.validate()?;
        identity.repository.validate()?;
        let source = identity
            .operator_checkout
            .canonicalize()
            .map_err(|e| Error::io("resolve shared source checkout", e))?;
        if identity.schema_version != 1
            || identity.runtime_id.is_nil()
            || identity.directory_id.is_nil()
            || identity.repository_path != boundary.plan().repository
            || identity.operator_checkout != source
            || source.starts_with(&identity.repository_path)
            || identity.repository_path.starts_with(source)
        {
            return Err(Error::new(
                "execution_ownership",
                "execution must be a separately owned directory, never the operator checkout",
            ));
        }
        Ok(boundary
            .plan()
            .execution_root
            .join(format!("owned-execution-{}.json", identity.directory_id)))
    }
    /// Call only after a host-created clone is populated. Never adopts an
    /// existing identity marker or resets any files after a partial creation.
    pub fn register(identity: DirectoryIdentity, boundary: &ValidatedIsolation) -> Result<Self> {
        let marker = Self::validate_identity(&identity, boundary)?;
        if marker.starts_with(&boundary.plan().repository)
            || marker.starts_with(&boundary.plan().scratch)
        {
            return Err(Error::new(
                "execution_ownership",
                "directory receipt must be outside worker writable roots",
            ));
        }
        let digest = canonical_digest(1, &identity)?;
        let bytes = serde_json::to_vec(&identity)
            .map_err(|_| Error::invalid("encode owned directory identity"))?;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        }
        let mut file = options
            .open(&marker)
            .map_err(|e| Error::io("create exclusive directory identity", e))?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| Error::io("persist directory identity", e))?;
        std::fs::File::open(&boundary.plan().execution_root)
            .and_then(|d| d.sync_all())
            .map_err(|e| Error::io("sync directory identity", e))?;
        Ok(Self {
            identity,
            marker,
            digest,
        })
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
    pub fn identity(&self) -> &DirectoryIdentity {
        &self.identity
    }
    pub fn marker(&self) -> &Path {
        &self.marker
    }
    pub fn reopen(
        marker: &Path,
        expected_digest: &str,
        boundary: &ValidatedIsolation,
    ) -> Result<Self> {
        validate_digest(expected_digest)?;
        let identity: DirectoryIdentity = serde_json::from_slice(&file_bytes(marker, 65_536)?)
            .map_err(|_| {
                Error::new(
                    "execution_ownership",
                    "owned directory identity is malformed",
                )
            })?;
        if Self::validate_identity(&identity, boundary)? != marker
            || canonical_digest(1, &identity)? != expected_digest
        {
            return Err(Error::new(
                "execution_ownership",
                "directory identity does not match the retained host receipt",
            ));
        }
        Ok(Self {
            identity,
            marker: marker.into(),
            digest: expected_digest.into(),
        })
    }
    pub(crate) fn verify(
        &self,
        binding: &ExecutionBinding,
        boundary: &ValidatedIsolation,
    ) -> Result<()> {
        Self::reopen(&self.marker, &self.digest, boundary)?;
        if self.identity.runtime_id != binding.runtime_id
            || self.identity.repository != binding.repository
            || self.identity.work != binding.work
        {
            return Err(Error::new(
                "execution_ownership",
                "directory belongs to another runtime, work generation or repository",
            ));
        }
        Ok(())
    }
}
impl ExecutionBinding {
    pub fn validate(&self, boundary: &ValidatedIsolation) -> Result<()> {
        self.work.validate()?;
        self.repository.validate()?;
        self.revision.validate()?;
        if self.runtime_id.is_nil()
            || self.operation_id.is_nil()
            || self.isolation_digest != boundary.digest()
        {
            return Err(Error::new(
                "execution_identity",
                "admitted work and actual isolation identity differ",
            ));
        }
        if let Some(digest) = &self.candidate_vector_digest {
            validate_digest(digest)?;
        }
        boundary.revalidate()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionLimits {
    pub command_timeout_ms: u64,
    pub output_bytes: usize,
    pub artifact_bytes: u64,
    pub source_bytes: u64,
    pub source_files: usize,
    pub media_timeout_ms: u64,
    pub media_seconds: f64,
}
impl Default for ExecutionLimits {
    fn default() -> Self {
        Self {
            command_timeout_ms: 600_000,
            output_bytes: 1_048_576,
            artifact_bytes: 268_435_456,
            source_bytes: 268_435_456,
            source_files: 100_000,
            media_timeout_ms: 120_000,
            media_seconds: 3600.0,
        }
    }
}
impl ExecutionLimits {
    fn validate(&self) -> Result<()> {
        if self.command_timeout_ms == 0
            || self.command_timeout_ms > 86_400_000
            || self.output_bytes == 0
            || self.output_bytes > 10 * 1024 * 1024
            || self.artifact_bytes == 0
            || self.artifact_bytes > 1024 * 1024 * 1024
            || self.source_bytes == 0
            || self.source_files == 0
            || self.source_files > 1_000_000
            || self.media_timeout_ms == 0
            || self.media_timeout_ms > 86_400_000
            || !self.media_seconds.is_finite()
            || self.media_seconds <= 0.0
        {
            return Err(Error::invalid(
                "execution bounds must be finite, positive and within supported limits",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceFile {
    pub digest: String,
    pub mode: u32,
    pub symlink: bool,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceSnapshot {
    pub revision: Revision,
    pub files: BTreeMap<String, SourceFile>,
}
impl SourceSnapshot {
    pub fn digest(&self) -> Result<String> {
        canonical_digest(1, self)
    }
}

fn file_bytes(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let metadata =
        std::fs::symlink_metadata(path).map_err(|e| Error::io("inspect output artifact", e))?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(Error::new(
            "artifact_invalid",
            "artifact must be a bounded regular file",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(Error::new(
                "artifact_invalid",
                "artifact must not alias another file",
            ));
        }
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let file = options
        .open(path)
        .map_err(|e| Error::io("open bounded artifact", e))?;
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|e| Error::io("read bounded artifact", e))?;
    if bytes.len() as u64 > limit {
        return Err(Error::new(
            "artifact_invalid",
            "artifact grew beyond its admitted bound",
        ));
    }
    Ok(bytes)
}

/// Observe actual working contents. Generic commands can begin with permitted
/// dirty source; only the existing validation path additionally requires a clean
/// exact revision. The revision here is the observed HEAD, never a clean-tree claim.
fn observed_source_snapshot(
    git: &Git,
    boundary: &ValidatedIsolation,
    limits: &ExecutionLimits,
    outputs: &BTreeSet<PathBuf>,
) -> Result<SourceSnapshot> {
    observed_source_snapshot_until(git, boundary, limits, outputs, None)
}
fn observed_source_snapshot_until(
    git: &Git,
    boundary: &ValidatedIsolation,
    limits: &ExecutionLimits,
    outputs: &BTreeSet<PathBuf>,
    deadline: Option<std::time::Instant>,
) -> Result<SourceSnapshot> {
    boundary.revalidate()?;
    let root = &boundary.plan().repository;
    let paths = git.observed_source_paths(root, limits.source_files)?;
    let mut files = BTreeMap::new();
    let mut bytes = 0u64;
    for relative in paths {
        if deadline.is_some_and(|deadline| std::time::Instant::now() >= deadline) {
            return Err(Error::new(
                "timeout",
                "original command deadline expired during source observation",
            ));
        }
        let path = root.join(&relative);
        if outputs.contains(&path) {
            continue;
        }
        // A tracked path can disappear, but its ancestors must never redirect
        // an observation outside the admitted repository through a symlink.
        let mut parent = path.parent();
        while let Some(directory) = parent.filter(|directory| *directory != root) {
            match std::fs::symlink_metadata(directory) {
                Ok(meta) if !meta.is_dir() => {
                    return Err(Error::new(
                        "scope_violation",
                        "source file ancestry crosses a link or non-directory",
                    ));
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(Error::io("inspect source ancestry", error)),
            }
            parent = directory.parent();
        }
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(Error::io("inspect actual source", error)),
        };
        if !meta.is_file() && !meta.is_symlink() {
            return Err(Error::new(
                "unsupported_repository",
                "source inventory requires ordinary files or symlinks",
            ));
        }
        let data = if meta.is_symlink() {
            std::fs::read_link(&path)
                .map_err(|e| Error::io("read source link", e))?
                .as_os_str()
                .as_encoded_bytes()
                .to_vec()
        } else {
            file_bytes(&path, limits.source_bytes.saturating_sub(bytes))?
        };
        bytes = bytes.saturating_add(data.len() as u64);
        if bytes > limits.source_bytes {
            return Err(Error::new(
                "source_limit",
                "source snapshot exceeds byte bound",
            ));
        }
        #[cfg(unix)]
        let mode = {
            use std::os::unix::fs::MetadataExt;
            meta.mode() & 0o7777
        };
        #[cfg(not(unix))]
        let mode = u32::from(meta.permissions().readonly());
        files.insert(
            relative,
            SourceFile {
                digest: bytes_digest(&data),
                mode,
                symlink: meta.is_symlink(),
            },
        );
    }
    let revision = git.revision(root, "HEAD")?;
    Ok(SourceSnapshot { revision, files })
}

pub fn artifact_reference(proof: &ArtifactProof) -> ArtifactReference {
    ArtifactReference {
        reference: proof.path.to_string_lossy().into_owned(),
        digest: proof.digest.clone(),
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckExecutionRecord {
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex: Option<CodexCheckProvenance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovered: Option<DiscoveredCheckProvenance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correction: Option<CorrectionExecutionProvenance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle: Option<CheckBundleExecutionRecord>,
    pub binding: ExecutionBinding,
    pub workflow_digest: String,
    pub check_name: String,
    pub kind: CheckKind,
    pub before: SourceSnapshot,
    pub after: SourceSnapshot,
    pub artifacts: Vec<ArtifactProof>,
    pub command: CommandOutcome,
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
    pub evidence_class: ExecutionEvidenceClass,
    pub successful: bool,
    pub blocker: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum CheckJournalEntry {
    Started {
        binding: Box<ExecutionBinding>,
        #[serde(skip_serializing_if = "Option::is_none")]
        discovered: Option<Box<DiscoveredCheckProvenance>>,
        correction: Option<CorrectionExecutionProvenance>,
        workflow_digest: String,
        check_name: String,
        source_digest: String,
        started_at: DateTime<Utc>,
    },
    Finished(Box<CheckExecutionRecord>),
    BundleStopped {
        operation_id: Uuid,
        plan_digest: String,
        members: Vec<CheckBundleMemberResult>,
        error: Option<Error>,
    },
    Uncertain {
        operation_id: Uuid,
        error_code: String,
    },
}

#[derive(Debug)]
pub struct VerifiedCheckExecution {
    record: CheckExecutionRecord,
    effective: Option<EffectiveExecutionBinding>,
    validation_plan: Option<crate::validation::ValidationPlan>,
}

#[derive(Debug)]
pub struct FailedCheckExecution {
    record: CheckExecutionRecord,
    effective: Option<EffectiveExecutionBinding>,
    validation_plan: Option<crate::validation::ValidationPlan>,
}
impl FailedCheckExecution {
    pub fn validation_plan(&self) -> Option<&crate::validation::ValidationPlan> {
        self.validation_plan.as_ref()
    }
    pub fn effective_binding(&self) -> Option<&EffectiveExecutionBinding> {
        self.effective.as_ref()
    }
    pub fn record(&self) -> &CheckExecutionRecord {
        &self.record
    }
    /// A repair trigger needs a known failed command from the qualified boundary.
    /// Timeouts, output truncation and changed source are blockers, not repair proof.
    pub fn production_failure(&self) -> Result<&CheckExecutionRecord> {
        if !matches!(
            self.record.evidence_class,
            ExecutionEvidenceClass::ConfinedExecution | ExecutionEvidenceClass::CodexExecution
        ) {
            return Err(Error::new(
                "unqualified_fixture",
                "fixture failure is not production repair evidence",
            ));
        }
        self.record.validate_shape()?;
        discovered::verify_capability(&self.record, self.validation_plan.as_ref())?;
        if self
            .record
            .bundle
            .as_ref()
            .is_some_and(|bundle| !bundle.services_completed_cleanly())
            || self.record.successful
            || self.record.before != self.record.after
            || self.record.command.exit_code.is_none_or(|code| code == 0)
            || self.record.command.timed_out
            || self.record.command.output_truncated
            || self.record.blocker.is_none()
        {
            return Err(Error::new(
                "no_work_trigger",
                "check has no unchanged-source known command failure",
            ));
        }
        Ok(&self.record)
    }
    pub fn retain_outputs(
        &self,
        boundary: &ValidatedIsolation,
        directory: &kegari_storage::PrivateDir,
    ) -> Result<Vec<RetainedArtifact>> {
        VerifiedCheckExecution::retain_record_outputs(&self.record, boundary, directory)
    }
    pub(crate) fn retain_outputs_into(
        &self,
        boundary: &ValidatedIsolation,
        directory: &kegari_storage::PrivateDir,
        retain: impl FnMut(RetainedArtifact) -> Result<()>,
    ) -> Result<()> {
        VerifiedCheckExecution::retain_record_outputs_into(
            &self.record,
            boundary,
            directory,
            retain,
        )
    }
}
#[derive(Debug)]
pub enum CheckCompletion {
    Passed(VerifiedCheckExecution),
    Failed(FailedCheckExecution),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetainedArtifact {
    original: ArtifactProof,
    retained_paths: Vec<PathBuf>,
}
impl RetainedArtifact {
    pub(crate) fn matches_reference(&self, expected: &ArtifactProof) -> bool {
        &self.original == expected
    }
    pub(crate) fn verify_reference(&self, expected: &ArtifactProof) -> Result<()> {
        if &self.original != expected {
            return Err(Error::new(
                "artifact_conflict",
                "retained artifact identity differs from requested evidence",
            ));
        }
        self.read_verified(expected).map(|_| ())
    }
    pub(crate) fn read_verified(&self, expected: &ArtifactProof) -> Result<Vec<u8>> {
        if &self.original != expected {
            return Err(Error::new(
                "artifact_conflict",
                "retained artifact identity differs from requested evidence",
            ));
        }
        self.read(expected.bytes)
    }
    pub(crate) fn inventory_digest(&self) -> Result<String> {
        canonical_digest(1, &(&self.original, &self.retained_paths))
    }
    pub(crate) fn proof_digest(&self) -> Result<String> {
        canonical_digest(1, &self.original)
    }
    fn verify(&self, maximum: u64) -> Result<()> {
        self.read(maximum).map(|_| ())
    }
    fn read(&self, maximum: u64) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        for path in &self.retained_paths {
            let chunk = file_bytes(path, 4 * 1024 * 1024)?;
            if (bytes.len() as u64).saturating_add(chunk.len() as u64) > maximum {
                return Err(Error::new(
                    "artifact_conflict",
                    "retained artifact exceeds its bound",
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        if bytes.len() as u64 != self.original.bytes || bytes_digest(&bytes) != self.original.digest
        {
            return Err(Error::new(
                "artifact_conflict",
                "retained prior artifact does not reproduce its evidence digest",
            ));
        }
        Ok(bytes)
    }
    pub fn retained_paths(&self) -> &[PathBuf] {
        &self.retained_paths
    }
}
impl VerifiedCheckExecution {
    pub fn validation_plan(&self) -> Option<&crate::validation::ValidationPlan> {
        self.validation_plan.as_ref()
    }
    pub fn effective_binding(&self) -> Option<&EffectiveExecutionBinding> {
        self.effective.as_ref()
    }
    pub fn record(&self) -> &CheckExecutionRecord {
        &self.record
    }
    pub fn digest(&self) -> Result<String> {
        canonical_digest(1, &self.record)
    }
    pub fn production_evidence(&self) -> Result<&CheckExecutionRecord> {
        if !matches!(
            self.record.evidence_class,
            ExecutionEvidenceClass::ConfinedExecution | ExecutionEvidenceClass::CodexExecution
        ) {
            return Err(Error::new(
                "unqualified_fixture",
                "deterministic execution fixture is not actual validation evidence",
            ));
        }
        self.record.validate_shape()?;
        discovered::verify_capability(&self.record, self.validation_plan.as_ref())?;
        Ok(&self.record)
    }
    /// Preserve bytes outside every worker read/write root before permitting a
    /// named check to replace its own previous output. A digest alone is not
    /// artifact retention.
    pub fn retain_outputs(
        &self,
        boundary: &ValidatedIsolation,
        directory: &kegari_storage::PrivateDir,
    ) -> Result<Vec<RetainedArtifact>> {
        Self::retain_record_outputs(&self.record, boundary, directory)
    }
    pub(crate) fn retain_outputs_into(
        &self,
        boundary: &ValidatedIsolation,
        directory: &kegari_storage::PrivateDir,
        retain: impl FnMut(RetainedArtifact) -> Result<()>,
    ) -> Result<()> {
        Self::retain_record_outputs_into(&self.record, boundary, directory, retain)
    }
    fn retain_record_outputs(
        record: &CheckExecutionRecord,
        boundary: &ValidatedIsolation,
        directory: &kegari_storage::PrivateDir,
    ) -> Result<Vec<RetainedArtifact>> {
        let mut retained = Vec::new();
        Self::retain_record_outputs_into(record, boundary, directory, |proof| {
            retained.push(proof);
            Ok(())
        })?;
        Ok(retained)
    }
    fn retain_record_outputs_into(
        record: &CheckExecutionRecord,
        boundary: &ValidatedIsolation,
        directory: &kegari_storage::PrivateDir,
        mut retain: impl FnMut(RetainedArtifact) -> Result<()>,
    ) -> Result<()> {
        boundary.revalidate()?;
        let private = directory.path();
        if std::iter::once(&boundary.plan().repository)
            .chain(std::iter::once(&boundary.plan().scratch))
            .chain(boundary.plan().read_roots.iter().map(|g| &g.path))
            .any(|root| private.starts_with(root) || root.starts_with(private))
        {
            return Err(Error::new(
                "artifact_conflict",
                "retained evidence must be outside all worker-readable roots",
            ));
        }
        for original in &record.artifacts {
            let bytes = file_bytes(&original.path, original.bytes)?;
            if bytes_digest(&bytes) != original.digest {
                return Err(Error::new(
                    "artifact_conflict",
                    "check output changed before evidence retention",
                ));
            }
            let address = original
                .digest
                .strip_prefix("sha256:")
                .ok_or_else(|| Error::invalid("artifact digest schema unsupported"))?;
            let mut retained_paths = Vec::new();
            for (index, chunk) in bytes.chunks(4 * 1024 * 1024).enumerate() {
                let leaf = format!("artifact-{address}-{index}.bin");
                match directory.read_bytes(&leaf, 4 * 1024 * 1024) {
                    Ok(existing) if existing == chunk => {}
                    Ok(_) => {
                        return Err(Error::new(
                            "artifact_conflict",
                            "retained content address conflicts with existing bytes",
                        ));
                    }
                    Err(error) if error.code == "record_missing" => {
                        directory.write_bytes(&leaf, chunk)?
                    }
                    Err(error) => return Err(error),
                }
                retained_paths.push(directory.path().join(leaf));
            }
            let proof = RetainedArtifact {
                original: original.clone(),
                retained_paths,
            };
            proof.verify(original.bytes)?;
            // Hand each completed retention to the original owner before any
            // later artifact or acknowledgement can fail.
            retain(proof)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookPhase {
    AfterCreate,
    BeforeRun,
    AfterRun,
    BeforeRemove,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookState {
    StartedUnknown,
    Succeeded,
    Failed,
    Skipped,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookRecord {
    pub phase: HookPhase,
    pub session: Option<Uuid>,
    pub workflow_digest: String,
    pub script_digest: Option<String>,
    pub cleanup_plan: Option<String>,
    pub state: HookState,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub stdout: String,
    pub stderr: String,
    pub error_code: Option<String>,
    pub evidence_class: ExecutionEvidenceClass,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionLifecycle {
    Launched,
    Ended,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookLedger {
    #[serde(
        default = "legacy_hook_schema",
        skip_serializing_if = "is_legacy_hook_schema"
    )]
    pub schema_version: u32,
    pub binding_digest: String,
    pub initialized: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherited_creation: Option<InheritedHookCreation>,
    pub sessions: BTreeMap<Uuid, SessionLifecycle>,
    pub records: BTreeMap<String, HookRecord>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InheritedHookCreation {
    original_operation: Uuid,
    original_binding_digest: String,
    original_ledger_digest: String,
    enrollment_digest: String,
}
fn legacy_hook_schema() -> u32 {
    1
}
fn is_legacy_hook_schema(version: &u32) -> bool {
    *version == 1
}
impl HookLedger {
    pub fn new(binding: &ExecutionBinding) -> Result<Self> {
        Ok(Self {
            schema_version: 2,
            binding_digest: canonical_digest(1, binding)?,
            initialized: false,
            inherited_creation: None,
            sessions: BTreeMap::new(),
            records: BTreeMap::new(),
        })
    }
    /// Only an actual resumed family enrollment can carry directory creation.
    /// This records no new AfterCreate attempt and does not copy session results.
    pub(crate) fn for_reused_execution(
        enrollment: &crate::housekeeping::EnrolledDirectoryReuse,
        workflow: &Workflow,
    ) -> Result<Self> {
        enrollment.revalidate()?;
        let (original_operation, original) = enrollment.creation_ledger()?;
        let creation = original
            .records
            .get("AfterCreate:directory")
            .ok_or_else(|| {
                Error::new("hook_gate", "original creation hook provenance is absent")
            })?;
        if original.schema_version != 2
            || !original.initialized
            || creation.phase != HookPhase::AfterCreate
            || creation.session.is_some()
            || creation.workflow_digest != workflow.digest
            || creation.timed_out
            || creation.error_code.is_some()
            || creation.script_digest
                != workflow
                    .front_matter
                    .hooks
                    .after_create
                    .as_ref()
                    .map(|script| bytes_digest(script.as_bytes()))
            || !matches!(creation.state, HookState::Succeeded | HookState::Skipped)
            || creation.state == HookState::Succeeded
                && creation.evidence_class != ExecutionEvidenceClass::CodexExecution
            || creation.state == HookState::Skipped && creation.script_digest.is_some()
            || original
                .records
                .values()
                .any(|record| record.state == HookState::StartedUnknown)
            || original
                .sessions
                .values()
                .any(|phase| *phase == SessionLifecycle::Launched)
        {
            return Err(Error::new(
                "hook_gate",
                "original directory creation is failed, unqualified, live or unknown",
            ));
        }
        let mut successor = Self::new(enrollment.binding())?;
        successor.initialized = true;
        successor.inherited_creation = Some(InheritedHookCreation {
            original_operation,
            original_binding_digest: original.binding_digest.clone(),
            original_ledger_digest: canonical_digest(1, &original)?,
            enrollment_digest: enrollment.digest().into(),
        });
        Ok(successor)
    }
    fn save(&mut self, next: Self, persist: &mut impl FnMut(&Self) -> Result<()>) -> Result<()> {
        persist(&next)?;
        *self = next;
        Ok(())
    }
    pub fn mark_launched(
        &mut self,
        session: Uuid,
        persist: &mut impl FnMut(&Self) -> Result<()>,
    ) -> Result<()> {
        let key = hook_key(HookPhase::BeforeRun, Some(session));
        if session.is_nil()
            || self.sessions.contains_key(&session)
            || !self.initialized
            || self
                .records
                .get(&key)
                .is_none_or(|r| !matches!(r.state, HookState::Succeeded | HookState::Skipped))
        {
            return Err(Error::new(
                "hook_gate",
                "session cannot launch without its successful before-run preparation",
            ));
        }
        let mut next = self.clone();
        next.sessions.insert(session, SessionLifecycle::Launched);
        self.save(next, persist)
    }
    pub fn mark_ended(
        &mut self,
        session: Uuid,
        persist: &mut impl FnMut(&Self) -> Result<()>,
    ) -> Result<()> {
        if self.sessions.get(&session) != Some(&SessionLifecycle::Launched) {
            return Err(Error::new("hook_gate", "session was not recorded launched"));
        }
        let mut next = self.clone();
        next.sessions.insert(session, SessionLifecycle::Ended);
        self.save(next, persist)
    }
}
fn hook_key(phase: HookPhase, session: Option<Uuid>) -> String {
    format!(
        "{phase:?}:{}",
        session.map_or_else(|| "directory".into(), |id| id.to_string())
    )
}

impl CommandCancellation for crate::git::GitCancellation {
    fn cancel(&self) -> Result<()> {
        crate::git::GitCancellation::cancel(self)
    }
    fn exited(&self) -> Result<bool> {
        crate::git::GitCancellation::exited(self)
    }
}
