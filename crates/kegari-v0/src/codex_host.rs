//! Native Codex launch and owned-process integration. Protocol data is not authority.
use crate::isolation::ValidatedIsolation;
use kegari_codex::{
    CodexConnectionMetadata, PROTOCOL_PROFILE, PermissionConfiguration, RpcClient,
    SUPPORTED_CODEX_VERSION, SessionScope,
};
use kegari_digest::canonical_digest;
use kegari_types::error::{Error, Result};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path, process::Stdio, time::Duration};
use tokio::{
    io::AsyncReadExt,
    process::{Child, Command},
    time::Instant,
};

/// The installed binary that actually returned the pinned version. Construction
/// is read-only metadata probing; native session launch still needs a host gate.
pub struct VerifiedCodexExecutable(kegari_platform::BinaryIdentity);

impl SessionScope for ValidatedIsolation {
    fn revalidate(&self) -> Result<()> {
        ValidatedIsolation::revalidate(self)
    }
    fn cwd(&self) -> &Path {
        &self.plan().cwd
    }
    fn isolation_digest(&self) -> &str {
        self.digest()
    }
}

/// Owns the actual spawned child and its original streams. Constructed only by
/// native launch, never by adopting persisted process metadata.
pub struct NativeCodexClient {
    client: RpcClient,
    child: std::sync::Arc<OwnedProviderProcess>,
    diagnostic_task: Option<tokio::task::JoinHandle<()>>,
}
impl NativeCodexClient {
    pub fn client_mut(&mut self) -> &mut RpcClient {
        &mut self.client
    }
    pub fn cancellation(&self) -> OwnedProviderCancellation {
        OwnedProviderCancellation {
            child: self.child.clone(),
        }
    }
    pub async fn terminate(&mut self) -> Result<()> {
        terminate_owned_provider(&self.child).await?;
        if let Some(task) = self.diagnostic_task.take() {
            task.abort();
        }
        Ok(())
    }
}
impl Drop for NativeCodexClient {
    fn drop(&mut self) {
        let _ = self.cancellation().request_cancel();
        if let Some(task) = self.diagnostic_task.take() {
            task.abort();
        }
    }
}

pub fn permission_configuration(
    isolation: &ValidatedIsolation,
    codex_home: &Path,
) -> Result<PermissionConfiguration> {
    isolation.revalidate()?;
    let p = isolation.plan();
    let home = codex_home
        .canonicalize()
        .map_err(|e| Error::io("resolve isolated Codex home", e))?;
    if p.repository.starts_with(&home)
        || home.starts_with(&p.repository)
        || home.starts_with(&p.scratch)
        || p.read_roots
            .iter()
            .chain(p.source_writes.iter())
            .chain(p.resource_writes.iter())
            .any(|g| g.path.starts_with(&home) || home.starts_with(&g.path))
    {
        return Err(Error::new(
            "scope_violation",
            "Codex authentication home must be outside worker access",
        ));
    }
    let profile_id = format!(
        "kegari_{}",
        isolation.digest().rsplit(':').next().unwrap_or_default()
    );
    let mut fs = BTreeMap::<String, Value>::from([
        (":root".into(), json!("deny")),
        (":minimal".into(), json!("read")),
        (":slash_tmp".into(), json!("deny")),
        (":tmpdir".into(), json!("deny")),
    ]);
    fs.insert(p.repository.to_string_lossy().into_owned(), json!("read"));
    fs.insert(p.scratch.to_string_lossy().into_owned(), json!("write"));
    for grant in &p.read_roots {
        fs.insert(grant.path.to_string_lossy().into_owned(), json!("read"));
    }
    for grant in p.source_writes.iter().chain(p.resource_writes.iter()) {
        fs.insert(grant.path.to_string_lossy().into_owned(), json!("write"));
    }
    // Scope grants cannot reopen privileged roots because IsolationPlan rejects overlap.
    for forbidden in p.forbidden_roots.iter().chain(std::iter::once(&home)) {
        fs.insert(forbidden.to_string_lossy().into_owned(), json!("deny"));
    }
    for folder in [".git", ".codex", ".agents"] {
        fs.insert(
            p.repository.join(folder).to_string_lossy().into_owned(),
            json!("read"),
        );
    }
    fs.insert(
        p.repository.join(".aws").to_string_lossy().into_owned(),
        json!("deny"),
    );
    let mut permissions = BTreeMap::new();
    permissions.insert(
        profile_id.clone(),
        json!({"filesystem":fs,"network":{"enabled":false}}),
    );
    Ok(PermissionConfiguration {
        profile_id: profile_id.clone(),
        isolation_digest: isolation.digest().into(),
        settings: json!({
            "default_permissions":profile_id, "permissions":permissions,
            "approval_policy":"never", "approvals_reviewer":"user", "web_search":"disabled", "forced_login_method":"chatgpt",
            "apps":{"_default":{"enabled":false,"default_tools_enabled":false}},
            "features":{"apps":false,"plugins":false,"browser_use":false,"browser_use_external":false,
                "computer_use":false,"in_app_browser":false,"image_generation":false,"multi_agent":false,
                "shell_tool":false,"code_mode":false,"memories":false,"hooks":false,
                "skill_mcp_dependency_install":false,"request_permissions_tool":false},
            "shell_environment_policy":{"inherit":"none","include_only":[],"set":{"HOME":p.scratch,"TMPDIR":p.scratch,"PATH":"/usr/bin:/bin:/usr/sbin:/sbin"}}
        }),
    })
}

pub async fn verify_version(executable: &Path, deadline: Duration) -> Result<()> {
    let identity = kegari_platform::BinaryIdentity::inspect(executable, false)?;
    start_version_probe(&identity, None)?
        .verify(deadline)
        .await
        .map(|_| ())
}
/// Split metadata probe: spawn/register while serialized, wait outside the gate.
pub fn start_version_probe(
    executable: &kegari_platform::BinaryIdentity,
    mut guard: Option<crate::process_records::SpawnGuard>,
) -> Result<VersionProbe> {
    executable.revalidate(false)?;
    if let Some(guard) = &guard
        && (guard.binding().executable != *executable
            || guard.binding().argv_digest
                != kegari_digest::canonical_digest(1, &vec!["--version"])?
            || guard.binding().kind != "codex-version"
            || !guard.binding().observed_executables.is_empty())
    {
        return Err(Error::new(
            "worker_binding",
            "version process guard differs from actual pinned launch",
        ));
    }
    let mut command = Command::new(&executable.path);
    command
        .arg("--version")
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    if let Some(guard) = &guard {
        command.current_dir(&guard.binding().cwd);
        guard.configure_tokio(&mut command)?;
    }
    let (mut child, group) = kegari_process::group::OwnedGroup::spawn_tokio(&mut command)?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::new("process_error", "version probe stdout is missing"))?;
    if let Some(guard) = &mut guard {
        // A quick metadata process may exit before inspection. Its actual
        // Child/OwnedGroup must still be joined before bytes qualify a version.
        let _ = guard.spawned_tokio(&child, &group);
    }
    Ok(VersionProbe {
        identity: executable.clone(),
        stdout: Some(stdout),
        child: std::sync::Arc::new(OwnedProviderProcess::new(child, group, guard)),
    })
}
pub async fn spawn(
    executable: &Path,
    codex_home: &Path,
    permissions: &PermissionConfiguration,
    request_timeout: Duration,
) -> Result<NativeCodexClient> {
    let verified = verify_executable(executable, request_timeout).await?;
    start_verified(&verified, codex_home, permissions, request_timeout)
}
pub async fn verify_executable(
    executable: &Path,
    deadline: Duration,
) -> Result<VerifiedCodexExecutable> {
    let identity = kegari_platform::BinaryIdentity::inspect(executable, false)?;
    verify_version(&identity.path, deadline).await?;
    identity.revalidate(false)?;
    Ok(VerifiedCodexExecutable(identity))
}
/// Synchronous process creation, for use inside the host's serialized fresh
/// permit gate. Install the returned owned cancellation handle before release.
pub fn start_verified(
    executable: &VerifiedCodexExecutable,
    codex_home: &Path,
    permissions: &PermissionConfiguration,
    request_timeout: Duration,
) -> Result<NativeCodexClient> {
    start_inner(executable, codex_home, permissions, request_timeout, None)
}
/// Native worker startup requires a durable, inherited guard bound before spawn.
pub fn start_verified_guarded(
    executable: &VerifiedCodexExecutable,
    codex_home: &Path,
    permissions: &PermissionConfiguration,
    request_timeout: Duration,
    guard: crate::process_records::SpawnGuard,
) -> Result<NativeCodexClient> {
    start_inner(
        executable,
        codex_home,
        permissions,
        request_timeout,
        Some(guard),
    )
}
fn start_inner(
    executable: &VerifiedCodexExecutable,
    codex_home: &Path,
    permissions: &PermissionConfiguration,
    request_timeout: Duration,
    mut guard: Option<crate::process_records::SpawnGuard>,
) -> Result<NativeCodexClient> {
    executable.0.revalidate(false)?;
    let args = kegari_codex::app_server_argv(permissions)?;
    if let Some(guard) = &guard
        && (guard.binding().executable != executable.0
            || guard.binding().argv_digest != kegari_digest::canonical_digest(1, &args)?
            || guard.binding().kind != "codex-app-server"
            || !guard.binding().observed_executables.is_empty())
    {
        return Err(Error::new(
            "worker_binding",
            "provider process guard differs from actual pinned launch",
        ));
    }
    if request_timeout.is_zero() {
        return Err(Error::invalid("request timeout must be positive"));
    }
    let home = codex_home
        .canonicalize()
        .map_err(|e| Error::io("Codex auth home", e))?;
    if !home.is_dir() {
        return Err(Error::invalid(
            "Codex auth home must be an existing isolated directory",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let m = home
            .metadata()
            .map_err(|e| Error::io("Codex auth home metadata", e))?;
        if m.permissions().mode() & 0o077 != 0 || m.uid() != unsafe { libc::geteuid() } {
            return Err(Error::new(
                "unsafe_permissions",
                "Codex auth home must be owner-only and owned by the execution user",
            ));
        }
    }
    let mut command = Command::new(&executable.0.path);
    command.args(args);
    if let Some(guard) = &guard {
        command.current_dir(&guard.binding().cwd);
        guard.configure_tokio(&mut command)?;
    }
    command
        .env_clear()
        .env("CODEX_HOME", &home)
        .env("HOME", &home)
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .env("LANG", "C.UTF-8")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let (mut child, group) = kegari_process::group::OwnedGroup::spawn_tokio(&mut command)?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::new("process_error", "app-server stdout unavailable"))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| Error::new("process_error", "app-server stdin unavailable"))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| Error::new("process_error", "app-server stderr unavailable"))?;
    // Diagnostics are drained separately. Arbitrary provider text is not a
    // protocol frame and cannot leak into status/log output.
    let diagnostic_task = tokio::spawn(async move {
        let mut buffer = [0u8; 4096];
        while matches!(stderr.read(&mut buffer).await,Ok(n) if n>0) {}
    });
    // Identity capture can race a quick exit. Retain the actual owned handle
    // even on capture failure; no protocol is sent and termination still joins it.
    let startup_error = guard
        .as_mut()
        .and_then(|guard| guard.spawned_tokio(&child, &group).err());
    let client =
        RpcClient::from_host_transport(stdout, stdin, request_timeout, home, startup_error)?;
    Ok(NativeCodexClient {
        client,
        child: std::sync::Arc::new(OwnedProviderProcess::new(child, group, guard)),
        diagnostic_task: Some(diagnostic_task),
    })
}
/// Owns only the pinned binary's finite metadata query, never a model turn.
pub struct VersionProbe {
    identity: kegari_platform::BinaryIdentity,
    stdout: Option<tokio::process::ChildStdout>,
    child: std::sync::Arc<OwnedProviderProcess>,
}
impl VersionProbe {
    pub fn cancellation(&self) -> OwnedProviderCancellation {
        OwnedProviderCancellation {
            child: self.child.clone(),
        }
    }
    pub async fn verify(mut self, deadline: Duration) -> Result<VerifiedCodexExecutable> {
        if deadline.is_zero() {
            return Err(Error::invalid("metadata deadline must be positive"));
        }
        let stdout = self.stdout.take().ok_or_else(|| {
            Error::new("ownership_conflict", "metadata probe was already consumed")
        })?;
        let mut bytes = Vec::new();
        let result = tokio::time::timeout(deadline, async {
            stdout
                .take(4097)
                .read_to_end(&mut bytes)
                .await
                .map_err(|error| Error::io("read Codex version", error))?;
            if bytes.len() > 4096 {
                return Err(Error::new(
                    "protocol_error",
                    "version response exceeds its finite bound",
                ));
            }
            loop {
                if let Some(status) = self.child.child.lock().await.observe_exit()? {
                    self.child.record_exit();
                    break Ok(status);
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .map_err(|_| Error::new("timeout", "Codex version probe timed out"));
        terminate_owned_provider(&self.child).await?;
        let status = result??;
        if !status.success() || String::from_utf8_lossy(&bytes).trim() != SUPPORTED_CODEX_VERSION {
            return Err(Error::new(
                "unsupported_codex_version",
                format!(
                    "required {SUPPORTED_CODEX_VERSION}; install and qualify a matching executable"
                ),
            ));
        }
        self.identity.revalidate(false)?;
        Ok(VerifiedCodexExecutable(self.identity.clone()))
    }
}
impl Drop for VersionProbe {
    fn drop(&mut self) {
        let _ = self.cancellation().request_cancel();
    }
}
/// Same-process capability for one unreaped spawned adapter, never a restored PID.
#[derive(Clone)]
pub struct OwnedProviderCancellation {
    child: std::sync::Arc<OwnedProviderProcess>,
}
struct OwnedProviderProcess {
    child: tokio::sync::Mutex<ProviderChild>,
    exited: std::sync::atomic::AtomicBool,
}
impl OwnedProviderProcess {
    fn new(
        child: Child,
        group: kegari_process::group::OwnedGroup,
        guard: Option<crate::process_records::SpawnGuard>,
    ) -> Self {
        Self {
            child: tokio::sync::Mutex::new(ProviderChild {
                child,
                group,
                guard,
                reaped: None,
            }),
            exited: std::sync::atomic::AtomicBool::new(false),
        }
    }
    fn record_exit(&self) {
        self.exited.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}
impl OwnedProviderCancellation {
    pub fn exited(&self) -> Result<bool> {
        if self.child.exited.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(true);
        }
        let Ok(mut child) = self.child.child.try_lock() else {
            return Ok(false);
        };
        let exited = child.observe_exit()?.is_some();
        if exited {
            self.child.record_exit();
        }
        Ok(exited)
    }
    pub fn request_cancel(&self) -> Result<()> {
        if self.child.exited.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(());
        }
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
            Error::new(
                "prerequisite",
                "owned adapter cancellation needs its running host executor",
            )
        })?;
        let child = self.child.clone();
        runtime.spawn(async move {
            let _ = terminate_owned_provider(&child).await;
        });
        Ok(())
    }
    pub async fn join_termination(&self) -> Result<()> {
        terminate_owned_provider(&self.child).await
    }
}
struct ProviderChild {
    child: Child,
    group: kegari_process::group::OwnedGroup,
    guard: Option<crate::process_records::SpawnGuard>,
    reaped: Option<kegari_process::group::OwnedGroupExit>,
}
impl ProviderChild {
    fn observe_exit(&mut self) -> Result<Option<std::process::ExitStatus>> {
        if self.reaped.is_none() {
            // Retain the original reaping proof before journal persistence. A
            // failed write must retry with this token, not reconstruct an exit.
            self.reaped = self.group.try_reap_tokio(&mut self.child)?;
        }
        let Some(reaped) = &self.reaped else {
            return Ok(None);
        };
        if let Some(guard) = &mut self.guard {
            guard.group_exited(reaped)?;
        }
        self.guard.take();
        Ok(Some(reaped.status()))
    }
}
async fn terminate_owned_provider(process: &std::sync::Arc<OwnedProviderProcess>) -> Result<()> {
    if process.exited.load(std::sync::atomic::Ordering::SeqCst) {
        return Ok(());
    }
    let mut child = process.child.lock().await;
    if child.observe_exit()?.is_some() {
        process.record_exit();
        return Ok(());
    }
    child
        .group
        .terminate(kegari_process::group::Termination::Terminate)?;
    let grace = Instant::now() + Duration::from_secs(5);
    let deadline = grace + Duration::from_secs(5);
    let mut forced = false;
    loop {
        if child.observe_exit()?.is_some() {
            process.record_exit();
            return Ok(());
        }
        if !forced && Instant::now() >= grace {
            child
                .group
                .terminate(kegari_process::group::Termination::Kill)?;
            forced = true;
        }
        if Instant::now() >= deadline {
            return Err(Error::new(
                "operation_uncertain",
                "owned provider exit remains unconfirmed after termination",
            ));
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Explicit native connection/configuration verification, for an operator's
/// setup/reconnect transaction. No login, token refresh, thread, turn or model
/// task is started. Do not invoke this merely to qualify an unattended build.
pub async fn probe_connection(
    executable: &Path,
    auth_home: &Path,
    permissions: &PermissionConfiguration,
    cwd: &Path,
    expected_account: &str,
) -> Result<CodexConnectionMetadata> {
    let mut client = spawn(executable, auth_home, permissions, Duration::from_secs(5)).await?;
    let result = tokio::time::timeout(
        Duration::from_secs(30),
        client
            .client_mut()
            .connection_metadata(expected_account, permissions, cwd),
    )
    .await
    .map_err(|_| {
        Error::new(
            "timeout",
            "Codex metadata verification exceeded its finite deadline",
        )
    });
    let terminated = client.terminate().await;
    let mut evidence = result??;
    terminated?;
    permissions
        .settings
        .as_object()
        .ok_or_else(|| Error::invalid("permission configuration must be an object"))?;
    let mut argv = vec![executable.to_string_lossy().into_owned()];
    argv.extend(kegari_codex::app_server_argv(permissions)?);
    evidence.argument_digest = Some(canonical_digest(
        1,
        &json!({"argv":argv,"auth_home":evidence.auth_home,"cwd":cwd,"protocol":PROTOCOL_PROFILE}),
    )?);
    Ok(evidence)
}

#[cfg(test)]
mod digest_parity_tests {
    use super::*;

    #[test]
    fn backend_digest_syntax_matches_codex_golden_cases() {
        let corpus: serde_json::Value = serde_json::from_str(include_str!(
            "../../kegari-codex/tests/fixtures/digest-syntax.json"
        ))
        .unwrap();
        let expected: Error = serde_json::from_value(corpus["error"].clone()).unwrap();
        for digest in corpus["accepted"].as_array().unwrap() {
            let digest = digest.as_str().unwrap();
            assert_eq!(
                crate::evidence::validate_digest(digest),
                Ok(()),
                "{digest:?}"
            );
        }
        for digest in corpus["rejected"].as_array().unwrap() {
            let digest = digest.as_str().unwrap();
            assert_eq!(
                crate::evidence::validate_digest(digest),
                Err(expected.clone()),
                "{digest:?}"
            );
        }
    }
}
