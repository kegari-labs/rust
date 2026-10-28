//! Git records and deterministic rules. Records alone confer no execution authority.
use kegari_digest::canonical_digest;
use kegari_types::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};
use uuid::Uuid;

pub const SUPPORTED_GIT_VERSION: &str = "git version 2.56.0";
pub const DELTA_SCHEMA: u32 = 1;

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

pub fn validate_repo_path(path: &str) -> Result<()> {
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

/// Patch bytes are an evidence artifact, not content copied into a run receipt.
#[derive(Clone, Debug)]
pub struct DeltaArtifact {
    pub manifest: DeltaManifest,
    pub patch: Vec<u8>,
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommitIdentity {
    pub name: String,
    pub email: String,
}

impl CommitIdentity {
    pub fn validate(&self) -> Result<()> {
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
    pub fn validate(&self) -> Result<()> {
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

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GitCommitIdentity {
    pub name: String,
    pub email: String,
    pub date: String,
}
