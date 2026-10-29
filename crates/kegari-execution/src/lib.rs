//! Role-scoped filesystem boundaries and Kegari-owned host process journals.
//! Codex supplies repository execution; these records do not create a runner.
use kegari_digest::{bytes_digest, canonical_digest};
use kegari_types::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

pub mod journal;

/// Descriptive output shared by deterministic host operations and Codex evidence.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandOutcome {
    pub schema_version: u32,
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub isolation_digest: String,
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub termination_signal: Option<i32>,
    pub timed_out: bool,
    pub stdout: String,
    pub stderr: String,
    pub output_truncated: bool,
    pub elapsed_ms: u64,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ServiceObservation {
    pub leader_running: bool,
    pub stop_requested: bool,
    pub kill_escalated: bool,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionRole {
    Child,
    Validation,
    Coordination,
    Review,
    IndependentReview,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PathGrant {
    pub path: PathBuf,
    /// A directory grant includes descendants; a file grant is exact.
    pub directory: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IsolationPlan {
    pub schema_version: u32,
    pub role: ExecutionRole,
    pub execution_root: PathBuf,
    pub repository: PathBuf,
    pub cwd: PathBuf,
    pub scratch: PathBuf,
    pub read_roots: Vec<PathGrant>,
    pub source_writes: Vec<PathGrant>,
    /// Explicit disposable test resources; never aliases product source.
    #[serde(default)]
    pub resource_writes: Vec<PathGrant>,
    pub forbidden_roots: Vec<PathBuf>,
}

/// Constructed only by checking the actual filesystem. Revalidated before use.
#[derive(Clone, Debug)]
pub struct ValidatedIsolation {
    plan: IsolationPlan,
    digest: String,
    // A cwd derivation keeps one original plan, never a recursive chain. This
    // private anchor is not recoverable from a serialized observation.
    origin: Option<std::sync::Arc<IsolationOrigin>>,
    runtime_build: Arc<str>,
}

#[derive(Debug)]
struct IsolationOrigin {
    plan: IsolationPlan,
    digest: String,
}

const MAX_SCOPE_PATH_BYTES: usize = 4096;

fn safe_absolute(path: &Path) -> Result<()> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        || path.to_str().is_none_or(|s| s.contains('\0'))
    {
        return Err(Error::new(
            "invalid_path",
            "path must be absolute, UTF-8 and normalized",
        ));
    }
    Ok(())
}

/// Canonicalize existing ancestry and preserve a missing final component only.
/// Missing intermediate directories are never treated as an established boundary.
pub fn canonical_target(path: &Path) -> Result<PathBuf> {
    safe_absolute(path)?;
    match path.canonicalize() {
        Ok(p) => Ok(p),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let parent = path
                .parent()
                .ok_or_else(|| Error::invalid("missing parent"))?;
            let parent = parent
                .canonicalize()
                .map_err(|e| Error::io("resolve parent", e))?;
            Ok(parent.join(
                path.file_name()
                    .ok_or_else(|| Error::invalid("missing filename"))?,
            ))
        }
        Err(e) => Err(Error::io("resolve execution path", e)),
    }
}

pub fn contained_path(root: &Path, path: &Path) -> Result<PathBuf> {
    let root = root
        .canonicalize()
        .map_err(|e| Error::io("resolve execution root", e))?;
    let target = canonical_target(path)?;
    if !target.starts_with(&root) || target == root {
        return Err(Error::new(
            "scope_violation",
            "path escapes or aliases the execution root",
        ));
    }
    Ok(target)
}

fn overlap(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}

impl IsolationPlan {
    pub fn validate(&self, runtime_build: &str) -> Result<ValidatedIsolation> {
        if self.schema_version != 1 {
            return Err(Error::invalid("unsupported isolation schema"));
        }
        let mut plan = self.clone();
        safe_absolute(&plan.execution_root)?;
        plan.execution_root = plan
            .execution_root
            .canonicalize()
            .map_err(|e| Error::io("execution root", e))?;
        plan.repository = contained_path(&plan.execution_root, &plan.repository)?;
        plan.scratch = contained_path(&plan.execution_root, &plan.scratch)?;
        if !plan.repository.is_dir()
            || !plan.scratch.is_dir()
            || overlap(&plan.repository, &plan.scratch)
        {
            return Err(Error::new(
                "scope_violation",
                "repository and private scratch must be separate existing directories",
            ));
        }
        plan.cwd = canonical_target(&plan.cwd)?;
        if !plan.cwd.is_dir() || !plan.cwd.starts_with(&plan.repository) {
            return Err(Error::new(
                "scope_violation",
                "cwd must be inside the isolated repository",
            ));
        }
        if matches!(
            plan.role,
            ExecutionRole::Coordination
                | ExecutionRole::Review
                | ExecutionRole::IndependentReview
                | ExecutionRole::Validation
        ) && !plan.source_writes.is_empty()
        {
            return Err(Error::new(
                "scope_violation",
                "this role cannot write product or test source",
            ));
        }
        for forbidden in &mut plan.forbidden_roots {
            *forbidden = canonical_target(forbidden)?;
        }
        if !matches!(plan.role, ExecutionRole::Child | ExecutionRole::Validation)
            && !plan.resource_writes.is_empty()
        {
            return Err(Error::new(
                "scope_violation",
                "this role cannot mutate test resources",
            ));
        }
        for allowed in [&plan.repository, &plan.scratch] {
            if plan.forbidden_roots.iter().any(|f| overlap(f, allowed)) {
                return Err(Error::new(
                    "scope_violation",
                    "execution directory overlaps privileged or unrelated context",
                ));
            }
        }
        for grant in plan
            .read_roots
            .iter_mut()
            .chain(plan.source_writes.iter_mut())
            .chain(plan.resource_writes.iter_mut())
        {
            grant.path = canonical_target(&grant.path)?;
            if grant.directory && !grant.path.is_dir() {
                return Err(Error::new(
                    "invalid_path",
                    "directory grant must already exist",
                ));
            }
            if grant.path == Path::new("/")
                || plan.forbidden_roots.iter().any(|f| overlap(f, &grant.path))
            {
                return Err(Error::new(
                    "scope_violation",
                    "grant overlaps a forbidden root",
                ));
            }
        }
        for grant in &plan.source_writes {
            if !grant.path.starts_with(&plan.repository)
                || grant.path.starts_with(plan.repository.join(".git"))
            {
                return Err(Error::new(
                    "scope_violation",
                    "source writes must belong to the assigned repository, excluding Git administration",
                ));
            }
            verify_writable_links(&grant.path, false)?;
        }
        for grant in &plan.resource_writes {
            if overlap(&grant.path, &plan.repository)
                || overlap(&grant.path, &plan.scratch)
                || !plan.read_roots.iter().any(|read| read == grant)
            {
                return Err(Error::new(
                    "scope_violation",
                    "test-resource writes require an exact read grant outside repository and scratch",
                ));
            }
            verify_writable_links(&grant.path, true)?;
        }
        let digest = canonical_digest(1, &plan)?;
        Ok(ValidatedIsolation {
            plan,
            digest,
            origin: None,
            runtime_build: Arc::from(runtime_build),
        })
    }
}

fn verify_writable_links(path: &Path, resource: bool) -> Result<()> {
    let mut pending = vec![path.to_path_buf()];
    let mut visited = 0usize;
    while let Some(path) = pending.pop() {
        visited += 1;
        if visited > 100_000 {
            return Err(Error::new(
                "isolation_unverified",
                "writable tree exceeds the bounded link inspection; narrow its grants",
            ));
        }
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(Error::io("inspect writable tree", e)),
        };
        // Symlink targets are separately enforced by the OS boundary; do not
        // follow them into unrelated trees during boundary preparation.
        if metadata.is_symlink() {
            if resource {
                return Err(Error::new(
                    "scope_violation",
                    "writable disposable resources cannot contain symbolic links",
                ));
            }
            continue;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.is_file() && metadata.nlink() > 1 {
                return Err(Error::new(
                    "scope_violation",
                    "multiply linked writable source needs isolation",
                ));
            }
        }
        if metadata.is_dir() {
            for entry in
                std::fs::read_dir(path).map_err(|e| Error::io("inspect writable directory", e))?
            {
                pending.push(
                    entry
                        .map_err(|e| Error::io("inspect writable entry", e))?
                        .path(),
                );
            }
        }
    }
    Ok(())
}

impl ValidatedIsolation {
    pub fn plan(&self) -> &IsolationPlan {
        &self.plan
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
    /// Derive only a working directory within the original repository. Relative
    /// paths are anchored at the current cwd; filesystem aliases are resolved
    /// before checking containment. No resource observation or grant is minted.
    pub fn with_cwd(&self, cwd: &Path) -> Result<Self> {
        self.revalidate()?;
        let text = cwd
            .to_str()
            .ok_or_else(|| Error::invalid("cwd must be UTF-8"))?;
        if text.is_empty() || text.len() > MAX_SCOPE_PATH_BYTES || text.contains('\0') {
            return Err(Error::invalid("cwd is empty, excessive or contains NUL"));
        }
        let candidate = if cwd.is_absolute() {
            cwd.to_owned()
        } else {
            self.plan.cwd.join(cwd)
        };
        let canonical = candidate
            .canonicalize()
            .map_err(|e| Error::io("resolve command cwd", e))?;
        if canonical.as_os_str().len() > MAX_SCOPE_PATH_BYTES
            || !canonical.is_dir()
            || !canonical.starts_with(&self.plan.repository)
        {
            return Err(Error::new(
                "scope_violation",
                "command cwd escapes the original repository",
            ));
        }
        let mut plan = self.plan.clone();
        plan.cwd = canonical;
        let mut derived = plan.validate(&self.runtime_build)?;
        derived.origin = Some(self.origin.clone().unwrap_or_else(|| {
            std::sync::Arc::new(IsolationOrigin {
                plan: self.plan.clone(),
                digest: self.digest.clone(),
            })
        }));
        derived.verify_cwd_derivation(self)?;
        Ok(derived)
    }
    /// Verify a derivation against the actual retained caller boundary. This is
    /// a structural/metadata check, not a full resource-content recapture.
    pub fn verify_cwd_derivation(&self, original: &ValidatedIsolation) -> Result<()> {
        self.revalidate()?;
        original.revalidate()?;
        let mut candidate = self.plan.clone();
        candidate.cwd = original.plan.cwd.clone();
        if self.original_digest() != original.original_digest()
            || canonical_digest(1, &candidate)? != original.digest
        {
            return Err(Error::new(
                "scope_violation",
                "command boundary is not a cwd-only derivation of the original",
            ));
        }
        Ok(())
    }
    fn original_digest(&self) -> &str {
        self.origin
            .as_ref()
            .map_or(&self.digest, |origin| &origin.digest)
    }
    pub fn revalidate(&self) -> Result<()> {
        if self.plan.validate(&self.runtime_build)?.digest != self.digest {
            return Err(Error::new(
                "scope_violation",
                "execution boundary changed since admission",
            ));
        }
        if let Some(origin) = &self.origin {
            let mut original_geometry = self.plan.clone();
            original_geometry.cwd = origin.plan.cwd.clone();
            if origin.plan.validate(&self.runtime_build)?.digest != origin.digest
                || canonical_digest(1, &original_geometry)? != origin.digest
            {
                return Err(Error::new(
                    "scope_violation",
                    "cwd derivation changed the original execution boundary",
                ));
            }
        }
        Ok(())
    }
    /// Inspect a retained boundary under the caller's original cutoff. This
    /// checks geometry and current root identities without scanning writable
    /// descendants or constructing execution authority.
    pub fn revalidate_inspection_until(&self, deadline: std::time::Instant) -> Result<()> {
        check_deadline(deadline)?;
        let plan = &self.plan;
        if plan.read_roots.len() > 256
            || plan.source_writes.len() > 256
            || plan.resource_writes.len() > 256
            || plan.forbidden_roots.len() > 256
            || canonical_digest(1, plan)? != self.digest
        {
            return Err(Error::new(
                "scope_violation",
                "retained inspection boundary changed or exceeds its finite bound",
            ));
        }
        check_deadline(deadline)?;
        if let Some(origin) = &self.origin {
            let mut geometry = plan.clone();
            geometry.cwd = origin.plan.cwd.clone();
            if canonical_digest(1, &origin.plan)? != origin.digest
                || canonical_digest(1, &geometry)? != origin.digest
            {
                return Err(Error::new(
                    "scope_violation",
                    "inspection changed the original cwd-only boundary",
                ));
            }
            check_deadline(deadline)?;
        }
        for path in [
            &plan.execution_root,
            &plan.repository,
            &plan.scratch,
            &plan.cwd,
        ]
        .into_iter()
        .chain(self.origin.as_ref().map(|origin| &origin.plan.cwd))
        {
            check_deadline(deadline)?;
            if path.as_os_str().len() > MAX_SCOPE_PATH_BYTES
                || canonical_target(path)? != *path
                || !path.is_dir()
            {
                return Err(Error::new(
                    "scope_violation",
                    "inspection root or cwd changed",
                ));
            }
            check_deadline(deadline)?;
        }
        for (path, directory) in plan
            .read_roots
            .iter()
            .chain(&plan.source_writes)
            .chain(&plan.resource_writes)
            .map(|grant| (&grant.path, grant.directory))
            .chain(plan.forbidden_roots.iter().map(|path| (path, false)))
        {
            check_deadline(deadline)?;
            if path.as_os_str().len() > MAX_SCOPE_PATH_BYTES
                || canonical_target(path)? != *path
                || (directory && !path.is_dir())
            {
                return Err(Error::new(
                    "scope_violation",
                    "inspection grant or forbidden root changed",
                ));
            }
            check_deadline(deadline)?;
        }
        check_deadline(deadline)
    }
    pub fn artifact(&self, path: &Path) -> Result<PathBuf> {
        self.revalidate()?;
        let candidate = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.plan.cwd.join(path)
        };
        let resolved = canonical_target(&candidate)?;
        if !resolved.starts_with(&self.plan.repository) && !resolved.starts_with(&self.plan.scratch)
        {
            return Err(Error::new(
                "scope_violation",
                "artifact is outside execution and private output roots",
            ));
        }
        Ok(resolved)
    }
}

fn check_deadline(deadline: std::time::Instant) -> Result<()> {
    if std::time::Instant::now() >= deadline {
        return Err(Error::new(
            "timeout",
            "filesystem scope inspection exceeded its original deadline",
        ));
    }
    Ok(())
}

/// A bounded readable name; canonical identity, repository and generation own it.
pub fn execution_name(
    display: &str,
    identity: &str,
    repository: &str,
    generation: &str,
) -> Result<String> {
    if identity.is_empty() || repository.is_empty() || generation.is_empty() {
        return Err(Error::invalid("execution identity must be complete"));
    }
    let readable: String = display
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect();
    let digest = canonical_digest(1, &(identity, repository, generation))?;
    Ok(format!(
        "{}-{}",
        readable.trim_matches('.'),
        digest.rsplit(':').next().unwrap_or_default()
    ))
}

pub fn artifact_digest(path: &Path, max_bytes: u64) -> Result<String> {
    let file = std::fs::File::open(path).map_err(|e| Error::io("open artifact", e))?;
    if file
        .metadata()
        .map_err(|e| Error::io("stat artifact", e))?
        .len()
        > max_bytes
    {
        return Err(Error::new(
            "artifact_too_large",
            "artifact exceeds its admitted bound",
        ));
    }
    use std::io::Read;
    let mut bytes = Vec::new();
    file.take(max_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| Error::io("read artifact", e))?;
    if bytes.len() as u64 > max_bytes {
        return Err(Error::new(
            "artifact_too_large",
            "artifact grew beyond its bound",
        ));
    }
    Ok(bytes_digest(&bytes))
}
