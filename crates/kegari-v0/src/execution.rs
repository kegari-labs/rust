//! Local execution tools. Production commands always use the OS boundary;
//! deterministic boundary fixtures never become qualified validation evidence.
use crate::digest::{bytes_digest, canonical_digest};
use crate::error::{Error, Result};
use crate::evidence::{ArtifactReference, CheckKind, WorkIdentity, validate_digest};
use crate::git::{Git, RepositoryId, Revision};
use crate::isolation::{self, CommandOutcome, ExecutionRole, ValidatedIsolation};
use crate::workflow::Workflow;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::Duration;
use uuid::Uuid;

#[path = "execution_corrections.rs"]
mod corrections;
pub use corrections::{CorrectionExecutionProvenance, EffectiveExecutionBinding};
#[path = "execution_check_bundle.rs"]
mod check_bundle;
#[path = "execution_record_validation.rs"]
mod record_validation;
pub use check_bundle::{
    CheckBundleExecutionRecord, CheckBundleMemberResult, CheckBundleOwner, ServiceExecutionRecord,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionEvidenceClass {
    ConfinedExecution,
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
    /// A warm pending-inventory query may inspect only this original aggregate.
    /// Ordinary, missing and decoded cancellation handles expose no bundle owner.
    fn check_bundle_owner(&self) -> Option<&CheckBundleOwner> {
        None
    }
    fn service_observation(&self) -> Result<isolation::ServiceObservation> {
        Err(Error::new(
            "service_observation_unavailable",
            "boundary has no original service leader observation",
        ))
    }
}
pub trait StartedCommand: Send {
    fn cancellation(&self) -> std::sync::Arc<dyn CommandCancellation>;
    fn wait(self: Box<Self>) -> Pin<Box<dyn Future<Output = Result<CommandOutcome>> + Send>>;
}

/// The injected implementation is a trusted host component, never a worker tool
/// argument. Fixture implementations must report DeterministicFixture.
pub trait CommandBoundary: Send + Sync {
    fn evidence_class(&self) -> ExecutionEvidenceClass;
    /// Process-free prerequisite check before reserving native ownership. The
    /// actual start must repeat its own checks; this is not a launch capability.
    /// Injected deterministic boundaries have no native prerequisite by default.
    fn preflight(&self, _isolation: &ValidatedIsolation) -> Result<()> {
        Ok(())
    }
    fn start(
        &self,
        isolation: &ValidatedIsolation,
        argv: &[String],
        timeout: Duration,
        output_limit: usize,
    ) -> Result<Box<dyn StartedCommand>>;
    /// Only the original Control launch token selects a planned bundle member.
    fn start_bundle_member(
        &self,
        _isolation: &ValidatedIsolation,
        _argv: &[String],
        _limits: isolation::BundleCommandLimits,
        _launch: crate::control::CheckBundleLaunch,
    ) -> Result<Box<dyn StartedCommand>> {
        Err(Error::new(
            "unsupported_check_bundle",
            "command boundary does not support registered check bundles",
        ))
    }
    fn run<'a>(
        &'a self,
        isolation: &'a ValidatedIsolation,
        argv: &'a [String],
        timeout: Duration,
        output_limit: usize,
    ) -> Pin<Box<dyn Future<Output = Result<CommandOutcome>> + Send + 'a>> {
        Box::pin(async move {
            self.start(isolation, argv, timeout, output_limit)?
                .wait()
                .await
        })
    }
}

impl CommandCancellation for isolation::OwnedCommandCancellation {
    fn cancel(&self) -> Result<()> {
        isolation::OwnedCommandCancellation::cancel(self)
    }
    fn exited(&self) -> Result<bool> {
        isolation::OwnedCommandCancellation::exited(self)
    }
    fn service_observation(&self) -> Result<isolation::ServiceObservation> {
        isolation::OwnedCommandCancellation::service_observation(self)
    }
}
impl StartedCommand for isolation::OwnedCommand {
    fn cancellation(&self) -> std::sync::Arc<dyn CommandCancellation> {
        std::sync::Arc::new(isolation::OwnedCommand::cancellation(self))
    }
    fn wait(self: Box<Self>) -> Pin<Box<dyn Future<Output = Result<CommandOutcome>> + Send>> {
        Box::pin(isolation::OwnedCommand::wait(*self))
    }
}

pub struct SandboxBoundary;
impl CommandBoundary for SandboxBoundary {
    fn evidence_class(&self) -> ExecutionEvidenceClass {
        ExecutionEvidenceClass::ConfinedExecution
    }
    fn preflight(&self, _isolation: &ValidatedIsolation) -> Result<()> {
        Err(Error::new(
            "process_journal_required",
            "native repository execution requires the current operation's durable process journal",
        ))
    }
    fn start(
        &self,
        _isolation: &ValidatedIsolation,
        _argv: &[String],
        _timeout: Duration,
        _output_limit: usize,
    ) -> Result<Box<dyn StartedCommand>> {
        Err(Error::new(
            "process_journal_required",
            "native repository execution requires the current operation's durable process journal",
        ))
    }
}

/// Native execution is bound to one immutable admitted operation. Reusing an
/// owned directory does not reuse its old permit or its process journal scope.
pub struct JournaledSandboxBoundary {
    journal: std::sync::Arc<crate::process_records::ProcessJournal>,
    permit: crate::control::Permit,
}
impl JournaledSandboxBoundary {
    pub fn new(
        journal: std::sync::Arc<crate::process_records::ProcessJournal>,
        permit: crate::control::Permit,
    ) -> Self {
        Self { journal, permit }
    }
}
impl CommandBoundary for JournaledSandboxBoundary {
    fn start_bundle_member(
        &self,
        isolation: &ValidatedIsolation,
        argv: &[String],
        limits: isolation::BundleCommandLimits,
        launch: crate::control::CheckBundleLaunch,
    ) -> Result<Box<dyn StartedCommand>> {
        if canonical_digest(1, launch.permit())? != canonical_digest(1, &self.permit)?
            || launch.launch_digest() != check_bundle::launch_digest(isolation, argv)?
        {
            return Err(Error::new(
                "check_bundle_identity",
                "native launch differs from its admitted bundle member",
            ));
        }
        Ok(Box::new(isolation::start_bundle_command(
            isolation,
            argv,
            limits,
            self.journal.clone(),
            launch,
        )?))
    }

    fn evidence_class(&self) -> ExecutionEvidenceClass {
        ExecutionEvidenceClass::ConfinedExecution
    }
    fn preflight(&self, isolation: &ValidatedIsolation) -> Result<()> {
        isolation::repository_command_prerequisite(isolation)
    }
    fn start(
        &self,
        isolation: &ValidatedIsolation,
        argv: &[String],
        timeout: Duration,
        output_limit: usize,
    ) -> Result<Box<dyn StartedCommand>> {
        Ok(Box::new(isolation::start_journaled_command(
            isolation,
            argv,
            timeout,
            output_limit,
            &[],
            self.journal.clone(),
            self.permit.clone(),
        )?))
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

fn source_snapshot(
    git: &Git,
    boundary: &ValidatedIsolation,
    limits: &ExecutionLimits,
    outputs: &BTreeSet<PathBuf>,
) -> Result<SourceSnapshot> {
    boundary.revalidate()?;
    let root = &boundary.plan().repository;
    let mut paths = vec![root.clone()];
    let mut files = BTreeMap::new();
    let mut bytes = 0u64;
    while let Some(path) = paths.pop() {
        if path == root.join(".git") || outputs.contains(&path) {
            continue;
        }
        let meta =
            std::fs::symlink_metadata(&path).map_err(|e| Error::io("inspect actual source", e))?;
        if meta.is_dir() {
            for item in std::fs::read_dir(&path).map_err(|e| Error::io("read actual source", e))? {
                paths.push(item.map_err(|e| Error::io("read source entry", e))?.path());
            }
            if paths.len().saturating_add(files.len()) > limits.source_files {
                return Err(Error::new(
                    "source_limit",
                    "source file inventory exceeds bound",
                ));
            }
            continue;
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
        let relative = path
            .strip_prefix(root)
            .map_err(|_| Error::new("scope_violation", "source escaped its root"))?
            .to_str()
            .ok_or_else(|| Error::invalid("source path is not UTF-8"))?;
        files.insert(
            relative.into(),
            SourceFile {
                digest: bytes_digest(&data),
                mode,
                symlink: meta.is_symlink(),
            },
        );
    }
    let revision = git.revision(root, "HEAD")?;
    git.verify_execution_tree(root, &revision, outputs)?;
    Ok(SourceSnapshot { revision, files })
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactProof {
    pub path: PathBuf,
    pub digest: String,
    pub bytes: u64,
    pub validation_operation: Uuid,
    pub candidate_vector_digest: Option<String>,
}
impl ArtifactProof {
    pub fn reference(&self) -> ArtifactReference {
        ArtifactReference {
            reference: self.path.to_string_lossy().into_owned(),
            digest: self.digest.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ArtifactStamp {
    digest: String,
    inode: u64,
    changed_ns: i128,
}
fn artifact_stamp(path: &Path, limit: u64) -> Result<Option<ArtifactStamp>> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::io("inspect prior artifact", e)),
        Ok(meta) => {
            let digest = bytes_digest(&file_bytes(path, limit)?);
            #[cfg(unix)]
            let (inode, changed_ns) = {
                use std::os::unix::fs::MetadataExt;
                (
                    meta.ino(),
                    i128::from(meta.ctime()) * 1_000_000_000 + i128::from(meta.ctime_nsec()),
                )
            };
            #[cfg(not(unix))]
            let (inode, changed_ns) = (
                0,
                meta.modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_nanos() as i128),
            );
            Ok(Some(ArtifactStamp {
                digest,
                inode,
                changed_ns,
            }))
        }
    }
}

fn executable_role(boundary: &ValidatedIsolation) -> Result<()> {
    if !matches!(
        boundary.plan().role,
        ExecutionRole::Child | ExecutionRole::Validation
    ) {
        return Err(Error::new(
            "scope_violation",
            "only child execution and validation roles may run repository commands or hooks",
        ));
    }
    boundary.revalidate()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckExecutionRecord {
    pub schema_version: u32,
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
}

#[derive(Debug)]
pub struct FailedCheckExecution {
    record: CheckExecutionRecord,
    effective: Option<EffectiveExecutionBinding>,
}
impl FailedCheckExecution {
    pub fn effective_binding(&self) -> Option<&EffectiveExecutionBinding> {
        self.effective.as_ref()
    }
    pub fn record(&self) -> &CheckExecutionRecord {
        &self.record
    }
    /// A repair trigger needs a known failed command from the qualified boundary.
    /// Timeouts, output truncation and changed source are blockers, not repair proof.
    pub fn production_failure(&self) -> Result<&CheckExecutionRecord> {
        if self.record.evidence_class != ExecutionEvidenceClass::ConfinedExecution {
            return Err(Error::new(
                "unqualified_fixture",
                "fixture failure is not production repair evidence",
            ));
        }
        self.record.validate_shape()?;
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
        directory: &crate::storage::PrivateDir,
    ) -> Result<Vec<RetainedArtifact>> {
        VerifiedCheckExecution::retain_record_outputs(&self.record, boundary, directory)
    }
}
#[derive(Debug)]
pub enum CheckCompletion {
    Passed(VerifiedCheckExecution),
    Failed(FailedCheckExecution),
}

#[derive(Clone, Debug)]
pub struct RetainedArtifact {
    original: ArtifactProof,
    retained_paths: Vec<PathBuf>,
}
impl RetainedArtifact {
    pub(crate) fn verify_reference(&self, expected: &ArtifactProof) -> Result<()> {
        if &self.original != expected {
            return Err(Error::new(
                "artifact_conflict",
                "retained artifact identity differs from requested evidence",
            ));
        }
        self.verify(expected.bytes)
    }
    fn verify(&self, maximum: u64) -> Result<()> {
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
        Ok(())
    }
    pub fn retained_paths(&self) -> &[PathBuf] {
        &self.retained_paths
    }
}
impl VerifiedCheckExecution {
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
        if self.record.evidence_class != ExecutionEvidenceClass::ConfinedExecution {
            return Err(Error::new(
                "unqualified_fixture",
                "deterministic execution fixture is not actual validation evidence",
            ));
        }
        self.record.validate_shape()?;
        Ok(&self.record)
    }
    /// Preserve bytes outside every worker read/write root before permitting a
    /// named check to replace its own previous output. A digest alone is not
    /// artifact retention.
    pub fn retain_outputs(
        &self,
        boundary: &ValidatedIsolation,
        directory: &crate::storage::PrivateDir,
    ) -> Result<Vec<RetainedArtifact>> {
        Self::retain_record_outputs(&self.record, boundary, directory)
    }
    fn retain_record_outputs(
        record: &CheckExecutionRecord,
        boundary: &ValidatedIsolation,
        directory: &crate::storage::PrivateDir,
    ) -> Result<Vec<RetainedArtifact>> {
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
        let mut retained = Vec::new();
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
            retained.push(proof);
        }
        Ok(retained)
    }
}

pub struct CheckInvocation<'a> {
    pub binding: &'a ExecutionBinding,
    pub workflow: &'a Workflow,
    pub name: &'a str,
    pub kind: CheckKind,
    /// Host-declared extra outputs (e.g. E2E recording), fixed before dispatch.
    pub additional_outputs: &'a [PathBuf],
    /// Prior authenticated output records allow a repeated configured check to
    /// replace its own known artifact while preserving the old evidence record.
    pub previous_outputs: &'a [RetainedArtifact],
}

pub struct LocalExecutor<'a> {
    pub isolation: &'a ValidatedIsolation,
    pub git: &'a Git,
    pub boundary: &'a dyn CommandBoundary,
    pub limits: ExecutionLimits,
    pub ownership: &'a OwnedDirectory,
}
#[derive(Debug)]
pub struct PreparedCheck {
    binding: ExecutionBinding,
    effective: Option<EffectiveExecutionBinding>,
    workflow_digest: String,
    check_name: String,
    kind: CheckKind,
    resolved: crate::workflow::ResolvedCheck,
    outputs: BTreeSet<PathBuf>,
    prior: BTreeMap<PathBuf, Option<ArtifactStamp>>,
    before: SourceSnapshot,
    started_at: DateTime<Utc>,
    evidence_class: ExecutionEvidenceClass,
}
pub struct RunningCheck {
    prepared: PreparedCheck,
    command: Box<dyn StartedCommand>,
}
impl PreparedCheck {
    pub(crate) fn requires_bundle(&self) -> bool {
        !self.resolved.services.is_empty()
    }
}
impl RunningCheck {
    pub fn cancellation(&self) -> std::sync::Arc<dyn CommandCancellation> {
        self.command.cancellation()
    }
}
impl LocalExecutor<'_> {
    fn launch_identity(&self, binding: &ExecutionBinding) -> Result<()> {
        self.limits.validate()?;
        executable_role(self.isolation)?;
        binding.validate(self.isolation)?;
        self.ownership.verify(binding, self.isolation)?;
        Ok(())
    }
    fn preflight(&self, binding: &ExecutionBinding) -> Result<()> {
        self.launch_identity(binding)?;
        if self
            .git
            .revision(&self.isolation.plan().repository, "HEAD")?
            != binding.revision
        {
            return Err(Error::new(
                "stale_revision",
                "execution repository differs from the admitted revision",
            ));
        }
        Ok(())
    }
    pub async fn run_check(
        &self,
        invocation: CheckInvocation<'_>,
        persist: impl FnMut(&CheckJournalEntry) -> Result<()>,
    ) -> Result<VerifiedCheckExecution> {
        match self.run_check_complete(invocation, persist).await? {
            CheckCompletion::Passed(verified) => Ok(verified),
            CheckCompletion::Failed(failed) => Err(Error::new(
                "validation_failed",
                failed.record.blocker.unwrap_or_default(),
            )),
        }
    }
    /// Retain the private failed capability when retrying a known unsuccessful
    /// check, so its actual artifacts remain recoverable before replacement.
    pub async fn run_check_complete(
        &self,
        invocation: CheckInvocation<'_>,
        mut persist: impl FnMut(&CheckJournalEntry) -> Result<()>,
    ) -> Result<CheckCompletion> {
        let prepared = self.prepare_check(invocation, &mut persist)?;
        let operation_id = prepared.binding.operation_id;
        let running = match self.start_check(prepared) {
            Ok(running) => running,
            Err(error) => {
                persist(&CheckJournalEntry::Uncertain {
                    operation_id,
                    error_code: error.code.clone(),
                })?;
                return Err(error);
            }
        };
        self.complete_check(running, persist).await
    }
    /// Produces no process or effect. The host may perform expensive source
    /// inspection outside its control mutex, then atomically start under a fresh
    /// admission gate with `start_check`.
    pub fn prepare_check(
        &self,
        invocation: CheckInvocation<'_>,
        persist: impl FnMut(&CheckJournalEntry) -> Result<()>,
    ) -> Result<PreparedCheck> {
        self.prepare_check_inner(invocation, None, persist)
    }
    fn prepare_check_inner(
        &self,
        invocation: CheckInvocation<'_>,
        effective: Option<EffectiveExecutionBinding>,
        mut persist: impl FnMut(&CheckJournalEntry) -> Result<()>,
    ) -> Result<PreparedCheck> {
        let original = self.check_ownership_binding(invocation.binding, effective.as_ref())?;
        self.preflight(original)?;
        let checks = match invocation.kind {
            CheckKind::Local => &invocation.workflow.front_matter.checks.local,
            CheckKind::Integrated => &invocation.workflow.front_matter.checks.integrated,
            _ => {
                return Err(Error::invalid(
                    "local executor cannot manufacture hosted CI results",
                ));
            }
        };
        if invocation.kind == CheckKind::Integrated
            && invocation.binding.candidate_vector_digest.is_none()
        {
            return Err(Error::new(
                "candidate_provenance",
                "integrated checks require the exact candidate vector",
            ));
        }
        let configured = checks.get(invocation.name).ok_or_else(|| {
            Error::new(
                "unknown_check",
                "named check is not configured in this workflow",
            )
        })?;
        let resolved = invocation.workflow.resolve_check(
            configured,
            &self.isolation.plan().cwd,
            &[
                self.isolation.plan().repository.clone(),
                self.isolation.plan().scratch.clone(),
            ],
        )?;
        self.isolation.resolve_executable(&resolved.argv[0])?;
        let mut outputs = BTreeSet::new();
        for path in std::iter::once(&resolved.result_artifact).chain(invocation.additional_outputs)
        {
            let path = self.isolation.artifact(path)?;
            if !outputs.insert(path) {
                return Err(Error::invalid("duplicate declared check output"));
            }
        }
        let mut prior = BTreeMap::new();
        for retained in invocation.previous_outputs {
            retained.verify(self.limits.artifact_bytes)?;
        }
        for path in &outputs {
            let stamp = artifact_stamp(path, self.limits.artifact_bytes)?;
            if let Some(stamp) = &stamp
                && !invocation
                    .previous_outputs
                    .iter()
                    .any(|p| p.original.path == *path && p.original.digest == stamp.digest)
            {
                return Err(Error::new(
                    "artifact_conflict",
                    "preexisting output lacks its retained prior evidence; preserve it before running",
                ));
            }
            prior.insert(path.clone(), stamp);
        }
        let before = source_snapshot(self.git, self.isolation, &self.limits, &outputs)?;
        let started_at = Utc::now();
        persist(&CheckJournalEntry::Started {
            binding: Box::new(invocation.binding.clone()),
            correction: effective
                .as_ref()
                .map(EffectiveExecutionBinding::provenance)
                .transpose()?,
            workflow_digest: invocation.workflow.digest.clone(),
            check_name: invocation.name.into(),
            source_digest: before.digest()?,
            started_at,
        })?;
        Ok(PreparedCheck {
            binding: invocation.binding.clone(),
            effective,
            workflow_digest: invocation.workflow.digest.clone(),
            check_name: invocation.name.into(),
            kind: invocation.kind,
            resolved,
            outputs,
            prior,
            before,
            started_at,
            evidence_class: self.boundary.evidence_class(),
        })
    }
    /// Recheck known no-spawn failures before reserving the effect's owned slot.
    pub fn preflight_check(&self, prepared: &PreparedCheck) -> Result<()> {
        let original =
            self.check_ownership_binding(&prepared.binding, prepared.effective.as_ref())?;
        self.launch_identity(original)?;
        if self.boundary.evidence_class() != prepared.evidence_class {
            return Err(Error::new(
                "stale_revision",
                "check preparation changed before actual process start",
            ));
        }
        for path in &prepared.outputs {
            // Only bounded metadata checks occur inside the control gate. The
            // full source/byte snapshots are prepared and completed outside it.
            let current = match std::fs::symlink_metadata(path) {
                Ok(metadata) => {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::MetadataExt;
                        if !metadata.is_file() || metadata.nlink() != 1 {
                            return Err(Error::new(
                                "artifact_conflict",
                                "prepared output became linked or nonregular",
                            ));
                        }
                        Some((
                            metadata.ino(),
                            i128::from(metadata.ctime()) * 1_000_000_000
                                + i128::from(metadata.ctime_nsec()),
                        ))
                    }
                    #[cfg(not(unix))]
                    {
                        return Err(Error::new(
                            "isolation_unavailable",
                            "output identity requires the supported Unix host",
                        ));
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(Error::io("recheck prepared output identity", error)),
            };
            if current
                != prepared.prior[path]
                    .as_ref()
                    .map(|stamp| (stamp.inode, stamp.changed_ns))
            {
                return Err(Error::new(
                    "artifact_conflict",
                    "output changed between preparation and process start",
                ));
            }
        }
        self.boundary.preflight(self.isolation)
    }
    /// Synchronous actual process creation. The host persists sent/unknown and
    /// reserves cancellation ownership in the same serialized effect gate.
    pub fn start_check(&self, prepared: PreparedCheck) -> Result<RunningCheck> {
        if prepared.requires_bundle() {
            return Err(Error::new(
                "check_bundle_admission_required",
                "service checks require the original serialized bundle admission",
            ));
        }
        self.preflight_check(&prepared)?;
        let command = self.boundary.start(
            self.isolation,
            &prepared.resolved.argv,
            Duration::from_millis(self.limits.command_timeout_ms),
            self.limits.output_bytes,
        )?;
        Ok(RunningCheck { prepared, command })
    }
    /// Wait outside the control mutex. The host must revalidate/finalize its
    /// admitted effect after this returns and before exposing a passed capability.
    pub async fn complete_check(
        &self,
        running: RunningCheck,
        mut persist: impl FnMut(&CheckJournalEntry) -> Result<()>,
    ) -> Result<CheckCompletion> {
        let RunningCheck { prepared, command } = running;
        let command = match command.wait().await {
            Ok(result) => result,
            Err(error) => {
                persist(&CheckJournalEntry::Uncertain {
                    operation_id: prepared.binding.operation_id,
                    error_code: error.code.clone(),
                })?;
                return Err(error);
            }
        };
        self.complete_check_result(prepared, command, None, persist)
    }
    fn complete_check_result(
        &self,
        prepared: PreparedCheck,
        command: CommandOutcome,
        bundle: Option<CheckBundleExecutionRecord>,
        mut persist: impl FnMut(&CheckJournalEntry) -> Result<()>,
    ) -> Result<CheckCompletion> {
        let PreparedCheck {
            binding,
            effective,
            workflow_digest,
            check_name,
            kind,
            resolved,
            outputs,
            prior,
            before,
            started_at,
            evidence_class,
        } = prepared;
        let after = match self
            .check_ownership_binding(&binding, effective.as_ref())
            .and_then(|original| self.preflight(original))
            .and_then(|()| source_snapshot(self.git, self.isolation, &self.limits, &outputs))
        {
            Ok(snapshot) => snapshot,
            Err(error) => {
                persist(&CheckJournalEntry::Uncertain {
                    operation_id: binding.operation_id,
                    error_code: error.code.clone(),
                })?;
                return Err(error);
            }
        };
        let mut artifacts = Vec::new();
        let mut blocker = None;
        for path in &outputs {
            let actual = self.isolation.artifact(path)?;
            if actual != *path {
                return Err(Error::new(
                    "artifact_invalid",
                    "declared output was redirected through a changed symlink or ancestry",
                ));
            }
            let stamp = artifact_stamp(&actual, self.limits.artifact_bytes)?;
            match stamp {
                Some(stamp) if Some(&stamp) != prior[path].as_ref() => {
                    artifacts.push(ArtifactProof {
                        path: actual.clone(),
                        digest: stamp.digest,
                        bytes: std::fs::metadata(actual)
                            .map_err(|e| Error::io("inspect resulting artifact", e))?
                            .len(),
                        validation_operation: binding.operation_id,
                        candidate_vector_digest: binding.candidate_vector_digest.clone(),
                    })
                }
                _ => {
                    blocker = Some(
                        "configured artifact was absent or unchanged from an earlier execution"
                            .into(),
                    )
                }
            }
        }
        if before != after {
            blocker =
                Some("source content, mode, links or revision changed while checks ran".into());
        }
        if command.exit_code != Some(0) || command.timed_out || command.output_truncated {
            blocker = Some("check failed, timed out, or exceeded output bounds".into());
        }
        if command.argv != resolved.argv
            || command.cwd != resolved.cwd
            || command.isolation_digest != self.isolation.digest()
        {
            return Err(Error::new(
                "execution_identity",
                "command receipt differs from the admitted command boundary",
            ));
        }
        let record = CheckExecutionRecord {
            schema_version: if bundle.is_some() {
                3
            } else if effective.is_some() {
                2
            } else {
                1
            },
            bundle,
            correction: effective
                .as_ref()
                .map(EffectiveExecutionBinding::provenance)
                .transpose()?,
            binding,
            workflow_digest,
            check_name,
            kind,
            before,
            after,
            artifacts,
            command,
            started_at,
            completed_at: Utc::now(),
            evidence_class,
            successful: blocker.is_none(),
            blocker,
        };
        record.validate_shape()?;
        let entry = CheckJournalEntry::Finished(Box::new(record));
        persist(&entry)?;
        let CheckJournalEntry::Finished(record) = entry else {
            unreachable!()
        };
        if !record.successful {
            return Ok(CheckCompletion::Failed(FailedCheckExecution {
                record: *record,
                effective,
            }));
        }
        Ok(CheckCompletion::Passed(VerifiedCheckExecution {
            record: *record,
            effective,
        }))
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
impl HookLedger {
    pub fn new(binding: &ExecutionBinding) -> Result<Self> {
        Ok(Self {
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
        if !original.initialized
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
                && creation.evidence_class != ExecutionEvidenceClass::ConfinedExecution
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

pub struct HookInvocation<'a> {
    pub binding: &'a ExecutionBinding,
    pub workflow: &'a Workflow,
    pub phase: HookPhase,
    pub session: Option<Uuid>,
    /// Supplied only by the explicit cleanup dispatcher after every preservation
    /// predicate passes; its digest is retained and must be rechecked afterward.
    pub eligible_cleanup_plan: Option<&'a str>,
}
#[derive(Debug)]
pub struct HookResult {
    pub state: HookState,
    pub allows_next_phase: bool,
    pub diagnostic_only: bool,
}

#[derive(Debug)]
pub struct PreparedHook {
    binding: ExecutionBinding,
    key: String,
    record: HookRecord,
    argv: Option<Vec<String>>,
    timeout: Duration,
}
impl PreparedHook {
    pub fn requires_execution(&self) -> bool {
        self.argv.is_some()
    }
}
pub struct RunningHook {
    prepared: PreparedHook,
    command: Option<Box<dyn StartedCommand>>,
}
impl RunningHook {
    /// Skipped hooks have no process and therefore no cancellation handle.
    pub fn cancellation(&self) -> Option<std::sync::Arc<dyn CommandCancellation>> {
        self.command.as_ref().map(|command| command.cancellation())
    }
}

impl LocalExecutor<'_> {
    pub async fn run_hook(
        &self,
        ledger: &mut HookLedger,
        invocation: HookInvocation<'_>,
        mut persist: impl FnMut(&HookLedger) -> Result<()>,
    ) -> Result<HookResult> {
        let prepared = self.prepare_hook(ledger, invocation, &mut persist)?;
        let key = prepared.key.clone();
        let phase = prepared.record.phase;
        let running = match self.start_hook(prepared) {
            Ok(running) => running,
            Err(error) => {
                let mut next = ledger.clone();
                let record = next.records.get_mut(&key).ok_or_else(|| {
                    Error::new("ownership_conflict", "prepared hook record disappeared")
                })?;
                record.error_code = Some(error.code);
                let state = record.state;
                ledger.save(next, &mut persist)?;
                return Ok(HookResult {
                    state,
                    allows_next_phase: false,
                    diagnostic_only: phase == HookPhase::AfterRun,
                });
            }
        };
        self.complete_hook(ledger, running, persist).await
    }

    /// Resolve and persist the attempt before entering the short control gate.
    /// Preparation starts no process. A persisted StartedUnknown attempt must
    /// never be replayed after interruption without explicit reconciliation.
    pub fn prepare_hook(
        &self,
        ledger: &mut HookLedger,
        invocation: HookInvocation<'_>,
        mut persist: impl FnMut(&HookLedger) -> Result<()>,
    ) -> Result<PreparedHook> {
        self.preflight(invocation.binding)?;
        if ledger.binding_digest != canonical_digest(1, invocation.binding)? {
            return Err(Error::new(
                "execution_identity",
                "hook ledger belongs to another admitted execution",
            ));
        }
        let key = if invocation.phase == HookPhase::BeforeRemove {
            format!(
                "BeforeRemove:{}",
                invocation.eligible_cleanup_plan.unwrap_or("missing-plan")
            )
        } else {
            hook_key(invocation.phase, invocation.session)
        };
        if ledger.records.contains_key(&key) {
            return Err(Error::new(
                "reconciliation_required",
                "hook already has a recorded attempt; do not replay partial side effects",
            ));
        }
        match invocation.phase {
            HookPhase::AfterCreate if invocation.session.is_none() && !ledger.initialized => {}
            HookPhase::BeforeRun
                if ledger.initialized
                    && invocation
                        .session
                        .is_some_and(|id| !id.is_nil() && !ledger.sessions.contains_key(&id)) => {}
            HookPhase::AfterRun
                if invocation.session.is_some_and(|id| {
                    ledger.sessions.get(&id) == Some(&SessionLifecycle::Ended)
                }) => {}
            HookPhase::BeforeRemove
                if invocation.session.is_none()
                    && invocation.eligible_cleanup_plan.is_some()
                    && !ledger
                        .sessions
                        .values()
                        .any(|s| *s == SessionLifecycle::Launched)
                    && !ledger
                        .records
                        .values()
                        .any(|r| r.state == HookState::StartedUnknown) => {}
            _ => {
                return Err(Error::new(
                    "hook_gate",
                    "hook phase is inconsistent with creation, actual session lifecycle or explicit cleanup eligibility",
                ));
            }
        }
        if let Some(digest) = invocation.eligible_cleanup_plan {
            validate_digest(digest)?;
        }
        let hooks = &invocation.workflow.front_matter.hooks;
        let script = match invocation.phase {
            HookPhase::AfterCreate => &hooks.after_create,
            HookPhase::BeforeRun => &hooks.before_run,
            HookPhase::AfterRun => &hooks.after_run,
            HookPhase::BeforeRemove => &hooks.before_remove,
        };
        if script.is_some() {
            if hooks.timeout_ms == 0 || hooks.timeout_ms > 86_400_000 {
                return Err(Error::invalid(
                    "hook timeout is outside finite supported bounds",
                ));
            }
            self.isolation.resolve_executable("/bin/sh")?;
        }
        let record = HookRecord {
            phase: invocation.phase,
            session: invocation.session,
            workflow_digest: invocation.workflow.digest.clone(),
            script_digest: script.as_ref().map(|s| bytes_digest(s.as_bytes())),
            cleanup_plan: invocation.eligible_cleanup_plan.map(str::to_owned),
            state: if script.is_some() {
                HookState::StartedUnknown
            } else {
                HookState::Skipped
            },
            exit_code: None,
            timed_out: false,
            stdout: String::new(),
            stderr: String::new(),
            error_code: None,
            evidence_class: self.boundary.evidence_class(),
        };
        let mut next = ledger.clone();
        next.records.insert(key.clone(), record.clone());
        if script.is_none() && invocation.phase == HookPhase::AfterCreate {
            next.initialized = true;
        }
        ledger.save(next, &mut persist)?;
        Ok(PreparedHook {
            binding: invocation.binding.clone(),
            key,
            record,
            argv: script
                .as_ref()
                .map(|script| vec!["/bin/sh".into(), "-c".into(), script.clone()]),
            timeout: Duration::from_millis(hooks.timeout_ms),
        })
    }

    /// A skipped hook has no native prerequisite and cannot start a process.
    pub fn preflight_hook(&self, prepared: &PreparedHook) -> Result<()> {
        self.launch_identity(&prepared.binding)?;
        if self.boundary.evidence_class() != prepared.record.evidence_class {
            return Err(Error::new(
                "execution_identity",
                "hook execution boundary changed after preparation",
            ));
        }
        if prepared.requires_execution() {
            self.boundary.preflight(self.isolation)?;
        }
        Ok(())
    }
    /// Start inside the durable effect gate after reserving cancellation ownership.
    pub fn start_hook(&self, prepared: PreparedHook) -> Result<RunningHook> {
        self.preflight_hook(&prepared)?;
        let command = prepared
            .argv
            .as_ref()
            .map(|argv| {
                self.boundary.start(
                    self.isolation,
                    argv,
                    prepared.timeout,
                    self.limits.output_bytes,
                )
            })
            .transpose()?;
        Ok(RunningHook { prepared, command })
    }

    /// Wait outside the control mutex and retain observed/unknown hook effects.
    /// The host must finalize its effect against current authority afterward.
    pub async fn complete_hook(
        &self,
        ledger: &mut HookLedger,
        running: RunningHook,
        mut persist: impl FnMut(&HookLedger) -> Result<()>,
    ) -> Result<HookResult> {
        let RunningHook { prepared, command } = running;
        let PreparedHook {
            binding,
            key,
            mut record,
            argv,
            ..
        } = prepared;
        let expected_record = canonical_digest(1, &record)?;
        if ledger.binding_digest != canonical_digest(1, &binding)?
            || ledger
                .records
                .get(&key)
                .map(|record| canonical_digest(1, record))
                .transpose()?
                .as_deref()
                != Some(expected_record.as_str())
        {
            return Err(Error::new(
                "ownership_conflict",
                "hook attempt changed before completion",
            ));
        }
        if let Some(command) = command {
            match command.wait().await {
                Err(error) => record.error_code = Some(error.code),
                Ok(outcome) => {
                    record.exit_code = outcome.exit_code;
                    record.timed_out = outcome.timed_out;
                    record.stdout = outcome.stdout;
                    record.stderr = outcome.stderr;
                    let matching = argv.as_ref() == Some(&outcome.argv)
                        && outcome.cwd == self.isolation.plan().cwd
                        && outcome.isolation_digest == self.isolation.digest();
                    record.state = if outcome.timed_out || !matching {
                        HookState::StartedUnknown
                    } else if outcome.exit_code == Some(0) && !outcome.output_truncated {
                        HookState::Succeeded
                    } else {
                        HookState::Failed
                    };
                    if !matching {
                        record.error_code = Some("execution_identity".into());
                    }
                }
            }
            if let Err(error) = self.preflight(&binding) {
                record.state = HookState::StartedUnknown;
                record.error_code = Some(error.code);
            }
            let mut next = ledger.clone();
            if record.phase == HookPhase::AfterCreate && record.state == HookState::Succeeded {
                next.initialized = true;
            }
            next.records.insert(key, record.clone());
            ledger.save(next, &mut persist)?;
        }
        let passed = matches!(record.state, HookState::Succeeded | HookState::Skipped);
        Ok(HookResult {
            state: record.state,
            allows_next_phase: passed,
            diagnostic_only: record.phase == HookPhase::AfterRun,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GitInspection {
    Status,
    Diff { staged: bool },
    ShowFile { commit: String, path: String },
}
fn source_relative(path: &str) -> Result<()> {
    if path.is_empty()
        || Path::new(path).is_absolute()
        || path.contains(['\0', '\\'])
        || Path::new(path)
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
        || path.split('/').any(|p| p.eq_ignore_ascii_case(".git"))
    {
        return Err(Error::new(
            "scope_violation",
            "source path must be relative and exclude Git administration",
        ));
    }
    Ok(())
}
impl LocalExecutor<'_> {
    pub async fn inspect_git(
        &self,
        binding: &ExecutionBinding,
        git_executable: &Path,
        inspection: GitInspection,
    ) -> Result<CommandOutcome> {
        self.limits.validate()?;
        self.isolation.revalidate()?;
        self.ownership.verify(binding, self.isolation)?;
        binding.validate(self.isolation)?;
        if std::fs::canonicalize(git_executable)
            .map_err(|e| Error::io("resolve selected Git tool", e))?
            != self.git.executable()
        {
            return Err(Error::new(
                "identity_mismatch",
                "inspection Git differs from admitted pinned tool",
            ));
        }
        let mut result = self
            .git
            .inspect_read_only(&self.isolation.plan().repository, &inspection)?;
        result.isolation_digest = self.isolation.digest().into();
        self.ownership.verify(binding, self.isolation)?;
        Ok(result)
    }
}

pub struct SourceWrite<'a> {
    pub relative_path: &'a str,
    pub expected_digest: Option<&'a str>,
    pub bytes: &'a [u8],
    pub executable: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceWriteReceipt {
    pub operation_id: Uuid,
    pub path: PathBuf,
    pub prior_digest: Option<String>,
    pub resulting_digest: String,
    pub executable: bool,
    pub completed: bool,
}
impl LocalExecutor<'_> {
    /// Narrow host file tool: exact path and expected bytes, no shell and no
    /// access to Git administration. The control host serializes this with all
    /// other effects for the same admitted execution.
    pub fn write_source(
        &self,
        binding: &ExecutionBinding,
        write: SourceWrite<'_>,
        mut persist: impl FnMut(&SourceWriteReceipt) -> Result<()>,
    ) -> Result<SourceWriteReceipt> {
        // The worker invokes this inside its serialized mutation gate. Source
        // writes validate exact path/content CAS here; Git reads belong outside
        // that gate and no validation capability is minted by this write.
        self.launch_identity(binding)?;
        if self.isolation.plan().role != ExecutionRole::Child
            || write.bytes.len() as u64 > self.limits.source_bytes
        {
            return Err(Error::new(
                "scope_violation",
                "source writes require an admitted child and bounded bytes",
            ));
        }
        source_relative(write.relative_path)?;
        let path = isolation::canonical_target(
            &self.isolation.plan().repository.join(write.relative_path),
        )?;
        if !path.starts_with(&self.isolation.plan().repository)
            || !self
                .isolation
                .plan()
                .source_writes
                .iter()
                .any(|g| path == g.path || (g.directory && path.starts_with(&g.path)))
        {
            return Err(Error::new(
                "scope_violation",
                "source file is outside this child's exact write grants",
            ));
        }
        let prior = artifact_stamp(&path, self.limits.source_bytes)?.map(|s| s.digest);
        if prior.as_deref() != write.expected_digest {
            return Err(Error::new(
                "source_conflict",
                "source changed from the tool's expected content; preserve and reread it",
            ));
        }
        let mut record = SourceWriteReceipt {
            operation_id: binding.operation_id,
            path: path.clone(),
            prior_digest: prior,
            resulting_digest: bytes_digest(write.bytes),
            executable: write.executable,
            completed: false,
        };
        persist(&record)?;
        let parent = path
            .parent()
            .ok_or_else(|| Error::invalid("source parent missing"))?;
        replace_source_at(parent, &path, write.bytes, write.executable, || {
            self.preflight(binding)?;
            if isolation::canonical_target(&path)? != path
                || artifact_stamp(&path, self.limits.source_bytes)?
                    .map(|s| s.digest)
                    .as_ref()
                    != record.prior_digest.as_ref()
            {
                return Err(Error::new(
                    "source_conflict",
                    "source changed before replacement; staged temporary file retained",
                ));
            }
            Ok(())
        })?;
        record.completed = true;
        persist(&record)?;
        Ok(record)
    }
}

#[cfg(unix)]
fn replace_source_at(
    parent: &Path,
    path: &Path,
    bytes: &[u8],
    executable: bool,
    precommit: impl FnOnce() -> Result<()>,
) -> Result<()> {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let directory = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(parent)
        .map_err(|e| Error::io("open admitted source parent", e))?;
    let temporary = std::ffi::CString::new(format!(".kegari-write-{}", Uuid::new_v4()))
        .map_err(|_| Error::invalid("temporary leaf invalid"))?;
    let leaf = std::ffi::CString::new(
        path.file_name()
            .ok_or_else(|| Error::invalid("source leaf missing"))?
            .as_encoded_bytes(),
    )
    .map_err(|_| Error::invalid("source leaf contains NUL"))?;
    let mode = if executable { 0o755 } else { 0o644 };
    // SAFETY: both C strings are single leaves; the live directory descriptor
    // fixes ancestry, and CREATE|EXCL|NOFOLLOW cannot adopt an existing alias.
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            temporary.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            mode,
        )
    };
    if fd < 0 {
        return Err(Error::io(
            "create admitted source write",
            std::io::Error::last_os_error(),
        ));
    }
    // SAFETY: successful openat returns one newly owned file descriptor.
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    // SAFETY: fchmod acts only on our live newly created descriptor.
    if unsafe { libc::fchmod(file.as_raw_fd(), mode) } != 0 {
        return Err(Error::io(
            "set source file mode",
            std::io::Error::last_os_error(),
        ));
    }
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| Error::io("persist admitted source write", e))?;
    precommit()?;
    let held = directory
        .metadata()
        .map_err(|e| Error::io("inspect held source parent", e))?;
    let actual =
        std::fs::symlink_metadata(parent).map_err(|e| Error::io("recheck source parent", e))?;
    if !actual.is_dir() || held.dev() != actual.dev() || held.ino() != actual.ino() {
        return Err(Error::new(
            "source_conflict",
            "source parent changed before replacement; temporary bytes retained",
        ));
    }
    // SAFETY: both leaves are relative to the same verified live directory.
    if unsafe {
        libc::renameat(
            directory.as_raw_fd(),
            temporary.as_ptr(),
            directory.as_raw_fd(),
            leaf.as_ptr(),
        )
    } != 0
    {
        return Err(Error::io(
            "replace admitted source file",
            std::io::Error::last_os_error(),
        ));
    }
    directory
        .sync_all()
        .map_err(|e| Error::io("sync admitted source parent", e))
}
#[cfg(not(unix))]
fn replace_source_at(
    _: &Path,
    _: &Path,
    _: &[u8],
    _: bool,
    _: impl FnOnce() -> Result<()>,
) -> Result<()> {
    Err(Error::new(
        "isolation_unavailable",
        "scoped source writes require the supported Unix directory-descriptor boundary",
    ))
}

#[derive(Clone, Debug)]
pub struct CaptureOrigin {
    artifact: ArtifactProof,
    check_digest: String,
    binding: ExecutionBinding,
    source_digest: String,
    outputs: BTreeSet<PathBuf>,
    evidence_class: ExecutionEvidenceClass,
}
impl VerifiedCheckExecution {
    /// The host declared this output before executing the E2E route. Supplying
    /// an arbitrary preexisting video path cannot manufacture this capability.
    pub fn capture_origin(&self, path: &Path) -> Result<CaptureOrigin> {
        if self.record.kind != CheckKind::Integrated
            || self.record.binding.candidate_vector_digest.is_none()
        {
            return Err(Error::new(
                "video_provenance",
                "recordings must originate in successful integrated candidate execution",
            ));
        }
        let artifact = self
            .record
            .artifacts
            .iter()
            .find(|a| a.path == path)
            .ok_or_else(|| {
                Error::new(
                    "video_provenance",
                    "recording was not a declared fresh output of this integrated check",
                )
            })?;
        Ok(CaptureOrigin {
            artifact: artifact.clone(),
            check_digest: self.digest()?,
            binding: self.record.binding.clone(),
            source_digest: self.record.after.digest()?,
            outputs: self
                .record
                .artifacts
                .iter()
                .map(|a| a.path.clone())
                .collect(),
            evidence_class: self.record.evidence_class,
        })
    }
}
#[derive(Clone, Debug)]
pub struct MediaTools {
    pub ffprobe: PathBuf,
    pub ffmpeg: PathBuf,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaValidationRecord {
    pub schema_version: u32,
    pub validation_operation: Uuid,
    pub candidate_vector_digest: String,
    pub originating_check_digest: String,
    pub originating_artifact: ArtifactProof,
    pub output: ArtifactProof,
    pub duration_seconds: f64,
    pub width: u64,
    pub height: u64,
    pub video_codec: String,
    pub audio_codec: Option<String>,
    pub ffprobe_binary_digest: String,
    pub ffmpeg_binary_digest: String,
    pub probe_receipt_digest: String,
    pub decode_receipt_digest: String,
    pub conversion_receipt_digest: Option<String>,
    pub evidence_class: ExecutionEvidenceClass,
}
#[derive(Debug)]
pub struct VerifiedVideo {
    record: MediaValidationRecord,
}

#[derive(Debug, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum MediaJournalEntry {
    Started {
        operation_id: Uuid,
        originating_check_digest: String,
        source: PathBuf,
        output: PathBuf,
    },
    Finished(Box<MediaValidationRecord>),
}
impl VerifiedVideo {
    pub fn record(&self) -> &MediaValidationRecord {
        &self.record
    }
    pub fn production_artifact(&self) -> Result<&ArtifactProof> {
        if self.record.evidence_class != ExecutionEvidenceClass::ConfinedExecution {
            return Err(Error::new(
                "unqualified_fixture",
                "simulated media processing cannot establish a live recording",
            ));
        }
        Ok(&self.record.output)
    }
}
fn command_succeeded(
    result: &CommandOutcome,
    argv: &[String],
    boundary: &ValidatedIsolation,
) -> Result<()> {
    if result.exit_code != Some(0)
        || result.timed_out
        || result.output_truncated
        || result.argv != argv
        || result.cwd != boundary.plan().cwd
        || result.isolation_digest != boundary.digest()
    {
        return Err(Error::new(
            "media_invalid",
            "bounded media command failed or does not match its confined invocation",
        ));
    }
    Ok(())
}
fn standard_mp4(bytes: &[u8]) -> bool {
    if bytes.len() < 16 || &bytes[4..8] != b"ftyp" {
        return false;
    }
    let size = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
    if size < 16 || size > bytes.len() {
        return false;
    }
    let brands: Vec<&[u8]> = std::iter::once(&bytes[8..12])
        .chain(
            bytes[16..size]
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| b.as_slice()),
        )
        .collect();
    brands.iter().any(|b| {
        [
            b"isom".as_slice(),
            b"iso2",
            b"iso3",
            b"iso4",
            b"iso5",
            b"iso6",
            b"mp41",
            b"mp42",
            b"avc1",
            b"M4V ",
        ]
        .contains(b)
    })
}
struct ProbeSummary {
    duration: f64,
    width: u64,
    height: u64,
    audio: Option<String>,
}
fn validate_probe(bytes: &str, limits: &ExecutionLimits) -> Result<ProbeSummary> {
    let value: Value = serde_json::from_str(bytes).map_err(|_| {
        Error::new(
            "media_invalid",
            "ffprobe returned malformed structured metadata",
        )
    })?;
    let streams = value
        .get("streams")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::new("media_invalid", "recording lacks stream metadata"))?;
    let video: Vec<_> = streams
        .iter()
        .filter(|s| s.get("codec_type").and_then(Value::as_str) == Some("video"))
        .collect();
    let audio: Vec<_> = streams
        .iter()
        .filter(|s| s.get("codec_type").and_then(Value::as_str) == Some("audio"))
        .collect();
    let duration = value
        .pointer("/format/duration")
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<f64>().ok())
        .ok_or_else(|| Error::new("media_invalid", "recording duration is unavailable"))?;
    if video.len() != 1
        || audio.len() > 1
        || streams.len() != video.len() + audio.len()
        || !duration.is_finite()
        || duration <= 0.0
        || duration > limits.media_seconds
        || video[0].get("codec_name").and_then(Value::as_str) != Some("h264")
        || audio
            .iter()
            .any(|s| s.get("codec_name").and_then(Value::as_str) != Some("aac"))
        || !value
            .pointer("/format/format_name")
            .and_then(Value::as_str)
            .is_some_and(|n| n.split(',').any(|n| n == "mp4"))
    {
        return Err(Error::new(
            "media_invalid",
            "recording must be bounded MP4 with one H.264 video and optional AAC audio",
        ));
    }
    let width = video[0].get("width").and_then(Value::as_u64).unwrap_or(0);
    let height = video[0].get("height").and_then(Value::as_u64).unwrap_or(0);
    if width == 0 || height == 0 || width > 16384 || height > 16384 {
        return Err(Error::new(
            "media_invalid",
            "recording has invalid or excessive dimensions",
        ));
    }
    Ok(ProbeSummary {
        duration,
        width,
        height,
        audio: (!audio.is_empty()).then(|| "aac".into()),
    })
}
impl LocalExecutor<'_> {
    /// Optional conversion preserves the entire captured flow. No generated test
    /// pattern, duration truncation, external media URL or overwriting is offered.
    /// Remote upload/association/playback remain separate provider gates.
    pub async fn validate_video(
        &self,
        origin: &CaptureOrigin,
        tools: &MediaTools,
        converted_output: Option<&Path>,
        mut persist: impl FnMut(&MediaJournalEntry) -> Result<()>,
    ) -> Result<VerifiedVideo> {
        self.preflight(&origin.binding)?;
        if source_snapshot(self.git, self.isolation, &self.limits, &origin.outputs)?.digest()?
            != origin.source_digest
        {
            return Err(Error::new(
                "video_provenance",
                "source changed since the integrated recording was produced",
            ));
        }
        let source = self.isolation.artifact(&origin.artifact.path)?;
        if source != origin.artifact.path {
            return Err(Error::new(
                "video_provenance",
                "recording path ancestry changed since capture",
            ));
        }
        if bytes_digest(&file_bytes(&source, self.limits.artifact_bytes)?) != origin.artifact.digest
        {
            return Err(Error::new(
                "video_provenance",
                "recording bytes differ from the observed E2E output",
            ));
        }
        let probe = tools
            .ffprobe
            .to_str()
            .ok_or_else(|| Error::invalid("ffprobe path must be UTF-8"))?;
        let ffmpeg = tools
            .ffmpeg
            .to_str()
            .ok_or_else(|| Error::invalid("ffmpeg path must be UTF-8"))?;
        let probe_path = self.isolation.resolve_executable(probe)?;
        let ffmpeg_path = self.isolation.resolve_executable(ffmpeg)?;
        let mut conversion_receipt_digest = None;
        let planned_output = match converted_output {
            Some(path) => self.isolation.artifact(path)?,
            None => source.clone(),
        };
        persist(&MediaJournalEntry::Started {
            operation_id: origin.binding.operation_id,
            originating_check_digest: origin.check_digest.clone(),
            source: source.clone(),
            output: planned_output,
        })?;
        let output = if let Some(path) = converted_output {
            let target = self.isolation.artifact(path)?;
            if !target.starts_with(&self.isolation.plan().scratch) || target.exists() {
                return Err(Error::new(
                    "artifact_conflict",
                    "converted recording requires a new owned scratch output",
                ));
            }
            let argv = vec![
                ffmpeg.into(),
                "-v".into(),
                "error".into(),
                "-nostdin".into(),
                "-xerror".into(),
                "-i".into(),
                source.to_string_lossy().into_owned(),
                "-map".into(),
                "0:v:0".into(),
                "-map".into(),
                "0:a?".into(),
                "-c:v".into(),
                "libx264".into(),
                "-pix_fmt".into(),
                "yuv420p".into(),
                "-c:a".into(),
                "aac".into(),
                "-movflags".into(),
                "+faststart".into(),
                "-f".into(),
                "mp4".into(),
                "-n".into(),
                target.to_string_lossy().into_owned(),
            ];
            let result = self
                .boundary
                .run(
                    self.isolation,
                    &argv,
                    Duration::from_millis(self.limits.media_timeout_ms),
                    self.limits.output_bytes,
                )
                .await?;
            command_succeeded(&result, &argv, self.isolation)?;
            conversion_receipt_digest = Some(canonical_digest(1, &result)?);
            target
        } else {
            source.clone()
        };
        let bytes = file_bytes(&output, self.limits.artifact_bytes)?;
        if !standard_mp4(&bytes) {
            return Err(Error::new(
                "media_invalid",
                "recording bytes do not identify a standard MP4 container",
            ));
        }
        let digest = bytes_digest(&bytes);
        let probe_args = vec![
            probe.into(),
            "-v".into(),
            "error".into(),
            "-show_streams".into(),
            "-show_format".into(),
            "-of".into(),
            "json".into(),
            output.to_string_lossy().into_owned(),
        ];
        let probe_result = self
            .boundary
            .run(
                self.isolation,
                &probe_args,
                Duration::from_millis(self.limits.media_timeout_ms),
                self.limits.output_bytes,
            )
            .await?;
        command_succeeded(&probe_result, &probe_args, self.isolation)?;
        let info = validate_probe(&probe_result.stdout, &self.limits)?;
        let decode_args = vec![
            ffmpeg.into(),
            "-v".into(),
            "error".into(),
            "-nostdin".into(),
            "-xerror".into(),
            "-err_detect".into(),
            "explode".into(),
            "-i".into(),
            output.to_string_lossy().into_owned(),
            "-map".into(),
            "0:v:0".into(),
            "-map".into(),
            "0:a?".into(),
            "-f".into(),
            "null".into(),
            "-".into(),
        ];
        let decoded = self
            .boundary
            .run(
                self.isolation,
                &decode_args,
                Duration::from_millis(self.limits.media_timeout_ms),
                self.limits.output_bytes,
            )
            .await?;
        command_succeeded(&decoded, &decode_args, self.isolation)?;
        if bytes_digest(&file_bytes(&output, self.limits.artifact_bytes)?) != digest
            || bytes_digest(&file_bytes(&source, self.limits.artifact_bytes)?)
                != origin.artifact.digest
        {
            return Err(Error::new(
                "video_provenance",
                "recording changed while media validation ran",
            ));
        }
        self.preflight(&origin.binding)?;
        let mut validated_outputs = origin.outputs.clone();
        validated_outputs.insert(output.clone());
        if source_snapshot(self.git, self.isolation, &self.limits, &validated_outputs)?.digest()?
            != origin.source_digest
        {
            return Err(Error::new(
                "video_provenance",
                "source changed while media validation ran",
            ));
        }
        let record = MediaValidationRecord {
            schema_version: 1,
            validation_operation: origin.binding.operation_id,
            candidate_vector_digest: origin
                .binding
                .candidate_vector_digest
                .clone()
                .ok_or_else(|| Error::new("video_provenance", "candidate identity missing"))?,
            originating_check_digest: origin.check_digest.clone(),
            originating_artifact: origin.artifact.clone(),
            output: ArtifactProof {
                path: output,
                digest,
                bytes: bytes.len() as u64,
                validation_operation: origin.binding.operation_id,
                candidate_vector_digest: origin.binding.candidate_vector_digest.clone(),
            },
            duration_seconds: info.duration,
            width: info.width,
            height: info.height,
            video_codec: "h264".into(),
            audio_codec: info.audio,
            ffprobe_binary_digest: isolation::artifact_digest(&probe_path, 268_435_456)?,
            ffmpeg_binary_digest: isolation::artifact_digest(&ffmpeg_path, 268_435_456)?,
            probe_receipt_digest: canonical_digest(1, &probe_result)?,
            decode_receipt_digest: canonical_digest(1, &decoded)?,
            conversion_receipt_digest,
            evidence_class: if origin.evidence_class == ExecutionEvidenceClass::ConfinedExecution
                && self.boundary.evidence_class() == ExecutionEvidenceClass::ConfinedExecution
            {
                ExecutionEvidenceClass::ConfinedExecution
            } else {
                ExecutionEvidenceClass::DeterministicFixture
            },
        };
        let entry = MediaJournalEntry::Finished(Box::new(record));
        persist(&entry)?;
        let MediaJournalEntry::Finished(record) = entry else {
            unreachable!()
        };
        Ok(VerifiedVideo { record: *record })
    }
}

impl CommandCancellation for crate::git::GitCancellation {
    fn cancel(&self) -> Result<()> {
        crate::git::GitCancellation::cancel(self)
    }
    fn exited(&self) -> Result<bool> {
        crate::git::GitCancellation::exited(self)
    }
}
