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
    pub forbidden_roots: Vec<PathBuf>,
    pub executables: Vec<PathBuf>,
}

/// Constructed only by checking the actual filesystem. Revalidated before use.
#[derive(Clone, Debug)]
pub struct ValidatedIsolation {
    plan: IsolationPlan,
    digest: String,
}

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
            verify_writable_links(&grant.path)?;
        }
        for executable in &mut plan.executables {
            *executable = executable
                .canonicalize()
                .map_err(|e| Error::io("resolve allowed executable", e))?;
            if !executable.is_file()
                || plan
                    .forbidden_roots
                    .iter()
                    .any(|f| executable.starts_with(f))
            {
                return Err(Error::new("scope_violation", "invalid executable grant"));
            }
        }
        if matches!(
            plan.role,
            ExecutionRole::Coordination | ExecutionRole::IndependentReview
        ) && !plan.executables.is_empty()
        {
            return Err(Error::new(
                "scope_violation",
                "coordination and independent review cannot execute commands",
            ));
        }
        let digest = canonical_digest(1, &plan)?;
        Ok(ValidatedIsolation { plan, digest })
    }
}

fn verify_writable_links(path: &Path) -> Result<()> {
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
    pub fn revalidate(&self) -> Result<()> {
        if self.plan.validate()?.digest != self.digest {
            return Err(Error::new(
                "scope_violation",
                "execution boundary changed since admission",
            ));
        }
        Ok(())
    }
    /// Bare executables follow the admitted PATH; explicit relative paths stay
    /// anchored at cwd. Arguments and script text are never rewritten.
    pub fn resolve_executable(&self, name: &str) -> Result<PathBuf> {
        self.revalidate()?;
        let path = Path::new(name);
        let resolved = if path.is_absolute() {
            path.to_path_buf()
        } else if name.contains('/') {
            self.plan.cwd.join(path)
        } else {
            let matches: Vec<_> = self
                .plan
                .executables
                .iter()
                .filter(|p| p.file_name().is_some_and(|n| n == name))
                .collect();
            if matches.len() != 1 {
                return Err(Error::new(
                    "scope_violation",
                    "bare executable is absent or ambiguous in the admitted PATH",
                ));
            }
            matches[0].clone()
        }
        .canonicalize()
        .map_err(|e| Error::io("resolve command in execution cwd", e))?;
        if !self.plan.executables.contains(&resolved) {
            return Err(Error::new("scope_violation", "executable is not admitted"));
        }
        Ok(resolved)
    }
    pub fn command_path(&self) -> Result<String> {
        let parents: std::collections::BTreeSet<_> = self
            .plan
            .executables
            .iter()
            .filter_map(|p| p.parent())
            .collect();
        Ok(parents
            .into_iter()
            .map(|p| {
                p.to_str()
                    .ok_or_else(|| Error::invalid("non-UTF-8 executable path"))
            })
            .collect::<Result<Vec<_>>>()?
            .join(":"))
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
            profile.push_str(&format!("(allow file-read* (subpath {}))\n", quote(path)));
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
                quote_path(&grant.path)?
            ));
        }
        for grant in &self.plan.source_writes {
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
        for executable in &self.plan.executables {
            profile.push_str(&format!(
                "(allow file-read* (literal {}))\n(allow process-exec (literal {}))\n",
                quote_path(executable)?,
                quote_path(executable)?
            ));
        }
        for forbidden in &self.plan.forbidden_roots {
            profile.push_str(&format!(
                "(deny file-read* file-write* (subpath {}))\n",
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandOutcome {
    pub schema_version: u32,
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub isolation_digest: String,
    pub exit_code: Option<i32>,
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

async fn bounded_output<R: tokio::io::AsyncRead + Unpin>(
    mut input: R,
    limit: usize,
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
        bytes.extend_from_slice(&buffer[..take]);
        truncated |= take < n;
    }
    Ok((bytes, truncated))
}

/// Run a check/hook only under the OS boundary. The caller persists its admitted
/// operation before calling; outputs never establish revision provenance alone.
pub async fn run_command(
    isolation: &ValidatedIsolation,
    argv: &[String],
    timeout: Duration,
    output_limit: usize,
    secret_values: &[String],
) -> Result<CommandOutcome> {
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
    let executable = isolation.resolve_executable(&argv[0])?;
    let profile = isolation.seatbelt_profile()?;
    let mut command = Command::new("/usr/bin/sandbox-exec");
    command
        .args(["-p", &profile])
        .arg(executable)
        .args(&argv[1..]);
    command
        .current_dir(&isolation.plan.cwd)
        .env_clear()
        .env("PATH", isolation.command_path()?)
        .env("HOME", &isolation.plan.scratch)
        .env("TMPDIR", &isolation.plan.scratch)
        .env("LANG", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    {
        command.process_group(0);
    }
    let started = std::time::Instant::now();
    let mut child = command
        .spawn()
        .map_err(|e| Error::io("spawn confined command", e))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::new("process_error", "missing stdout pipe"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| Error::new("process_error", "missing stderr pipe"))?;
    let mut out_task = tokio::spawn(bounded_output(stdout, output_limit));
    let mut err_task = tokio::spawn(bounded_output(stderr, output_limit));
    let result = tokio::time::timeout(timeout, child.wait()).await;
    let timed_out = result.is_err();
    let status = match result {
        Ok(status) => status.map_err(|e| Error::io("wait confined command", e))?,
        Err(_) => {
            // The unreaped Child owns this PID, so it cannot already name a reused process.
            #[cfg(unix)]
            if let Some(id) = child.id() {
                unsafe {
                    libc::kill(-(id as i32), libc::SIGKILL);
                }
            }
            child
                .kill()
                .await
                .map_err(|e| Error::io("terminate confined command", e))?;
            child
                .wait()
                .await
                .map_err(|e| Error::io("reap confined command", e))?
        }
    };
    // Descendants holding inherited pipes cannot prolong the result forever.
    let collect = async {
        let out = (&mut out_task)
            .await
            .map_err(|_| Error::new("process_error", "stdout collector interrupted"))?
            .map_err(|e| Error::io("read command output", e))?;
        let err = (&mut err_task)
            .await
            .map_err(|_| Error::new("process_error", "stderr collector interrupted"))?
            .map_err(|e| Error::io("read command diagnostics", e))?;
        Ok::<_, Error>((out, err))
    };
    let ((out, out_truncated), (err, err_truncated)) =
        match tokio::time::timeout(Duration::from_secs(1), collect).await {
            Ok(result) => result?,
            Err(_) => {
                out_task.abort();
                err_task.abort();
                return Err(Error::new(
                    "operation_uncertain",
                    "command descendants retain output pipes; preserve the run",
                ));
            }
        };
    let stderr = redact(String::from_utf8_lossy(&err).into_owned(), secret_values);
    if stderr.contains("sandbox-exec: sandbox_apply")
        || stderr.contains("sandbox-exec: sandbox_init")
    {
        return Err(Error::new(
            "isolation_unavailable",
            "host policy prevented applying the execution sandbox",
        ));
    }
    Ok(CommandOutcome {
        schema_version: 1,
        argv: argv.to_vec(),
        cwd: isolation.plan.cwd.clone(),
        isolation_digest: isolation.digest.clone(),
        exit_code: status.code(),
        timed_out,
        stdout: redact(String::from_utf8_lossy(&out).into_owned(), secret_values),
        stderr,
        output_truncated: out_truncated || err_truncated,
        elapsed_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
    })
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
