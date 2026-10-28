//! Managed execution environments and observations. These do not grant launch authority.
use kegari_digest::{bytes_digest, canonical_digest};
use kegari_types::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

#[cfg(test)]
const TEST_BUILD: &str = "fixture-runtime-build";

#[path = "managed_execution.rs"]
mod managed_execution;
pub use managed_execution::BoundManagedExecution;
#[path = "managed_resources.rs"]
mod managed_resources;

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
    runtime_build: Arc<str>,
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
        if let Some(managed) = &plan.managed_execution {
            managed.validate_plan(&plan, runtime_build)?;
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
    /// checks geometry, current root identities and retained resource metadata;
    /// it does not scan writable descendants, read resource contents, or qualify
    /// a launch. Actual command preparation retains its full validation path.
    pub fn revalidate_inspection_until(&self, deadline: std::time::Instant) -> Result<()> {
        self.revalidate_inspection_with_budget(&mut managed_resources::ResourceBudget::until(
            deadline,
        ))
    }
    fn revalidate_inspection_with_budget(
        &self,
        budget: &mut managed_resources::ResourceBudget,
    ) -> Result<()> {
        budget.check()?;
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
        budget.check()?;
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
            budget.check()?;
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
            budget.check()?;
            if path.as_os_str().len() > MAX_COMMAND_CWD_BYTES
                || canonical_target(path)? != *path
                || !path.is_dir()
            {
                return Err(Error::new(
                    "scope_violation",
                    "inspection root or cwd changed",
                ));
            }
            budget.check()?;
        }
        for (path, directory) in plan
            .read_roots
            .iter()
            .chain(&plan.source_writes)
            .chain(&plan.resource_writes)
            .map(|grant| (&grant.path, grant.directory))
            .chain(plan.forbidden_roots.iter().map(|path| (path, false)))
        {
            budget.check()?;
            if path.as_os_str().len() > MAX_COMMAND_CWD_BYTES
                || canonical_target(path)? != *path
                || (directory && !path.is_dir())
            {
                return Err(Error::new(
                    "scope_violation",
                    "inspection grant or forbidden root changed",
                ));
            }
            budget.check()?;
        }
        if let Some(domain) = &plan.managed_execution {
            domain.validate_plan_with_budget(plan, budget, &self.runtime_build)?;
        }
        budget.check()
    }
    /// Full content observation belongs outside serialized Control callbacks.
    /// A successful observation binds resources; it does not qualify a workload.
    pub fn revalidate_resource_contents(&self) -> Result<()> {
        self.revalidate()?;
        if let Some(managed) = &self.plan.managed_execution {
            managed.revalidate(&self.runtime_build)?;
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

/// Exact process-free launch identity for an admitted repository command. The
/// profile digest binds the resolved executable and full Seatbelt policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandLaunchIdentity {
    pub cwd: PathBuf,
    pub launcher: kegari_platform::BinaryIdentity,
    pub target: kegari_platform::BinaryIdentity,
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
    pub canonical_executable: kegari_platform::BinaryIdentity,
    pub launcher: kegari_platform::BinaryIdentity,
    pub cwd: PathBuf,
    pub environment: BTreeMap<String, String>,
    pub original_isolation_digest: String,
    pub isolation_digest: String,
    pub launch_digest: String,
    pub backend_policy_digest: String,
}

/// Description of the existing command boundary, not an environment request or
/// a qualification/dispatch capability. No process, executable lookup, binary
/// content inspection or ambient environment lookup is needed to produce it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentObservation {
    #[serde(deserialize_with = "environment_observation_schema")]
    pub schema_version: u32,
    pub role: ExecutionRole,
    pub original_cwd: PathBuf,
    pub cwd: PathBuf,
    pub original_isolation_digest: String,
    pub isolation_digest: String,
    pub original_domain_digest: String,
    pub domain_digest: String,
    pub original_backend_policy_digest: String,
    pub backend_policy_digest: String,
    pub search_path: Vec<PathBuf>,
    pub environment: BTreeMap<String, String>,
    pub limits: EnvironmentLimits,
    /// Actual current preflight result. None would describe only that check,
    /// never a permit, ownership, result or whole-runtime qualification.
    pub native_prerequisite: Option<Error>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentLimits {
    pub max_arguments: usize,
    pub max_argument_bytes: usize,
    pub max_argv_bytes: usize,
    pub max_cwd_bytes: usize,
    pub lifecycle: EnvironmentLifecycle,
    pub network: EnvironmentNetwork,
    pub detached_descendants: EnvironmentDetachedDescendants,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentLifecycle {
    OwnedGroupsAndRegisteredServices,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentNetwork {
    Denied,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentDetachedDescendants {
    OutsideGuarantee,
}

const MAX_ENVIRONMENT_OBSERVATION_BYTES: usize = 48 * 1024;

fn environment_observation_schema<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<u32, D::Error> {
    let schema = u32::deserialize(deserializer)?;
    if schema != 1 {
        return Err(serde::de::Error::custom(
            "unsupported environment observation schema",
        ));
    }
    Ok(schema)
}

/// Observe only the retained effective environment and boundary. Revalidation
/// checks existing metadata; it never recaptures input bytes or adopts new
/// roots. Callers retain their own operation/role and original time bounds.
pub fn observe_environment(isolation: &ValidatedIsolation) -> Result<EnvironmentObservation> {
    observe_environment_until(
        isolation,
        std::time::Instant::now() + Duration::from_secs(5),
    )
}

/// Retains the caller's absolute cutoff throughout descriptive inspection.
/// No default or renewed operation budget is substituted by this entry point.
pub fn observe_environment_until(
    isolation: &ValidatedIsolation,
    deadline: std::time::Instant,
) -> Result<EnvironmentObservation> {
    let mut budget = managed_resources::ResourceBudget::until(deadline);
    isolation.revalidate_inspection_with_budget(&mut budget)?;
    let unavailable = || {
        Error::new(
            "execution_environment_unavailable",
            "this operation has no admitted repository command domain",
        )
    };
    let domain = isolation
        .plan
        .managed_execution
        .as_ref()
        .ok_or_else(unavailable)?;
    let original_plan = isolation
        .origin
        .as_ref()
        .map_or(&isolation.plan, |origin| &origin.plan);
    let original_domain = original_plan
        .managed_execution
        .as_ref()
        .ok_or_else(unavailable)?;
    let native_prerequisite = match checked_repository_command_prerequisite(isolation) {
        Ok(()) => None,
        Err(error) if error.code == "managed_profile_unqualified" => Some(error),
        Err(error) => return Err(error),
    };
    let observation = EnvironmentObservation {
        schema_version: 1,
        role: isolation.plan.role,
        original_cwd: original_plan.cwd.clone(),
        cwd: isolation.plan.cwd.clone(),
        original_isolation_digest: isolation.original_digest().to_owned(),
        isolation_digest: isolation.digest.clone(),
        original_domain_digest: canonical_digest(1, original_domain)?,
        domain_digest: canonical_digest(1, domain)?,
        original_backend_policy_digest: original_domain.policy_digest().to_owned(),
        backend_policy_digest: domain.policy_digest().to_owned(),
        search_path: domain.search_roots().to_vec(),
        environment: effective_environment_with_path(isolation, domain.command_path()?),
        limits: EnvironmentLimits {
            max_arguments: MAX_COMMAND_ARGS,
            max_argument_bytes: MAX_COMMAND_ARG_BYTES,
            max_argv_bytes: MAX_COMMAND_BYTES,
            max_cwd_bytes: MAX_COMMAND_CWD_BYTES,
            lifecycle: EnvironmentLifecycle::OwnedGroupsAndRegisteredServices,
            network: EnvironmentNetwork::Denied,
            detached_descendants: EnvironmentDetachedDescendants::OutsideGuarantee,
        },
        native_prerequisite,
    };
    // Count the encoded public result without allocating a second full copy.
    // Existing domain bounds already bound the cloned path/metadata inventory.
    budget.check()?;
    struct SizeBound<'a>(usize, &'a managed_resources::ResourceBudget);
    impl std::io::Write for SizeBound<'_> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.1.check().map_err(std::io::Error::other)?;
            if bytes.len() > MAX_ENVIRONMENT_OBSERVATION_BYTES.saturating_sub(self.0) {
                return Err(std::io::Error::other("environment observation bound"));
            }
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let encoded = serde_json::to_writer(&mut SizeBound(0, &budget), &observation);
    budget.check()?;
    encoded.map_err(|_| {
        Error::new(
            "execution_environment_limit",
            "complete environment observation exceeds its public result bound",
        )
    })?;
    Ok(observation)
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
    Ok(render_observed_command(isolation, argv)?.1)
}
pub fn command_launch_identity(
    isolation: &ValidatedIsolation,
    argv: &[String],
) -> Result<CommandLaunchIdentity> {
    Ok(observe_command(isolation, argv)?.launch_identity())
}

/// Exact process-free rendering. It grants no admission or launch authority.
pub struct RenderedCommand {
    executable: PathBuf,
    argv: Vec<String>,
    environment: BTreeMap<String, String>,
    profile: String,
}
impl RenderedCommand {
    pub fn executable(&self) -> &Path {
        &self.executable
    }
    pub fn argv(&self) -> &[String] {
        &self.argv
    }
    pub fn environment(&self) -> &BTreeMap<String, String> {
        &self.environment
    }
    pub fn profile(&self) -> &str {
        &self.profile
    }
}

/// Render and observe the same invocation in the existing validation order.
/// The result is descriptive; native callers still require their own live gate.
pub fn render_observed_command(
    isolation: &ValidatedIsolation,
    argv: &[String],
) -> Result<(RenderedCommand, CommandObservation)> {
    let rendered = render_command(isolation, argv)?;
    let observation = observe_rendered_command(isolation, &rendered)?;
    Ok((rendered, observation))
}

/// One exact rendering for inspection and every actual repository invocation.
/// Deliberately reads no inherited environment; PATH is the recorded domain.
fn effective_environment(isolation: &ValidatedIsolation) -> Result<BTreeMap<String, String>> {
    Ok(effective_environment_with_path(
        isolation,
        isolation.command_path()?,
    ))
}
fn effective_environment_with_path(
    isolation: &ValidatedIsolation,
    path: String,
) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("PATH".into(), path),
        (
            "HOME".into(),
            isolation.plan.scratch.to_string_lossy().into_owned(),
        ),
        (
            "TMPDIR".into(),
            isolation.plan.scratch.to_string_lossy().into_owned(),
        ),
        ("LANG".into(), "C.UTF-8".into()),
    ])
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
    let environment = effective_environment(isolation)?;
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
        launcher: kegari_platform::BinaryIdentity::inspect(
            Path::new("/usr/bin/sandbox-exec"),
            false,
        )?,
        canonical_executable: kegari_platform::BinaryIdentity::inspect(
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
    checked_repository_command_prerequisite(isolation)
}
fn checked_repository_command_prerequisite(isolation: &ValidatedIsolation) -> Result<()> {
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
        .validate(TEST_BUILD)
        .unwrap();
        assert_eq!(
            repository_command_prerequisite(&isolation)
                .unwrap_err()
                .code,
            "process_tree_unverified"
        );
    }
}

#[cfg(test)]
#[path = "observation_tests.rs"]
mod observation_tests;
