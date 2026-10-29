//! Explicit workflow hooks run inside the pinned Codex app-server sandbox.
//! The ledger records workflow outcomes, not ownership of Codex-internal PIDs.
use super::*;
use crate::execution::{
    ExecutionBinding, ExecutionEvidenceClass, HookLedger, HookPhase, HookRecord, HookState,
    SessionLifecycle,
};
use kegari_digest::bytes_digest;
use kegari_storage::PrivateDir;

fn ledger(records: &PrivateDir, binding: &ExecutionBinding) -> Result<HookLedger> {
    let ledger: HookLedger = records.read("hooks.json")?;
    if ledger.schema_version != 2 || ledger.binding_digest != canonical_digest(1, binding)? {
        return Err(Error::new(
            "resume_incompatible",
            "legacy or foreign lifecycle records cannot become Codex hook evidence",
        ));
    }
    Ok(ledger)
}
fn key(phase: HookPhase, session: Option<Uuid>) -> String {
    format!(
        "{phase:?}:{}",
        session.map_or_else(|| "directory".into(), |id| id.to_string())
    )
}
fn script<'a>(workflow: &'a Workflow, phase: HookPhase) -> Option<&'a str> {
    let hooks = &workflow.front_matter.hooks;
    match phase {
        HookPhase::AfterCreate => hooks.after_create.as_deref(),
        HookPhase::BeforeRun => hooks.before_run.as_deref(),
        HookPhase::AfterRun => hooks.after_run.as_deref(),
        HookPhase::BeforeRemove => hooks.before_remove.as_deref(),
    }
}
fn output_observation(value: &str) -> String {
    // Hook output can contain credentials outside the model's known-secret set.
    // Retain an exact digest/length observation rather than leaking raw diagnostics.
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
fn begin_hook(
    records: &PrivateDir,
    binding: &ExecutionBinding,
    workflow: &Workflow,
    phase: HookPhase,
    session: Option<Uuid>,
) -> Result<bool> {
    let mut current = ledger(records, binding)?;
    let id = key(phase, session);
    if current.records.contains_key(&id)
        || (phase == HookPhase::AfterCreate) == current.initialized
        || (phase == HookPhase::AfterCreate) != session.is_none()
        || phase == HookPhase::AfterRun
            && current.sessions.get(&session.unwrap_or_default())
                != Some(&SessionLifecycle::Launched)
        || current
            .records
            .values()
            .any(|record| record.state == HookState::StartedUnknown)
    {
        return Err(Error::new(
            "hook_gate",
            "lifecycle attempt is repeated, unknown, or out of order",
        ));
    }
    let command = script(workflow, phase);
    current.records.insert(
        id,
        HookRecord {
            phase,
            session,
            workflow_digest: workflow.digest.clone(),
            script_digest: command.map(|s| bytes_digest(s.as_bytes())),
            cleanup_plan: None,
            state: if command.is_some() {
                HookState::StartedUnknown
            } else {
                HookState::Skipped
            },
            exit_code: None,
            timed_out: false,
            stdout: String::new(),
            stderr: String::new(),
            error_code: None,
            evidence_class: ExecutionEvidenceClass::CodexExecution,
        },
    );
    if phase == HookPhase::AfterCreate && command.is_none() {
        current.initialized = true;
    }
    records.write("hooks.json", &current)?;
    Ok(command.is_some())
}
async fn finish_hook(
    rpc: &mut RpcClient,
    permissions: &PermissionConfiguration,
    scope: &ValidatedIsolation,
    records: &PrivateDir,
    binding: &ExecutionBinding,
    workflow: &Workflow,
    phase: HookPhase,
    session: Option<Uuid>,
    output_limit: usize,
) -> Result<()> {
    let process_id = format!("kegari:{}:{}", binding.operation_id, key(phase, session));
    let command = vec![
        "/bin/sh".into(),
        "-c".into(),
        script(workflow, phase)
            .ok_or_else(|| Error::new("hook_gate", "started lifecycle command disappeared"))?
            .into(),
    ];
    let result = rpc
        .execute_command(
            &crate::codex_host::IsolationSessionScope::new(scope),
            permissions,
            &command,
            &process_id,
            Duration::from_millis(workflow.front_matter.hooks.timeout_ms),
            output_limit.min(1024 * 1024),
        )
        .await;
    record_hook_result(records, binding, phase, session, result)
}
fn record_hook_result(
    records: &PrivateDir,
    binding: &ExecutionBinding,
    phase: HookPhase,
    session: Option<Uuid>,
    result: Result<kegari_codex::CommandExecResult>,
) -> Result<()> {
    let mut current = ledger(records, binding)?;
    let record = current
        .records
        .get_mut(&key(phase, session))
        .filter(|record| record.state == HookState::StartedUnknown)
        .ok_or_else(|| {
            Error::new(
                "hook_gate",
                "lifecycle result lost its original pending record",
            )
        })?;
    match &result {
        Ok(result) => {
            record.exit_code = Some(result.exit_code);
            record.stdout = output_observation(&result.stdout);
            record.stderr = output_observation(&result.stderr);
            record.state = if result.exit_code == 0 {
                HookState::Succeeded
            } else {
                HookState::Failed
            };
            if result.exit_code != 0 {
                record.error_code = Some("hook_failed".into());
            }
            if phase == HookPhase::AfterCreate && result.exit_code == 0 {
                current.initialized = true;
            }
        }
        Err(error) => {
            // No late response or cancellation acknowledgement converts this
            // retained attempt into successful execution or permits replay.
            record.timed_out = error.code == "timeout";
            record.error_code = Some(error.code.clone());
        }
    }
    records.write("hooks.json", &current)?;
    match result {
        Ok(result) if result.exit_code == 0 => Ok(()),
        Ok(result) => Err(Error::new(
            "hook_failed",
            "Codex lifecycle command exited unsuccessfully",
        )
        .detail(json!({"phase":phase,"exit_code":result.exit_code,"session":session}))),
        Err(error) => Err(error),
    }
}

/// Uses the preparation permit and actual owned directory once, before context
/// assembly. No model turn or custom command subprocess is launched by Kegari.
pub(crate) async fn prepare_creation_hooks(
    input: &Preparation,
    runtime: &Arc<crate::runtime::RuntimeCoordinator>,
    permit: &Permit,
    processes: Arc<crate::process_records::ProcessJournal>,
    execution: &WorkerExecution,
    records: &PrivateDir,
) -> Result<()> {
    if !input.role.permits_repository_execution() || input.repository_id.is_none() {
        return Ok(());
    }
    let scope = input.isolation.validate(crate::BUILD_ID)?;
    let _ledger_lock = records.lock("hooks.lock")?;
    runtime.control.validate_permit(permit, || {
        runtime.verify_owner(&permit.lease)?;
        crate::worker::effective_authority(
            runtime.as_ref(),
            permit.operation_id,
            &input.authority,
        )?;
        match records.read::<HookLedger>("hooks.json") {
            Err(error) if error.code == "record_missing" => {}
            Err(error) => return Err(error),
            Ok(_) => {
                return Err(Error::new(
                    "hook_gate",
                    "directory creation lifecycle already exists",
                ));
            }
        }
        records.write("hooks.json", &HookLedger::new(&execution.binding)?)
    })?;
    runtime
        .services
        .housekeeping
        .register_hook_ledger_location(execution.binding.operation_id, records)?;
    if !begin_hook(
        records,
        &execution.binding,
        &input.workflow,
        HookPhase::AfterCreate,
        None,
    )? {
        return Ok(());
    }
    let workspace = input
        .config
        .workspaces
        .values()
        .find(|workspace| {
            workspace.id
                == match &input.authority {
                    WorkerAuthority::Authorized(authority) => authority.workspace_id(),
                    WorkerAuthority::Independent(authority) => authority.workspace_id(),
                }
        })
        .ok_or_else(|| Error::new("scope_violation", "creation hook workspace is absent"))?;
    let connection = input
        .config
        .connections
        .get(&workspace.codex_connection)
        .ok_or_else(|| Error::new("invalid_config", "creation hook Codex connection is absent"))?;
    if connection.kind != ProviderKind::Codex
        || connection.argv.len() != 4
        || connection.argv[1..] != ["app-server", "--listen", "stdio://"]
    {
        return Err(Error::new(
            "unsupported_profile",
            "creation hook requires the pinned Codex stdio connection",
        ));
    }
    let auth_home = PathBuf::from(
        connection
            .auth_home
            .as_deref()
            .ok_or_else(|| Error::invalid("Codex auth_home is missing"))?,
    );
    PrivateDir::open(&auth_home)?;
    let permissions = crate::codex_host::permission_configuration(&scope, &auth_home)?;
    let gate = Arc::new(ProtocolGate {
        control: runtime.control.clone(),
        permit: permit.clone(),
        host: runtime.clone(),
        authority: input.authority.clone(),
        resume_source: None,
        pending: Mutex::new(BTreeMap::new()),
    });
    let cancellation = Arc::new(Cancellation {
        requested: AtomicBool::new(false),
        task_finished: AtomicBool::new(false),
        notification: Notify::new(),
        provider: Mutex::new(vec![]),
    });
    runtime.control.validate_permit(permit, || {
        gate.fresh()?;
        runtime
            .executions
            .register(permit.operation_id, Uuid::new_v4(), cancellation.clone())
    })?;
    let finished = FinishTask(cancellation.clone());
    let launch = ProviderLaunchInput {
        control: runtime.control.clone(),
        permit: permit.clone(),
        processes,
        executable: resolve_codex(&connection.argv[0])?,
        auth_home,
        permissions: permissions.clone(),
        isolation: scope.clone(),
        timeout: Duration::from_millis(input.config.runtime.request_timeout_ms),
    };
    let mut client = match launch_provider(&launch, &cancellation, gate).await {
        Ok(client) => client,
        Err(error) => {
            runtime.control.fence(permit.operation_id)?;
            cancellation.cancel()?;
            return Err(error);
        }
    };
    let result = async {
        let rpc = client.client_mut();
        rpc.initialize().await?;
        rpc.verify_account(&connection.account_id).await?;
        rpc.verify_execution_configuration(&permissions, &scope.plan().cwd)
            .await?;
        finish_hook(
            rpc,
            &permissions,
            &scope,
            records,
            &execution.binding,
            &input.workflow,
            HookPhase::AfterCreate,
            None,
            execution.limits.output_bytes,
        )
        .await
    }
    .await;
    if result.is_err() {
        runtime.control.fence(permit.operation_id)?;
    }
    let _ = client.client_mut().terminate_command().await;
    let terminated = client.terminate().await;
    drop(finished);
    runtime.executions.observe_exited(permit.operation_id)?;
    terminated?;
    result
}

pub struct CodexRunHooks {
    admission: NonExecutingRunHooks,
    storage: Option<(PrivateDir, ExecutionBinding, usize)>,
    _ledger_lock: Option<kegari_storage::ExclusiveLock>,
    access: tokio::sync::Mutex<()>,
}
impl CodexRunHooks {
    pub fn new(
        contract: Arc<WorkerContract>,
        control: Arc<ControlHost>,
        host: Arc<dyn WorkerHost>,
        execution: Option<&WorkerExecution>,
        records: PrivateDir,
    ) -> Result<Self> {
        let mut ledger_lock = None;
        let storage = if contract.is_unborn_bootstrap() && execution.is_none() {
            let hooks = &contract.workflow().front_matter.hooks;
            if hooks.after_create.is_some()
                || hooks.before_run.is_some()
                || hooks.after_run.is_some()
                || hooks.before_remove.is_some()
            {
                return Err(Error::new(
                    "baseline_absent",
                    "configured lifecycle hooks require a committed repository baseline",
                ));
            }
            None
        } else if contract
            .instructions()
            .manifest
            .permissions
            .repository_code_execution
        {
            let execution = execution.ok_or_else(|| {
                Error::new(
                    "hook_gate",
                    "executable lifecycle lacks its owned directory",
                )
            })?;
            if execution.binding.operation_id != contract.instructions().manifest.operation_id
                || execution.binding.isolation_digest != contract.isolation().digest()
            {
                return Err(Error::new(
                    "scope_violation",
                    "hook lifecycle differs from its admitted owned directory",
                ));
            }
            for grant in std::iter::once(&contract.isolation().plan().repository)
                .chain(std::iter::once(&contract.isolation().plan().scratch))
                .chain(
                    contract
                        .isolation()
                        .plan()
                        .read_roots
                        .iter()
                        .map(|grant| &grant.path),
                )
            {
                if records.path().starts_with(grant) || grant.starts_with(records.path()) {
                    return Err(Error::new(
                        "scope_violation",
                        "hook ledger overlaps worker access",
                    ));
                }
            }
            if !contract
                .isolation()
                .plan()
                .forbidden_roots
                .iter()
                .any(|root| records.path().starts_with(root))
            {
                return Err(Error::new(
                    "scope_violation",
                    "hook ledger needs an explicit forbidden root",
                ));
            }
            ledger_lock = Some(records.lock("hooks.lock")?);
            let current = ledger(&records, &execution.binding)?;
            if !current.initialized {
                return Err(Error::new(
                    "hook_gate",
                    "directory creation did not succeed",
                ));
            }
            Some((
                records,
                execution.binding.clone(),
                execution.limits.output_bytes,
            ))
        } else {
            None
        };
        Ok(Self {
            admission: NonExecutingRunHooks {
                contract,
                control,
                host,
            },
            storage,
            _ledger_lock: ledger_lock,
            access: tokio::sync::Mutex::new(()),
        })
    }
}
impl RunHooks for CodexRunHooks {
    fn before<'a>(&'a self, permit: &'a Permit) -> RunFuture<'a, ()> {
        Box::pin(async move {
            let _access = self.access.lock().await;
            self.admission.verify(permit)?;
            if let Some((records, binding, _)) = &self.storage
                && self
                    .admission
                    .contract
                    .workflow()
                    .front_matter
                    .hooks
                    .before_run
                    .is_none()
            {
                begin_hook(
                    records,
                    binding,
                    self.admission.contract.workflow(),
                    HookPhase::BeforeRun,
                    Some(permit.operation_id),
                )?;
            }
            Ok(())
        })
    }
    fn codex_before<'a>(
        &'a self,
        permit: &'a Permit,
        rpc: &'a mut RpcClient,
        permissions: &'a PermissionConfiguration,
    ) -> RunFuture<'a, ()> {
        Box::pin(async move {
            let _access = self.access.lock().await;
            self.admission.verify(permit)?;
            let Some((records, binding, limit)) = &self.storage else {
                return Ok(());
            };
            let contract = &self.admission.contract;
            if contract.workflow().front_matter.hooks.before_run.is_none() {
                return Ok(());
            }
            if begin_hook(
                records,
                binding,
                contract.workflow(),
                HookPhase::BeforeRun,
                Some(permit.operation_id),
            )? {
                finish_hook(
                    rpc,
                    permissions,
                    contract.isolation(),
                    records,
                    binding,
                    contract.workflow(),
                    HookPhase::BeforeRun,
                    Some(permit.operation_id),
                    *limit,
                )
                .await?;
            }
            Ok(())
        })
    }
    fn launched<'a>(&'a self, permit: &'a Permit) -> RunFuture<'a, ()> {
        Box::pin(async move {
            let _access = self.access.lock().await;
            self.admission.verify(permit)?;
            if let Some((records, binding, _)) = &self.storage {
                let mut current = ledger(records, binding)?;
                current.mark_launched(permit.operation_id, &mut |next| {
                    records.write("hooks.json", next)
                })?;
            }
            Ok(())
        })
    }
    fn codex_after<'a>(
        &'a self,
        permit: &'a Permit,
        state: &'a SessionState,
        rpc: &'a mut RpcClient,
        permissions: &'a PermissionConfiguration,
    ) -> RunFuture<'a, ()> {
        Box::pin(async move {
            let _access = self.access.lock().await;
            self.admission.verify(permit)?;
            verify_after_observation(
                &self.admission.contract,
                &self.admission.control,
                permit,
                state,
            )?;
            let Some((records, binding, limit)) = &self.storage else {
                return Ok(());
            };
            let contract = &self.admission.contract;
            if begin_hook(
                records,
                binding,
                contract.workflow(),
                HookPhase::AfterRun,
                Some(permit.operation_id),
            )? {
                let result = finish_hook(
                    rpc,
                    permissions,
                    contract.isolation(),
                    records,
                    binding,
                    contract.workflow(),
                    HookPhase::AfterRun,
                    Some(permit.operation_id),
                    *limit,
                )
                .await;
                if result.is_err() {
                    self.admission.control.fence(permit.operation_id)?;
                }
                result?;
            }
            Ok(())
        })
    }
    fn after<'a>(
        &'a self,
        permit: &'a Permit,
        state: &'a SessionState,
    ) -> RunFuture<'a, AfterHookObservation> {
        Box::pin(async move {
            let _access = self.access.lock().await;
            verify_after_observation(
                &self.admission.contract,
                &self.admission.control,
                permit,
                state,
            )?;
            let Some((records, binding, _)) = &self.storage else {
                return Ok(if self.admission.contract.is_unborn_bootstrap() {
                    AfterHookObservation::Skipped {
                        reason: AfterHookSkip::HookNotConfigured,
                    }
                } else {
                    AfterHookObservation::NotApplicable
                });
            };
            let mut current = ledger(records, binding)?;
            if current.sessions.get(&permit.operation_id) != Some(&SessionLifecycle::Launched) {
                return Ok(AfterHookObservation::Skipped {
                    reason: AfterHookSkip::SessionNotLaunched,
                });
            }
            if self
                .admission
                .contract
                .workflow()
                .front_matter
                .hooks
                .after_run
                .is_none()
                && !current
                    .records
                    .contains_key(&key(HookPhase::AfterRun, Some(permit.operation_id)))
            {
                begin_hook(
                    records,
                    binding,
                    self.admission.contract.workflow(),
                    HookPhase::AfterRun,
                    Some(permit.operation_id),
                )?;
                current = ledger(records, binding)?;
            }
            let observation = match current
                .records
                .get(&key(HookPhase::AfterRun, Some(permit.operation_id)))
            {
                Some(record) if record.state == HookState::Succeeded => {
                    Ok(AfterHookObservation::Completed)
                }
                Some(record) if record.state == HookState::Skipped => {
                    Ok(AfterHookObservation::Skipped {
                        reason: AfterHookSkip::HookNotConfigured,
                    })
                }
                None if self
                    .admission
                    .contract
                    .workflow()
                    .front_matter
                    .hooks
                    .after_run
                    .is_none() =>
                {
                    Ok(AfterHookObservation::Skipped {
                        reason: AfterHookSkip::HookNotConfigured,
                    })
                }
                _ => Err(Error::new(
                    "hook_unresolved",
                    "after-run lifecycle command failed, was interrupted, or has no actual Codex result",
                )),
            };
            current.mark_ended(permit.operation_id, &mut |next| {
                records.write("hooks.json", next)
            })?;
            observation
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_creation_failure_is_durable_and_never_replayed_or_legacy_qualified() {
        let directory = tempfile::tempdir().unwrap();
        let records =
            PrivateDir::create(&directory.path().canonicalize().unwrap().join("records")).unwrap();
        let id = Uuid::from_u128(7);
        let digest = bytes_digest(b"hook fixture");
        let binding = ExecutionBinding {
            runtime_id: Uuid::from_u128(1),
            operation_id: id,
            work: crate::evidence::WorkIdentity {
                linear_endpoint: "https://api.linear.app/graphql".into(),
                linear_workspace: Uuid::from_u128(2),
                parent: Uuid::from_u128(3),
                child: Some(Uuid::from_u128(4)),
                generation: crate::evidence::Generation {
                    authorization: digest.clone(),
                    definition: digest.clone(),
                    correction: digest.clone(),
                    scope: digest.clone(),
                },
            },
            repository: kegari_git::RepositoryId {
                hostname: "github.com".into(),
                id: "repository".into(),
            },
            revision: kegari_git::Revision {
                commit: "1".repeat(40),
                tree: "2".repeat(40),
            },
            candidate_vector_digest: None,
            isolation_digest: digest,
        };
        let workflow = Workflow::parse(
            "---\nhooks:\n  after_create: exit 7\n---\nDo the work",
            "/fixture/WORKFLOW.md",
        )
        .unwrap();
        records
            .write("hooks.json", &HookLedger::new(&binding).unwrap())
            .unwrap();
        assert!(begin_hook(&records, &binding, &workflow, HookPhase::AfterCreate, None).unwrap());
        let failed = record_hook_result(
            &records,
            &binding,
            HookPhase::AfterCreate,
            None,
            Ok(kegari_codex::CommandExecResult {
                exit_code: 7,
                stdout: "secret-fixture-value".into(),
                stderr: "failure detail".into(),
            }),
        )
        .unwrap_err();
        assert_eq!(failed.code, "hook_failed");
        let observed = ledger(&records, &binding).unwrap();
        assert_eq!(observed.schema_version, 2);
        assert!(!observed.initialized);
        let record = &observed.records["AfterCreate:directory"];
        assert_eq!(record.state, HookState::Failed);
        assert_eq!(record.exit_code, Some(7));
        assert_eq!(
            record.evidence_class,
            ExecutionEvidenceClass::CodexExecution
        );
        assert!(!record.stdout.contains("secret-fixture-value"));
        assert!(
            record
                .stdout
                .contains(&bytes_digest(b"secret-fixture-value"))
        );
        assert_eq!(
            begin_hook(&records, &binding, &workflow, HookPhase::AfterCreate, None)
                .unwrap_err()
                .code,
            "hook_gate"
        );
        let mut legacy = serde_json::to_value(&observed).unwrap();
        legacy.as_object_mut().unwrap().remove("schema_version");
        records.write("hooks.json", &legacy).unwrap();
        assert_eq!(
            ledger(&records, &binding).unwrap_err().code,
            "resume_incompatible"
        );
    }
}
