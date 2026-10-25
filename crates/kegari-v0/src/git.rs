//! Deterministic Git mechanics for reviewed child deltas and isolated candidates.
//!
//! This module never pushes, fetches a network URL, repairs a patch, resets an
//! existing checkout, or deletes a failed candidate. Callers must hold the host
//! admission permit and the repository integration/release lock before effects.

use crate::digest::{bytes_digest, canonical_digest};
use crate::error::{Error, Result};
use crate::evidence::VerifiedAcceptedCandidate;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output, Stdio};
use uuid::Uuid;

pub const SUPPORTED_GIT_VERSION: &str = "git version 2.56.0";
pub const DELTA_SCHEMA: u32 = 1;

#[path = "git_acquired_clone.rs"]
mod acquired_clone;
#[path = "git_object_import.rs"]
mod object_import;
pub(crate) use object_import::retained_object_read_root;

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryId {
    pub hostname: String,
    pub id: String,
}

impl RepositoryId {
    pub fn validate(&self) -> Result<()> {
        if self.hostname.is_empty()
            || self.hostname != self.hostname.to_ascii_lowercase()
            || self.hostname.contains(['/', '\\', ':', ' ', '\0'])
            || self.id.trim().is_empty()
            || self.id.chars().any(char::is_control)
        {
            return Err(Error::invalid(
                "repository requires canonical hostname and stable ID",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Revision {
    pub commit: String,
    pub tree: String,
}

impl Revision {
    pub fn validate(&self) -> Result<()> {
        validate_oid(&self.commit)?;
        validate_oid(&self.tree)
    }
}

pub fn validate_oid(value: &str) -> Result<()> {
    if !matches!(value.len(), 40 | 64)
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(Error::new(
            "invalid_revision",
            "an exact lowercase Git object ID is required",
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BranchName {
    pub title_digest: String,
    pub suffix: String,
    pub name: String,
}

/// The title is never shortened or transliterated. Limits are checked on the
/// finished UTF-8 name, so an unsupported name is a blocker, not a silent rename.
pub fn title_suffix(title: &str) -> Result<String> {
    let replaced: String = title
        .chars()
        .map(|c| {
            if c.is_whitespace()
                || c.is_control()
                || matches!(c, '/' | '\\' | '~' | '^' | ':' | '?' | '*' | '[')
            {
                '-'
            } else {
                c
            }
        })
        .collect();
    let replaced = replaced.replace("@{", "@-");
    let mut dots = String::new();
    let mut chars = replaced.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '.' && chars.peek() == Some(&'.') {
            while chars.peek() == Some(&'.') {
                chars.next();
            }
            dots.push('-');
        } else {
            dots.push(c);
        }
    }
    if dots.ends_with(".lock") {
        dots.truncate(dots.len() - 5);
        dots.push_str("-lock");
    }
    let mut collapsed = String::new();
    for c in dots.chars() {
        if c != '-' || !collapsed.ends_with('-') {
            collapsed.push(c);
        }
    }
    let mut suffix = collapsed.trim_matches(['.', '-']).to_string();
    // Trimming separators can expose a terminal .lock suffix.
    if suffix.ends_with(".lock") {
        suffix.truncate(suffix.len() - 5);
        suffix.push_str("-lock");
    }
    if suffix.is_empty() {
        return Err(Error::new(
            "invalid_branch_name",
            "parent title has no representable Git suffix",
        ));
    }
    Ok(suffix)
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WriteScope {
    /// `.` means the whole assigned repository. A trailing slash is a directory
    /// prefix; every other entry is an exact file, never a glob.
    pub paths: Vec<String>,
}

impl WriteScope {
    pub fn validate(&self) -> Result<()> {
        if self.paths.is_empty() {
            return Err(Error::invalid("a writing child needs explicit write paths"));
        }
        let mut seen = BTreeSet::new();
        for path in &self.paths {
            if !seen.insert(path) {
                return Err(Error::invalid("duplicate write path"));
            }
            if path == "." {
                continue;
            }
            validate_repo_path(path.trim_end_matches('/'))?;
            if path.contains(['*', '?', '[']) {
                return Err(Error::invalid(
                    "write paths are exact files or directory prefixes, not globs",
                ));
            }
        }
        Ok(())
    }

    pub fn permits(&self, path: &str) -> bool {
        validate_repo_path(path).is_ok()
            && self.paths.iter().any(|allowed| {
                allowed == "."
                    || allowed == path
                    || (allowed.ends_with('/') && path.starts_with(allowed))
            })
    }
}

fn validate_repo_path(path: &str) -> Result<()> {
    if path.is_empty()
        || path.contains(['\0', '\\'])
        || Path::new(path).is_absolute()
        || Path::new(path).components().any(|c| {
            !matches!(c, Component::Normal(_))
                || matches!(c, Component::Normal(s) if s.as_encoded_bytes().eq_ignore_ascii_case(b".git"))
        })
    {
        return Err(Error::new("scope_violation", "unsafe repository-relative path"));
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeltaManifest {
    pub schema_version: u32,
    pub git_version: String,
    pub repository: RepositoryId,
    pub parent_id: Uuid,
    pub child_id: Uuid,
    pub work_generation: String,
    pub correction_generation: String,
    pub scope: WriteScope,
    pub validated_base: Revision,
    pub validated_head: Revision,
    pub staging_parent: Revision,
    pub staging_commit: Revision,
    pub patch_digest: String,
    pub paths: Vec<String>,
    /// Retained superseded artifacts are immutable history. A replacement is
    /// authored by the same scoped child, not synthesized by this controller.
    pub replacement: Option<ReplacementMapping>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplacementMapping {
    pub originals: Vec<DeltaManifest>,
    pub reason: String,
    pub staging_result_digest: String,
}

impl DeltaManifest {
    pub fn digest(&self) -> Result<String> {
        canonical_digest(DELTA_SCHEMA, self)
    }

    /// Review precedes integration. Its canonical identity excludes the future
    /// squash commit while binding every reviewed byte, scope and generation.
    pub fn review_digest(&self) -> Result<String> {
        ReviewDelta {
            schema_version: 1,
            git_version: self.git_version.clone(),
            repository: self.repository.clone(),
            parent_id: self.parent_id,
            child_id: self.child_id,
            work_generation: self.work_generation.clone(),
            correction_generation: self.correction_generation.clone(),
            scope: self.scope.clone(),
            base: self.validated_base.clone(),
            head: self.validated_head.clone(),
            patch_digest: self.patch_digest.clone(),
            paths: self.paths.clone(),
            replacement_digest: self
                .replacement
                .as_ref()
                .map(|m| canonical_digest(1, m))
                .transpose()?,
        }
        .digest()
    }

    pub fn original_contributions(&self) -> Result<BTreeSet<String>> {
        fn collect(m: &DeltaManifest, depth: usize, found: &mut BTreeSet<String>) -> Result<()> {
            if depth > 8 {
                return Err(Error::new(
                    "replacement_mapping",
                    "replacement history exceeds supported depth",
                ));
            }
            if let Some(mapping) = &m.replacement {
                if mapping.originals.is_empty() {
                    return Err(Error::new(
                        "replacement_mapping",
                        "replacement drops its original contribution inventory",
                    ));
                }
                for original in &mapping.originals {
                    collect(original, depth + 1, found)?;
                }
            } else if !found.insert(m.digest()?) {
                return Err(Error::new(
                    "replacement_mapping",
                    "original contribution is accounted for more than once",
                ));
            }
            Ok(())
        }
        let mut found = BTreeSet::new();
        collect(self, 0, &mut found)?;
        Ok(found)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewDelta {
    pub schema_version: u32,
    pub git_version: String,
    pub repository: RepositoryId,
    pub parent_id: Uuid,
    pub child_id: Uuid,
    pub work_generation: String,
    pub correction_generation: String,
    pub scope: WriteScope,
    pub base: Revision,
    pub head: Revision,
    pub patch_digest: String,
    pub paths: Vec<String>,
    pub replacement_digest: Option<String>,
}
impl ReviewDelta {
    pub fn digest(&self) -> Result<String> {
        canonical_digest(1, self)
    }
}
#[derive(Clone, Debug)]
pub struct ProposedDelta {
    review: ReviewDelta,
    patch: Vec<u8>,
}
impl ProposedDelta {
    pub fn review(&self) -> &ReviewDelta {
        &self.review
    }
    pub fn patch(&self) -> &[u8] {
        &self.patch
    }
    pub fn review_digest(&self) -> Result<String> {
        self.review.digest()
    }
}
pub struct ProposeDelta<'a> {
    pub repository: RepositoryId,
    pub parent_id: Uuid,
    pub child_id: Uuid,
    pub work_generation: &'a str,
    pub correction_generation: &'a str,
    pub scope: WriteScope,
    pub base: &'a str,
    pub head: &'a str,
}

/// Patch bytes are an evidence artifact, not content copied into a run receipt.
#[derive(Clone, Debug)]
pub struct DeltaArtifact {
    pub manifest: DeltaManifest,
    pub patch: Vec<u8>,
}

pub struct CaptureDelta<'a> {
    pub repository: RepositoryId,
    pub parent_id: Uuid,
    pub child_id: Uuid,
    pub work_generation: &'a str,
    pub correction_generation: &'a str,
    pub scope: WriteScope,
    pub base: &'a str,
    pub head: &'a str,
    pub staging_parent: &'a str,
    pub staging_commit: &'a str,
}

pub struct CaptureReplacement<'a> {
    pub repository: RepositoryId,
    pub parent_id: Uuid,
    pub child_id: Uuid,
    pub work_generation: &'a str,
    pub correction_generation: &'a str,
    pub scope: WriteScope,
    pub production_base: &'a str,
    pub head: &'a str,
    pub staging_revision: &'a str,
    pub staging_result_digest: &'a str,
    pub originals: &'a [DeltaArtifact],
    pub reason: &'a str,
}

/// An exact observed squash, independently of whether deployment succeeded.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProductionIntegration {
    pub repository: RepositoryId,
    pub parent_id: Uuid,
    pub release_operation: Uuid,
    pub pull_request: u64,
    pub source_candidate_digest: String,
    pub accepted_evidence_token: String,
    pub acceptance_digest: String,
    pub production_parent: Revision,
    pub candidate: Revision,
    pub merged: Revision,
    pub observed_main: Revision,
    pub selected_delta_digests: Vec<String>,
    pub selected_patch_digests: Vec<String>,
    pub selected_contributions: BTreeMap<String, BTreeSet<String>>,
    pub combined_patch_digest: String,
}

impl ProductionIntegration {
    pub fn digest(&self) -> Result<String> {
        canonical_digest(1, self)
    }
}

/// Not deserializable: recovery must reproduce the Git-object proof before it
/// can omit a recorded production delta from the next candidate.
#[derive(Clone, Debug)]
pub struct VerifiedProductionIntegration {
    record: ProductionIntegration,
}

impl VerifiedProductionIntegration {
    pub fn record(&self) -> &ProductionIntegration {
        &self.record
    }
}

pub struct ObserveProductionIntegration<'a> {
    pub release_operation: Uuid,
    pub pull_request: u64,
    pub candidate: &'a Candidate,
    pub authority: &'a VerifiedAcceptedCandidate,
    pub merged_commit: &'a str,
    pub current_main: &'a str,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateManifest {
    pub schema_version: u32,
    pub operation_id: Uuid,
    pub runtime_id: Uuid,
    pub repository: RepositoryId,
    pub parent_id: Uuid,
    pub work_generation: String,
    pub correction_generation: String,
    pub git_version: String,
    pub production: Revision,
    pub ordered_delta_digests: Vec<String>,
    pub ordered_patch_digests: Vec<String>,
    pub selected_contributions: BTreeMap<String, BTreeSet<String>>,
    pub production_integrations: Vec<ProductionIntegration>,
    pub result: Option<Revision>,
    pub combined_patch_digest: Option<String>,
}

/// Created only by `assemble_candidate`; never adopt an arbitrary checkout as a
/// candidate. Failure leaves its directory and manifest available for recovery.
#[derive(Debug)]
pub struct Candidate {
    path: PathBuf,
    pub manifest: CandidateManifest,
}

impl Candidate {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn proof_digest(&self) -> Result<String> {
        canonical_digest(1, &self.manifest)
    }
}

pub struct CandidateRequest<'a> {
    pub operation_id: Uuid,
    pub runtime_id: Uuid,
    pub repository: RepositoryId,
    pub parent_id: Uuid,
    pub work_generation: &'a str,
    pub correction_generation: &'a str,
    pub production_commit: &'a str,
    /// In already-verified dependency/integration order, never staging tip order.
    pub deltas: &'a [DeltaArtifact],
    pub child_dependencies: &'a BTreeMap<Uuid, BTreeSet<Uuid>>,
    pub satisfied_children: &'a BTreeSet<Uuid>,
    pub production_integrations: &'a [VerifiedProductionIntegration],
    pub identity: CommitIdentity,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommitIdentity {
    pub name: String,
    pub email: String,
}

impl CommitIdentity {
    fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty()
            || self.email.trim().is_empty()
            || self.name.len() > 1024
            || self.email.len() > 1024
            || self.name.contains(['\0', '\r', '\n', '<', '>'])
            || self.email.contains(['\0', '\r', '\n', '<', '>'])
        {
            return Err(Error::invalid(
                "a valid configured human commit identity is required",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceCommitMetadata {
    pub identity: CommitIdentity,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub message: String,
}
impl SourceCommitMetadata {
    fn validate(&self) -> Result<()> {
        self.identity.validate()?;
        if self.timestamp.timestamp_subsec_nanos() != 0
            || self.message.trim().is_empty()
            || self.message.len() > 65536
            || self.message.contains('\0')
            || !self.message.ends_with(&format!(
                "Signed-off-by: {} <{}>\n",
                self.identity.name, self.identity.email
            ))
        {
            return Err(Error::invalid(
                "source commit needs bounded explicit message, exact-second time and configured human signoff",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceTreeEntry {
    pub path: String,
    pub mode: String,
    pub object: Option<String>,
}
#[derive(Clone)]
pub struct CommittedSource {
    base: Revision,
    revision: Revision,
    metadata: SourceCommitMetadata,
    entries: Vec<SourceTreeEntry>,
    blobs: BTreeMap<String, Vec<u8>>,
}
impl CommittedSource {
    pub fn base(&self) -> &Revision {
        &self.base
    }
    pub fn revision(&self) -> &Revision {
        &self.revision
    }
    pub fn metadata(&self) -> &SourceCommitMetadata {
        &self.metadata
    }
    pub fn entries(&self) -> &[SourceTreeEntry] {
        &self.entries
    }
    pub fn blobs(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.blobs
    }
}

#[derive(Clone, Eq, PartialEq)]
struct RawSourceFile {
    mode: String,
    oid: String,
    bytes: Vec<u8>,
}
fn raw_blob_oid(bytes: &[u8]) -> String {
    use sha1::{Digest, Sha1};
    let mut digest = Sha1::new();
    digest.update(format!("blob {}\0", bytes.len()));
    digest.update(bytes);
    hex::encode(digest.finalize())
}

fn raw_source_file(path: &Path, metadata: &fs::Metadata) -> Result<(String, Vec<u8>)> {
    use std::io::Read;
    if metadata.is_symlink() {
        let target = fs::read_link(path).map_err(|e| Error::io("read source symlink", e))?;
        return Ok((
            "120000".into(),
            target.as_os_str().as_encoded_bytes().to_vec(),
        ));
    }
    if !metadata.is_file() || metadata.len() > 64 * 1024 * 1024 {
        return Err(Error::new(
            "source_limit",
            "source path is not a bounded ordinary file or symlink",
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        if metadata.nlink() != 1 {
            return Err(Error::new(
                "scope_violation",
                "source file must not alias another file",
            ));
        }
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let file = options
        .open(path)
        .map_err(|e| Error::io("open raw source file", e))?;
    let opened = file
        .metadata()
        .map_err(|e| Error::io("inspect opened source", e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if opened.ino() != metadata.ino() || opened.dev() != metadata.dev() || opened.nlink() != 1 {
            return Err(Error::new(
                "stale_revision",
                "source file identity changed while opening",
            ));
        }
    }
    let mut bytes = Vec::new();
    file.take(64 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| Error::io("read raw source", e))?;
    if bytes.len() > 64 * 1024 * 1024 {
        return Err(Error::new("source_limit", "source file grew beyond bound"));
    }
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        if opened.permissions().mode() & 0o111 != 0 {
            "100755"
        } else {
            "100644"
        }
    };
    #[cfg(not(unix))]
    let mode = "100644";
    Ok((mode.into(), bytes))
}

#[derive(Clone)]
pub struct Git {
    executable: PathBuf,
    version: std::sync::Arc<std::sync::OnceLock<String>>,
    binary: crate::platform::BinaryIdentity,
    operation: Option<std::sync::Arc<GitOperationState>>,
    scope: Option<std::sync::Arc<GitExecutionScope>>,
    host_scope: Option<std::sync::Arc<HostGitExecutionScope>>,
}
#[derive(Clone)]
pub struct GitExecutionScope {
    pub limits: GitLimits,
    pub start_gate: GitStartGate,
    pub processes: std::sync::Arc<crate::process_records::ProcessJournal>,
    pub permit: crate::control::Permit,
    pub executions: std::sync::Arc<crate::worker::ExecutionRegistry>,
}
#[derive(Clone)]
pub struct HostGitExecutionScope {
    pub limits: GitLimits,
    pub authority: std::sync::Arc<crate::host_process::LocalProcessPermit>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExecutionCloneRecord {
    schema_version: u32,
    git_version: String,
    source: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_provenance: Option<String>,
    identity: CloneOwner,
    revision: Revision,
    prepared: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum CloneOwner {
    Child(crate::execution::DirectoryIdentity),
    IndependentReview(IndependentCloneIdentity),
}
impl CloneOwner {
    fn directory_id(&self) -> Uuid {
        match self {
            Self::Child(i) => i.directory_id,
            Self::IndependentReview(i) => i.directory_id,
        }
    }
    fn repository_path(&self) -> &Path {
        match self {
            Self::Child(i) => &i.repository_path,
            Self::IndependentReview(i) => &i.repository_path,
        }
    }
    fn operator_checkout(&self) -> &Path {
        match self {
            Self::Child(i) => &i.operator_checkout,
            Self::IndependentReview(i) => &i.operator_checkout,
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndependentCloneIdentity {
    pub schema_version: u32,
    pub runtime_id: Uuid,
    pub workspace_id: Uuid,
    pub directory_id: Uuid,
    pub repository: RepositoryId,
    pub pull_request: u64,
    pub head: Revision,
    pub base: Revision,
    pub review_identity: String,
    pub repository_path: PathBuf,
    pub operator_checkout: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct CloneDirectoryStamp {
    path: PathBuf,
    device: u64,
    inode: u64,
    uid: u32,
    mode: u32,
}
fn clone_directory_stamp(path: &Path) -> Result<CloneDirectoryStamp> {
    use std::os::unix::fs::MetadataExt;
    let canonical =
        fs::canonicalize(path).map_err(|e| Error::io("resolve retained clone path", e))?;
    let meta =
        fs::symlink_metadata(path).map_err(|e| Error::io("inspect retained clone path", e))?;
    if canonical != path || !meta.is_dir() {
        return Err(Error::new(
            "execution_ownership",
            "retained clone path was linked or replaced",
        ));
    }
    Ok(CloneDirectoryStamp {
        path: canonical,
        device: meta.dev(),
        inode: meta.ino(),
        uid: meta.uid(),
        mode: meta.mode(),
    })
}
#[derive(Clone, Debug)]
pub struct VerifiedIndependentClone {
    identity: IndependentCloneIdentity,
    root: PathBuf,
    paths: Vec<CloneDirectoryStamp>,
    marker_digest: String,
}
impl VerifiedIndependentClone {
    pub fn identity(&self) -> &IndependentCloneIdentity {
        &self.identity
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn digest(&self) -> Result<String> {
        canonical_digest(
            1,
            &serde_json::json!({"identity":self.identity,"root":self.root,"paths":self.paths,"marker":self.marker_digest}),
        )
    }
    /// Rechecks owned filesystem identities without launching repository tools.
    /// The caller must still exclude active writers before preserving/removing.
    pub fn revalidate_filesystem(&self) -> Result<()> {
        for path in &self.paths {
            if clone_directory_stamp(&path.path)? != *path {
                return Err(Error::new(
                    "execution_ownership",
                    "retained clone filesystem identity changed",
                ));
            }
        }
        let marker = self.root.join(format!(
            "prepared-clone-{}.json",
            self.identity.directory_id
        ));
        let record = read_clone_record(&marker)?;
        if canonical_digest(1, &record)? != self.marker_digest {
            return Err(Error::new(
                "execution_ownership",
                "retained clone owner marker changed",
            ));
        }
        Ok(())
    }
}
impl std::fmt::Debug for Git {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Git")
            .field("executable", &self.executable)
            .field("version", &self.version)
            .field("guarded", &self.operation.is_some())
            .finish()
    }
}

impl Git {
    /// Recover the exact host-created source object from immutable Git bytes.
    /// Retained JSON supplies an expectation, never the object proof itself.
    pub fn inspect_committed_source(
        &self,
        source: &Path,
        base: &Revision,
        head: &Revision,
        scope: &WriteScope,
        metadata: SourceCommitMetadata,
    ) -> Result<CommittedSource> {
        metadata.validate()?;
        scope.validate()?;
        if self.exact_revision(source, &base.commit)? != *base
            || self.exact_revision(source, &head.commit)? != *head
            || base == head
        {
            return Err(Error::new(
                "provenance_mismatch",
                "retained source revisions differ from actual Git objects",
            ));
        }
        self.single_parent(source, &head.commit, &base.commit)?;
        let expected = format!(
            "tree {}\nparent {}\nauthor {} <{}> {} +0000\ncommitter {} <{}> {} +0000\n\n{}",
            head.tree,
            base.commit,
            metadata.identity.name,
            metadata.identity.email,
            metadata.timestamp.timestamp(),
            metadata.identity.name,
            metadata.identity.email,
            metadata.timestamp.timestamp(),
            metadata.message
        );
        if self.run(source, &["cat-file", "commit", &head.commit], None)? != expected.as_bytes() {
            return Err(Error::new(
                "provenance_mismatch",
                "retained source commit metadata or message changed",
            ));
        }
        let original = self.source_tree(source, &base.commit)?;
        let current = self.source_tree(source, &head.commit)?;
        let mut entries = Vec::new();
        let mut blobs = BTreeMap::new();
        for path in original
            .keys()
            .chain(current.keys())
            .collect::<BTreeSet<_>>()
        {
            if original.get(path) == current.get(path) {
                continue;
            }
            if !scope.permits(path) {
                return Err(Error::new(
                    "scope_violation",
                    "retained source commit contains an unauthorized path",
                ));
            }
            let old = original.get(path);
            let new = current.get(path);
            if let Some((_, oid)) = new {
                blobs.insert(
                    oid.clone(),
                    self.run(source, &["cat-file", "blob", oid], None)?,
                );
            }
            entries.push(SourceTreeEntry {
                path: path.clone(),
                mode: new.or(old).expect("changed path exists").0.clone(),
                object: new.map(|(_, oid)| oid.clone()),
            });
        }
        if entries.is_empty() {
            return Err(Error::new(
                "no_change",
                "retained source commit manufactured an empty change",
            ));
        }
        Ok(CommittedSource {
            base: base.clone(),
            revision: head.clone(),
            metadata,
            entries,
            blobs,
        })
    }
    /// Commit stopped child implementation bytes in its owned clone. The host
    /// must fence further writer activity first, execute this inside its owned
    /// Git effect, and obtain a fresh admission for the returned revision.
    pub fn commit_execution_changes(
        &self,
        ownership: &crate::execution::OwnedDirectory,
        binding: &crate::execution::ExecutionBinding,
        boundary: &crate::isolation::ValidatedIsolation,
        scope: &WriteScope,
        metadata: SourceCommitMetadata,
    ) -> Result<Option<CommittedSource>> {
        if self.host_scope.is_some() {
            return Err(Error::new(
                "scope_violation",
                "host metadata authority cannot mutate local repository or execution files",
            ));
        }
        ownership.verify(binding, boundary)?;
        binding.validate(boundary)?;
        scope.validate()?;
        metadata.validate()?;
        if boundary.plan().role != crate::isolation::ExecutionRole::Child
            || binding.work.child.is_none()
        {
            return Err(Error::new(
                "scope_violation",
                "only a scoped implementation child may create source commits",
            ));
        }
        let path = &boundary.plan().repository;
        self.verify_execution_clone(path, &binding.revision)?;
        let base_tree = self.source_tree(path, &binding.revision.commit)?;
        let staged = self.output(
            path,
            [
                "diff-index",
                "--cached",
                "--quiet",
                "--no-ext-diff",
                &binding.revision.commit,
                "--",
            ]
            .map(OsStr::new)
            .as_slice(),
            None,
        )?;
        if !staged.status.success() {
            return Err(Error::new(
                "dirty_index",
                "ambient staging must be preserved and explicitly reconciled before source commit",
            ));
        }
        let before = self.raw_source_snapshot(path, &base_tree)?;
        let mut entries = Vec::new();
        let mut blobs = BTreeMap::new();
        for file in base_tree
            .keys()
            .chain(before.keys())
            .collect::<BTreeSet<_>>()
        {
            let original = base_tree.get(file);
            let actual = before.get(file);
            if original.map(|(mode, oid)| (mode, oid)) == actual.map(|file| (&file.mode, &file.oid))
            {
                continue;
            }
            let absolute = path.join(file);
            if !scope.permits(file)
                || !boundary.plan().source_writes.iter().any(|grant| {
                    grant.path == absolute || grant.directory && absolute.starts_with(&grant.path)
                })
            {
                return Err(Error::new(
                    "scope_violation",
                    "working source contains a change outside its exact child write boundary",
                ));
            }
            if let Some(actual) = actual {
                blobs.insert(actual.oid.clone(), actual.bytes.clone());
            }
            entries.push(SourceTreeEntry {
                path: file.clone(),
                mode: actual.map_or_else(
                    || original.expect("deleted tracked path").0.clone(),
                    |file| file.mode.clone(),
                ),
                object: actual.map(|file| file.oid.clone()),
            });
        }
        if entries.is_empty() {
            return Ok(None);
        }
        let index = path
            .join(".git")
            .join(format!("kegari-index-{}", binding.operation_id));
        if index.symlink_metadata().is_ok()
            || index.with_extension("lock").symlink_metadata().is_ok()
        {
            return Err(Error::new(
                "reconciliation_required",
                "source commit has a retained temporary index; never overwrite an interrupted attempt",
            ));
        }
        self.index_command(path, &index, &["read-tree", &binding.revision.commit], None)?;
        for (expected, bytes) in &blobs {
            let actual = String::from_utf8(self.run(
                path,
                &["hash-object", "-w", "--stdin", "--no-filters"],
                Some(bytes),
            )?)
            .map_err(|_| Error::new("git_response", "blob ID is not UTF-8"))?;
            if actual.trim_end() != expected {
                return Err(Error::new(
                    "provenance_mismatch",
                    "raw blob identity differs from Git object",
                ));
            }
        }
        let mut update = Vec::new();
        for entry in &entries {
            update.extend_from_slice(
                format!(
                    "{} {}\t{}\0",
                    if entry.object.is_some() {
                        entry.mode.as_str()
                    } else {
                        "0"
                    },
                    entry
                        .object
                        .as_deref()
                        .unwrap_or("0000000000000000000000000000000000000000"),
                    entry.path
                )
                .as_bytes(),
            );
        }
        self.index_command(
            path,
            &index,
            &["update-index", "-z", "--index-info"],
            Some(&update),
        )?;
        let tree = String::from_utf8(self.index_command(path, &index, &["write-tree"], None)?)
            .map_err(|_| Error::new("git_response", "source tree ID is not UTF-8"))?
            .trim_end()
            .to_string();
        validate_oid(&tree)?;
        if self.raw_source_snapshot(path, &base_tree)? != before
            || self.revision(path, "HEAD")? != binding.revision
        {
            return Err(Error::new(
                "stale_revision",
                "source changed while preparing exact commit objects",
            ));
        }
        let mut command = self.command(path);
        let date = metadata
            .timestamp
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        command
            .env("GIT_AUTHOR_NAME", &metadata.identity.name)
            .env("GIT_AUTHOR_EMAIL", &metadata.identity.email)
            .env("GIT_COMMITTER_NAME", &metadata.identity.name)
            .env("GIT_COMMITTER_EMAIL", &metadata.identity.email)
            .env("GIT_AUTHOR_DATE", &date)
            .env("GIT_COMMITTER_DATE", &date)
            .args([
                "-c",
                "commit.gpgSign=false",
                "commit-tree",
                &tree,
                "-p",
                &binding.revision.commit,
                "-F",
                "-",
            ]);
        let output = self.finish_command(command, Some(metadata.message.as_bytes()))?;
        if !output.status.success() {
            return Err(Error::new(
                "git_operation_failed",
                "source commit object creation failed; preserve temporary index",
            ));
        }
        let commit = String::from_utf8(output.stdout)
            .map_err(|_| Error::new("git_response", "source commit ID is not UTF-8"))?
            .trim_end()
            .to_string();
        let revision = self.exact_revision(path, &commit)?;
        self.single_parent(path, &commit, &binding.revision.commit)?;
        if revision.tree != tree {
            return Err(Error::new(
                "provenance_mismatch",
                "created source commit has a different tree",
            ));
        }
        self.run(
            path,
            &["update-ref", "HEAD", &commit, &binding.revision.commit],
            None,
        )?;
        self.run(path, &["read-tree", &tree], None)?;
        if self.raw_source_snapshot(path, &base_tree)? != before {
            return Err(Error::new(
                "stale_revision",
                "source changed while finalizing its commit",
            ));
        }
        self.assert_clean(path)?;
        Ok(Some(CommittedSource {
            base: binding.revision.clone(),
            revision,
            metadata,
            entries,
            blobs,
        }))
    }

    fn index_command(
        &self,
        path: &Path,
        index: &Path,
        args: &[&str],
        input: Option<&[u8]>,
    ) -> Result<Vec<u8>> {
        let mut command = self.command(path);
        command.env("GIT_INDEX_FILE", index).args(args);
        let output = self.finish_command(command, input)?;
        if !output.status.success() {
            return Err(Error::new(
                "git_operation_failed",
                "temporary source index operation failed; preserve its artifacts",
            ));
        }
        Ok(output.stdout)
    }

    fn source_tree(
        &self,
        path: &Path,
        revision: &str,
    ) -> Result<BTreeMap<String, (String, String)>> {
        let output = self.run(path, &["ls-tree", "-r", "-z", revision], None)?;
        let mut result = BTreeMap::new();
        for entry in output
            .split(|byte| *byte == 0)
            .filter(|entry| !entry.is_empty())
        {
            let entry = std::str::from_utf8(entry).map_err(|_| {
                Error::new("unsupported_repository", "source tree paths must be UTF-8")
            })?;
            let (meta, name) = entry
                .split_once('\t')
                .ok_or_else(|| Error::new("git_response", "tree entry lacks path"))?;
            let fields: Vec<_> = meta.split(' ').collect();
            validate_repo_path(name)?;
            if fields.len() != 3
                || fields[1] != "blob"
                || !matches!(fields[0], "100644" | "100755" | "120000")
                || result.len() >= 100_000
            {
                return Err(Error::new(
                    "unsupported_repository",
                    "source tree requires bounded ordinary files/symlinks without Gitlinks",
                ));
            }
            validate_oid(fields[2])?;
            result.insert(name.into(), (fields[0].into(), fields[2].into()));
        }
        Ok(result)
    }

    fn raw_source_snapshot(
        &self,
        path: &Path,
        tracked: &BTreeMap<String, (String, String)>,
    ) -> Result<BTreeMap<String, RawSourceFile>> {
        let others = self.run(path, &["ls-files", "--others", "-z"], None)?;
        let mut names: BTreeSet<String> = tracked.keys().cloned().collect();
        for name in others
            .split(|byte| *byte == 0)
            .filter(|name| !name.is_empty())
        {
            let name = std::str::from_utf8(name).map_err(|_| {
                Error::new("unsupported_repository", "source file path must be UTF-8")
            })?;
            validate_repo_path(name)?;
            names.insert(name.into());
        }
        if names.len() > 100_000 {
            return Err(Error::new("source_limit", "source inventory exceeds bound"));
        }
        let mut result = BTreeMap::new();
        let mut total = 0usize;
        for name in names {
            let file = path.join(&name);
            let mut parent = file.parent();
            while let Some(directory) = parent.filter(|directory| *directory != path) {
                if fs::symlink_metadata(directory).is_ok_and(|meta| !meta.is_dir()) {
                    return Err(Error::new(
                        "scope_violation",
                        "source file ancestry crosses a link or non-directory",
                    ));
                }
                parent = directory.parent();
            }
            let metadata = match fs::symlink_metadata(&file) {
                Ok(meta) => meta,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(Error::io("inspect source file", error)),
            };
            let (mode, bytes) = raw_source_file(&file, &metadata)?;
            total = total
                .checked_add(bytes.len())
                .ok_or_else(|| Error::new("source_limit", "source size overflow"))?;
            if total > 256 * 1024 * 1024 {
                return Err(Error::new(
                    "source_limit",
                    "source byte inventory exceeds bound",
                ));
            }
            result.insert(
                name,
                RawSourceFile {
                    mode,
                    oid: raw_blob_oid(&bytes),
                    bytes,
                },
            );
        }
        Ok(result)
    }

    /// Create only a separate owned execution clone at an observed exact
    /// revision. Run through `start_operation` in production so every Git
    /// subprocess passes the current authority gate. A matching existing clone
    /// is returned without changing its files or index, including dirty work.
    pub fn prepare_execution_clone(
        &self,
        source: &Path,
        execution_root: &Path,
        identity: &crate::execution::DirectoryIdentity,
        revision: &Revision,
    ) -> Result<PathBuf> {
        if self.host_scope.is_some() {
            return Err(Error::new(
                "scope_violation",
                "host metadata authority cannot mutate local repository or execution files",
            ));
        }
        identity.work.validate()?;
        identity.repository.validate()?;
        revision.validate()?;
        if identity.schema_version != 1
            || identity.runtime_id.is_nil()
            || identity.directory_id.is_nil()
        {
            return Err(Error::new(
                "execution_ownership",
                "execution identity is incomplete",
            ));
        }
        self.prepare_owned_clone(
            source,
            execution_root,
            CloneOwner::Child(identity.clone()),
            revision,
            true,
            None,
        )
    }

    /// Read-only independent review clone has its own provider identity and does
    /// not invent a Linear parent, implementation scope, hooks or write rights.
    pub fn prepare_independent_review_clone(
        &self,
        source: &Path,
        execution_root: &Path,
        identity: &IndependentCloneIdentity,
    ) -> Result<PathBuf> {
        if self.host_scope.is_some() {
            return Err(Error::new(
                "scope_violation",
                "host metadata authority cannot mutate local repository or execution files",
            ));
        }
        validate_independent_clone_identity(identity)?;
        if self.exact_revision(source, &identity.base.commit)? != identity.base {
            return Err(Error::new(
                "stale_revision",
                "review source lacks its observed base revision",
            ));
        }
        self.prepare_owned_clone(
            source,
            execution_root,
            CloneOwner::IndependentReview(identity.clone()),
            &identity.head,
            true,
            None,
        )
    }

    /// Inspect an existing prepared review clone without creating, repairing or
    /// changing its files/index. A missing or partial ownership receipt blocks.
    pub fn observe_independent_review_clone(
        &self,
        execution_root: &Path,
        identity: &IndependentCloneIdentity,
    ) -> Result<VerifiedIndependentClone> {
        validate_independent_clone_identity(identity)?;
        self.prepare_owned_clone(
            &identity.operator_checkout,
            execution_root,
            CloneOwner::IndependentReview(identity.clone()),
            &identity.head,
            false,
            None,
        )?;
        if self.exact_revision(&identity.repository_path, &identity.base.commit)? != identity.base {
            return Err(Error::new(
                "stale_revision",
                "retained review clone lost its exact base object",
            ));
        }
        let root = owner_directory(execution_root)?;
        let paths = [
            root.as_path(),
            identity.repository_path.as_path(),
            identity.operator_checkout.as_path(),
            &identity.repository_path.join(".git"),
        ]
        .into_iter()
        .map(clone_directory_stamp)
        .collect::<Result<_>>()?;
        let marker = root.join(format!("prepared-clone-{}.json", identity.directory_id));
        let proof = VerifiedIndependentClone {
            identity: identity.clone(),
            root,
            paths,
            marker_digest: canonical_digest(1, &read_clone_record(&marker)?)?,
        };
        proof.revalidate_filesystem()?;
        Ok(proof)
    }

    fn prepare_owned_clone(
        &self,
        source: &Path,
        execution_root: &Path,
        identity: CloneOwner,
        revision: &Revision,
        create: bool,
        source_provenance: Option<String>,
    ) -> Result<PathBuf> {
        let source = fs::canonicalize(source).map_err(|e| Error::io("resolve clone source", e))?;
        let root = owner_directory(execution_root)?;
        let path = identity.repository_path();
        let operator = fs::canonicalize(identity.operator_checkout())
            .map_err(|e| Error::io("resolve configured operator attribution", e))?;
        if identity.operator_checkout() != operator
            || (source_provenance.is_none() && operator != source)
            || operator.starts_with(&root)
            || root.starts_with(&operator)
            || path.parent() != Some(root.as_path())
            || path.file_name().is_none()
            || source.starts_with(&root)
            || root.starts_with(&source)
        {
            return Err(Error::new(
                "execution_ownership",
                "execution clone requires a disjoint owned root and exact source identity",
            ));
        }
        let mut expected = ExecutionCloneRecord {
            schema_version: if source_provenance.is_some() { 2 } else { 1 },
            git_version: self.version().to_owned(),
            source: source.clone(),
            source_provenance,
            identity: identity.clone(),
            revision: revision.clone(),
            prepared: false,
        };
        let marker = root.join(format!("prepared-clone-{}.json", identity.directory_id()));
        if marker.symlink_metadata().is_ok() {
            let record = read_clone_record(&marker)?;
            expected.prepared = true;
            if record != expected || !record.prepared {
                return Err(Error::new(
                    "reconciliation_required",
                    "partial or differently owned clone must be preserved",
                ));
            }
            self.verify_execution_clone(path, revision)?;
            return Ok(path.to_owned());
        }
        if !create {
            return Err(Error::new(
                "ownership_conflict",
                "existing review clone has no preparation receipt; observation cannot create it",
            ));
        }
        if path.symlink_metadata().is_ok() {
            return Err(Error::new(
                "ownership_conflict",
                "execution clone destination already exists without its receipt",
            ));
        }
        if self.exact_revision(&source, &revision.commit)? != *revision {
            return Err(Error::new(
                "stale_revision",
                "clone source does not contain observed exact revision",
            ));
        }
        for kind in ["--absolute-git-dir", "--git-common-dir"] {
            let directory = self.text(&source, &["rev-parse", "--path-format=absolute", kind])?;
            if Path::new(&directory)
                .join("objects/info/alternates")
                .exists()
            {
                return Err(Error::new(
                    "unsupported_repository",
                    "execution clone source uses external object alternates",
                ));
            }
        }
        write_clone_record(&marker, &expected, true)?;
        let args: Vec<OsString> = [
            "clone",
            "--local",
            "--no-hardlinks",
            "--no-checkout",
            "--template=/dev/null",
            "--",
        ]
        .into_iter()
        .map(OsString::from)
        .chain([source.as_os_str().to_owned(), path.as_os_str().to_owned()])
        .collect();
        let output = self.output(
            &root,
            &args.iter().map(OsString::as_os_str).collect::<Vec<_>>(),
            None,
        )?;
        if !output.status.success() {
            return Err(Error::new(
                "git_operation_failed",
                "execution clone failed; retain partial clone and its receipt",
            ));
        }
        set_owner_directory(path)?;
        self.run(
            path,
            &["checkout", "--detach", &revision.commit, "--"],
            None,
        )?;
        self.verify_execution_clone(path, revision)?;
        self.assert_clean(path)?;
        if let Some(operation) = &self.operation {
            operation.current()?;
        }
        expected.prepared = true;
        write_clone_record(&marker, &expected, false)?;
        Ok(path.to_owned())
    }

    fn verify_execution_clone(&self, path: &Path, revision: &Revision) -> Result<()> {
        let path = owner_directory(path)?;
        let git = path.join(".git");
        let config = self.run(&path, &["config", "--local", "--null", "--list"], None)?;
        for entry in config
            .split(|byte| *byte == 0)
            .filter(|entry| !entry.is_empty())
        {
            let key = entry
                .split(|byte| *byte == b'\n')
                .next()
                .unwrap_or_default();
            let key = String::from_utf8_lossy(key).to_ascii_lowercase();
            if key.starts_with("filter.")
                || key.starts_with("include.")
                || key.starts_with("includeif.")
                || key == "core.worktree"
                || key == "core.sparsecheckout"
            {
                return Err(Error::new(
                    "unsupported_repository",
                    "owned execution Git configuration must not run filters, load external configuration, redirect worktrees or hide source paths",
                ));
            }
        }
        for option in ["--absolute-git-dir", "--git-common-dir"] {
            if Path::new(&self.text(&path, &["rev-parse", "--path-format=absolute", option])?)
                != git
            {
                return Err(Error::new(
                    "execution_ownership",
                    "execution Git administration is not independently owned",
                ));
            }
        }
        if git.join("objects/info/alternates").exists()
            || self.revision(&path, "HEAD")? != *revision
        {
            return Err(Error::new(
                "stale_revision",
                "retained execution clone lost its exact revision or independent objects",
            ));
        }
        Ok(())
    }

    /// Inspect a pinned local executable without launching it. Repository commands
    /// remain unavailable until an owned operation verifies the supported version.
    pub fn from_pinned_executable(executable: impl AsRef<Path>) -> Result<Self> {
        let executable =
            fs::canonicalize(executable).map_err(|e| Error::io("resolve Git executable", e))?;
        let binary = crate::platform::BinaryIdentity::inspect(&executable, false)?;
        Ok(Self {
            executable,
            binary,
            version: std::sync::Arc::new(std::sync::OnceLock::new()),
            operation: None,
            scope: None,
            host_scope: None,
        })
    }
    /// Bounded metadata convenience for explicit local fixtures and operator
    /// preflight. Production helper phases use from_pinned_executable followed
    /// by start_journaled_operation, which gates and journals the probe.
    pub fn new(executable: impl AsRef<Path>) -> Result<Self> {
        let git = Self::from_pinned_executable(executable)?;
        git.verify_version()?;
        Ok(git)
    }
    fn verify_version(&self) -> Result<()> {
        self.binary.revalidate(false)?;
        let mut command = Command::new(&self.executable);
        command
            .env_clear()
            .current_dir(
                self.executable
                    .parent()
                    .ok_or_else(|| Error::invalid("Git binary has no parent"))?,
            )
            .arg("--version");
        let output = match &self.operation {
            Some(operation) => operation.run(command, None)?,
            None if self.scope.is_some() => self.scoped_command(command, None)?,
            None if self.host_scope.is_some() => self.host_scoped_command(command, None)?,
            None => GitOperationState::new(
                GitLimits {
                    command_timeout: std::time::Duration::from_secs(3),
                    operation_timeout: std::time::Duration::from_secs(5),
                    output_bytes: 1024,
                    ..GitLimits::default()
                },
                None,
            )?
            .run(command, None)?,
        };
        self.binary.revalidate(false)?;
        let version = String::from_utf8(output.stdout)
            .map_err(|_| Error::new("unsupported_git_version", "Git version is not UTF-8"))?
            .trim()
            .to_string();
        if !output.status.success() || version != SUPPORTED_GIT_VERSION {
            return Err(Error::new(
                "unsupported_git_version",
                format!("supported Git profile is {SUPPORTED_GIT_VERSION}; observed {version}"),
            ));
        }
        let _ = self.version.set(version);
        Ok(())
    }
    pub fn version(&self) -> &str {
        self.version.get().map(String::as_str).unwrap_or("")
    }
    pub fn executable(&self) -> &Path {
        &self.executable
    }
    /// Structured trusted reads never run repository hooks, filters, shell or
    /// external diff/textconv drivers. Scoped runtime handles gate and journal
    /// these commands exactly like all their other metadata subprocesses.
    pub fn inspect_read_only(
        &self,
        cwd: &Path,
        inspection: &crate::execution::GitInspection,
    ) -> Result<crate::isolation::CommandOutcome> {
        let mut args: Vec<String> = Vec::new();
        match inspection {
            crate::execution::GitInspection::Status => args
                .extend(["status", "--porcelain=v1", "--untracked-files=all"].map(str::to_owned)),
            crate::execution::GitInspection::Diff { staged } => {
                args.extend(
                    [
                        "diff",
                        "--binary",
                        "--full-index",
                        "--no-ext-diff",
                        "--no-textconv",
                        "--no-renames",
                    ]
                    .map(str::to_owned),
                );
                if *staged {
                    args.push("--cached".into());
                }
                args.push("--".into());
            }
            crate::execution::GitInspection::ShowFile { commit, path } => {
                validate_oid(commit)?;
                validate_repo_path(path)?;
                args.extend([
                    "show".into(),
                    "--no-ext-diff".into(),
                    "--no-textconv".into(),
                    format!("{commit}:{path}"),
                ]);
            }
        }
        let mut command = self.command(cwd);
        command.args(&args);
        let argv = std::iter::once(command.get_program().to_string_lossy().into_owned())
            .chain(command.get_args().map(|a| a.to_string_lossy().into_owned()))
            .collect();
        let started = std::time::Instant::now();
        let output = self.finish_command(command, None)?;
        Ok(crate::isolation::CommandOutcome {
            schema_version: 1,
            argv,
            cwd: cwd.to_owned(),
            isolation_digest: String::new(),
            exit_code: output.status.code(),
            timed_out: false,
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            output_truncated: false,
            elapsed_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
        })
    }
    /// Fixed read-only inventory queries for explicit cleanup eligibility.
    pub fn cleanup_read(&self, root: &Path, query: CleanupRead) -> Result<String> {
        self.text(root, query.args())
    }
    pub fn with_execution_scope(&self, scope: GitExecutionScope) -> Result<Self> {
        scope.limits.validate()?;
        let mut owned = self.clone();
        owned.operation = None;
        owned.scope = Some(std::sync::Arc::new(scope));
        owned.host_scope = None;
        owned.verify_version()?;
        Ok(owned)
    }
    /// Host-local metadata is a distinct authenticated control effect and works
    /// while STOPPED. Its command grammar cannot launch mutating Git operations.
    pub fn with_host_execution_scope(&self, scope: HostGitExecutionScope) -> Result<Self> {
        scope.limits.validate()?;
        if self.binary != *scope.authority.binary() {
            return Err(Error::new(
                "ownership_conflict",
                "host metadata authority names another Git binary",
            ));
        }
        let mut owned = self.clone();
        owned.operation = None;
        owned.scope = None;
        owned.host_scope = Some(std::sync::Arc::new(scope));
        owned.verify_version()?;
        Ok(owned)
    }
    fn host_scoped_command(&self, command: Command, input: Option<&[u8]>) -> Result<Output> {
        if input.is_some_and(|bytes| !bytes.is_empty()) {
            return Err(Error::new(
                "scope_violation",
                "host metadata Git accepts no stdin instructions",
            ));
        }
        let scope = self
            .host_scope
            .as_ref()
            .ok_or_else(|| Error::new("process_journal_required", "host metadata scope missing"))?
            .clone();
        let cwd = command
            .get_current_dir()
            .ok_or_else(|| Error::invalid("host Git cwd missing"))?
            .to_path_buf();
        let argv = command
            .get_args()
            .map(|arg| {
                arg.to_str()
                    .map(str::to_owned)
                    .ok_or_else(|| Error::invalid("host Git argument is not UTF-8"))
            })
            .collect::<Result<Vec<_>>>()?;
        let mut initial = GitOperationState::new(scope.limits.clone(), None)?;
        let authority = scope.authority.clone();
        let state = std::sync::Arc::new_cyclic(|weak: &std::sync::Weak<GitOperationState>| {
            let weak = weak.clone();
            initial.gate = Some(std::sync::Arc::new(move |spawn| {
                let state = weak.upgrade().ok_or_else(|| {
                    Error::new("ownership_conflict", "host Git waiter disappeared")
                })?;
                let cancellation = std::sync::Arc::new(GitCancellation {
                    state: state.clone(),
                });
                authority.start(&cwd, &argv, cancellation, |guard| {
                    *state.provided_guard.lock().map_err(|_| {
                        Error::new("ownership_conflict", "host Git guard unavailable")
                    })? = Some(guard);
                    spawn()
                })
            }));
            initial
        });
        let result = state.run(command, input);
        state
            .closed
            .store(true, std::sync::atomic::Ordering::SeqCst);
        result
    }
    fn scoped_command(&self, command: Command, input: Option<&[u8]>) -> Result<Output> {
        let scope = self
            .scope
            .as_ref()
            .ok_or_else(|| {
                Error::new(
                    "process_journal_required",
                    "Git command has no admitted scope",
                )
            })?
            .clone();
        let effect = Uuid::new_v4();
        let mut initial = GitOperationState::new(scope.limits.clone(), None)?;
        initial.journal = Some((scope.processes.clone(), scope.permit.clone()));
        let registration = scope.clone();
        let state = std::sync::Arc::new_cyclic(|weak: &std::sync::Weak<GitOperationState>| {
            let weak = weak.clone();
            initial.gate = Some(std::sync::Arc::new(move |spawn| {
                (registration.start_gate)(&mut || {
                    let state = weak.upgrade().ok_or_else(|| {
                        Error::new("ownership_conflict", "Git waiter disappeared during spawn")
                    })?;
                    registration.executions.register(
                        registration.permit.operation_id,
                        effect,
                        std::sync::Arc::new(GitCancellation { state }),
                    )?;
                    spawn()
                })
            }));
            initial
        });
        let result = state.run(command, input);
        state
            .closed
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let cancellation = GitCancellation { state };
        if cancellation.exited()? {
            scope.executions.remove(scope.permit.operation_id, effect)?;
        }
        result
    }

    fn command(&self, cwd: &Path) -> Command {
        let mut cmd = Command::new(&self.executable);
        cmd.env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("LC_ALL", "C")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_NO_REPLACE_OBJECTS", "1")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .current_dir(cwd)
            .args([
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "core.fsmonitor=false",
                "-c",
                "core.autocrlf=false",
                "-c",
                "core.filemode=true",
                "-c",
                "core.symlinks=true",
                "-c",
                "core.quotePath=true",
                "-c",
                "color.ui=false",
                "-c",
                "diff.external=",
                "-c",
                "diff.orderFile=/dev/null",
                "-c",
                "apply.ignoreWhitespace=false",
                "-c",
                "protocol.allow=never",
                "-c",
                "protocol.file.allow=always",
            ]);
        cmd
    }

    fn output(&self, cwd: &Path, args: &[&OsStr], input: Option<&[u8]>) -> Result<Output> {
        let mut command = self.command(cwd);
        command.args(args);
        self.finish_command(command, input)
    }
    fn finish_command(&self, command: Command, input: Option<&[u8]>) -> Result<Output> {
        if self.version() != SUPPORTED_GIT_VERSION {
            return Err(Error::new(
                "unsupported_git_version",
                "Git version has not been verified by an owned probe",
            ));
        }
        self.binary.revalidate(false)?;
        match &self.operation {
            Some(operation) => operation.run(command, input),
            None if self.scope.is_some() => self.scoped_command(command, input),
            None if self.host_scope.is_some() => self.host_scoped_command(command, input),
            None => GitOperationState::new(GitLimits::default(), None)?.run(command, input),
        }
    }

    fn run(&self, cwd: &Path, args: &[&str], input: Option<&[u8]>) -> Result<Vec<u8>> {
        let args: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
        let output = self.output(cwd, &args, input)?;
        if !output.status.success() {
            // Git stderr can contain arbitrary repository content. Keep it out
            // of general errors; the owned evidence sink may retain it safely.
            return Err(Error::new(
                "git_operation_failed",
                format!(
                    "Git {} failed with {}; preserved files require inspection",
                    args.first()
                        .map_or("operation", |s| s.to_str().unwrap_or("operation")),
                    output.status
                ),
            ));
        }
        Ok(output.stdout)
    }

    fn text(&self, cwd: &Path, args: &[&str]) -> Result<String> {
        String::from_utf8(self.run(cwd, args, None)?)
            .map(|s| s.trim_end_matches('\n').to_owned())
            .map_err(|_| Error::new("git_response", "Git returned non-UTF-8 metadata"))
    }

    pub fn branch_name(
        &self,
        child_identifier: &str,
        parent_title: &str,
        max_bytes: usize,
    ) -> Result<BranchName> {
        if child_identifier.is_empty()
            || child_identifier.starts_with('-')
            || child_identifier.contains(['/', '\\', '\0'])
            || child_identifier.chars().any(char::is_whitespace)
        {
            return Err(Error::new(
                "invalid_branch_name",
                "child identifier is not a single safe ref component",
            ));
        }
        let suffix = title_suffix(parent_title)?;
        let name = format!("{child_identifier}-{suffix}");
        if name.len() > max_bytes || max_bytes == 0 {
            return Err(Error::new(
                "invalid_branch_name",
                "complete child branch exceeds the configured provider/filesystem limit",
            ));
        }
        let full = format!("refs/heads/{name}");
        let output = self.output(
            Path::new("/"),
            &[OsStr::new("check-ref-format"), OsStr::new(&full)],
            None,
        )?;
        if !output.status.success() {
            return Err(Error::new(
                "invalid_branch_name",
                "complete branch fails Git ref-name rules",
            ));
        }
        Ok(BranchName {
            title_digest: bytes_digest(parent_title.as_bytes()),
            suffix,
            name,
        })
    }

    pub fn revision(&self, repository: &Path, reference: &str) -> Result<Revision> {
        if reference.contains(['\0', '\r', '\n']) {
            return Err(Error::new("invalid_revision", "invalid Git reference"));
        }
        let expression = format!("{reference}^{{commit}}");
        let commit = self.text(
            repository,
            &["rev-parse", "--verify", "--end-of-options", &expression],
        )?;
        validate_oid(&commit)?;
        let tree = self.text(
            repository,
            &["rev-parse", "--verify", &format!("{commit}^{{tree}}")],
        )?;
        validate_oid(&tree)?;
        Ok(Revision { commit, tree })
    }

    pub fn exact_revision(&self, repository: &Path, commit: &str) -> Result<Revision> {
        validate_oid(commit)?;
        let revision = self.revision(repository, commit)?;
        if revision.commit != commit {
            return Err(Error::new(
                "provenance_mismatch",
                "Git object is not the expected commit",
            ));
        }
        Ok(revision)
    }

    pub fn is_ancestor(&self, repository: &Path, ancestor: &str, descendant: &str) -> Result<bool> {
        self.exact_revision(repository, ancestor)?;
        self.exact_revision(repository, descendant)?;
        let output = self.output(
            repository,
            &[
                OsStr::new("merge-base"),
                OsStr::new("--is-ancestor"),
                OsStr::new(ancestor),
                OsStr::new(descendant),
            ],
            None,
        )?;
        match output.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(Error::new(
                "git_operation_failed",
                "Git ancestry query failed",
            )),
        }
    }

    pub fn patch(&self, repository: &Path, base: &str, head: &str) -> Result<Vec<u8>> {
        self.exact_revision(repository, base)?;
        self.exact_revision(repository, head)?;
        self.run(
            repository,
            &[
                "diff",
                "--binary",
                "--full-index",
                "--no-ext-diff",
                "--no-textconv",
                "--no-renames",
                "--no-color",
                "--diff-algorithm=myers",
                "--no-indent-heuristic",
                "--unified=3",
                "--src-prefix=a/",
                "--dst-prefix=b/",
                "--line-prefix=",
                "--output-indicator-new=+",
                "--output-indicator-old=-",
                "--output-indicator-context= ",
                "--ignore-submodules=none",
                "-O/dev/null",
                base,
                head,
                "--",
            ],
            None,
        )
    }

    pub fn changed_paths(&self, repository: &Path, base: &str, head: &str) -> Result<Vec<String>> {
        self.exact_revision(repository, base)?;
        self.exact_revision(repository, head)?;
        let bytes = self.run(
            repository,
            &[
                "diff",
                "--name-only",
                "-z",
                "--no-renames",
                "--no-ext-diff",
                "--no-textconv",
                "-O/dev/null",
                base,
                head,
                "--",
            ],
            None,
        )?;
        let mut paths = Vec::new();
        for path in bytes.split(|b| *b == 0).filter(|p| !p.is_empty()) {
            let path = std::str::from_utf8(path)
                .map_err(|_| Error::new("unsupported_path", "changed file name is not UTF-8"))?;
            validate_repo_path(path)?;
            paths.push(path.to_owned());
        }
        let sorted: BTreeSet<String> = paths.iter().cloned().collect();
        if paths.len() != sorted.len() {
            return Err(Error::new(
                "provenance_mismatch",
                "duplicate Git path in delta",
            ));
        }
        Ok(sorted.into_iter().collect())
    }

    fn single_parent(&self, repository: &Path, commit: &str, expected: &str) -> Result<()> {
        let line = self.text(
            repository,
            &["rev-list", "--parents", "-n", "1", commit, "--"],
        )?;
        let values: Vec<&str> = line.split_whitespace().collect();
        if values != [commit, expected] {
            return Err(Error::new(
                "provenance_mismatch",
                "observed integration is not a squash onto the expected parent",
            ));
        }
        Ok(())
    }

    pub fn propose_delta(&self, source: &Path, request: ProposeDelta<'_>) -> Result<ProposedDelta> {
        request.repository.validate()?;
        request.scope.validate()?;
        if request.parent_id.is_nil()
            || request.child_id.is_nil()
            || request.work_generation.is_empty()
            || request.correction_generation.is_empty()
        {
            return Err(Error::invalid(
                "proposed delta requires current child and generation identities",
            ));
        }
        let base = self.exact_revision(source, request.base)?;
        let head = self.exact_revision(source, request.head)?;
        if !self.is_ancestor(source, &base.commit, &head.commit)? {
            return Err(Error::new(
                "provenance_mismatch",
                "proposed head is not based on recorded staging",
            ));
        }
        let patch = self.patch(source, &base.commit, &head.commit)?;
        let paths = self.changed_paths(source, &base.commit, &head.commit)?;
        if patch.is_empty() || paths.is_empty() {
            return Err(Error::new(
                "no_change",
                "empty proposal requires evidenced no-change completion",
            ));
        }
        if paths.iter().any(|p| !request.scope.permits(p)) {
            return Err(Error::new(
                "scope_violation",
                "proposed patch escapes the authorized child paths",
            ));
        }
        Ok(ProposedDelta {
            review: ReviewDelta {
                schema_version: 1,
                git_version: self.version().to_owned(),
                repository: request.repository,
                parent_id: request.parent_id,
                child_id: request.child_id,
                work_generation: request.work_generation.into(),
                correction_generation: request.correction_generation.into(),
                scope: request.scope,
                base,
                head,
                patch_digest: bytes_digest(&patch),
                paths,
                replacement_digest: None,
            },
            patch,
        })
    }

    pub fn capture_delta(&self, source: &Path, request: CaptureDelta<'_>) -> Result<DeltaArtifact> {
        request.repository.validate()?;
        request.scope.validate()?;
        if request.work_generation.is_empty() || request.correction_generation.is_empty() {
            return Err(Error::invalid(
                "delta requires work and correction generations",
            ));
        }
        let base = self.exact_revision(source, request.base)?;
        let head = self.exact_revision(source, request.head)?;
        let staging_parent = self.exact_revision(source, request.staging_parent)?;
        let staging_commit = self.exact_revision(source, request.staging_commit)?;
        if base != staging_parent
            || head.tree != staging_commit.tree
            || !self.is_ancestor(source, &base.commit, &head.commit)?
        {
            return Err(Error::new(
                "provenance_mismatch",
                "squash base/tree differs from the validated candidate",
            ));
        }
        self.single_parent(source, &staging_commit.commit, &staging_parent.commit)?;
        let patch = self.patch(source, &base.commit, &head.commit)?;
        if patch.is_empty() {
            return Err(Error::new(
                "no_change",
                "empty change must use evidenced no-change completion",
            ));
        }
        if patch != self.patch(source, &staging_parent.commit, &staging_commit.commit)? {
            return Err(Error::new(
                "provenance_mismatch",
                "observed squash patch differs from the reviewed delta",
            ));
        }
        let paths = self.changed_paths(source, &base.commit, &head.commit)?;
        if paths.iter().any(|path| !request.scope.permits(path)) {
            return Err(Error::new(
                "scope_violation",
                "child delta writes outside its authorized paths",
            ));
        }
        let manifest = DeltaManifest {
            schema_version: DELTA_SCHEMA,
            git_version: self.version().to_owned(),
            repository: request.repository,
            parent_id: request.parent_id,
            child_id: request.child_id,
            work_generation: request.work_generation.into(),
            correction_generation: request.correction_generation.into(),
            scope: request.scope,
            validated_base: base,
            validated_head: head,
            staging_parent,
            staging_commit,
            patch_digest: bytes_digest(&patch),
            paths,
            replacement: None,
        };
        Ok(DeltaArtifact { manifest, patch })
    }

    pub fn verify_delta(&self, source: &Path, delta: &DeltaArtifact) -> Result<()> {
        let m = &delta.manifest;
        if m.schema_version != DELTA_SCHEMA
            || m.git_version != self.version()
            || m.patch_digest != bytes_digest(&delta.patch)
        {
            return Err(Error::new(
                "provenance_mismatch",
                "delta schema, Git profile or exact artifact digest mismatch",
            ));
        }
        if let Some(mapping) = &m.replacement {
            // Bound recursion and reject duplicate contribution mappings before
            // traversing retained Git objects.
            m.original_contributions()?;
            let originals = mapping
                .originals
                .iter()
                .map(|original| {
                    Ok(DeltaArtifact {
                        patch: self.patch(
                            source,
                            &original.validated_base.commit,
                            &original.validated_head.commit,
                        )?,
                        manifest: original.clone(),
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let reproduced = self.capture_replacement(
                source,
                CaptureReplacement {
                    repository: m.repository.clone(),
                    parent_id: m.parent_id,
                    child_id: m.child_id,
                    work_generation: &m.work_generation,
                    correction_generation: &m.correction_generation,
                    scope: m.scope.clone(),
                    production_base: &m.validated_base.commit,
                    head: &m.validated_head.commit,
                    staging_revision: &m.staging_commit.commit,
                    staging_result_digest: &mapping.staging_result_digest,
                    originals: &originals,
                    reason: &mapping.reason,
                },
            )?;
            if reproduced.manifest != *m || reproduced.patch != delta.patch {
                return Err(Error::new(
                    "provenance_mismatch",
                    "retained objects do not reproduce the replacement mapping",
                ));
            }
            return Ok(());
        }
        let captured = self.capture_delta(
            source,
            CaptureDelta {
                repository: m.repository.clone(),
                parent_id: m.parent_id,
                child_id: m.child_id,
                work_generation: &m.work_generation,
                correction_generation: &m.correction_generation,
                scope: m.scope.clone(),
                base: &m.validated_base.commit,
                head: &m.validated_head.commit,
                staging_parent: &m.staging_parent.commit,
                staging_commit: &m.staging_commit.commit,
            },
        )?;
        if captured.manifest != *m || captured.patch != delta.patch {
            return Err(Error::new(
                "provenance_mismatch",
                "retained Git objects do not reproduce the delta manifest",
            ));
        }
        Ok(())
    }

    pub fn capture_replacement(
        &self,
        source: &Path,
        request: CaptureReplacement<'_>,
    ) -> Result<DeltaArtifact> {
        request.repository.validate()?;
        request.scope.validate()?;
        if request.originals.is_empty()
            || request.originals.len() > 1024
            || request.reason.trim().is_empty()
            || request.work_generation.is_empty()
            || request.correction_generation.is_empty()
            || request.staging_result_digest.is_empty()
        {
            return Err(Error::new(
                "replacement_mapping",
                "replacement needs original contributions, reason and explicit staged-result evidence",
            ));
        }
        let mut originals = Vec::new();
        for original in request.originals {
            self.verify_delta(source, original)?;
            let m = &original.manifest;
            if m.repository != request.repository
                || m.parent_id != request.parent_id
                || m.child_id != request.child_id
                || m.scope != request.scope
            {
                return Err(Error::new(
                    "scope_violation",
                    "a replacement may account only for its own child's unchanged write boundary; route other contributions through their children",
                ));
            }
            originals.push(m.clone());
        }
        let base = self.exact_revision(source, request.production_base)?;
        let head = self.exact_revision(source, request.head)?;
        let staging = self.exact_revision(source, request.staging_revision)?;
        if !self.is_ancestor(source, &base.commit, &head.commit)? {
            return Err(Error::new(
                "replacement_mapping",
                "replacement head must be authored against the exact isolated production baseline",
            ));
        }
        let patch = self.patch(source, &base.commit, &head.commit)?;
        let paths = self.changed_paths(source, &base.commit, &head.commit)?;
        if patch.is_empty() || paths.is_empty() || paths.iter().any(|p| !request.scope.permits(p)) {
            return Err(Error::new(
                "scope_violation",
                "replacement requires a nonempty exact patch inside the original child scope",
            ));
        }
        let manifest = DeltaManifest {
            schema_version: DELTA_SCHEMA,
            git_version: self.version().to_owned(),
            repository: request.repository,
            parent_id: request.parent_id,
            child_id: request.child_id,
            work_generation: request.work_generation.into(),
            correction_generation: request.correction_generation.into(),
            scope: request.scope,
            validated_base: base,
            validated_head: head,
            staging_parent: staging.clone(),
            staging_commit: staging,
            patch_digest: bytes_digest(&patch),
            paths,
            replacement: Some(ReplacementMapping {
                originals,
                reason: request.reason.into(),
                staging_result_digest: request.staging_result_digest.into(),
            }),
        };
        manifest.original_contributions()?;
        Ok(DeltaArtifact { manifest, patch })
    }

    pub fn observe_production_integration(
        &self,
        source: &Path,
        request: ObserveProductionIntegration<'_>,
    ) -> Result<VerifiedProductionIntegration> {
        let c = &request.candidate.manifest;
        let result = c.result.as_ref().ok_or_else(|| {
            Error::new("production_provenance", "candidate assembly is unfinished")
        })?;
        if request.release_operation.is_nil()
            || request.pull_request == 0
            || c.ordered_delta_digests.is_empty()
            || c.ordered_delta_digests.iter().collect::<BTreeSet<_>>()
                != c.selected_contributions.keys().collect()
            || c.ordered_delta_digests.len() != c.ordered_patch_digests.len()
        {
            return Err(Error::new(
                "production_provenance",
                "production integration needs a completely accounted nonempty candidate and observed release identity",
            ));
        }
        let record = ProductionIntegration {
            repository: c.repository.clone(),
            parent_id: c.parent_id,
            release_operation: request.release_operation,
            pull_request: request.pull_request,
            source_candidate_digest: canonical_digest(1, c)?,
            accepted_evidence_token: request.authority.evidence_token().into(),
            acceptance_digest: request.authority.acceptance_digest().into(),
            production_parent: c.production.clone(),
            candidate: result.clone(),
            merged: self.exact_revision(source, request.merged_commit)?,
            observed_main: self.exact_revision(source, request.current_main)?,
            selected_delta_digests: c.ordered_delta_digests.clone(),
            selected_patch_digests: c.ordered_patch_digests.clone(),
            selected_contributions: c.selected_contributions.clone(),
            combined_patch_digest: c.combined_patch_digest.clone().ok_or_else(|| {
                Error::new("production_provenance", "candidate patch proof is missing")
            })?,
        };
        self.reverify_production_integration(
            source,
            &record,
            request.candidate,
            request.authority,
            request.current_main,
        )
    }

    pub fn reverify_production_integration(
        &self,
        source: &Path,
        recorded: &ProductionIntegration,
        retained_candidate: &Candidate,
        authority: &VerifiedAcceptedCandidate,
        current_main: &str,
    ) -> Result<VerifiedProductionIntegration> {
        let mut record = recorded.clone();
        let retained = self.inspect_candidate(
            retained_candidate.path.parent().ok_or_else(|| {
                Error::new("production_provenance", "retained candidate root missing")
            })?,
            retained_candidate.manifest.operation_id,
        )?;
        let c = &retained.manifest;
        if record.release_operation.is_nil()
            || record.pull_request == 0
            || record.source_candidate_digest != authority.candidate_digest()
            || record.accepted_evidence_token != authority.evidence_token()
            || record.acceptance_digest != authority.acceptance_digest()
            || c != &retained_candidate.manifest
            || record.source_candidate_digest != canonical_digest(1, c)?
            || record.repository != c.repository
            || record.parent_id != c.parent_id
            || record.production_parent != c.production
            || Some(&record.candidate) != c.result.as_ref()
            || record.selected_delta_digests != c.ordered_delta_digests
            || record.selected_patch_digests != c.ordered_patch_digests
            || record.selected_contributions != c.selected_contributions
            || Some(&record.combined_patch_digest) != c.combined_patch_digest.as_ref()
            || self.exact_revision(source, &record.production_parent.commit)?
                != record.production_parent
            || self.exact_revision(source, &record.candidate.commit)? != record.candidate
            || self.exact_revision(source, &record.merged.commit)? != record.merged
            || record.merged.tree != record.candidate.tree
        {
            return Err(Error::new(
                "production_provenance",
                "observed production objects differ from the exact candidate",
            ));
        }
        self.single_parent(
            source,
            &record.merged.commit,
            &record.production_parent.commit,
        )?;
        self.single_parent(
            source,
            &record.candidate.commit,
            &record.production_parent.commit,
        )?;
        let patch = self.patch(
            source,
            &record.production_parent.commit,
            &record.candidate.commit,
        )?;
        if patch.is_empty()
            || bytes_digest(&patch) != record.combined_patch_digest
            || patch
                != self.patch(
                    source,
                    &record.production_parent.commit,
                    &record.merged.commit,
                )?
            || !self.is_ancestor(source, &record.merged.commit, current_main)?
        {
            return Err(Error::new(
                "production_provenance",
                "production squash is not the exact assembled patch or is absent from current main",
            ));
        }
        record.observed_main = self.exact_revision(source, current_main)?;
        Ok(VerifiedProductionIntegration { record })
    }

    /// Requires a unique order from child dependencies and observed staging
    /// ancestry. Missing dependencies are not assumed Done; the caller supplies
    /// only dependencies already verified complete in `satisfied_children`.
    pub fn order_deltas(
        &self,
        source: &Path,
        deltas: &[DeltaArtifact],
        dependencies: &BTreeMap<Uuid, BTreeSet<Uuid>>,
        satisfied_children: &BTreeSet<Uuid>,
    ) -> Result<Vec<usize>> {
        let mut edges: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); deltas.len()];
        let mut digests = BTreeSet::new();
        for (index, delta) in deltas.iter().enumerate() {
            self.verify_delta(source, delta)?;
            if !digests.insert(delta.manifest.digest()?) {
                return Err(Error::new(
                    "duplicate_delta",
                    "selected child delta appears more than once",
                ));
            }
            for dependency in dependencies
                .get(&delta.manifest.child_id)
                .into_iter()
                .flatten()
            {
                let matches: Vec<usize> = deltas
                    .iter()
                    .enumerate()
                    .filter(|(_, d)| d.manifest.child_id == *dependency)
                    .map(|(i, _)| i)
                    .collect();
                if matches.is_empty() && !satisfied_children.contains(dependency) {
                    return Err(Error::new(
                        "missing_dependency",
                        "child dependency has no verified selected or satisfied outcome",
                    ));
                }
                edges[index].extend(matches);
            }
            for (earlier_index, earlier) in deltas.iter().enumerate() {
                if earlier_index != index
                    && earlier.manifest.repository == delta.manifest.repository
                    && self.is_ancestor(
                        source,
                        &earlier.manifest.staging_commit.commit,
                        &delta.manifest.staging_parent.commit,
                    )?
                {
                    edges[index].insert(earlier_index);
                }
            }
        }
        let mut selected = BTreeSet::new();
        let mut order = Vec::new();
        while selected.len() < deltas.len() {
            let ready: Vec<usize> = (0..deltas.len())
                .filter(|i| !selected.contains(i) && edges[*i].is_subset(&selected))
                .collect();
            if ready.len() != 1 {
                return Err(Error::new(
                    if ready.is_empty() {
                        "dependency_cycle"
                    } else {
                        "ambiguous_delta_order"
                    },
                    "selected deltas have no unique dependency/integration order",
                ));
            }
            selected.insert(ready[0]);
            order.push(ready[0]);
        }
        Ok(order)
    }

    fn assert_clean(&self, path: &Path) -> Result<()> {
        if !self
            .run(
                path,
                &[
                    "status",
                    "--porcelain=v1",
                    "--untracked-files=all",
                    "--ignored=matching",
                ],
                None,
            )?
            .is_empty()
        {
            return Err(Error::new(
                "dirty_candidate",
                "candidate contains ambient, untracked or uncommitted files; preserved for inspection",
            ));
        }
        Ok(())
    }

    /// Validation gates must test the recorded commit/tree, not a dirty working
    /// tree accidentally labelled with HEAD. Only previously declared untracked
    /// output files may differ; tracked source changes always invalidate it.
    pub fn verify_execution_tree(
        &self,
        source: &Path,
        expected: &Revision,
        outputs: &BTreeSet<PathBuf>,
    ) -> Result<()> {
        if self.revision(source, "HEAD")? != *expected {
            return Err(Error::new(
                "stale_revision",
                "execution HEAD differs from its admitted revision",
            ));
        }
        let status = self.run(
            source,
            &[
                "status",
                "--porcelain=v1",
                "-z",
                "--untracked-files=all",
                "--ignored=matching",
            ],
            None,
        )?;
        for entry in status.split(|b| *b == 0).filter(|p| !p.is_empty()) {
            if entry.len() < 4 || !matches!(&entry[..2], b"??" | b"!!") {
                return Err(Error::new(
                    "dirty_execution",
                    "tracked source/index differs from the exact revision under validation",
                ));
            }
            let path = std::str::from_utf8(&entry[3..]).map_err(|_| {
                Error::new("dirty_execution", "untracked execution path is not UTF-8")
            })?;
            validate_repo_path(path)?;
            if !outputs.contains(&source.join(path)) {
                return Err(Error::new(
                    "dirty_execution",
                    "execution contains an undeclared untracked or ignored file",
                ));
            }
        }
        Ok(())
    }

    /// Copies committed local Git objects into a fresh owned directory. The
    /// operator checkout/index and its dirty or partially staged files are read
    /// only and are never used as candidate input.
    pub fn assemble_candidate(
        &self,
        source: &Path,
        execution_root: &Path,
        request: CandidateRequest<'_>,
    ) -> Result<Candidate> {
        if self.host_scope.is_some() {
            return Err(Error::new(
                "scope_violation",
                "host metadata authority cannot mutate local repository or execution files",
            ));
        }
        request.repository.validate()?;
        request.identity.validate()?;
        let source =
            fs::canonicalize(source).map_err(|e| Error::io("resolve source repository", e))?;
        let root = owner_directory(execution_root)?;
        if root.starts_with(&source) || source.starts_with(&root) {
            return Err(Error::new(
                "unsafe_path",
                "candidate root and mutable source checkout must be separate",
            ));
        }
        let production = self.exact_revision(&source, request.production_commit)?;
        let mut ordered_delta_digests = Vec::new();
        let mut ordered_patch_digests = Vec::new();
        let mut selected_contributions = BTreeMap::new();
        let mut accounted_contributions = BTreeSet::new();
        let mut production_integrations = Vec::new();
        let mut integrated_ids = BTreeSet::new();
        for verified in request.production_integrations {
            let integration = verified.record();
            if integration.repository != request.repository
                || integration.parent_id != request.parent_id
                || integration.observed_main != production
                || !integrated_ids.insert(integration.digest()?)
                || !self.is_ancestor(&source, &integration.merged.commit, &production.commit)?
            {
                return Err(Error::new(
                    "production_provenance",
                    "prior integration proof is duplicated, stale or outside this parent's exact production baseline",
                ));
            }
            for contribution in integration.selected_contributions.values().flatten() {
                if !accounted_contributions.insert(contribution.clone()) {
                    return Err(Error::new(
                        "duplicate_delta",
                        "production contribution is counted twice",
                    ));
                }
            }
            production_integrations.push(integration.clone());
        }
        let mut seen = BTreeSet::new();
        for delta in request.deltas {
            self.verify_delta(&source, delta)?;
            if delta.manifest.parent_id != request.parent_id
                || delta.manifest.repository != request.repository
            {
                return Err(Error::new(
                    "scope_violation",
                    "candidate includes another parent or repository delta",
                ));
            }
            let digest = delta.manifest.digest()?;
            if !seen.insert(digest.clone()) {
                return Err(Error::new(
                    "duplicate_delta",
                    "candidate includes a delta more than once",
                ));
            }
            if delta.manifest.replacement.is_some() && delta.manifest.validated_base != production {
                return Err(Error::new(
                    "replacement_baseline",
                    "replacement was authored against a different isolated production baseline",
                ));
            }
            let contributions = delta.manifest.original_contributions()?;
            for contribution in &contributions {
                if !accounted_contributions.insert(contribution.clone()) {
                    return Err(Error::new(
                        "duplicate_delta",
                        "selected contribution is already integrated or selected through another artifact",
                    ));
                }
            }
            selected_contributions.insert(digest.clone(), contributions);
            ordered_delta_digests.push(digest);
            ordered_patch_digests.push(delta.manifest.patch_digest.clone());
        }
        let verified_order = self.order_deltas(
            &source,
            request.deltas,
            request.child_dependencies,
            request.satisfied_children,
        )?;
        if verified_order != (0..request.deltas.len()).collect::<Vec<_>>() {
            return Err(Error::new(
                "invalid_delta_order",
                "candidate patch order differs from verified dependency/integration ancestry",
            ));
        }
        let path = root.join(format!("candidate-{}", request.operation_id));
        if path.symlink_metadata().is_ok() {
            return Err(Error::new(
                "ownership_conflict",
                "candidate path already exists; reconcile it instead of resetting or cloning over it",
            ));
        }
        // No network or source hooks: --local copies objects; --no-hardlinks
        // gives the candidate independent storage. Git rejects symlinked object
        // directories. Alternates would defeat that independent retention.
        let git_dir = self.text(&source, &["rev-parse", "--absolute-git-dir"])?;
        let common_dir = self.text(
            &source,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )?;
        for dir in [Path::new(&git_dir), Path::new(&common_dir)] {
            if dir.join("objects/info/alternates").exists() {
                return Err(Error::new(
                    "unsupported_repository",
                    "source object alternates require explicit independent object retention",
                ));
            }
        }
        let args: Vec<OsString> = [
            "clone",
            "--local",
            "--no-hardlinks",
            "--no-checkout",
            "--template=/dev/null",
            "--",
        ]
        .into_iter()
        .map(OsString::from)
        .chain([source.as_os_str().to_owned(), path.as_os_str().to_owned()])
        .collect();
        let output = self.output(
            &root,
            &args.iter().map(OsString::as_os_str).collect::<Vec<_>>(),
            None,
        )?;
        if !output.status.success() {
            return Err(Error::new(
                "git_operation_failed",
                format!(
                    "candidate clone failed; preserve {} for reconciliation",
                    path.display()
                ),
            ));
        }
        set_owner_directory(&path)?;
        let mut manifest = CandidateManifest {
            schema_version: 1,
            operation_id: request.operation_id,
            runtime_id: request.runtime_id,
            repository: request.repository,
            parent_id: request.parent_id,
            work_generation: request.work_generation.into(),
            correction_generation: request.correction_generation.into(),
            git_version: self.version().to_owned(),
            production: production.clone(),
            ordered_delta_digests,
            ordered_patch_digests,
            selected_contributions,
            production_integrations,
            result: None,
            combined_patch_digest: None,
        };
        write_candidate_manifest(&path, &manifest)?;
        self.run(
            &path,
            &["checkout", "--detach", &production.commit, "--"],
            None,
        )?;
        self.assert_clean(&path)?;
        for delta in request.deltas {
            self.run(&path, &["apply", "--check", "--index", "--binary", "--whitespace=nowarn", "--"], Some(&delta.patch))
                .map_err(|_| Error::new("isolated_patch_conflict", "selected patch cannot apply unchanged to production; route to scoped child work or a declared dependency"))?;
            self.run(
                &path,
                &["apply", "--index", "--binary", "--whitespace=nowarn", "--"],
                Some(&delta.patch),
            )?;
        }
        if !self
            .run(
                &path,
                &[
                    "diff",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--name-only",
                    "--",
                ],
                None,
            )?
            .is_empty()
            || !self
                .run(&path, &["ls-files", "--others", "-z"], None)?
                .is_empty()
        {
            return Err(Error::new(
                "dirty_candidate",
                "candidate contains changes outside its selected patch application",
            ));
        }
        let tree = self.text(&path, &["write-tree"])?;
        validate_oid(&tree)?;
        if tree == production.tree {
            if !request.deltas.is_empty() {
                return Err(Error::new(
                    "no_change",
                    "selected patches cancel out; establish an evidenced no-change outcome before assembly",
                ));
            }
            manifest.result = Some(production);
            manifest.combined_patch_digest = Some(bytes_digest(&[]));
            write_candidate_manifest(&path, &manifest)?;
            return Ok(Candidate { path, manifest });
        }
        let message = format!(
            "feat(kegari): assemble accepted parent candidate\n\nParent: {}\nOperation: {}\nGeneration: {}\nPatch-manifest: {}\n",
            manifest.parent_id,
            manifest.operation_id,
            manifest.work_generation,
            canonical_digest(1, &manifest.ordered_delta_digests)?
        );
        let mut command = self.command(&path);
        command
            .env("GIT_AUTHOR_NAME", &request.identity.name)
            .env("GIT_AUTHOR_EMAIL", &request.identity.email)
            .env("GIT_COMMITTER_NAME", &request.identity.name)
            .env("GIT_COMMITTER_EMAIL", &request.identity.email)
            .args(["commit", "--signoff", "--no-gpg-sign", "-m", &message]);
        let output = self.finish_command(command, None)?;
        if !output.status.success() {
            return Err(Error::new(
                "git_operation_failed",
                "candidate commit failed; preserve the staged selected changes",
            ));
        }
        let result = self.revision(&path, "HEAD")?;
        self.single_parent(&path, &result.commit, &production.commit)?;
        if result.tree != tree {
            return Err(Error::new(
                "provenance_mismatch",
                "candidate commit changed the selected index tree",
            ));
        }
        self.assert_clean(&path)?;
        manifest.combined_patch_digest = Some(bytes_digest(&self.patch(
            &path,
            &production.commit,
            &result.commit,
        )?));
        manifest.result = Some(result);
        write_candidate_manifest(&path, &manifest)?;
        Ok(Candidate { path, manifest })
    }

    /// Recovery is verification-only: no reset, clean, reapply or new commit.
    pub fn inspect_candidate(
        &self,
        execution_root: &Path,
        operation_id: Uuid,
    ) -> Result<Candidate> {
        let root = owner_directory(execution_root)?;
        let path = root.join(format!("candidate-{operation_id}"));
        let actual = owner_directory(&path)?;
        if actual != path || actual.parent() != Some(root.as_path()) {
            return Err(Error::new(
                "unsafe_path",
                "candidate path escaped its owned root",
            ));
        }
        let bytes = fs::read(path.join(".git/kegari-candidate.json"))
            .map_err(|e| Error::io("read candidate identity", e))?;
        let manifest: CandidateManifest = serde_json::from_slice(&bytes)
            .map_err(|_| Error::new("corrupt_record", "candidate manifest is invalid"))?;
        if manifest.schema_version != 1
            || manifest.operation_id != operation_id
            || manifest.git_version != self.version()
        {
            return Err(Error::new(
                "ownership_conflict",
                "candidate identity/version mismatch",
            ));
        }
        let result = manifest.result.as_ref().ok_or_else(|| {
            Error::new(
                "unfinished_candidate",
                "candidate has not completed assembly; preserve its partial work",
            )
        })?;
        if self.exact_revision(&path, &manifest.production.commit)? != manifest.production
            || self.revision(&path, "HEAD")? != *result
            || manifest.combined_patch_digest
                != Some(bytes_digest(&self.patch(
                    &path,
                    &manifest.production.commit,
                    &result.commit,
                )?))
        {
            return Err(Error::new(
                "provenance_mismatch",
                "candidate Git state differs from its recorded identity",
            ));
        }
        if result != &manifest.production {
            self.single_parent(&path, &result.commit, &manifest.production.commit)?;
        } else if !manifest.ordered_delta_digests.is_empty()
            || !manifest.ordered_patch_digests.is_empty()
        {
            return Err(Error::new(
                "provenance_mismatch",
                "unchanged candidate must not claim an applied patch",
            ));
        }
        self.assert_clean(&path)?;
        Ok(Candidate { path, manifest })
    }
}

fn owner_directory(path: &Path) -> Result<PathBuf> {
    let canonical = fs::canonicalize(path).map_err(|e| Error::io("resolve owned directory", e))?;
    if path
        .symlink_metadata()
        .map_err(|e| Error::io("inspect owned directory", e))?
        .file_type()
        .is_symlink()
    {
        return Err(Error::new(
            "unsafe_path",
            "owned directory must not be a symlink",
        ));
    }
    let metadata = fs::metadata(&canonical).map_err(|e| Error::io("inspect owned directory", e))?;
    if !metadata.is_dir() {
        return Err(Error::new("unsafe_path", "owned path is not a directory"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // SAFETY: geteuid has no preconditions or memory arguments.
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
            return Err(Error::new(
                "unsafe_permissions",
                "execution root/candidate must be owner-only",
            ));
        }
    }
    Ok(canonical)
}

fn read_clone_record(path: &Path) -> Result<ExecutionCloneRecord> {
    let metadata = path
        .symlink_metadata()
        .map_err(|e| Error::io("inspect clone receipt", e))?;
    if !metadata.is_file() || metadata.len() > 64 * 1024 {
        return Err(Error::new(
            "ownership_conflict",
            "clone receipt is linked or invalid",
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        if metadata.nlink() != 1 {
            return Err(Error::new(
                "ownership_conflict",
                "clone receipt must not be hardlinked",
            ));
        }
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    use std::io::Read;
    let mut bytes = Vec::new();
    options
        .open(path)
        .map_err(|e| Error::io("open clone receipt", e))?
        .take(64 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| Error::io("read clone receipt", e))?;
    let record: ExecutionCloneRecord = serde_json::from_slice(&bytes)
        .map_err(|_| Error::new("ownership_conflict", "clone receipt is malformed"))?;
    Ok(record)
}

fn write_clone_record(path: &Path, record: &ExecutionCloneRecord, exclusive: bool) -> Result<()> {
    let bytes =
        serde_json::to_vec(record).map_err(|_| Error::invalid("encode execution clone receipt"))?;
    let target = if exclusive {
        path.to_owned()
    } else {
        path.with_extension(format!("{}.tmp", Uuid::new_v4()))
    };
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let mut file = options
        .open(&target)
        .map_err(|e| Error::io("create execution clone receipt", e))?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|e| Error::io("persist execution clone receipt", e))?;
    if !exclusive {
        fs::rename(&target, path).map_err(|e| Error::io("finalize execution clone receipt", e))?;
    }
    File::open(
        path.parent()
            .ok_or_else(|| Error::invalid("clone receipt parent absent"))?,
    )
    .and_then(|parent| parent.sync_all())
    .map_err(|e| Error::io("sync execution clone receipt", e))
}

fn set_owner_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|e| Error::io("protect candidate directory", e))?;
    }
    Ok(())
}

fn write_candidate_manifest(path: &Path, manifest: &CandidateManifest) -> Result<()> {
    let git_dir = path.join(".git");
    let tmp = git_dir.join(format!(".kegari-candidate-{}.tmp", Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&tmp)
        .map_err(|e| Error::io("create candidate manifest", e))?;
    let bytes = serde_json::to_vec(manifest)
        .map_err(|_| Error::invalid("candidate manifest serialization failed"))?;
    file.write_all(&bytes)
        .map_err(|e| Error::io("write candidate manifest", e))?;
    file.sync_all()
        .map_err(|e| Error::io("flush candidate manifest", e))?;
    fs::rename(&tmp, git_dir.join("kegari-candidate.json"))
        .map_err(|e| Error::io("publish candidate manifest", e))?;
    File::open(&git_dir)
        .and_then(|f| f.sync_all())
        .map_err(|e| Error::io("flush candidate directory", e))?;
    Ok(())
}

/// Invokes the actual synchronous spawn while the host holds its short current
/// admission gate. The outer operation already has a durable Sent receipt.
pub type GitStartGate =
    std::sync::Arc<dyn Fn(&mut dyn FnMut() -> Result<()>) -> Result<()> + Send + Sync>;
#[derive(Clone, Debug)]
pub struct GitLimits {
    pub command_timeout: std::time::Duration,
    pub operation_timeout: std::time::Duration,
    pub output_bytes: usize,
    pub input_bytes: usize,
}
impl Default for GitLimits {
    fn default() -> Self {
        Self {
            command_timeout: std::time::Duration::from_secs(30),
            operation_timeout: std::time::Duration::from_secs(300),
            output_bytes: 64 * 1024 * 1024,
            input_bytes: 256 * 1024 * 1024,
        }
    }
}
impl GitLimits {
    fn validate(&self) -> Result<()> {
        if self.command_timeout.is_zero()
            || self.command_timeout > std::time::Duration::from_secs(300)
            || self.operation_timeout.is_zero()
            || self.operation_timeout > std::time::Duration::from_secs(3600)
            || self.output_bytes == 0
            || self.output_bytes > 256 * 1024 * 1024
            || self.input_bytes == 0
            || self.input_bytes > 256 * 1024 * 1024
        {
            return Err(Error::invalid(
                "Git operation limits must be finite and bounded",
            ));
        }
        Ok(())
    }
}
struct ActiveGitChild {
    child: std::process::Child,
    group: Option<crate::process_group::OwnedGroup>,
    terminal: Option<std::process::ExitStatus>,
    guard: Option<crate::process_records::SpawnGuard>,
}
impl ActiveGitChild {
    fn observe(&mut self) -> Result<Option<std::process::ExitStatus>> {
        if self.terminal.is_none()
            && self
                .group
                .as_mut()
                .map_or(Ok(true), |group| group.observe_exit_before_reap())?
        {
            self.terminal = self
                .child
                .try_wait()
                .map_err(|error| Error::io("reap owned Git after group cleanup", error))?;
        }
        if let Some(status) = self.terminal
            && let Some(guard) = self.guard.as_mut()
        {
            guard.group_exited(status)?;
            self.guard = None;
        }
        Ok(self.terminal)
    }
}
struct GitOperationState {
    cancelled: std::sync::atomic::AtomicBool,
    closed: std::sync::atomic::AtomicBool,
    uncertain_descendants: std::sync::atomic::AtomicBool,
    active: std::sync::Mutex<Option<ActiveGitChild>>,
    provided_guard: std::sync::Mutex<Option<crate::process_records::SpawnGuard>>,
    deadline: std::time::Instant,
    limits: GitLimits,
    gate: Option<GitStartGate>,
    journal: Option<(
        std::sync::Arc<crate::process_records::ProcessJournal>,
        crate::control::Permit,
    )>,
}
impl GitOperationState {
    fn new(limits: GitLimits, gate: Option<GitStartGate>) -> Result<Self> {
        limits.validate()?;
        Ok(Self {
            cancelled: false.into(),
            closed: false.into(),
            uncertain_descendants: false.into(),
            active: std::sync::Mutex::new(None),
            provided_guard: std::sync::Mutex::new(None),
            deadline: std::time::Instant::now() + limits.operation_timeout,
            limits,
            gate,
            journal: None,
        })
    }
    fn current(&self) -> Result<()> {
        use std::sync::atomic::Ordering;
        if self.cancelled.load(Ordering::SeqCst) || self.closed.load(Ordering::SeqCst) {
            return Err(Error::new("cancelled", "Git operation is fenced or ended"));
        }
        if std::time::Instant::now() >= self.deadline {
            return Err(Error::new(
                "timeout",
                "Git operation exceeded its finite deadline",
            ));
        }
        Ok(())
    }
    fn cancel(&self) -> Result<()> {
        self.cancelled
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let mut active = self
            .active
            .lock()
            .map_err(|_| Error::new("ownership_conflict", "Git child ownership lock failed"))?;
        if let Some(process) = active.as_mut()
            && process.observe()?.is_none()
        {
            if let Some(group) = process.group.as_mut() {
                group.terminate(crate::process_group::Termination::Kill)?;
            } else if let Err(error) = process.child.kill()
                && error.raw_os_error() != Some(libc::ESRCH)
            {
                return Err(Error::io(
                    "terminate owned Git child without group proof",
                    error,
                ));
            }
        }

        Ok(())
    }
    #[cfg(unix)]
    fn run(&self, mut command: Command, input: Option<&[u8]>) -> Result<Output> {
        use std::io::Read;
        use std::os::fd::AsRawFd;
        use std::os::unix::process::CommandExt;
        self.current()?;
        let input = input.unwrap_or(&[]);
        if input.len() > self.limits.input_bytes {
            return Err(Error::new(
                "bounded_input",
                "Git input exceeds configured bound",
            ));
        }
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);
        let mut pipes = None;
        let mut start = || -> Result<()> {
            self.current()?;
            let mut active = self
                .active
                .lock()
                .map_err(|_| Error::new("ownership_conflict", "Git child ownership lock failed"))?;
            if active.is_some() {
                return Err(Error::new(
                    "ownership_conflict",
                    "one owned Git sequence cannot run parallel subprocesses",
                ));
            }
            self.current()?;
            let mut guard = self
                .journal
                .as_ref()
                .map(|(journal, permit)| {
                    use std::os::unix::ffi::OsStrExt;
                    let executable = crate::platform::BinaryIdentity::inspect(
                        Path::new(command.get_program()),
                        false,
                    )?;
                    let cwd = command
                        .get_current_dir()
                        .ok_or_else(|| Error::invalid("journaled Git needs explicit cwd"))?
                        .canonicalize()
                        .map_err(|e| Error::io("resolve journaled Git cwd", e))?;
                    journal.begin(crate::process_records::SpawnBinding {
                        operation_id: permit.operation_id,
                        effect_id: Uuid::new_v4(),
                        lease: Some(permit.lease.clone()),
                        control_owner: None,
                        work_key: permit.work_key.clone(),
                        scope_digest: permit.scope_digest.clone(),
                        cwd,
                        observed_executables: vec![],
                        executable,
                        argv_digest: canonical_digest(
                            1,
                            &command
                                .get_args()
                                .map(|a| a.as_bytes().to_vec())
                                .collect::<Vec<_>>(),
                        )?,
                        kind: "trusted_git".into(),
                    })
                })
                .transpose()?;
            if let Some(provided) = self
                .provided_guard
                .lock()
                .map_err(|_| Error::new("ownership_conflict", "host Git guard unavailable"))?
                .take()
            {
                if guard.is_some() {
                    return Err(Error::new(
                        "ownership_conflict",
                        "Git process has two distinct authorities",
                    ));
                }
                guard = Some(provided);
            }
            if let Some(guard) = &guard {
                guard.configure_std(&mut command)?;
            }
            let (mut child, mut group) = crate::process_group::OwnedGroup::spawn_std(&mut command)?;
            let capture_error = guard
                .as_mut()
                .and_then(|guard| guard.spawned_std(&child).err());
            let capture_error = match capture_error {
                Some(error) => match group.observe_exit_before_reap() {
                    Ok(true) => None,
                    Ok(false) => Some(error),
                    Err(error) => Some(error),
                },
                None => None,
            };
            pipes = Some((
                child.stdin.take().expect("piped stdin"),
                child.stdout.take().expect("piped stdout"),
                child.stderr.take().expect("piped stderr"),
            ));
            *active = Some(ActiveGitChild {
                child,
                group: Some(group),
                terminal: None,
                guard,
            });
            match capture_error {
                Some(error) => Err(error),
                None => Ok(()),
            }
        };
        let gated = if let Some(gate) = &self.gate {
            gate(&mut start)
        } else {
            start()
        };
        // Even a faulty gate which fails after spawning must retain and reap
        // the owned child; returning early would detach an external effect.
        if pipes.is_none() {
            gated?;
            return Err(Error::new(
                "ownership_conflict",
                "Git start gate did not launch owned process",
            ));
        }
        let (stdin, mut stdout, mut stderr) = pipes.ok_or_else(|| {
            Error::new(
                "ownership_conflict",
                "Git start gate did not launch owned process",
            )
        })?;
        let nonblocking = |fd| -> Result<()> {
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
            if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
            {
                return Err(Error::io(
                    "configure bounded Git pipe",
                    std::io::Error::last_os_error(),
                ));
            }
            Ok(())
        };
        let configured = nonblocking(stdin.as_raw_fd())
            .and_then(|_| nonblocking(stdout.as_raw_fd()))
            .and_then(|_| nonblocking(stderr.as_raw_fd()));
        let mut failure = gated.err().or_else(|| configured.err());
        let mut stdin = Some(stdin);
        let mut written = 0usize;
        let mut out = Vec::new();
        let mut err = Vec::new();
        let mut out_eof = false;
        let mut err_eof = false;
        let started = std::time::Instant::now();
        let mut terminal_at = None;
        let mut cancelled_at = None;
        let drain = |stream: &mut dyn Read, bytes: &mut Vec<u8>| -> Result<bool> {
            let mut buffer = [0u8; 8192];
            loop {
                match stream.read(&mut buffer) {
                    Ok(0) => return Ok(true),
                    Ok(n) => {
                        if bytes
                            .len()
                            .checked_add(n)
                            .is_none_or(|size| size > self.limits.output_bytes)
                        {
                            return Err(Error::new(
                                "output_limit",
                                "Git output exceeded its configured byte bound",
                            ));
                        }
                        bytes.extend_from_slice(&buffer[..n]);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(e) => return Err(Error::io("read owned Git output", e)),
                }
            }
        };
        loop {
            if failure.is_none() {
                failure = self.current().err();
            }
            if started.elapsed() >= self.limits.command_timeout && failure.is_none() {
                failure = Some(Error::new("timeout", "Git command exceeded finite timeout"));
            }
            if failure.is_none() {
                if written == input.len() {
                    stdin.take();
                } else if let Some(pipe) = stdin.as_mut() {
                    match pipe.write(&input[written..input.len().min(written + 65536)]) {
                        Ok(n) => written += n,
                        Err(e)
                            if e.kind() == std::io::ErrorKind::WouldBlock
                                || e.kind() == std::io::ErrorKind::Interrupted => {}
                        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => {
                            stdin.take();
                        }
                        Err(e) => failure = Some(Error::io("write owned Git input", e)),
                    }
                }
                match drain(&mut stdout, &mut out) {
                    Ok(done) => out_eof = done,
                    Err(e) => failure = Some(e),
                }
                match drain(&mut stderr, &mut err) {
                    Ok(done) => err_eof = done,
                    Err(e) => {
                        if failure.is_none() {
                            failure = Some(e)
                        }
                    }
                }
            }
            if failure.is_some() && cancelled_at.is_none() {
                self.cancel()?;
                cancelled_at = Some(std::time::Instant::now());
                stdin.take();
            }
            let terminal = {
                let mut active = self.active.lock().map_err(|_| {
                    Error::new("ownership_conflict", "Git child ownership lock failed")
                })?;
                let process = active.as_mut().ok_or_else(|| {
                    Error::new("ownership_conflict", "owned Git child disappeared")
                })?;
                process.observe()?
            };
            if let Some(status) = terminal {
                let since = terminal_at.get_or_insert_with(std::time::Instant::now);
                if failure.is_some()
                    || out_eof && err_eof
                    || since.elapsed() >= std::time::Duration::from_secs(1)
                {
                    self.active
                        .lock()
                        .map_err(|_| {
                            Error::new("ownership_conflict", "Git child ownership lock failed")
                        })?
                        .take();
                    if let Some(error) = failure {
                        return Err(error);
                    }
                    if !out_eof || !err_eof {
                        self.uncertain_descendants
                            .store(true, std::sync::atomic::Ordering::SeqCst);
                        return Err(Error::new(
                            "operation_uncertain",
                            "Git output pipes remained open after owned child exit",
                        ));
                    }
                    if status.success() && written != input.len() {
                        return Err(Error::new(
                            "git_operation_failed",
                            "Git did not consume the complete intended input",
                        ));
                    }
                    return Ok(Output {
                        status,
                        stdout: out,
                        stderr: err,
                    });
                }
            }
            if cancelled_at.is_some_and(|t| t.elapsed() >= std::time::Duration::from_secs(5)) {
                return Err(Error::new(
                    "operation_uncertain",
                    "cancelled Git child has not been observed exited",
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
    #[cfg(not(unix))]
    fn run(&self, _: Command, _: Option<&[u8]>) -> Result<Output> {
        Err(Error::new(
            "prerequisite",
            "bounded Git execution requires the supported Unix host",
        ))
    }
}
#[derive(Clone)]
pub struct GitCancellation {
    state: std::sync::Arc<GitOperationState>,
}
impl GitCancellation {
    pub fn cancel(&self) -> Result<()> {
        self.state.cancel()
    }
    pub fn exited(&self) -> Result<bool> {
        if self
            .state
            .uncertain_descendants
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Ok(false);
        }
        let mut active = self
            .state
            .active
            .lock()
            .map_err(|_| Error::new("ownership_conflict", "Git child ownership lock failed"))?;
        if let Some(process) = active.as_mut()
            && process.observe()?.is_none()
        {
            return Ok(false);
        }

        Ok(self.state.closed.load(std::sync::atomic::Ordering::SeqCst))
    }
}
pub struct RunningGitOperation<T: Send + 'static> {
    state: std::sync::Arc<GitOperationState>,
    thread: Option<std::thread::JoinHandle<Result<T>>>,
}
impl<T: Send + 'static> RunningGitOperation<T> {
    pub fn cancellation(&self) -> GitCancellation {
        GitCancellation {
            state: self.state.clone(),
        }
    }
    pub async fn wait(mut self) -> Result<T> {
        let thread = self
            .thread
            .take()
            .ok_or_else(|| Error::new("ownership_conflict", "Git operation already awaited"))?;
        tokio::task::spawn_blocking(move || {
            thread.join().map_err(|_| {
                Error::new(
                    "operation_uncertain",
                    "owned Git operation panicked; retain its artifacts",
                )
            })?
        })
        .await
        .map_err(|_| Error::new("operation_uncertain", "owned Git waiter interrupted"))?
    }
}
impl<T: Send + 'static> Drop for RunningGitOperation<T> {
    fn drop(&mut self) {
        let _ = self.state.cancel();
    }
}
impl Git {
    /// Launch under ControlHost::begin_effect and register cancellation before
    /// releasing its gate; await outside and complete the effect afterward.
    pub fn start_operation<T: Send + 'static>(
        &self,
        limits: GitLimits,
        start_gate: GitStartGate,
        work: impl FnOnce(Git) -> Result<T> + Send + 'static,
    ) -> Result<RunningGitOperation<T>> {
        self.start_operation_inner(limits, start_gate, None, work)
    }
    pub fn start_journaled_operation<T: Send + 'static>(
        &self,
        limits: GitLimits,
        start_gate: GitStartGate,
        journal: std::sync::Arc<crate::process_records::ProcessJournal>,
        permit: crate::control::Permit,
        work: impl FnOnce(Git) -> Result<T> + Send + 'static,
    ) -> Result<RunningGitOperation<T>> {
        self.start_operation_inner(limits, start_gate, Some((journal, permit)), work)
    }
    fn start_operation_inner<T: Send + 'static>(
        &self,
        limits: GitLimits,
        start_gate: GitStartGate,
        journal: Option<(
            std::sync::Arc<crate::process_records::ProcessJournal>,
            crate::control::Permit,
        )>,
        work: impl FnOnce(Git) -> Result<T> + Send + 'static,
    ) -> Result<RunningGitOperation<T>> {
        if self.host_scope.is_some() {
            return Err(Error::new(
                "scope_violation",
                "read-only host Git authority cannot start a mutating worker operation",
            ));
        }
        let mut state = GitOperationState::new(limits, Some(start_gate))?;
        state.journal = journal;
        let state = std::sync::Arc::new(state);
        let mut owned = self.clone();
        owned.operation = Some(state.clone());
        let finished = state.clone();
        let thread = std::thread::Builder::new()
            .name("kegari-owned-git".into())
            .spawn(move || {
                struct Closed(std::sync::Arc<GitOperationState>);
                impl Drop for Closed {
                    fn drop(&mut self) {
                        self.0
                            .closed
                            .store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                }
                let _closed = Closed(finished);
                if owned.version().is_empty()
                    || owned
                        .operation
                        .as_ref()
                        .is_some_and(|s| s.journal.is_some())
                {
                    owned.verify_version()?;
                }
                work(owned)
            })
            .map_err(|e| Error::io("start owned Git operation", e))?;
        Ok(RunningGitOperation {
            state,
            thread: Some(thread),
        })
    }
}

#[cfg(all(test, unix))]
mod bounded_process_tests {
    use super::*;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    // Ordinary owned OS process fixtures test lifecycle, not sandbox enforcement.
    #[test]
    fn timeout_and_post_spawn_gate_failure_reap_owned_processes() {
        let state = GitOperationState::new(
            GitLimits {
                command_timeout: Duration::from_millis(30),
                ..GitLimits::default()
            },
            None,
        )
        .unwrap();
        let mut sleeper = Command::new("/bin/sleep");
        sleeper.arg("10");
        let started = Instant::now();
        assert_eq!(state.run(sleeper, None).unwrap_err().code, "timeout");
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(state.active.lock().unwrap().is_none());

        let state = GitOperationState::new(
            GitLimits::default(),
            Some(Arc::new(|launch| {
                launch()?;
                Err(Error::new(
                    "fixture_after_spawn",
                    "failed final gate record",
                ))
            })),
        )
        .unwrap();
        let mut sleeper = Command::new("/bin/sleep");
        sleeper.arg("10");
        assert_eq!(
            state.run(sleeper, None).unwrap_err().code,
            "fixture_after_spawn"
        );
        assert!(state.active.lock().unwrap().is_none());
    }
}

fn validate_independent_clone_identity(identity: &IndependentCloneIdentity) -> Result<()> {
    identity.repository.validate()?;
    identity.head.validate()?;
    identity.base.validate()?;
    let repository = identity
        .repository
        .id
        .parse::<u64>()
        .map_err(|_| Error::invalid("review repository needs canonical numeric GitHub ID"))?;
    let expected = canonical_digest(
        1,
        &serde_json::json!({"repository":repository,"pr":identity.pull_request,"head":identity.head.commit,"base":identity.base.commit}),
    )?;
    if identity.schema_version != 1
        || identity.runtime_id.is_nil()
        || identity.workspace_id.is_nil()
        || identity.directory_id.is_nil()
        || identity.pull_request == 0
        || identity.review_identity != expected
    {
        return Err(Error::new(
            "execution_ownership",
            "independent review clone identity is incomplete or mismatched",
        ));
    }
    Ok(())
}

/// Read-only source roots admitted by typed host configuration or authenticated
/// candidate retention. This capability never permits Git writes or execution
/// of repository scripts; host_process additionally checks the fixed grammar.
#[derive(Clone, Debug)]
pub struct VerifiedHostReadRoot {
    repository: RepositoryId,
    root: PathBuf,
    configuration_digest: String,
    lineage_digest: String,
    stamps: Vec<HostReadStamp>,
    marker_digest: Option<(PathBuf, String)>,
}
#[derive(Clone, Debug, Serialize)]
struct HostReadStamp {
    path: PathBuf,
    device: u64,
    inode: u64,
    uid: u32,
    mode: u32,
    directory: bool,
}
fn host_read_stamp(path: &Path) -> Result<HostReadStamp> {
    use std::os::unix::fs::MetadataExt;
    let metadata =
        std::fs::symlink_metadata(path).map_err(|e| Error::io("inspect host Git read root", e))?;
    if metadata.file_type().is_symlink() || !(metadata.is_dir() || metadata.is_file()) {
        return Err(Error::new(
            "execution_ownership",
            "host Git read path is redirected or special",
        ));
    }
    Ok(HostReadStamp {
        path: path.to_path_buf(),
        device: metadata.dev(),
        inode: metadata.ino(),
        uid: metadata.uid(),
        mode: metadata.mode(),
        directory: metadata.is_dir(),
    })
}
impl VerifiedHostReadRoot {
    pub fn config_revision(&self) -> &str {
        &self.configuration_digest
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn repository(&self) -> &RepositoryId {
        &self.repository
    }
    pub fn digest(&self) -> Result<String> {
        canonical_digest(
            1,
            &serde_json::json!({"repository":self.repository,"root":self.root,
        "configuration":self.configuration_digest,"lineage":self.lineage_digest,"stamps":self.stamps,"marker":self.marker_digest}),
        )
    }
    pub fn revalidate_filesystem(&self) -> Result<()> {
        for expected in &self.stamps {
            if canonical_digest(1, expected)?
                != canonical_digest(1, &host_read_stamp(&expected.path)?)?
            {
                return Err(Error::new(
                    "execution_ownership",
                    "host Git read path identity changed",
                ));
            }
        }
        if let Some((path, digest)) = &self.marker_digest
            && bytes_digest(&bounded_host_marker(path)?) != *digest
        {
            return Err(Error::new(
                "execution_ownership",
                "host Git retained marker changed",
            ));
        }
        Ok(())
    }
}
fn bounded_host_marker(path: &Path) -> Result<Vec<u8>> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| Error::io("read host Git identity marker", e))?;
    let meta = file
        .metadata()
        .map_err(|e| Error::io("inspect host Git identity marker", e))?;
    if !meta.is_file() || meta.len() > 1024 * 1024 {
        return Err(Error::new(
            "bounded_input",
            "Git identity marker is not a bounded file",
        ));
    }
    let mut bytes = Vec::new();
    file.take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| Error::io("read bounded Git marker", e))?;
    if bytes.len() > 1024 * 1024 {
        return Err(Error::new(
            "bounded_input",
            "Git identity marker grew beyond limit",
        ));
    }
    Ok(bytes)
}
fn host_read_root(
    repository: RepositoryId,
    path: &Path,
    configuration_digest: String,
    lineage_digest: String,
    marker: Option<PathBuf>,
) -> Result<VerifiedHostReadRoot> {
    repository.validate()?;
    let root = path
        .canonicalize()
        .map_err(|e| Error::io("resolve configured Git read root", e))?;
    if root != path {
        return Err(Error::new(
            "execution_ownership",
            "Git read root must be its configured canonical path",
        ));
    }
    let mut stamps = vec![host_read_stamp(&root)?];
    if !stamps[0].directory {
        return Err(Error::new(
            "execution_ownership",
            "Git source root is not a directory",
        ));
    }
    let metadata = root.join(".git");
    if metadata
        .try_exists()
        .map_err(|e| Error::io("inspect Git metadata presence", e))?
    {
        stamps.push(host_read_stamp(&metadata)?);
    }
    let marker_digest = marker
        .map(|path| Ok((path.clone(), bytes_digest(&bounded_host_marker(&path)?))))
        .transpose()?;
    Ok(VerifiedHostReadRoot {
        repository,
        root,
        configuration_digest,
        lineage_digest,
        stamps,
        marker_digest,
    })
}
#[derive(Clone, Copy, Debug)]
pub enum CleanupRead {
    Status,
    Heads,
    Head,
    Fsck,
}
impl CleanupRead {
    pub fn args(self) -> &'static [&'static str] {
        match self {
            Self::Status => &["status", "--porcelain=v1", "--untracked-files=all"],
            Self::Heads => &["for-each-ref", "--format=%(objectname)", "refs/heads"],
            Self::Head => &["rev-parse", "--verify", "HEAD"],
            Self::Fsck => &["fsck", "--full", "--no-reflogs"],
        }
    }
}
pub fn cleanup_host_read_root(
    proof: &crate::housekeeping::VerifiedCleanupReadRoot,
    configuration: &crate::config::Config,
) -> Result<VerifiedHostReadRoot> {
    proof.revalidate_filesystem()?;
    if proof.runtime_id() != configuration.runtime.id {
        return Err(Error::new(
            "identity_mismatch",
            "cleanup read belongs to another runtime",
        ));
    }
    host_read_root(
        proof.repository().clone(),
        proof.root(),
        configuration.revision()?,
        proof.digest()?,
        None,
    )
}
/// This constructor performs only filesystem/configuration reads. Provider
/// repository identity still requires its ordinary current account/scope check.
pub fn configured_host_read_root(
    configuration: &crate::config::Config,
    workspace: Uuid,
    contract: &crate::delivery::RepositoryContract,
) -> Result<VerifiedHostReadRoot> {
    let selected = configuration
        .workspaces
        .values()
        .find(|w| w.id == workspace)
        .and_then(|w| w.repositories.get(&contract.configuration_name))
        .ok_or_else(|| {
            Error::new(
                "scope_violation",
                "Git read repository is not configured in this workspace",
            )
        })?;
    if canonical_digest(1, selected)? != canonical_digest(1, &contract.configuration)?
        || contract.repository.id != contract.provider.id.to_string()
    {
        return Err(Error::new(
            "scope_violation",
            "Git read source differs from exact repository contract",
        ));
    }
    let root = Path::new(&selected.source);
    let marker = root.join(".git");
    host_read_root(
        contract.repository.clone(),
        root,
        configuration.revision()?,
        contract.digest()?,
        marker.is_file().then_some(marker),
    )
}
/// Only the delivery adapter calls this after authenticating the complete
/// retained slice and its exact acknowledged local effect.
pub(crate) fn retained_candidate_read_root(
    repository: RepositoryId,
    path: &Path,
    config_revision: String,
    record_digest: String,
    expected: &CandidateManifest,
) -> Result<VerifiedHostReadRoot> {
    let root = owner_directory(path)?;
    let actual: CandidateManifest = serde_json::from_slice(&bounded_host_marker(
        &root.join(".git/kegari-candidate.json"),
    )?)
    .map_err(|_| Error::new("record_corrupt", "candidate marker invalid"))?;
    if &actual != expected {
        return Err(Error::new(
            "provenance_mismatch",
            "candidate marker differs from authenticated slice",
        ));
    }
    host_read_root(
        repository,
        &root,
        config_revision,
        record_digest,
        Some(root.join(".git/kegari-candidate.json")),
    )
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GitCommitIdentity {
    pub name: String,
    pub email: String,
    pub date: String,
}
/// Exact source object bytes exported for the configured provider's Git data
/// API. The returned SHA must still equal this actual Git commit after creation.
pub struct GitPublicationObjects {
    base: Revision,
    revision: Revision,
    author: GitCommitIdentity,
    committer: GitCommitIdentity,
    message: String,
    entries: Vec<SourceTreeEntry>,
    blobs: BTreeMap<String, Vec<u8>>,
}
impl GitPublicationObjects {
    pub fn base(&self) -> &Revision {
        &self.base
    }
    pub fn revision(&self) -> &Revision {
        &self.revision
    }
    pub fn author(&self) -> &GitCommitIdentity {
        &self.author
    }
    pub fn committer(&self) -> &GitCommitIdentity {
        &self.committer
    }
    pub fn message(&self) -> &str {
        &self.message
    }
    pub fn entries(&self) -> &[SourceTreeEntry] {
        &self.entries
    }
    pub fn blobs(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.blobs
    }
}
fn commit_object_identity(line: &str, prefix: &str) -> Result<GitCommitIdentity> {
    let text = line
        .strip_prefix(prefix)
        .ok_or_else(|| Error::new("unsupported_repository", "commit identity header missing"))?;
    let (identity, date) = text
        .rsplit_once("> ")
        .ok_or_else(|| Error::new("unsupported_repository", "commit identity is malformed"))?;
    let (name, email) = identity
        .split_once(" <")
        .ok_or_else(|| Error::new("unsupported_repository", "commit identity is malformed"))?;
    let identity = CommitIdentity {
        name: name.into(),
        email: email.into(),
    };
    identity.validate()?;
    let (seconds, offset) = date
        .split_once(' ')
        .ok_or_else(|| Error::new("unsupported_repository", "commit date is malformed"))?;
    let seconds: i64 = seconds
        .parse()
        .map_err(|_| Error::new("unsupported_repository", "commit timestamp is invalid"))?;
    if offset == "-0000" {
        return Err(Error::new(
            "unsupported_repository",
            "negative-zero commit timezone cannot be reproduced by the provider date contract",
        ));
    }
    if offset.len() != 5
        || !matches!(offset.as_bytes()[0], b'+' | b'-')
        || !offset.as_bytes()[1..].iter().all(u8::is_ascii_digit)
    {
        return Err(Error::new(
            "unsupported_repository",
            "commit timezone is invalid",
        ));
    }
    let hour: i32 = offset[1..3]
        .parse()
        .map_err(|_| Error::new("unsupported_repository", "commit timezone is invalid"))?;
    let minute: i32 = offset[3..5]
        .parse()
        .map_err(|_| Error::new("unsupported_repository", "commit timezone is invalid"))?;
    if hour > 23 || minute > 59 {
        return Err(Error::new(
            "unsupported_repository",
            "commit timezone is invalid",
        ));
    }
    let zone = chrono::FixedOffset::east_opt(
        (hour * 3600 + minute * 60) * if offset.starts_with('-') { -1 } else { 1 },
    )
    .ok_or_else(|| Error::new("unsupported_repository", "commit timezone is invalid"))?;
    let date = chrono::DateTime::from_timestamp(seconds, 0)
        .ok_or_else(|| {
            Error::new(
                "unsupported_repository",
                "commit date outside supported range",
            )
        })?
        .with_timezone(&zone)
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, false);
    Ok(GitCommitIdentity {
        name: identity.name,
        email: identity.email,
        date,
    })
}
impl Git {
    /// Read only the explicitly configured repository-local human identity.
    /// No global or inferred provider identity can satisfy source signoff.
    pub fn configured_commit_identity(&self, source: &Path) -> Result<CommitIdentity> {
        let read = |name: &str| -> Result<String> {
            let output = self
                .run(source, &["config", "--local", "--get", name], None)
                .map_err(|error| {
                    Error::new(
                        "commit_identity_required",
                        "configure repository-local user.name and user.email before source commits",
                    )
                    .detail(serde_json::json!({"field":name,"cause":error.code}))
                })?;
            let value = String::from_utf8(output).map_err(|_| {
                Error::new(
                    "commit_identity_required",
                    "local commit identity is not UTF-8",
                )
            })?;
            Ok(value.strip_suffix('\n').unwrap_or(&value).to_owned())
        };
        let identity = CommitIdentity {
            name: read("user.name")?,
            email: read("user.email")?,
        };
        identity.validate()?;
        Ok(identity)
    }
    /// Read-only object export. Only a plain one-parent commit is supported;
    /// signatures/extra headers are never silently dropped to create a new SHA.
    pub fn publication_objects(
        &self,
        source: &Path,
        base: &Revision,
        head: &Revision,
    ) -> Result<GitPublicationObjects> {
        if self.exact_revision(source, &base.commit)? != *base
            || self.exact_revision(source, &head.commit)? != *head
            || base.tree == head.tree
        {
            return Err(Error::new(
                "provenance_mismatch",
                "publication object revisions are missing or unchanged",
            ));
        }
        self.single_parent(source, &head.commit, &base.commit)?;
        let raw =
            String::from_utf8(self.run(source, &["cat-file", "commit", &head.commit], None)?)
                .map_err(|_| Error::new("unsupported_repository", "commit object is not UTF-8"))?;
        let (headers, message) = raw.split_once("\n\n").ok_or_else(|| {
            Error::new(
                "unsupported_repository",
                "commit object has no message boundary",
            )
        })?;
        let headers: Vec<_> = headers.split('\n').collect();
        if headers.len() != 4
            || headers[0] != format!("tree {}", head.tree)
            || headers[1] != format!("parent {}", base.commit)
            || message.is_empty()
            || message.len() > 65536
            || message.contains('\0')
        {
            return Err(Error::new(
                "unsupported_repository",
                "commit contains unsupported headers or message",
            ));
        }
        let author = commit_object_identity(headers[2], "author ")?;
        let committer = commit_object_identity(headers[3], "committer ")?;
        let original = self.source_tree(source, &base.commit)?;
        let current = self.source_tree(source, &head.commit)?;
        let mut entries = Vec::new();
        let mut blobs = BTreeMap::new();
        let mut blob_bytes = 0usize;
        for path in original
            .keys()
            .chain(current.keys())
            .collect::<BTreeSet<_>>()
        {
            if original.get(path) == current.get(path) {
                continue;
            }
            let old = original.get(path);
            let new = current.get(path);
            if let Some((_, oid)) = new
                && !blobs.contains_key(oid)
            {
                let bytes = self.run(source, &["cat-file", "blob", oid], None)?;
                blob_bytes = blob_bytes
                    .checked_add(bytes.len())
                    .filter(|n| *n <= 256 * 1024 * 1024)
                    .ok_or_else(|| {
                        Error::new("bounded_input", "Git publication objects exceed 256 MiB")
                    })?;
                blobs.insert(oid.clone(), bytes);
            }
            entries.push(SourceTreeEntry {
                path: path.clone(),
                mode: new.or(old).expect("changed path exists").0.clone(),
                object: new.map(|(_, oid)| oid.clone()),
            });
        }
        Ok(GitPublicationObjects {
            base: base.clone(),
            revision: head.clone(),
            author,
            committer,
            message: message.into(),
            entries,
            blobs,
        })
    }
}
