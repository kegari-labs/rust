//! Deterministic Git mechanics for reviewed child deltas and isolated candidates.
//!
//! This module never pushes, fetches a network URL, repairs a patch, resets an
//! existing checkout, or deletes a failed candidate. Callers must hold the host
//! admission permit and the repository integration/release lock before effects.

use crate::digest::{bytes_digest, canonical_digest};
use crate::error::{Error, Result};
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
}

impl DeltaManifest {
    pub fn digest(&self) -> Result<String> {
        canonical_digest(DELTA_SCHEMA, self)
    }
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
    pub identity: CommitIdentity,
}

#[derive(Clone, Debug)]
pub struct CommitIdentity {
    pub name: String,
    pub email: String,
}

impl CommitIdentity {
    fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty()
            || self.email.trim().is_empty()
            || self.name.contains(['\r', '\n', '<', '>'])
            || self.email.contains(['\r', '\n', '<', '>'])
        {
            return Err(Error::invalid(
                "a valid configured human commit identity is required",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct Git {
    executable: PathBuf,
    version: String,
}

impl Git {
    pub fn new(executable: impl AsRef<Path>) -> Result<Self> {
        let executable =
            fs::canonicalize(executable).map_err(|e| Error::io("resolve Git executable", e))?;
        let output = Command::new(&executable)
            .env_clear()
            .arg("--version")
            .output()
            .map_err(|e| Error::io("inspect Git version", e))?;
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
        Ok(Self {
            executable,
            version,
        })
    }

    pub fn version(&self) -> &str {
        &self.version
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
        command
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(input) = input {
            let mut child = command
                .stdin(Stdio::piped())
                .spawn()
                .map_err(|e| Error::io("start Git", e))?;
            let write_result = child.stdin.take().expect("piped stdin").write_all(input);
            let output = child
                .wait_with_output()
                .map_err(|e| Error::io("wait for Git", e))?;
            if let Err(e) = write_result
                && output.status.success()
            {
                return Err(Error::io("write Git input", e));
            }
            Ok(output)
        } else {
            command
                .stdin(Stdio::null())
                .output()
                .map_err(|e| Error::io("run Git", e))
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
            git_version: self.version.clone(),
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
        };
        Ok(DeltaArtifact { manifest, patch })
    }

    pub fn verify_delta(&self, source: &Path, delta: &DeltaArtifact) -> Result<()> {
        let m = &delta.manifest;
        if m.schema_version != DELTA_SCHEMA
            || m.git_version != self.version
            || m.patch_digest != bytes_digest(&delta.patch)
        {
            return Err(Error::new(
                "provenance_mismatch",
                "delta schema, Git profile or exact artifact digest mismatch",
            ));
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

    /// Copies committed local Git objects into a fresh owned directory. The
    /// operator checkout/index and its dirty or partially staged files are read
    /// only and are never used as candidate input.
    pub fn assemble_candidate(
        &self,
        source: &Path,
        execution_root: &Path,
        request: CandidateRequest<'_>,
    ) -> Result<Candidate> {
        request.repository.validate()?;
        request.identity.validate()?;
        if request.deltas.is_empty() {
            return Err(Error::new(
                "no_change",
                "no selected patch: validate an already-in-production outcome without a candidate commit",
            ));
        }
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
            git_version: self.version.clone(),
            production: production.clone(),
            ordered_delta_digests,
            ordered_patch_digests,
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
            return Err(Error::new(
                "no_change",
                "selected patches have no net production change; require evidenced no-change classification",
            ));
        }
        let message = format!(
            "feat(kegari): assemble accepted parent candidate\n\nParent: {}\nOperation: {}\nGeneration: {}\nPatch-manifest: {}\n",
            manifest.parent_id,
            manifest.operation_id,
            manifest.work_generation,
            canonical_digest(1, &manifest.ordered_delta_digests)?
        );
        let mut command = self.command(&path);
        let output = command
            .env("GIT_AUTHOR_NAME", &request.identity.name)
            .env("GIT_AUTHOR_EMAIL", &request.identity.email)
            .env("GIT_COMMITTER_NAME", &request.identity.name)
            .env("GIT_COMMITTER_EMAIL", &request.identity.email)
            .args(["commit", "--signoff", "--no-gpg-sign", "-m", &message])
            .stdin(Stdio::null())
            .output()
            .map_err(|e| Error::io("commit isolated candidate", e))?;
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
            || manifest.git_version != self.version
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
        self.single_parent(&path, &result.commit, &manifest.production.commit)?;
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
