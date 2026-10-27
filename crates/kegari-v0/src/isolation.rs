//! Execution boundaries for owned runs. No unconfined fallback is provided.
use crate::digest::{bytes_digest, canonical_digest};
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

#[path = "managed_execution.rs"]
mod managed_execution;
pub use managed_execution::BoundManagedExecution;
#[path = "managed_resources.rs"]
mod managed_resources;

#[cfg(all(test, target_os = "macos"))]
#[path = "isolation_native_tests.rs"]
mod native_tests;

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed_execution: Option<BoundManagedExecution>,
}

/// Constructed only by checking the actual filesystem. Revalidated before use.
#[derive(Clone, Debug)]
pub struct ValidatedIsolation {
    plan: IsolationPlan,
    digest: String,
    // A cwd derivation keeps one original plan, never a recursive chain. This
    // private anchor is not recoverable from a serialized observation.
    origin: Option<std::sync::Arc<IsolationOrigin>>,
}

#[derive(Debug)]
struct IsolationOrigin {
    plan: IsolationPlan,
    digest: String,
}

const MAX_COMMAND_ARGS: usize = 128;
const MAX_COMMAND_ARG_BYTES: usize = 4096;
const MAX_COMMAND_BYTES: usize = 65_536;
const MAX_COMMAND_CWD_BYTES: usize = 4096;

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
    pub fn validate(&self) -> Result<ValidatedIsolation> {
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
        if let Some(managed) = &plan.managed_execution {
            managed.validate_plan(&plan)?;
        }
        let digest = canonical_digest(1, &plan)?;
        Ok(ValidatedIsolation {
            plan,
            digest,
            origin: None,
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
        if text.is_empty() || text.len() > MAX_COMMAND_CWD_BYTES || text.contains('\0') {
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
        if canonical.as_os_str().len() > MAX_COMMAND_CWD_BYTES
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
        let mut derived = plan.validate()?;
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
        if self.plan.validate()?.digest != self.digest {
            return Err(Error::new(
                "scope_violation",
                "execution boundary changed since admission",
            ));
        }
        if let Some(origin) = &self.origin {
            let mut original_geometry = self.plan.clone();
            original_geometry.cwd = origin.plan.cwd.clone();
            if origin.plan.validate()?.digest != origin.digest
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
    /// Full content observation belongs outside serialized Control callbacks.
    /// A successful observation binds resources; it does not qualify a workload.
    pub fn revalidate_resource_contents(&self) -> Result<()> {
        self.revalidate()?;
        if let Some(managed) = &self.plan.managed_execution {
            managed.revalidate()?;
        }
        Ok(())
    }
    /// Bare executables follow the admitted PATH; explicit relative paths stay
    /// anchored at cwd. Arguments and script text are never rewritten.
    pub fn resolve_executable(&self, name: &str) -> Result<PathBuf> {
        self.revalidate()?;
        self.plan
            .managed_execution
            .as_ref()
            .ok_or_else(|| {
                Error::new(
                    "scope_violation",
                    "operation has no repository command domain",
                )
            })?
            .resolve(&self.plan.cwd, name)
    }
    pub fn command_path(&self) -> Result<String> {
        self.revalidate()?;
        self.plan
            .managed_execution
            .as_ref()
            .ok_or_else(|| {
                Error::new(
                    "scope_violation",
                    "operation has no repository command domain",
                )
            })?
            .command_path()
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
    /// This profile applies to hooks/checks, not the trusted app-server transport.
    /// Default-deny also denies TCP, Unix sockets, Keychain IPC and other processes.
    pub fn seatbelt_profile(&self) -> Result<String> {
        self.revalidate()?;
        let mut profile = String::from(
            "(version 1)\n(deny default)\n(allow process-fork)\n(allow sysctl-read)\n",
        );
        for path in ["/usr/lib", "/System/Library"] {
            profile.push_str(&format!(
                "(allow file-read* file-map-executable (subpath {}))\n",
                quote(path)
            ));
        }
        // Current macOS dyld also loads OS libraries from Cryptex grafts.
        // Keep these fixed OS resources separate from arbitrary configured read
        // roots. libignition opens the literal root/ancestor directories for
        // openat-based bootstrap: metadata alone cannot open them for reading.
        // These fixed directory reads do not grant descendant access. Do not
        // import Apple's general system profile (IPC/network etc.).
        for path in [
            "/System/Volumes/Preboot/Cryptexes/App/System",
            "/System/Volumes/Preboot/Cryptexes/Incoming/OS",
            "/System/Volumes/Preboot/Cryptexes/OS",
            "/System/Volumes/Preboot/Cryptexes/Rosetta",
            "/System/Cryptexes/App",
            "/System/Cryptexes/OS",
            "/System/Cryptexes/Rosetta",
        ] {
            profile.push_str(&format!(
                "(allow file-read* file-test-existence file-map-executable (subpath {}))\n",
                quote(path)
            ));
            for parent in Path::new(path).ancestors().skip(1) {
                profile.push_str(&format!(
                    "(allow file-read* file-test-existence (literal {}))\n",
                    quote_path(parent)?
                ));
            }
        }
        for path in ["/dev/null", "/dev/urandom", "/dev/random"] {
            profile.push_str(&format!("(allow file-read* (literal {}))\n", quote(path)));
        }
        profile.push_str("(allow file-write-data (literal \"/dev/null\"))\n");
        for path in [&self.plan.repository, &self.plan.scratch] {
            profile.push_str(&format!(
                "(allow file-read* (subpath {}))\n",
                quote_path(path)?
            ));
        }
        for grant in &self.plan.read_roots {
            profile.push_str(&format!(
                "(allow file-read* ({} {}))\n",
                if grant.directory {
                    "subpath"
                } else {
                    "literal"
                },
                quote_path(&grant.path)?,
            ));
        }
        for grant in self
            .plan
            .source_writes
            .iter()
            .chain(self.plan.resource_writes.iter())
        {
            profile.push_str(&format!(
                "(allow file-write* ({} {}))\n",
                if grant.directory {
                    "subpath"
                } else {
                    "literal"
                },
                quote_path(&grant.path)?
            ));
        }
        profile.push_str(&format!(
            "(allow file-write* (subpath {}))\n",
            quote_path(&self.plan.scratch)?
        ));
        profile.push_str(&format!(
            "(deny file-write* (subpath {}))\n",
            quote_path(&self.plan.repository.join(".git"))?
        ));
        if let Some(managed) = &self.plan.managed_execution {
            for grant in managed.execution_grants() {
                profile.push_str(&format!(
                    "(allow file-read* file-map-executable process-exec ({} {}))\n",
                    if grant.directory {
                        "subpath"
                    } else {
                        "literal"
                    },
                    quote_path(&grant.path)?,
                ));
            }
        }
        for forbidden in &self.plan.forbidden_roots {
            profile.push_str(&format!(
                "(deny file-read* file-test-existence file-write* file-map-executable process-exec (subpath {}))\n",
                quote_path(forbidden)?
            ));
        }
        Ok(profile)
    }
}

fn quote(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
    )
}
fn quote_path(path: &Path) -> Result<String> {
    path.to_str()
        .map(quote)
        .ok_or_else(|| Error::invalid("non-UTF-8 sandbox path"))
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

fn redact(mut value: String, secrets: &[String]) -> String {
    for secret in secrets.iter().filter(|s| !s.is_empty()) {
        value = value.replace(secret, "[REDACTED]");
    }
    value
}

fn rendered_command_output(
    out: &[u8],
    err: &[u8],
    secrets: &[String],
    combined_limit: Option<usize>,
) -> Result<(String, String, bool)> {
    let mut stdout = redact(String::from_utf8_lossy(out).into_owned(), secrets);
    let mut stderr = redact(String::from_utf8_lossy(err).into_owned(), secrets);
    // Diagnose sandbox setup before optional output truncation can hide it.
    if stderr.contains("sandbox-exec: sandbox_apply")
        || stderr.contains("sandbox-exec: sandbox_init")
    {
        return Err(Error::new(
            "isolation_unavailable",
            "host policy prevented applying the execution sandbox",
        ));
    }
    let mut truncated = false;
    if let Some(limit) = combined_limit {
        let mut remaining = limit;
        // Redact before shortening: a rendered prefix must not reintroduce any
        // bytes of a matched secret. Order is deterministic, not a chronology
        // claim about the two independently drained streams.
        for text in [&mut stdout, &mut stderr] {
            if text.len() > remaining {
                let mut end = remaining;
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                text.truncate(end);
                truncated = true;
            }
            remaining -= text.len();
        }
    }
    Ok((stdout, stderr, truncated))
}

/// One original retained-output allowance shared by all members of a bundle.
/// Exhaustion truncates capture while actual pipes continue draining to EOF.
pub struct CommandOutputBudget {
    remaining: std::sync::atomic::AtomicUsize,
}
impl CommandOutputBudget {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            remaining: std::sync::atomic::AtomicUsize::new(limit),
        }
    }
    fn take(&self, requested: usize) -> usize {
        let before = self
            .remaining
            .try_update(
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
                |left| Some(left.saturating_sub(requested)),
            )
            .expect("output budget update cannot refuse");
        requested.min(before)
    }
}
/// Original resource limits supplied only with an admitted bundle launch.
/// These values grant no execution or filesystem authority.
pub struct BundleCommandLimits {
    pub(crate) timeout: Duration,
    pub(crate) deadline: std::time::Instant,
    pub(crate) output_limit: usize,
    pub(crate) output_budget: std::sync::Arc<CommandOutputBudget>,
}
/// Original generic-invocation resource bounds. These are not launch authority;
/// the actual journal, permit, isolation and qualification checks still apply.
pub struct InvocationCommandLimits {
    pub timeout: Duration,
    pub deadline: std::time::Instant,
    pub output_limit: usize,
}
fn remaining_bundle_timeout(
    timeout: Duration,
    started: std::time::Instant,
    deadline: Option<std::time::Instant>,
) -> Result<Duration> {
    match deadline {
        Some(deadline) if deadline <= std::time::Instant::now() => Err(Error::new(
            "timeout",
            "original command deadline expired before process launch",
        )),
        Some(deadline) => Ok(timeout.min(deadline.saturating_duration_since(started))),
        None => Ok(timeout),
    }
}
async fn bounded_output<R: tokio::io::AsyncRead + Unpin>(
    mut input: R,
    limit: usize,
    shared: Option<std::sync::Arc<CommandOutputBudget>>,
) -> std::io::Result<(Vec<u8>, bool)> {
    let mut bytes = Vec::new();
    let mut truncated = false;
    let mut buffer = [0; 8192];
    loop {
        let n = input.read(&mut buffer).await?;
        if n == 0 {
            break;
        }
        let take = n.min(limit.saturating_sub(bytes.len()));
        let take = shared.as_ref().map_or(take, |shared| shared.take(take));
        bytes.extend_from_slice(&buffer[..take]);
        truncated |= take < n;
        // A ready pipe must not starve cancellation or the supervisor's deadline.
        tokio::task::yield_now().await;
    }
    Ok((bytes, truncated))
}

/// Exact process-free launch identity for an admitted repository command. The
/// profile digest binds the resolved executable and full Seatbelt policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandLaunchIdentity {
    pub cwd: PathBuf,
    pub launcher: crate::platform::BinaryIdentity,
    pub target: crate::platform::BinaryIdentity,
    pub argv_digest: String,
}

/// Descriptive, process-free evidence for one rendered invocation. Decoding this
/// record never constructs an execution boundary, a permit or qualification.
/// `launch_digest` is the existing CommandLaunchIdentity argv/profile/environment
/// digest; cwd, original boundary and binary identities remain explicit fields.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CommandObservation {
    #[serde(deserialize_with = "command_observation_schema")]
    pub schema_version: u32,
    pub requested_argv: Vec<String>,
    pub selected_executable: PathBuf,
    pub canonical_executable: crate::platform::BinaryIdentity,
    pub launcher: crate::platform::BinaryIdentity,
    pub cwd: PathBuf,
    pub environment: BTreeMap<String, String>,
    pub original_isolation_digest: String,
    pub isolation_digest: String,
    pub launch_digest: String,
    pub backend_policy_digest: String,
}
fn command_observation_schema<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<u32, D::Error> {
    let schema = u32::deserialize(deserializer)?;
    if schema != 1 {
        return Err(serde::de::Error::custom(
            "unsupported command observation schema",
        ));
    }
    Ok(schema)
}
impl CommandObservation {
    /// A projection of recorded facts only. Callers still need the actual
    /// admitted boundary and its live gate before starting any process.
    pub fn launch_identity(&self) -> CommandLaunchIdentity {
        CommandLaunchIdentity {
            cwd: self.cwd.clone(),
            launcher: self.launcher.clone(),
            target: self.canonical_executable.clone(),
            argv_digest: self.launch_digest.clone(),
        }
    }
}

/// Observe exact rendered facts without starting the launcher or the command.
/// Executable bytes are read here; invoke outside serialized admission callbacks.
pub fn observe_command(
    isolation: &ValidatedIsolation,
    argv: &[String],
) -> Result<CommandObservation> {
    let rendered = render_command(isolation, argv)?;
    observe_rendered_command(isolation, &rendered)
}
pub fn command_launch_identity(
    isolation: &ValidatedIsolation,
    argv: &[String],
) -> Result<CommandLaunchIdentity> {
    Ok(observe_command(isolation, argv)?.launch_identity())
}

struct RenderedCommand {
    executable: PathBuf,
    argv: Vec<String>,
    environment: BTreeMap<String, String>,
    profile: String,
}
fn render_command(isolation: &ValidatedIsolation, argv: &[String]) -> Result<RenderedCommand> {
    if argv.is_empty()
        || argv.len() > MAX_COMMAND_ARGS
        || argv[0].is_empty()
        || argv
            .iter()
            .any(|arg| arg.len() > MAX_COMMAND_ARG_BYTES || arg.contains('\0'))
        || argv
            .iter()
            .try_fold(0usize, |total, arg| total.checked_add(arg.len() + 1))
            .is_none_or(|total| total > MAX_COMMAND_BYTES)
        || isolation.plan.cwd.as_os_str().len() > MAX_COMMAND_CWD_BYTES
    {
        return Err(Error::invalid(
            "command argv or cwd exceeds its finite input bounds",
        ));
    }
    isolation.revalidate()?;
    let executable = isolation.resolve_executable(&argv[0])?;
    let environment = BTreeMap::from([
        ("PATH".into(), isolation.command_path()?),
        (
            "HOME".into(),
            isolation.plan.scratch.to_string_lossy().into_owned(),
        ),
        (
            "TMPDIR".into(),
            isolation.plan.scratch.to_string_lossy().into_owned(),
        ),
        ("LANG".into(), "C.UTF-8".into()),
    ]);
    let profile = isolation.seatbelt_profile()?;
    Ok(RenderedCommand {
        executable,
        argv: argv.to_vec(),
        environment,
        profile,
    })
}
fn observe_rendered_command(
    isolation: &ValidatedIsolation,
    rendered: &RenderedCommand,
) -> Result<CommandObservation> {
    let mut launch = serde_json::json!({"argv":rendered.argv,"profile":rendered.profile,"resolved":rendered.executable,"environment":rendered.environment});
    if let Some(managed) = &isolation.plan.managed_execution {
        launch["managed_execution"] = serde_json::json!(canonical_digest(1, managed)?);
    }
    Ok(CommandObservation {
        schema_version: 1,
        requested_argv: rendered.argv.clone(),
        selected_executable: rendered.executable.clone(),
        cwd: isolation.plan.cwd.clone(),
        launcher: crate::platform::BinaryIdentity::inspect(
            Path::new("/usr/bin/sandbox-exec"),
            false,
        )?,
        canonical_executable: crate::platform::BinaryIdentity::inspect(
            &rendered
                .executable
                .canonicalize()
                .map_err(|error| Error::io("observe actual command target", error))?,
            false,
        )?,
        environment: rendered.environment.clone(),
        original_isolation_digest: isolation.original_digest().to_owned(),
        isolation_digest: isolation.digest.clone(),
        launch_digest: canonical_digest(1, &launch)?,
        backend_policy_digest: isolation
            .plan
            .managed_execution
            .as_ref()
            .ok_or_else(|| {
                Error::new(
                    "scope_violation",
                    "command observation requires the retained execution domain",
                )
            })?
            .policy_digest()
            .to_owned(),
    })
}
/// A resolved managed declaration is distinct from native qualification. Retain
/// pre-spawn refusal until actual release qualification supports the generic
/// command boundary. Removing a specialized profile does not qualify its replacement.
pub fn repository_command_prerequisite(isolation: &ValidatedIsolation) -> Result<()> {
    isolation.revalidate()?;
    match &isolation.plan.managed_execution {
        None => Err(Error::new(
            "process_tree_unverified",
            "no supported managed execution profile is bound to this operation",
        )),
        Some(profile) => Err(Error::new(
            "managed_profile_unqualified",
            "managed foreground policy is bound, but the generic command boundary still requires native qualification evidence",
        ).detail(serde_json::json!({"profile": profile.policy_digest()}))),
    }
}
enum CommandJournal {
    Worker {
        journal: std::sync::Arc<crate::process_records::ProcessJournal>,
        permit: crate::control::Permit,
        deadline: Option<std::time::Instant>,
    },
    Guard(Box<crate::process_records::SpawnGuard>),
    Bundle {
        journal: std::sync::Arc<crate::process_records::ProcessJournal>,
        launch: crate::control::CheckBundleLaunch,
        output_budget: std::sync::Arc<CommandOutputBudget>,
        deadline: std::time::Instant,
    },
}
impl CommandJournal {
    fn deadline(&self) -> Option<std::time::Instant> {
        match self {
            Self::Worker { deadline, .. } => *deadline,
            Self::Bundle { deadline, .. } => Some(*deadline),
            Self::Guard(_) => None,
        }
    }
}
pub fn start_guarded_command(
    isolation: &ValidatedIsolation,
    argv: &[String],
    timeout: Duration,
    output_limit: usize,
    secret_values: &[String],
    guard: crate::process_records::SpawnGuard,
) -> Result<OwnedCommand> {
    start_command_inner(
        isolation,
        argv,
        timeout,
        output_limit,
        secret_values,
        Some(CommandJournal::Guard(Box::new(guard))),
    )
}

/// Run a check/hook only under the OS boundary. The caller persists its admitted
/// operation before calling; outputs never establish revision provenance alone.
pub fn start_command(
    isolation: &ValidatedIsolation,
    argv: &[String],
    timeout: Duration,
    output_limit: usize,
    secret_values: &[String],
) -> Result<OwnedCommand> {
    start_command_inner(isolation, argv, timeout, output_limit, secret_values, None)
}
pub fn start_journaled_command(
    isolation: &ValidatedIsolation,
    argv: &[String],
    timeout: Duration,
    output_limit: usize,
    secret_values: &[String],
    journal: std::sync::Arc<crate::process_records::ProcessJournal>,
    permit: crate::control::Permit,
) -> Result<OwnedCommand> {
    start_command_inner(
        isolation,
        argv,
        timeout,
        output_limit,
        secret_values,
        Some(CommandJournal::Worker {
            journal,
            permit,
            deadline: None,
        }),
    )
}
pub fn start_journaled_command_until(
    isolation: &ValidatedIsolation,
    argv: &[String],
    limits: InvocationCommandLimits,
    secret_values: &[String],
    journal: std::sync::Arc<crate::process_records::ProcessJournal>,
    permit: crate::control::Permit,
) -> Result<OwnedCommand> {
    start_command_inner(
        isolation,
        argv,
        limits.timeout,
        limits.output_limit,
        secret_values,
        Some(CommandJournal::Worker {
            journal,
            permit,
            deadline: Some(limits.deadline),
        }),
    )
}
pub(crate) fn start_bundle_command(
    isolation: &ValidatedIsolation,
    argv: &[String],
    limits: BundleCommandLimits,
    journal: std::sync::Arc<crate::process_records::ProcessJournal>,
    launch: crate::control::CheckBundleLaunch,
) -> Result<OwnedCommand> {
    remaining_bundle_timeout(
        limits.timeout,
        std::time::Instant::now(),
        Some(limits.deadline),
    )?;
    start_command_inner(
        isolation,
        argv,
        limits.timeout,
        limits.output_limit,
        &[],
        Some(CommandJournal::Bundle {
            journal,
            launch,
            output_budget: limits.output_budget,
            deadline: limits.deadline,
        }),
    )
}

fn start_command_inner(
    isolation: &ValidatedIsolation,
    argv: &[String],
    timeout: Duration,
    output_limit: usize,
    secret_values: &[String],
    journal: Option<CommandJournal>,
) -> Result<OwnedCommand> {
    remaining_bundle_timeout(
        timeout,
        std::time::Instant::now(),
        journal.as_ref().and_then(CommandJournal::deadline),
    )?;
    isolation.revalidate()?;
    if matches!(
        isolation.plan.role,
        ExecutionRole::Coordination | ExecutionRole::IndependentReview
    ) {
        return Err(Error::new(
            "scope_violation",
            "this role may not execute repository commands",
        ));
    }
    if argv.is_empty() || timeout.is_zero() || output_limit == 0 || output_limit > 10 * 1024 * 1024
    {
        return Err(Error::invalid(
            "command requires argv and finite positive timeout/output bounds",
        ));
    }
    if !cfg!(target_os = "macos") {
        return Err(Error::new(
            "isolation_unavailable",
            "the supported local command boundary requires macOS sandbox-exec",
        ));
    }
    repository_command_prerequisite(isolation)?;
    spawn_command_inner(
        isolation,
        argv,
        timeout,
        output_limit,
        secret_values,
        journal,
    )
}

// Only the qualified entry above calls this in production. Native qualification
// fixtures exercise this exact spawn path with a disposable journal; they do not
// create runtime qualification or relax the public entry's prerequisite.
fn spawn_command_inner(
    isolation: &ValidatedIsolation,
    argv: &[String],
    timeout: Duration,
    output_limit: usize,
    secret_values: &[String],
    journal: Option<CommandJournal>,
) -> Result<OwnedCommand> {
    if journal.is_none() {
        return Err(Error::new(
            "execution_authority_absent",
            "managed commands require an admitted durable process journal before spawn",
        ));
    }
    let original_deadline = journal.as_ref().and_then(CommandJournal::deadline);
    remaining_bundle_timeout(timeout, std::time::Instant::now(), original_deadline)?;
    let rendered = render_command(isolation, argv)?;
    let observation = observe_rendered_command(isolation, &rendered)?;
    let identity = observation.launch_identity();
    let mut command = Command::new("/usr/bin/sandbox-exec");
    command
        .args(["-p", &rendered.profile])
        .arg(&rendered.executable)
        .args(&rendered.argv[1..]);
    command
        .current_dir(&isolation.plan.cwd)
        .env_clear()
        .envs(&rendered.environment)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    {
        command.process_group(0);
    }
    let started = std::time::Instant::now();
    remaining_bundle_timeout(timeout, started, original_deadline)?;
    let service = matches!(&journal, Some(CommandJournal::Bundle { launch, .. })
        if matches!(launch.member().kind, crate::process_records::CheckMemberKind::Service));
    let output_budget = match &journal {
        Some(CommandJournal::Bundle { output_budget, .. }) => Some(output_budget.clone()),
        Some(CommandJournal::Worker {
            deadline: Some(_), ..
        }) => Some(std::sync::Arc::new(CommandOutputBudget::new(output_limit))),
        _ => None,
    };
    let rendered_output_limit = matches!(
        journal.as_ref(),
        Some(CommandJournal::Worker {
            deadline: Some(_),
            ..
        })
    )
    .then_some(output_limit);
    let mut guard = match journal {
        Some(CommandJournal::Worker {
            journal, permit, ..
        }) => Some(journal.begin(crate::process_records::SpawnBinding {
            operation_id: permit.operation_id,
            effect_id: uuid::Uuid::new_v4(),
            lease: Some(permit.lease),
            control_owner: None,
            work_key: permit.work_key,
            scope_digest: permit.scope_digest,
            cwd: identity.cwd.clone(),
            executable: identity.launcher.clone(),
            observed_executables: vec![identity.target.clone()],
            argv_digest: identity.argv_digest.clone(),
            kind: "repository_command".into(),
            check_bundle: None,
        })?),
        Some(CommandJournal::Bundle {
            journal, launch, ..
        }) => {
            let permit = launch.permit();
            Some(journal.begin(crate::process_records::SpawnBinding {
                operation_id: permit.operation_id,
                effect_id: launch.process_id(),
                lease: Some(permit.lease.clone()),
                control_owner: None,
                work_key: permit.work_key.clone(),
                scope_digest: permit.scope_digest.clone(),
                cwd: identity.cwd.clone(),
                executable: identity.launcher.clone(),
                observed_executables: vec![identity.target.clone()],
                argv_digest: identity.argv_digest.clone(),
                kind: "repository_command".into(),
                check_bundle: Some(launch.member().clone()),
            })?)
        }
        Some(CommandJournal::Guard(guard)) => {
            let binding = guard.binding();
            if binding.cwd != identity.cwd
                || binding.executable != identity.launcher
                || binding.observed_executables != vec![identity.target]
                || binding.argv_digest != identity.argv_digest
                || binding.kind != "repository_command"
            {
                return Err(Error::new(
                    "scope_violation",
                    "provided process guard differs from exact repository command, executable or isolation policy",
                ));
            }
            Some(*guard)
        }
        None => None,
    };
    if let Some(guard) = &guard {
        guard.configure_tokio(&mut command)?;
    }
    // The original absolute cutoff survives preflight, journal I/O and lock
    // waits. Synchronous spawn itself cannot be preempted by an async timeout.
    let timeout = remaining_bundle_timeout(timeout, started, original_deadline)?;
    let (mut child, mut group) = crate::process_group::OwnedGroup::spawn_tokio_before(
        &mut command,
        Some(started + timeout),
    )?;
    // A quick child can exit before native identity capture. Keep its actual
    // handle and Starting journal until owned cleanup and reap establish exit.
    let capture_error = guard
        .as_mut()
        .and_then(|guard| guard.spawned_tokio(&child, &group).err());
    let capture_error = match capture_error {
        Some(error) => match group.observe_exit_before_reap() {
            Ok(true) => None,
            Ok(false) => Some(error),
            Err(error) => Some(error),
        },
        None => None,
    };
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::new("process_error", "missing stdout pipe"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| Error::new("process_error", "missing stderr pipe"))?;
    Ok(OwnedCommand::supervise(
        CommandRun {
            argv: argv.to_vec(),
            cwd: isolation.plan.cwd.clone(),
            isolation_digest: isolation.digest.clone(),
            started,
            timeout,
            secret_values: secret_values.to_vec(),
        },
        CommandProcess::new(child, group, guard, capture_error)
            .with_service(service)
            .with_output_budget(output_budget)
            .with_rendered_output_limit(rendered_output_limit),
        stdout,
        stderr,
        output_limit,
    ))
}

const COMMAND_EXIT_GRACE: Duration = Duration::from_secs(5);
const COMMAND_OUTPUT_GRACE: Duration = Duration::from_secs(1);

#[derive(Default)]
struct CommandOutputs {
    eof: [std::sync::atomic::AtomicBool; 2],
    joined: std::sync::atomic::AtomicBool,
    uncertain: std::sync::atomic::AtomicBool,
}
impl CommandOutputs {
    fn fail(&self) {
        self.uncertain
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
    fn closed(&self) -> bool {
        use std::sync::atomic::Ordering::SeqCst;
        // Read the publication flag before the monotone failure latch. A reader
        // must not pair an old clean latch with a newly joined failed collector.
        self.joined.load(SeqCst)
            && self.eof[0].load(SeqCst)
            && self.eof[1].load(SeqCst)
            && !self.uncertain.load(SeqCst)
    }
}
struct CommandCollector {
    outputs: std::sync::Arc<CommandOutputs>,
    stream: usize,
    complete: bool,
}
impl CommandCollector {
    fn finish(&mut self) {
        self.outputs.eof[self.stream].store(true, std::sync::atomic::Ordering::SeqCst);
        self.complete = true;
    }
}
impl Drop for CommandCollector {
    fn drop(&mut self) {
        if !self.complete {
            self.outputs.fail();
        }
    }
}
type CollectedCommandOutput = (Vec<u8>, bool);
type CommandCollectorTask = tokio::task::JoinHandle<std::io::Result<CollectedCommandOutput>>;
struct CommandOutputTasks {
    stdout: Option<CommandCollectorTask>,
    stderr: Option<CommandCollectorTask>,
    outputs: std::sync::Arc<CommandOutputs>,
}
impl CommandOutputTasks {
    fn spawn<R>(
        input: R,
        limit: usize,
        outputs: std::sync::Arc<CommandOutputs>,
        stream: usize,
        shared: Option<std::sync::Arc<CommandOutputBudget>>,
    ) -> CommandCollectorTask
    where
        R: tokio::io::AsyncRead + Unpin + Send + 'static,
    {
        // Construct the guard before spawn: cancellation before the task's first
        // poll must also record that EOF was not collected.
        let mut guard = CommandCollector {
            outputs,
            stream,
            complete: false,
        };
        tokio::spawn(async move {
            let result = bounded_output(input, limit, shared).await;
            if result.is_ok() {
                // Borrow the whole guard so its Drop state is retained by the
                // future, including cancellation before the first task poll.
                guard.finish();
            }
            result
        })
    }
    async fn join_one(
        task: &mut Option<CommandCollectorTask>,
    ) -> Option<Result<CollectedCommandOutput>> {
        let result = task.as_mut()?.await;
        // An already joined handle must never be polled or aborted again, even
        // if the other collector later fails or reaches its deadline.
        task.take();
        Some(
            result
                .map_err(|_| {
                    Error::new(
                        "operation_uncertain",
                        "command output collector interrupted",
                    )
                })
                .and_then(|result| result.map_err(|error| Error::io("read command output", error))),
        )
    }
    async fn join_pending(
        &mut self,
    ) -> (
        Option<Result<CollectedCommandOutput>>,
        Option<Result<CollectedCommandOutput>>,
    ) {
        tokio::join!(
            Self::join_one(&mut self.stdout),
            Self::join_one(&mut self.stderr)
        )
    }
    fn abort_pending(&self) {
        for task in [&self.stdout, &self.stderr].into_iter().flatten() {
            task.abort();
        }
    }
    async fn collect(mut self) -> Result<(CollectedCommandOutput, CollectedCommandOutput)> {
        let joined = tokio::time::timeout(COMMAND_OUTPUT_GRACE, self.join_pending()).await;
        match joined {
            Ok((stdout, stderr)) => {
                let result = stdout
                    .zip(stderr)
                    .ok_or_else(|| {
                        Error::new("operation_uncertain", "command output ownership was lost")
                    })
                    .and_then(|(stdout, stderr)| Ok((stdout?, stderr?)));
                if result.is_err() {
                    self.outputs.fail();
                }
                self.outputs
                    .joined
                    .store(true, std::sync::atomic::Ordering::SeqCst);
                result
            }
            Err(_) => {
                // Failure is visible before joining or publishing cancellation
                // status. Actual Tokio pipe reads and bounded_output cooperate
                // with abort; neither collector is left as a running drain loop.
                self.outputs.fail();
                self.abort_pending();
                if tokio::time::timeout(COMMAND_OUTPUT_GRACE, self.join_pending())
                    .await
                    .is_ok()
                {
                    self.outputs
                        .joined
                        .store(true, std::sync::atomic::Ordering::SeqCst);
                }
                Err(Error::new(
                    "operation_uncertain",
                    "command output did not reach joined EOF; preserve run ownership",
                ))
            }
        }
    }
}
impl Drop for CommandOutputTasks {
    fn drop(&mut self) {
        if self.stdout.is_some() || self.stderr.is_some() {
            self.outputs.fail();
            self.abort_pending();
        }
    }
}

struct CommandProcess {
    child: tokio::process::Child,
    group: crate::process_group::OwnedGroup,
    reaped: Option<crate::process_group::OwnedGroupExit>,
    outputs: std::sync::Arc<CommandOutputs>,
    guard: Option<crate::process_records::SpawnGuard>,
    journal_retired: bool,
    capture_error: Option<Error>,
    cancelled_at: Option<std::time::Instant>,
    service: bool,
    kill_escalated: bool,
    output_budget: Option<std::sync::Arc<CommandOutputBudget>>,
    rendered_output_limit: Option<usize>,
}
impl CommandProcess {
    fn new(
        child: tokio::process::Child,
        group: crate::process_group::OwnedGroup,
        guard: Option<crate::process_records::SpawnGuard>,
        capture_error: Option<Error>,
    ) -> Self {
        Self {
            child,
            group,
            reaped: None,
            outputs: Default::default(),
            guard,
            journal_retired: false,
            capture_error,
            cancelled_at: None,
            service: false,
            kill_escalated: false,
            output_budget: None,
            rendered_output_limit: None,
        }
    }
    fn with_output_budget(mut self, budget: Option<std::sync::Arc<CommandOutputBudget>>) -> Self {
        self.output_budget = budget;
        self
    }
    fn with_rendered_output_limit(mut self, limit: Option<usize>) -> Self {
        self.rendered_output_limit = limit;
        self
    }
    fn with_service(mut self, service: bool) -> Self {
        self.service = service;
        self
    }
    fn advance_service_stop(&mut self) -> Result<()> {
        if self.service
            && self.reaped.is_none()
            && !self.kill_escalated
            && self
                .cancelled_at
                .is_some_and(|at| at.elapsed() >= COMMAND_EXIT_GRACE)
        {
            self.group
                .terminate(crate::process_group::Termination::Kill)?;
            self.kill_escalated = true;
        }
        Ok(())
    }
    fn observe(&mut self) -> Result<Option<std::process::ExitStatus>> {
        if self.reaped.is_none() {
            self.reaped = self.group.try_reap_tokio(&mut self.child)?;
        }
        if let Some(exit) = self.reaped.as_ref()
            && let Some(guard) = self.guard.as_mut()
        {
            // Retain both the actual exit token and guard on a storage failure.
            // A later cancellation observation retries this exact retirement.
            guard.group_exited(exit)?;
            self.journal_retired = true;
            self.guard = None;
        }
        Ok(self.reaped.as_ref().map(|exit| exit.status()))
    }
    fn cancel(&mut self) -> Result<()> {
        self.cancelled_at
            .get_or_insert_with(std::time::Instant::now);
        let observed = self.observe();
        if self.reaped.is_none() {
            // Observation failure is not permission to abandon the owned child.
            // Signalling still requires the original unreaped group identity.
            self.group
                .terminate(if self.service && !self.kill_escalated {
                    crate::process_group::Termination::Terminate
                } else {
                    crate::process_group::Termination::Kill
                })?;
        }
        observed.map(|_| ())
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ServiceObservation {
    pub leader_running: bool,
    pub stop_requested: bool,
    pub kill_escalated: bool,
}
/// Non-serializable capability anchored to an actual unreaped owned child.
/// The control host must persist/fence operator cancellation before using it.
#[derive(Clone)]
pub struct OwnedCommandCancellation {
    process: std::sync::Arc<std::sync::Mutex<CommandProcess>>,
}
impl OwnedCommandCancellation {
    pub fn service_observation(&self) -> Result<ServiceObservation> {
        let process = self
            .process
            .lock()
            .map_err(|_| Error::new("process_error", "owned command lock failed"))?;
        if !process.service {
            return Err(Error::new(
                "scope_violation",
                "command is not a registered bundle service",
            ));
        }
        let leader_running =
            process.reaped.is_none() && process.group.leader_running_tokio(&process.child)?;
        Ok(ServiceObservation {
            leader_running,
            stop_requested: process.cancelled_at.is_some(),
            kill_escalated: process.kill_escalated || process.group.forced_kill_requested(),
        })
    }
    /// Observes only the directly owned managed group and joined inherited
    /// streams. It does not establish arbitrary escaped-descendant containment.
    pub fn group_exited(&self) -> Result<bool> {
        let mut process = self
            .process
            .lock()
            .map_err(|_| Error::new("process_error", "owned command lock failed"))?;
        process.observe()?;
        Ok(process.reaped.is_some() && process.outputs.closed())
    }
    pub fn exited(&self) -> Result<bool> {
        let mut process = self
            .process
            .lock()
            .map_err(|_| Error::new("process_error", "owned command lock failed"))?;
        process.observe()?;
        Ok(process.reaped.is_some() && process.journal_retired && process.outputs.closed())
    }
    pub fn cancel(&self) -> Result<()> {
        self.process
            .lock()
            .map_err(|_| Error::new("process_error", "owned command lock failed"))?
            .cancel()
    }
}
struct CommandRun {
    argv: Vec<String>,
    cwd: PathBuf,
    isolation_digest: String,
    started: std::time::Instant,
    timeout: Duration,
    secret_values: Vec<String>,
}
pub struct OwnedCommand {
    process: std::sync::Arc<std::sync::Mutex<CommandProcess>>,
    supervisor: Option<tokio::task::JoinHandle<Result<CommandOutcome>>>,
    settled: bool,
}
impl OwnedCommand {
    fn supervise<O, E>(
        run: CommandRun,
        process: CommandProcess,
        stdout: O,
        stderr: E,
        limit: usize,
    ) -> Self
    where
        O: tokio::io::AsyncRead + Unpin + Send + 'static,
        E: tokio::io::AsyncRead + Unpin + Send + 'static,
    {
        let outputs = process.outputs.clone();
        let rendered_output_limit = process.rendered_output_limit;
        let tasks = CommandOutputTasks {
            stdout: Some(CommandOutputTasks::spawn(
                stdout,
                limit,
                outputs.clone(),
                0,
                process.output_budget.clone(),
            )),
            stderr: Some(CommandOutputTasks::spawn(
                stderr,
                limit,
                outputs.clone(),
                1,
                process.output_budget.clone(),
            )),
            outputs,
        };
        let process = std::sync::Arc::new(std::sync::Mutex::new(process));
        let cancellation = OwnedCommandCancellation {
            process: process.clone(),
        };
        // This task owns cleanup even when the caller drops wait(). The only
        // streaming input is a Tokio pipe. Deadlines bound polling/retries and
        // cooperative async reads; they cannot preempt a blocked synchronous
        // OS or journal syscall. No arbitrary user callback runs in this task.
        let supervisor = tokio::spawn(async move {
            run.supervise(cancellation, tasks, rendered_output_limit)
                .await
        });
        Self {
            process,
            supervisor: Some(supervisor),
            settled: false,
        }
    }
    pub fn cancellation(&self) -> OwnedCommandCancellation {
        OwnedCommandCancellation {
            process: self.process.clone(),
        }
    }
    pub async fn wait(mut self) -> Result<CommandOutcome> {
        let result = self
            .supervisor
            .take()
            .ok_or_else(|| {
                Error::new(
                    "operation_uncertain",
                    "command supervisor ownership was lost",
                )
            })?
            .await;
        self.settled = true;
        match result {
            Ok(result) => result,
            Err(_) => {
                if let Ok(process) = self.process.lock() {
                    process.outputs.fail();
                }
                let _ = self.cancellation().cancel();
                Err(Error::new(
                    "operation_uncertain",
                    "command supervisor interrupted; preserve run ownership",
                ))
            }
        }
    }
}
impl Drop for OwnedCommand {
    fn drop(&mut self) {
        if !self.settled {
            // Do not abort the supervisor: it retains both collectors and the
            // actual child until its bounded cleanup attempt has settled.
            let _ = self.cancellation().cancel();
        }
    }
}
impl CommandRun {
    async fn observe_until_exit(
        &self,
        cancellation: &OwnedCommandCancellation,
    ) -> Result<(std::process::ExitStatus, bool)> {
        let mut failure = cancellation
            .process
            .lock()
            .map_err(|_| Error::new("process_error", "owned command lock failed"))?
            .capture_error
            .clone();
        if failure.is_some() {
            let _ = cancellation.cancel();
        }
        let mut timed_out = false;
        loop {
            let (observed, cancelled_at, service, output_uncertain) = {
                let mut process = cancellation
                    .process
                    .lock()
                    .map_err(|_| Error::new("process_error", "owned command lock failed"))?;
                (
                    process
                        .advance_service_stop()
                        .and_then(|()| process.observe()),
                    process.cancelled_at,
                    process.service,
                    process
                        .outputs
                        .uncertain
                        .load(std::sync::atomic::Ordering::SeqCst),
                )
            };
            match observed {
                Ok(Some(status)) => return failure.map_or(Ok((status, timed_out)), Err),
                Ok(None) => (),
                Err(error) => {
                    failure.get_or_insert(error);
                    let _ = cancellation.cancel();
                }
            }
            if output_uncertain {
                failure.get_or_insert_with(|| {
                    Error::new(
                        "operation_uncertain",
                        "command output collector failed; preserve ownership",
                    )
                });
                let _ = cancellation.cancel();
            }
            if !timed_out && self.started.elapsed() >= self.timeout {
                timed_out = true;
                if let Err(error) = cancellation.cancel() {
                    failure.get_or_insert(error);
                }
            }
            let exit_grace = if service {
                COMMAND_EXIT_GRACE.saturating_mul(2)
            } else {
                COMMAND_EXIT_GRACE
            };
            if cancelled_at.is_some_and(|at| at.elapsed() >= exit_grace) {
                return Err(failure.unwrap_or_else(|| {
                    Error::new(
                        "operation_uncertain",
                        "terminated command has not retired; preserve ownership",
                    )
                }));
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    async fn supervise(
        self,
        cancellation: OwnedCommandCancellation,
        tasks: CommandOutputTasks,
        rendered_output_limit: Option<usize>,
    ) -> Result<CommandOutcome> {
        let status = self.observe_until_exit(&cancellation).await;
        if status.is_err() {
            let _ = cancellation.cancel();
        }
        // Always settle BOTH collectors, including after a process/journal error.
        let collected = tasks.collect().await;
        let (status, timed_out) = status?;
        let ((out, out_truncated), (err, err_truncated)) = collected?;
        if !cancellation.exited()? {
            return Err(Error::new(
                "operation_uncertain",
                "managed command lacks durable group retirement and joined output; preserve ownership",
            ));
        }
        let (stdout, stderr, rendered_truncated) =
            rendered_command_output(&out, &err, &self.secret_values, rendered_output_limit)?;
        #[cfg(unix)]
        let termination_signal = {
            use std::os::unix::process::ExitStatusExt;
            status.signal()
        };
        #[cfg(not(unix))]
        let termination_signal = None;
        Ok(CommandOutcome {
            schema_version: 1,
            argv: self.argv,
            cwd: self.cwd,
            isolation_digest: self.isolation_digest,
            exit_code: status.code(),
            termination_signal,
            timed_out,
            stdout,
            stderr,
            output_truncated: out_truncated || err_truncated || rendered_truncated,
            elapsed_ms: self.started.elapsed().as_millis().min(u64::MAX as u128) as u64,
        })
    }
}

pub async fn run_command(
    isolation: &ValidatedIsolation,
    argv: &[String],
    timeout: Duration,
    output_limit: usize,
    secret_values: &[String],
) -> Result<CommandOutcome> {
    start_command(isolation, argv, timeout, output_limit, secret_values)?
        .wait()
        .await
}

/// Explicit allowlist rather than deleting a few known credential names from an
/// otherwise broad inherited environment. Aliases therefore cannot escape it.
pub fn worker_environment(
    scratch: &Path,
    allowed: &BTreeMap<String, String>,
    secret_names: &[String],
) -> Result<BTreeMap<String, String>> {
    let mut environment = BTreeMap::from([
        ("HOME".into(), scratch.to_string_lossy().into_owned()),
        ("TMPDIR".into(), scratch.to_string_lossy().into_owned()),
        ("PATH".into(), "/usr/bin:/bin:/usr/sbin:/sbin".into()),
        ("LANG".into(), "C.UTF-8".into()),
    ]);
    for (name, value) in allowed {
        if secret_names.contains(name)
            || name.contains("TOKEN")
            || name.contains("SECRET")
            || name.ends_with("_KEY")
            || [
                "HOME",
                "TMPDIR",
                "CODEX_HOME",
                "PATH",
                "DYLD_INSERT_LIBRARIES",
                "LD_PRELOAD",
                "SSH_AUTH_SOCK",
            ]
            .contains(&name.as_str())
        {
            return Err(Error::new(
                "scope_violation",
                "worker environment contains a privileged or credential-bearing variable",
            ));
        }
        if name.is_empty()
            || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            || value.contains('\0')
        {
            return Err(Error::invalid("invalid worker environment entry"));
        }
        environment.insert(name.clone(), value.clone());
    }
    Ok(environment)
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

#[cfg(test)]
mod owned_exit_tests {
    use super::*;
    #[test]
    fn unqualified_repository_tree_is_rejected_before_native_launch() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let repository = root.join("repository");
        let scratch = root.join("scratch");
        std::fs::create_dir(&repository).unwrap();
        std::fs::create_dir(&scratch).unwrap();
        let isolation = IsolationPlan {
            schema_version: 1,
            role: ExecutionRole::Child,
            execution_root: root,
            cwd: repository.clone(),
            repository,
            scratch,
            read_roots: vec![],
            source_writes: vec![],
            resource_writes: vec![],
            forbidden_roots: vec![],
            managed_execution: None,
        }
        .validate()
        .unwrap();
        assert_eq!(
            repository_command_prerequisite(&isolation)
                .unwrap_err()
                .code,
            "process_tree_unverified"
        );
    }
}

#[cfg(all(test, target_os = "macos"))]
#[path = "isolation_command_tests.rs"]
mod command_tests;

#[cfg(test)]
#[path = "isolation_observation_tests.rs"]
mod observation_tests;
