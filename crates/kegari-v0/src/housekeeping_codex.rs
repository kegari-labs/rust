//! The explicit cleanup hook uses Codex's bounded command interface. Kegari owns
//! the adapter process and cleanup decision, not the repository command process.
use super::*;
use crate::codex_host::OwnedProviderCancellation;
use crate::execution::{
    CommandCancellation, ExecutionEvidenceClass, HookPhase, HookRecord, HookState,
};
use crate::host_process::LocalProcessPermit;
use kegari_codex::{PermissionConfiguration, RequestGate};
use kegari_config::{Config, Connection, ProviderKind};
use serde_json::Value;

struct PreparedHook {
    connection: Connection,
    executable: BinaryIdentity,
    home: PathBuf,
    permissions: PermissionConfiguration,
}
fn prepare(
    config: &Config,
    binding: &ExecutionBinding,
    isolation: &kegari_execution::ValidatedIsolation,
) -> Result<PreparedHook> {
    binding.validate(isolation)?;
    if !matches!(
        isolation.plan().role,
        kegari_execution::ExecutionRole::Child | kegari_execution::ExecutionRole::Validation
    ) {
        return Err(Error::new(
            "scope_violation",
            "cleanup hooks require a repository-execution role",
        ));
    }
    let matches = config
        .workspaces
        .values()
        .filter(|workspace| {
            workspace.linear_workspace_id == binding.work.linear_workspace
                && workspace
                    .repositories
                    .get(&binding.repository.id)
                    .is_some_and(|repository| repository.hostname == binding.repository.hostname)
                && config
                    .connections
                    .get(&workspace.linear_connection)
                    .is_some_and(|connection| {
                        connection
                            .endpoint
                            .as_deref()
                            .unwrap_or("https://api.linear.app/graphql")
                            == binding.work.linear_endpoint
                    })
        })
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(Error::new(
            "scope_violation",
            "cleanup source does not resolve to one current configured workspace",
        ));
    }
    let connection = config
        .connections
        .get(&matches[0].codex_connection)
        .ok_or_else(|| Error::invalid("cleanup Codex connection is missing"))?
        .clone();
    if connection.kind != ProviderKind::Codex
        || connection.argv.len() != 4
        || connection.argv[1..] != ["app-server", "--listen", "stdio://"]
    {
        return Err(Error::new(
            "unsupported_profile",
            "cleanup requires the pinned Codex stdio connection",
        ));
    }
    let home = PathBuf::from(
        connection
            .auth_home
            .as_ref()
            .ok_or_else(|| Error::invalid("Codex auth home is missing"))?,
    );
    PrivateDir::open(&home)?;
    let permissions = crate::codex_host::permission_configuration(isolation, &home)?;
    let executable = BinaryIdentity::inspect(
        &crate::configuration::resolve_executable(&connection.argv[0])?,
        false,
    )?;
    Ok(PreparedHook {
        connection,
        executable,
        home,
        permissions,
    })
}

pub(super) fn preflight(
    services: &crate::host_process::HostProcessServices,
    binding: &ExecutionBinding,
    isolation: &IsolationPlan,
) -> Result<()> {
    prepare(
        &services.configuration.snapshot()?,
        binding,
        &isolation.validate(crate::BUILD_ID)?,
    )?;
    Ok(())
}
struct ProviderCancellation(OwnedProviderCancellation);
impl CommandCancellation for ProviderCancellation {
    fn cancel(&self) -> Result<()> {
        self.0.request_cancel()
    }
    fn exited(&self) -> Result<bool> {
        self.0.exited()
    }
}
struct HookGate(Arc<LocalProcessPermit>);
struct FenceBeforeDrop<'a> {
    services: &'a crate::host_process::HostProcessServices,
    proof: &'a VerifiedCleanupExecution,
    armed: bool,
}
impl FenceBeforeDrop<'_> {
    fn fence(&mut self) -> Result<()> {
        if self.armed {
            self.services
                .control
                .fence_host_process(&self.proof.token())?;
            // Request supported command termination before joining the adapter.
            // On future drop the client drops next and cancels its owned child.
            self.armed = false;
        }
        Ok(())
    }
}
impl Drop for FenceBeforeDrop<'_> {
    fn drop(&mut self) {
        let _ = self.fence();
    }
}
impl RequestGate for HookGate {
    fn dispatch(
        &self,
        _id: u64,
        method: &str,
        frame: &Value,
        transmit: &mut dyn FnMut() -> Result<()>,
    ) -> Result<()> {
        self.0.dispatch_cleanup_command(method, frame, transmit)
    }
    fn observed(&self, _id: u64, _method: &str, _result: &Result<Value>) -> Result<()> {
        Ok(())
    }
    fn uncertain(&self, _id: u64, _method: &str) -> Result<()> {
        Ok(())
    }
}
fn hook_record(request: &CleanupHook<'_>, state: HookState) -> Result<HookRecord> {
    Ok(HookRecord {
        phase: HookPhase::BeforeRemove,
        session: None,
        workflow_digest: request.workflow.digest.clone(),
        script_digest: request
            .workflow
            .front_matter
            .hooks
            .before_remove
            .as_ref()
            .map(|s| bytes_digest(s.as_bytes())),
        cleanup_plan: Some(request.proof.plan().digest().into()),
        state,
        exit_code: None,
        timed_out: false,
        stdout: String::new(),
        stderr: String::new(),
        error_code: None,
        evidence_class: ExecutionEvidenceClass::CodexExecution,
    })
}
fn read_ledger(request: &CleanupHook<'_>) -> Result<HookLedger> {
    let ledger: HookLedger = request.directory.read(&request.ledger_name)?;
    if ledger.schema_version != 2 || ledger.binding_digest != canonical_digest(1, request.binding)?
    {
        return Err(Error::new(
            "resume_incompatible",
            "cleanup requires its original Codex hook ledger",
        ));
    }
    Ok(ledger)
}
fn output(value: &str) -> String {
    if value.is_empty() {
        String::new()
    } else {
        format!(
            "[Codex lifecycle output withheld: {} bytes; {}]",
            value.len(),
            bytes_digest(value.as_bytes())
        )
    }
}

pub(super) async fn run(
    services: &crate::host_process::HostProcessServices,
    request: CleanupHook<'_>,
) -> Result<()> {
    request.proof.validate_current()?;
    request
        .ownership
        .verify(request.binding, request.isolation)?;
    let configuration = services.configuration.snapshot()?;
    let prepared = prepare(&configuration, request.binding, request.isolation)?;
    let timeout = Duration::from_millis(request.workflow.front_matter.hooks.timeout_ms);
    if timeout.is_zero() || timeout > Duration::from_secs(86_400) {
        return Err(Error::invalid(
            "cleanup hook timeout exceeds the Codex command bound",
        ));
    }
    let script = request
        .workflow
        .front_matter
        .hooks
        .before_remove
        .as_ref()
        .ok_or_else(|| Error::new("hook_gate", "cleanup hook disappeared"))?;
    let command = vec!["/bin/sh".to_string(), "-c".into(), script.clone()];
    let process_id = format!("kegari:cleanup:{}", request.proof.receipt().operation_id);
    let mut environment = prepared
        .permissions
        .settings
        .pointer("/shell_environment_policy/set")
        .and_then(Value::as_object)
        .cloned()
        .ok_or_else(|| {
            Error::new(
                "isolation_unverified",
                "cleanup command environment is absent",
            )
        })?;
    environment.insert("CODEX_HOME".into(), Value::Null);
    let expected = json!({"command":command,"processId":process_id,"cwd":request.isolation.plan().cwd,
        "permissionProfile":prepared.permissions.profile_id,"timeoutMs":timeout.as_millis() as u64,
        "outputBytesCap":1_048_576,"disableTimeout":false,"disableOutputCap":false,
        "tty":false,"streamStdin":false,"streamStdoutStderr":false,"env":environment});
    let authority = LocalProcessPermit::cleanup_codex(
        services.clone(),
        request.proof.clone(),
        request.root.clone(),
        prepared.executable.clone(),
        &kegari_codex::app_server_argv(&prepared.permissions)?,
        expected,
    )?;
    let _ledger_lock = request.directory.lock("hooks.lock")?;
    let key = format!("BeforeRemove:{}", request.proof.plan().digest());
    request.proof.apply_effect(|| {
        let mut ledger = read_ledger(&request)?;
        if ledger.records.contains_key(&key) {
            return Err(Error::new(
                "hook_gate",
                "cleanup hook attempt already exists; do not replay",
            ));
        }
        ledger.records.insert(
            key.clone(),
            hook_record(&request, HookState::StartedUnknown)?,
        );
        request.directory.write(&request.ledger_name, &ledger)
    })?;
    let mut owned = Vec::<OwnedProviderCancellation>::new();
    let result = async {
        let probe = authority.start_cleanup_codex(true, |guard| {
            let started =
                crate::codex_host::start_version_probe(&prepared.executable, Some(guard))?;
            let cancellation = started.cancellation();
            owned.push(cancellation.clone());
            Ok((started, Arc::new(ProviderCancellation(cancellation))))
        })?;
        let verified = probe
            .verify(Duration::from_millis(
                configuration.runtime.request_timeout_ms,
            ))
            .await?;
        let mut client = authority.start_cleanup_codex(false, |guard| {
            let started = crate::codex_host::start_verified_guarded(
                &verified,
                &prepared.home,
                &prepared.permissions,
                Duration::from_millis(configuration.runtime.request_timeout_ms),
                guard,
            )?;
            let cancellation = started.cancellation();
            owned.push(cancellation.clone());
            Ok((started, Arc::new(ProviderCancellation(cancellation))))
        })?;
        // Declared after the client so cancellation fences first on future drop.
        let mut fence = FenceBeforeDrop {
            services,
            proof: &request.proof,
            armed: true,
        };
        let result = async {
            client
                .client_mut()
                .bind_request_gate(Arc::new(HookGate(authority.clone())))?;
            let rpc = client.client_mut();
            rpc.initialize().await?;
            rpc.verify_account(&prepared.connection.account_id).await?;
            rpc.verify_execution_configuration(
                &prepared.permissions,
                &request.isolation.plan().cwd,
            )
            .await?;
            rpc.execute_command(
                &crate::codex_host::IsolationSessionScope::new(request.isolation),
                &prepared.permissions,
                &command,
                &process_id,
                timeout,
                1_048_576,
            )
            .await
        }
        .await;
        if result.is_err() {
            fence.fence()?;
        }
        let _ = client.client_mut().terminate_command().await;
        let joined = client.terminate().await;
        joined?;
        fence.armed = false;
        result
    }
    .await;
    // Errors never authorize removal. Join actual adapter handles before a hook
    // success can be used; opaque Codex-internal commands remain provider facts.
    let mut joined = Ok(());
    for process in &owned {
        if let Err(error) = process.join_termination().await {
            joined = Err(error);
        }
    }
    if joined.is_ok() {
        joined=authority.finish(json!({"cleanup_hook":"before_remove","success":result.as_ref().is_ok_and(|r|r.exit_code==0)})).map(|_|());
    }
    let result = joined.and(result);
    let mut ledger = read_ledger(&request)?;
    let record = ledger
        .records
        .get_mut(&key)
        .ok_or_else(|| Error::new("hook_gate", "cleanup hook record disappeared"))?;
    match &result {
        Ok(value) => {
            record.exit_code = Some(value.exit_code);
            record.stdout = output(&value.stdout);
            record.stderr = output(&value.stderr);
            record.state = if value.exit_code == 0 {
                HookState::Succeeded
            } else {
                HookState::Failed
            };
            if value.exit_code != 0 {
                record.error_code = Some("hook_failed".into());
            }
        }
        Err(error) => {
            record.error_code = Some(error.code.clone());
            record.timed_out = error.code == "timeout";
        }
    }
    request.directory.write(&request.ledger_name, &ledger)?;
    match result {
        Ok(value) if value.exit_code == 0 => Ok(()),
        Ok(_) => Err(Error::new(
            "hook_failed",
            "before_remove exited unsuccessfully; directory preserved",
        )),
        Err(error) => Err(error),
    }
}
