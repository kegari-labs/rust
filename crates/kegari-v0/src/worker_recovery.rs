//! Guarded, metadata-only recovery of one retained terminal failed thread.
use super::*;
use crate::{
    codex::{RetainedResumeSession, VerifiedThreadRecovery},
    control::{Lease, SessionRecoveryPreparationReference},
    storage::PrivateDir,
    worker::RecoveryJournal,
};

pub struct RecoveryPreparation {
    pub config: Arc<Config>,
    pub contract: Arc<WorkerContract>,
    pub candidate: RetainedResumeSession,
    pub records: PrivateDir,
    pub lease: Lease,
    pub control: Arc<ControlHost>,
    pub host: Arc<dyn WorkerHost>,
    pub processes: Arc<crate::process_records::ProcessJournal>,
    pub executions: Arc<ExecutionRegistry>,
    pub timeout: Duration,
    pub attempt: u32,
    pub max_attempts: u32,
}
/// A successful readback and actual joined metadata process, never a worker
/// continuation permit. Native runtime must also verify its retained original
/// completion, compatible assembly and directory-family ownership.
#[derive(Clone, Debug)]
pub struct VerifiedProviderRecovery {
    observation: VerifiedThreadRecovery,
    preparation: crate::control::Receipt,
    digest: String,
}
impl VerifiedProviderRecovery {
    pub fn observation(&self) -> &VerifiedThreadRecovery {
        &self.observation
    }
    pub fn preparation(&self) -> &crate::control::Receipt {
        &self.preparation
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
}
pub struct RecoveryProviderLaunch {
    input: ProviderLaunchInput,
    candidate: RetainedResumeSession,
    journal: RecoveryJournal,
    gate: Arc<ProtocolGate>,
    cancellation: Arc<Cancellation>,
    registry: Arc<ExecutionRegistry>,
    finished: bool,
}
impl RecoveryProviderLaunch {
    /// Acquires the original journal exclusively before actual finite metadata
    /// admission. Does not create a process, clone, hook, thread or model turn.
    pub fn admit(input: RecoveryPreparation) -> Result<Self> {
        input.config.validate()?;
        let contract = &input.contract;
        let candidate = &input.candidate;
        let reference = candidate.reference();
        let workspace = input
            .config
            .workspaces
            .values()
            .find(|w| w.id == reference.workspace)
            .ok_or_else(|| Error::new("scope_violation", "recovery workspace is not configured"))?;
        let connection = input
            .config
            .connections
            .get(&workspace.codex_connection)
            .ok_or_else(|| Error::new("invalid_config", "recovery Codex connection is absent"))?;
        let revision = match contract.authority() {
            WorkerAuthority::Authorized(g) => g.config_revision(),
            WorkerAuthority::Independent(g) => g.config_revision(),
        };
        let operation = Uuid::parse_str(&candidate.state().binding.operation_id)
            .map_err(|_| Error::new("record_corrupt", "recovery operation is invalid"))?;
        let original = input.control.operation(operation)?;
        if input.config.revision()? != revision
            || connection.kind != ProviderKind::Codex
            || connection.account_id != reference.account
            || contract.account_id() != reference.account
            || contract.instructions().manifest_digest != reference.instruction_digest
            || contract.instructions().manifest.operation_id != operation
            || contract.work_key()? != reference.phase.work
            || input.lease.runtime_id != original.runtime_id
            || canonical_digest(1, &original)? != candidate.admission_digest()
            || input.timeout.is_zero()
            || input.timeout > Duration::from_secs(300)
            || input.timeout > Duration::from_millis(input.config.runtime.request_timeout_ms)
            || input.max_attempts > input.config.runtime.max_attempts_per_phase
            || reference.attempt >= input.config.runtime.max_attempts_per_phase
        {
            return Err(Error::new(
                "resume_incompatible",
                "metadata recovery changed original account, configuration, contract or receipt",
            ));
        }
        let executable = BinaryIdentity::inspect(
            std::path::Path::new(
                connection
                    .argv
                    .first()
                    .ok_or_else(|| Error::new("invalid_config", "recovery executable missing"))?,
            ),
            false,
        )?;
        let auth_home = PathBuf::from(
            connection
                .auth_home
                .as_ref()
                .ok_or_else(|| Error::new("invalid_config", "recovery auth home missing"))?,
        )
        .canonicalize()
        .map_err(|e| Error::io("resolve recovery auth home", e))?;
        let permissions = PermissionConfiguration::for_isolation(contract.isolation(), &auth_home)?;
        let journal = RecoveryJournal::open(
            &input.records,
            &original,
            candidate,
            contract,
            input.host.clone(),
        )?;
        let recovery = SessionRecoveryPreparationReference {
            predecessor: operation,
            predecessor_receipt_digest: candidate.admission_digest().into(),
            candidate_digest: candidate.digest().into(),
            workspace: reference.workspace,
            account: reference.account.clone(),
            phase: reference.phase.clone(),
            attempt: input.attempt,
            max_attempts: input.max_attempts,
        };
        let scope = canonical_digest(
            1,
            &json!({"kind":"session_recovery_metadata","candidate":candidate.digest(),"thread":candidate.state().thread_id,"cwd":candidate.state().binding.cwd,"executable":executable,"auth_home":auth_home,"permissions":permissions,"methods":["initialize","account/read","config/read","thread/read"],"protocol":crate::codex::PROTOCOL_PROFILE}),
        )?;
        let permit = input.control.admit_session_recovery_preparation(
            &input.lease,
            recovery,
            scope,
            |_| {
                input.host.verify_owner(&input.lease)?;
                contract
                    .authority()
                    .compare(&input.host.current_authority(contract.authority())?)?;
                contract.isolation().revalidate()?;
                executable.revalidate(false)
            },
        )?;
        let gate = Arc::new(ProtocolGate {
            control: input.control.clone(),
            permit: permit.clone(),
            host: input.host,
            authority: contract.authority().clone(),
            resume_source: None,
            pending: Mutex::new(BTreeMap::new()),
        });
        Ok(Self {
            input: ProviderLaunchInput {
                control: input.control,
                permit,
                processes: input.processes,
                executable,
                auth_home,
                permissions,
                isolation: contract.isolation().clone(),
                timeout: input.timeout,
            },
            candidate: input.candidate,
            journal,
            gate,
            cancellation: Arc::new(Cancellation {
                requested: AtomicBool::new(false),
                task_finished: AtomicBool::new(false),
                notification: Notify::new(),
                provider: Mutex::new(vec![]),
            }),
            registry: input.executions,
            finished: false,
        })
    }
    pub fn operation_id(&self) -> Uuid {
        self.input.permit.operation_id
    }
    fn fresh(&self) -> Result<()> {
        self.input
            .control
            .validate_permit(&self.input.permit, || self.gate.fresh())
    }
    fn fence(&self) -> Result<()> {
        self.input.control.fence(self.operation_id())?;
        self.registry
            .cancel_fenced(&self.input.control, self.operation_id())?;
        self.cancellation.cancel()
    }
    /// No general RPC client escapes this method. Each outbound read is preceded
    /// by the actual current permit gate; no model/tool endpoint is called.
    pub async fn recover(mut self) -> Result<VerifiedProviderRecovery> {
        self.input.control.validate_permit(&self.input.permit, || {
            self.gate.fresh()?;
            self.registry.register(
                self.operation_id(),
                Uuid::new_v4(),
                self.cancellation.clone(),
            )
        })?;
        let task = FinishTask(self.cancellation.clone());
        let mut client = None;
        let control = self.input.control.clone();
        let permit = self.input.permit.clone();
        let registry = self.registry.clone();
        let cancellation = self.cancellation.clone();
        let deadline = tokio::time::Instant::now() + self.input.timeout;
        let mut core = Box::pin(async {
            client =
                Some(launch_provider(&self.input, &self.cancellation, self.gate.clone()).await?);
            let rpc = client.as_mut().ok_or_else(|| {
                Error::new("ownership_conflict", "recovery provider was not launched")
            })?;
            let control = self.input.control.clone();
            let permit = self.input.permit.clone();
            let gate = self.gate.clone();
            let fresh = || control.validate_permit(&permit, || gate.fresh());
            let proof = rpc
                .recover_metadata(
                    &self.candidate,
                    &self.input.permissions,
                    &mut self.journal,
                    &fresh,
                )
                .await?;
            self.fresh()?;
            if self.journal.observed() {
                return Err(Error::new(
                    "reconciliation_required",
                    "metadata recovery observed additional session events; retained history must be reread",
                ));
            }
            Ok(proof)
        });
        // Declare after both the client and in-flight future: cancellation of
        // this method fences before either owned process destructor can run.
        let mut guard = RecoveryDrop {
            control,
            permit,
            registry,
            cancel: cancellation,
            armed: true,
        };
        let result = tokio::select! {
            result=&mut core => result,
            _=tokio::time::sleep_until(deadline) => {
                guard.fence()?;
                Err(Error::new("timeout", "metadata recovery exceeded its finite deadline"))
            }
        };
        if result.is_err() {
            guard.fence()?;
        }
        drop(core);
        if let Some(rpc) = &mut client {
            rpc.terminate().await?;
        }
        let providers = self
            .cancellation
            .provider
            .lock()
            .map_err(|_| Error::new("ownership_conflict", "recovery cancellation lock failed"))?
            .clone();
        for provider in providers {
            provider.join_termination().await?;
        }
        drop(task);
        if !self.cancellation.exited()? {
            return Err(Error::new(
                "operation_uncertain",
                "metadata recovery process exit is unconfirmed",
            ));
        }
        self.registry.observe_exited(self.operation_id())?;
        if result.is_ok() {
            self.fresh()?;
        }
        let observation = match &result {
            Ok(proof) => {
                json!({"metadata_recovery":"verified","proof_digest":proof.digest(),"local_exit_confirmed":true})
            }
            Err(error) => {
                json!({"metadata_recovery":"failed_no_effect","error":error,"retryable":error.retryable && !matches!(error.code.as_str(),"timeout"|"operation_uncertain"|"provider_intervention"|"reconciliation_required"),"local_exit_confirmed":true})
            }
        };
        let receipt = self
            .input
            .control
            .retire_local(self.operation_id(), observation)?;
        guard.armed = false;
        self.finished = true;
        let proof = result?;
        let digest = canonical_digest(
            1,
            &json!({"observation":proof.digest(),"preparation":receipt,"executable":self.input.executable,"permissions":self.input.permissions,"protocol":crate::codex::PROTOCOL_PROFILE}),
        )?;
        Ok(VerifiedProviderRecovery {
            observation: proof,
            preparation: receipt,
            digest,
        })
    }
}
impl Drop for RecoveryProviderLaunch {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.fence();
        }
    }
}
struct RecoveryDrop {
    control: Arc<ControlHost>,
    permit: Permit,
    registry: Arc<ExecutionRegistry>,
    cancel: Arc<Cancellation>,
    armed: bool,
}
impl RecoveryDrop {
    fn fence(&self) -> Result<()> {
        self.control.fence(self.permit.operation_id)?;
        self.registry
            .cancel_fenced(&self.control, self.permit.operation_id)?;
        self.cancel.cancel()
    }
}
impl Drop for RecoveryDrop {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.fence();
        }
    }
}

/// Same-process resume ingredients, before Control admission or directory
/// enrollment. Runtime must obtain `previous` from its actual joined cache and
/// recheck pointer identity in the final serialized predicate.
pub struct VerifiedSessionResume {
    previous: Arc<RunCompletion>,
    recovery: VerifiedProviderRecovery,
    instructions: instructions::VerifiedResumeInstructions,
    next_manifest: String,
    next_operation: Uuid,
    prompt: String,
    digest: String,
}
impl VerifiedSessionResume {
    pub(crate) fn prepare(
        previous: Arc<RunCompletion>,
        recovery: VerifiedProviderRecovery,
        next: &PreparedRun,
    ) -> Result<Self> {
        let candidate = recovery.observation().retained();
        let old = previous.contract.as_ref().ok_or_else(|| {
            Error::new(
                "resume_incompatible",
                "original complete worker contract is unavailable",
            )
        })?;
        let execution = previous.execution.as_ref().ok_or_else(|| {
            Error::new(
                "resume_incompatible",
                "original owned execution is unavailable",
            )
        })?;
        let manifest = &next.contract.instructions().manifest;
        if !previous.local_exit_confirmed
            || !matches!(previous.outcome, RunOutcome::Failed)
            || previous.hook_error.is_some()
            || previous.state.binding.operation_id != previous.operation_id.to_string()
            || canonical_digest(1, &previous.state)? != canonical_digest(1, candidate.state())?
            || previous
                .error
                .as_ref()
                .is_none_or(|error| error.code != "turn_incomplete")
            || !previous.checks.is_empty()
            || !previous.failed_checks.is_empty()
            || previous.operation_id == manifest.operation_id
            || old.account_id() != next.contract.account_id()
            || execution.binding.operation_id != previous.operation_id
            || next.contract.isolation().digest() != old.isolation().digest()
            || old.tools().digest() != next.contract.tools().digest()
            || old.workflow().front_matter.hooks.before_run.is_some()
            || old.workflow().front_matter.hooks.after_run.is_some()
            || matches!(
                manifest.role,
                AgentRole::Review | AgentRole::IndependentReview
            )
            || candidate.reference().phase.phase != manifest.phase
            || previous.state.advertised_tools
                != next
                    .contract
                    .tools()
                    .definitions()
                    .iter()
                    .map(|tool| tool["name"].as_str().unwrap_or_default().to_owned())
                    .collect()
        {
            return Err(Error::new(
                "resume_incompatible",
                "resume changed original terminal failure, hooks, execution, role or phase",
            ));
        }
        old.authority().compare(next.contract.authority())?;
        execution
            .ownership
            .verify(&execution.binding, old.isolation())?;
        let instructions = old
            .instructions()
            .verify_resume(next.contract.instructions())?;
        let next_manifest = next.contract.instructions().manifest_digest.clone();
        let next_operation = manifest.operation_id;
        let prompt = next.contract.instructions().prompt.clone();
        let digest = canonical_digest(
            1,
            &json!({"predecessor":previous.operation_id,"recovery":recovery.digest(),"instructions":instructions.digest(),"next_operation":next_operation,"next_manifest":next_manifest,"original_execution":execution.binding,"directory":execution.ownership.digest()}),
        )?;
        Ok(Self {
            previous,
            recovery,
            instructions,
            next_manifest,
            next_operation,
            prompt,
            digest,
        })
    }
    pub fn predecessor(&self) -> Uuid {
        self.previous.operation_id
    }
    pub fn next_operation(&self) -> Uuid {
        self.next_operation
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
    pub fn recovery(&self) -> &VerifiedProviderRecovery {
        &self.recovery
    }
    pub fn instructions(&self) -> &instructions::VerifiedResumeInstructions {
        &self.instructions
    }
    pub fn next_manifest(&self) -> &str {
        &self.next_manifest
    }
    pub(crate) fn prompt(&self) -> &str {
        &self.prompt
    }
    pub(crate) fn new_session(
        &self,
        binding: SessionBinding,
        max_turns: u32,
    ) -> Result<SessionState> {
        let old = &self.previous.state;
        let mut normalized = binding.clone();
        normalized.operation_id = old.binding.operation_id.clone();
        normalized.instruction_digest = old.binding.instruction_digest.clone();
        if binding.operation_id != self.next_operation.to_string()
            || binding.instruction_digest != self.next_manifest
            || normalized != old.binding
            || max_turns > self.recovery.observation().retained().reference().max_turns
        {
            return Err(Error::new(
                "resume_incompatible",
                "new session differs from verified original binding and retained budget",
            ));
        }
        let thread = old
            .thread_id
            .as_ref()
            .ok_or_else(|| Error::new("resume_incompatible", "original thread is absent"))?;
        let mut state = SessionState::new(binding, max_turns)?;
        state.thread_id = Some(thread.clone());
        state.advertised_tools = old.advertised_tools.clone();
        state.provider_instruction_sources = old.provider_instruction_sources.clone();
        state.usage = old.usage.resumed_session(&old.binding.account_id, thread);
        state.resume_origin = Some(crate::codex::SessionResumeOrigin {
            predecessor: self.previous.operation_id,
            recovery_digest: self.recovery.digest().into(),
            instruction_compatibility_digest: self.instructions.digest().into(),
            history_digest: self.recovery.observation().history_digest().into(),
            historical_turns: old.completed_turns.keys().cloned().collect(),
            thread: thread.clone(),
            readback_digest: None,
        });
        Ok(state)
    }
    pub(crate) fn verify_session(&self, state: &SessionState) -> Result<()> {
        let expected = self.new_session(state.binding.clone(), state.max_turns)?;
        if canonical_digest(1, &expected)? != canonical_digest(1, state)? {
            return Err(Error::new(
                "resume_incompatible",
                "provider resume must begin from the exact new empty session with original usage baseline",
            ));
        }
        Ok(())
    }
    pub fn verify_completion(&self, actual: &Arc<RunCompletion>) -> Result<()> {
        if !Arc::ptr_eq(actual, &self.previous) {
            return Err(Error::new(
                "resume_incompatible",
                "resume lost its actual joined completion identity",
            ));
        }
        Ok(())
    }
    pub fn verify_contract(&self, current: &WorkerContract) -> Result<()> {
        let old =
            self.previous.contract.as_ref().ok_or_else(|| {
                Error::new("resume_incompatible", "original contract disappeared")
            })?;
        if current.instructions().manifest.operation_id != self.next_operation
            || current.instructions().manifest_digest != self.next_manifest
            || old
                .instructions()
                .verify_resume(current.instructions())?
                .digest()
                != self.instructions.digest()
            || old.tools().digest() != current.tools().digest()
        {
            return Err(Error::new(
                "resume_incompatible",
                "resume assembly changed after semantic verification",
            ));
        }
        old.authority().compare(current.authority())
    }
}

/// Opaque model-launch mode obtained only after actual typed Control admission
/// and immutable directory-family enrollment. A raw prior-session JSON value
/// cannot select this mode.
pub struct AdmittedSessionResume {
    bridge: Arc<VerifiedSessionResume>,
    enrollment: crate::housekeeping::EnrolledDirectoryReuse,
}
impl VerifiedSessionResume {
    pub(crate) fn admitted(
        self: Arc<Self>,
        permit: &Permit,
        control: &ControlHost,
        enrollment: crate::housekeeping::EnrolledDirectoryReuse,
    ) -> Result<AdmittedSessionResume> {
        let result = AdmittedSessionResume {
            bridge: self,
            enrollment,
        };
        result.verify_admission(permit, control)?;
        Ok(result)
    }
}
impl AdmittedSessionResume {
    pub(crate) fn startup_source(&self) -> ResumeStartupSource {
        ResumeStartupSource::new(self.enrollment.clone())
    }
    fn verify_admission(&self, permit: &Permit, control: &ControlHost) -> Result<()> {
        self.enrollment.revalidate()?;
        let receipt = control.operation(permit.operation_id)?;
        let reference =
            crate::control::RunResumeReference::from_receipt(&receipt)?.ok_or_else(|| {
                Error::new(
                    "resume_incompatible",
                    "resumed provider requires an actual typed run admission",
                )
            })?;
        let old =
            self.bridge.previous.execution.as_ref().ok_or_else(|| {
                Error::new("resume_incompatible", "original execution disappeared")
            })?;
        let mut binding = old.binding.clone();
        binding.operation_id = permit.operation_id;
        if permit.operation_id != self.bridge.next_operation
            || self.enrollment.operation_id() != permit.operation_id
            || self.enrollment.binding() != &binding
            || reference.predecessor() != self.bridge.predecessor()
            || reference.predecessor_receipt_digest
                != self
                    .bridge
                    .recovery
                    .observation()
                    .retained()
                    .admission_digest()
            || reference.recovery_digest() != self.bridge.recovery.digest()
            || reference.instruction_compatibility_digest != self.bridge.instructions.digest()
        {
            return Err(Error::new(
                "resume_incompatible",
                "resumed provider admission differs from its metadata, semantic or enrolled directory proof",
            ));
        }
        Ok(())
    }
    pub(crate) fn verify(
        &self,
        permit: &Permit,
        control: &ControlHost,
        contract: &WorkerContract,
        execution: Option<&Arc<WorkerExecution>>,
    ) -> Result<()> {
        self.verify_admission(permit, control)?;
        self.bridge.verify_contract(contract)?;
        let execution = execution.ok_or_else(|| {
            Error::new(
                "resume_incompatible",
                "resumed worker lacks its actual enrolled execution",
            )
        })?;
        if &execution.binding != self.enrollment.binding()
            || execution.ownership.digest()
                != self
                    .bridge
                    .previous
                    .execution
                    .as_ref()
                    .unwrap()
                    .ownership
                    .digest()
        {
            return Err(Error::new(
                "resume_incompatible",
                "resumed execution differs from its original owned directory",
            ));
        }
        execution
            .ownership
            .verify(&execution.binding, contract.isolation())
    }
    pub(crate) fn bridge(&self) -> &VerifiedSessionResume {
        &self.bridge
    }
    pub(crate) fn new_session(
        &self,
        binding: SessionBinding,
        max_turns: u32,
    ) -> Result<SessionState> {
        self.bridge.new_session(binding, max_turns)
    }
    pub fn prior_record(&self) -> Value {
        json!({"schema_version":1,"predecessor":self.bridge.predecessor(),"resume":self.bridge.digest(),"recovery":self.bridge.recovery.digest(),"instructions":self.bridge.instructions.digest(),"enrollment":self.enrollment.digest(),"operation":self.bridge.next_operation})
    }
}
