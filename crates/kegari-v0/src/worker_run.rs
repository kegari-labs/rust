//! Actual assembled worker sessions. The host chooses work and reserves the
//! immutable contract; this module owns the bounded adapter/task lifetime.
use crate::{
    codex::{
        CodexProfile, OwnedProviderCancellation, PermissionConfiguration, RequestGate, RpcClient,
        SessionBinding, SessionState,
    },
    control::{Actor, ControlHost, InFlightEffect, MutationRequest, Permit},
    evaluation::TrialHost,
    execution::{CommandCancellation, VerifiedCheckExecution},
    instructions::{
        self, AgentRole, AssemblyInput, PermissionManifest, ProviderDiscovery, RepositoryContext,
        WorkerNetwork,
    },
    isolation::{IsolationPlan, ValidatedIsolation},
    platform::BinaryIdentity,
    worker::{
        ContractInput, ExecutionRegistry, ReadResource, ReviewTarget, ScopedWorker,
        WorkerAuthority, WorkerContract, WorkerExecution, WorkerHost, WorkerServices,
    },
    workflow::Workflow,
};
use kegari_config::{Config, ProviderKind, Speed};
use kegari_digest::canonical_digest;
use kegari_types::error::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    future::Future,
    path::PathBuf,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::Notify;
use uuid::Uuid;
#[path = "worker_recovery.rs"]
mod recovery;
pub use recovery::{
    AdmittedSessionResume, RecoveryPreparation, RecoveryProviderLaunch, VerifiedProviderRecovery,
    VerifiedSessionResume,
};

#[path = "worker_run_evaluation_return.rs"]
mod evaluation_return;
pub(crate) use evaluation_return::{
    OwnedTrialReturn, RetainedTrialCompletion, VerifiedUnusedAdmission, returned_admission_digest,
};

#[derive(Clone)]
pub struct Preparation {
    pub config: Arc<Config>,
    pub workflow: Workflow,
    pub authority: WorkerAuthority,
    pub operation_id: Uuid,
    pub role: AgentRole,
    pub phase: String,
    pub context: Value,
    pub isolation: IsolationPlan,
    pub repository: Option<RepositoryContext>,
    pub repository_id: Option<crate::git::RepositoryId>,
    pub read_resources: BTreeMap<PathBuf, ReadResource>,
    pub review: Option<ReviewTarget>,
    pub required_context: Vec<PathBuf>,
    pub project_doc_max_bytes: usize,
    pub provider_discovery: ProviderDiscovery,
    pub known_secrets: Vec<String>,
}
pub struct PreparedRun {
    contract: Arc<WorkerContract>,
    isolation: ValidatedIsolation,
    permissions: PermissionConfiguration,
    executable: BinaryIdentity,
    auth_home: PathBuf,
    runtime: kegari_config::RuntimeConfig,
}
impl PreparedRun {
    pub fn contract(&self) -> &Arc<WorkerContract> {
        &self.contract
    }
    pub fn isolation(&self) -> &ValidatedIsolation {
        &self.isolation
    }
    pub fn permissions(&self) -> &PermissionConfiguration {
        &self.permissions
    }
}
/// Preparation reads exact current files and shipped assets; it starts no model,
/// hook, test or provider flow and grants no operation permit.
pub fn prepare(input: Preparation) -> Result<PreparedRun> {
    input.config.validate()?;
    let (workspace_id, revision, profile, generation, correction) = match &input.authority {
        WorkerAuthority::Authorized(g) => (
            g.workspace_id(),
            g.config_revision().to_owned(),
            g.profile().clone(),
            g.work().digest()?,
            g.work().generation.correction.clone(),
        ),
        WorkerAuthority::Independent(g) => (
            g.workspace_id(),
            g.config_revision().to_owned(),
            g.profile().clone(),
            g.review_identity().to_owned(),
            "independent-review".into(),
        ),
    };
    if crate::config::config_revision(&input.config)? != revision || input.operation_id.is_nil() {
        return Err(Error::new(
            "stale_config",
            "worker preparation differs from admitted configuration",
        ));
    }
    let workspace = input
        .config
        .workspaces
        .values()
        .find(|w| w.id == workspace_id)
        .ok_or_else(|| Error::new("scope_violation", "worker workspace is not configured"))?;
    if !crate::config::load_workflow(workspace)?.same_source(&input.workflow) {
        return Err(Error::new(
            "stale_config",
            "workflow changed before worker assembly",
        ));
    }
    let connection = input
        .config
        .connections
        .get(&workspace.codex_connection)
        .ok_or_else(|| Error::new("invalid_config", "workspace Codex connection is missing"))?;
    for (path, resource) in &input.read_resources {
        let configured = match resource {
            ReadResource::Repository { id } => workspace
                .repositories
                .get(id)
                .map(|repository| repository.source.as_str()),
            ReadResource::TestResource {
                name,
                writable,
                disposable,
            } => workspace
                .test_resources
                .get(name)
                .filter(|resource| {
                    resource.writable == *writable && resource.disposable == *disposable
                })
                .map(|resource| resource.path.as_str()),
        }
        .ok_or_else(|| {
            Error::new(
                "scope_violation",
                "worker resource differs from current configured access",
            )
        })?;
        if PathBuf::from(configured)
            .canonicalize()
            .map_err(|error| Error::io("resolve configured worker resource", error))?
            != *path
        {
            return Err(Error::new(
                "scope_violation",
                "worker resource aliases a different configured path",
            ));
        }
    }
    if connection.kind != ProviderKind::Codex
        || connection.argv.len() != 4
        || connection.argv[1..] != ["app-server", "--listen", "stdio://"]
    {
        return Err(Error::new(
            "unsupported_profile",
            "worker requires the pinned Codex stdio connection",
        ));
    }
    let executable = resolve_codex(&connection.argv[0])?;
    let auth_home = PathBuf::from(
        connection
            .auth_home
            .as_deref()
            .ok_or_else(|| Error::invalid("Codex auth_home is missing"))?,
    );
    crate::storage::PrivateDir::open(&auth_home)?;
    let isolation = input.isolation.validate()?;
    let permissions = PermissionConfiguration::for_isolation(&isolation, &auth_home)?;
    let repository_execution =
        input.role.permits_repository_execution() && input.repository_id.is_some();
    let definitions = crate::worker::operation_tool_definitions(
        input.role,
        !isolation.plan().source_writes.is_empty(),
        repository_execution,
    );
    let assembled = instructions::assemble(AssemblyInput {
        workflow: &input.workflow,
        role: input.role,
        phase: &input.phase,
        context: &input.context,
        workspace_id,
        operation_id: input.operation_id,
        work_generation: &generation,
        correction_revision: &correction,
        profile: &profile,
        cwd: &isolation.plan().cwd,
        codex_home: &auth_home,
        codex_version: instructions::SUPPORTED_CODEX_VERSION,
        repository: input.repository,
        required_context: &input.required_context,
        project_doc_max_bytes: input.project_doc_max_bytes,
        permissions: PermissionManifest {
            read_roots: isolation
                .plan()
                .read_roots
                .iter()
                .map(|g| g.path.clone())
                .collect(),
            resource_write_paths: isolation
                .plan()
                .resource_writes
                .iter()
                .map(|grant| grant.path.clone())
                .collect(),
            source_write_paths: isolation
                .plan()
                .source_writes
                .iter()
                .map(|g| g.path.clone())
                .collect(),
            scratch_roots: vec![isolation.plan().scratch.clone()],
            repository_code_execution: repository_execution,
            enforcement_profile: isolation.digest().into(),
            network: WorkerNetwork::BrokerOnly,
        },
        tools: definitions
            .into_iter()
            .map(|(exposed, _)| exposed)
            .collect(),
        provider_discovery: input.provider_discovery,
        known_secrets: &input.known_secrets,
    })?;
    let contract = Arc::new(WorkerContract::new_configured(
        ContractInput {
            assembled,
            authority: input.authority,
            isolation: isolation.clone(),
            workflow: input.workflow,
            context: input.context,
            review: input.review,
            account_id: connection.account_id.clone(),
            repository: input.repository_id,
            read_resources: input.read_resources,
        },
        workspace,
    )?);
    Ok(PreparedRun {
        contract,
        isolation,
        permissions,
        executable,
        auth_home,
        runtime: input.config.runtime.clone(),
    })
}
fn resolve_codex(value: &str) -> Result<BinaryIdentity> {
    let path = PathBuf::from(value);
    let path = if path.is_absolute() {
        path
    } else if path.components().count() == 1 {
        std::env::var_os("PATH")
            .and_then(|paths| {
                std::env::split_paths(&paths)
                    .map(|root| root.join(value))
                    .find(|p| p.is_file())
            })
            .ok_or_else(|| {
                Error::new(
                    "prerequisite",
                    "configured Codex executable is absent from the host PATH",
                )
            })?
    } else {
        return Err(Error::new(
            "invalid_config",
            "Codex executable must be absolute or a bare configured program",
        ));
    };
    BinaryIdentity::inspect(&path, false)
}

pub type RunFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;
/// A required host lifecycle boundary; the concrete executor implementation runs
/// the configured hooks with the same scope, receipt and cancellation gates.
pub trait RunHooks: Send + Sync {
    fn before<'a>(&'a self, permit: &'a Permit) -> RunFuture<'a, ()>;
    fn launched<'a>(&'a self, permit: &'a Permit) -> RunFuture<'a, ()>;
    fn after<'a>(
        &'a self,
        permit: &'a Permit,
        session: &'a SessionState,
    ) -> RunFuture<'a, AfterHookObservation>;
}
/// Actual lifecycle observation, never a fabricated successful command result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum AfterHookObservation {
    NotApplicable,
    Skipped { reason: AfterHookSkip },
    Completed,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AfterHookSkip {
    HookNotConfigured,
    SessionNotLaunched,
}
pub struct RunServices {
    pub processes: Arc<crate::process_records::ProcessJournal>,
    pub worker: WorkerServices,
    pub hooks: Arc<dyn RunHooks>,
}
pub struct RunOptions {
    pub initial_input: String,
    pub max_seconds: u64,
    pub max_turns: u32,
    pub resume: Option<AdmittedSessionResume>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunOutcome {
    Completed,
    Blocked,
    Failed,
    Interrupted,
}
pub struct RunCompletion {
    pub operation_id: Uuid,
    pub state: SessionState,
    pub outcome: RunOutcome,
    pub error: Option<Error>,
    pub elapsed_ms: u64,
    pub hook_error: Option<Error>,
    pub checks: Vec<VerifiedCheckExecution>,
    pub failed_checks: Vec<crate::execution::FailedCheckExecution>,
    /// Current host-enrolled plan, including revisions with no executed checks.
    pub validation_plan: Option<crate::validation::ValidationPlan>,
    /// All completed discovered attempts, including failures that cannot grant repair.
    pub validation_attempts: Vec<crate::execution::CheckExecutionRecord>,
    /// Original retained byte capabilities, including failed and superseded attempts.
    pub validation_artifacts: Vec<crate::execution::RetainedArtifact>,
    /// Missing or unsettled attempt evidence blocks delivery without erasing the run.
    pub validation_error: Option<Error>,
    /// Original worker inventory seal; public descriptive vectors cannot erase attempts.
    pub validation_history: Option<crate::worker::ValidationHistorySeal>,
    /// Original host-owned checkout capability, retained for separately admitted source sealing.
    pub execution: Option<Arc<WorkerExecution>>,
    pub contract: Option<Arc<WorkerContract>>,
    /// Actual owned adapter and registered command handles have confirmed exit.
    /// False keeps local ownership held independently of remote-effect recovery.
    pub local_exit_confirmed: bool,
}
impl RunCompletion {
    pub(crate) fn validation_artifact_reader(&self) -> Result<ValidationArtifactReader<'_>> {
        let seal = self.validation_history.as_ref().ok_or_else(|| {
            Error::new(
                "validation_history_binding",
                "original validation history seal is missing",
            )
        })?;
        seal.verify(
            self.operation_id,
            self.validation_plan.as_ref(),
            &self.validation_attempts,
            self.validation_error.as_ref(),
        )?;
        seal.verify_artifacts(&self.validation_artifacts)?;
        let mut retained = std::collections::BTreeMap::new();
        for artifact in &self.validation_artifacts {
            if retained
                .insert(artifact.proof_digest()?, artifact)
                .is_some()
            {
                return Err(Error::new(
                    "artifact_conflict",
                    "duplicate original retained artifact capability",
                ));
            }
        }
        let mut artifacts = std::collections::BTreeMap::new();
        for proof in self
            .validation_attempts
            .iter()
            .flat_map(|attempt| &attempt.artifacts)
        {
            let digest = kegari_digest::canonical_digest(1, proof)?;
            let original = retained
                .get(&digest)
                .filter(|artifact| artifact.matches_reference(proof))
                .ok_or_else(|| {
                    Error::new(
                        "artifact_conflict",
                        "original retained artifact bytes are unavailable",
                    )
                })?;
            artifacts.insert(digest, (proof, *original));
        }
        Ok(ValidationArtifactReader { artifacts })
    }
    pub fn observation(&self) -> Value {
        json!({"operation_id":self.operation_id,"outcome":self.outcome,"error":self.error,"hook_error":self.hook_error,"session":self.state,"elapsed_ms":self.elapsed_ms,"verified_check_count":self.checks.len(),"failed_check_count":self.failed_checks.len(),"validation_plan_digest":self.validation_plan.as_ref().map(|plan|plan.digest()),"validation_attempt_count":self.validation_attempts.len(),"validation_error":self.validation_error,"local_exit_confirmed":self.local_exit_confirmed,"completion":"worker_claim_requires_host_delivery_verification"})
    }
}
/// Borrowed access to original retained bytes after one complete membership
/// verification. This is not an execution, validation, or readiness capability.
pub(crate) struct ValidationArtifactReader<'a> {
    artifacts: std::collections::BTreeMap<
        String,
        (
            &'a crate::execution::ArtifactProof,
            &'a crate::execution::RetainedArtifact,
        ),
    >,
}
impl ValidationArtifactReader<'_> {
    pub(crate) fn read(&self, expected: &crate::execution::ArtifactProof) -> Result<Vec<u8>> {
        let digest = kegari_digest::canonical_digest(1, expected)?;
        let (proof, original) = self
            .artifacts
            .get(&digest)
            .filter(|(proof, _)| *proof == expected)
            .ok_or_else(|| {
                Error::new(
                    "artifact_conflict",
                    "artifact is not a member of the original validation attempt history",
                )
            })?;
        original.read_verified(proof)
    }
}
struct Cancellation {
    requested: AtomicBool,
    task_finished: AtomicBool,
    notification: Notify,
    provider: Mutex<Vec<OwnedProviderCancellation>>,
}
struct FinishTask(Arc<Cancellation>);
impl Drop for FinishTask {
    fn drop(&mut self) {
        self.0.task_finished.store(true, Ordering::SeqCst);
    }
}
impl Cancellation {
    fn before_start(&self) -> Result<()> {
        if self.requested.load(Ordering::SeqCst) || self.task_finished.load(Ordering::SeqCst) {
            return Err(Error::new(
                "admission_closed",
                "owned provider scope is cancelled or closed",
            ));
        }
        let owned = self.provider.lock().map_err(|_| {
            Error::new(
                "ownership_conflict",
                "provider cancellation registry failed",
            )
        })?;
        if owned.len() >= 2
            || owned
                .iter()
                .any(|prior| !matches!(prior.exited(), Ok(true)))
        {
            return Err(Error::new(
                "ownership_conflict",
                "prior owned provider has not exited",
            ));
        }
        Ok(())
    }
    fn install(&self, provider: OwnedProviderCancellation) -> Result<()> {
        let mut owned = self.provider.lock().map_err(|_| {
            Error::new(
                "ownership_conflict",
                "provider cancellation registry failed",
            )
        })?;
        if owned.len() >= 2
            || owned
                .iter()
                .any(|prior| !matches!(prior.exited(), Ok(true)))
        {
            return Err(Error::new(
                "ownership_conflict",
                "prior owned provider probe is not confirmed exited",
            ));
        }
        if self.requested.load(Ordering::SeqCst) {
            provider.request_cancel()?;
        }
        owned.push(provider);
        Ok(())
    }
    async fn notified(&self) {
        loop {
            let next = self.notification.notified();
            if self.requested.load(Ordering::SeqCst) {
                return;
            }
            next.await;
        }
    }
}
impl CommandCancellation for Cancellation {
    fn exited(&self) -> Result<bool> {
        if !self.task_finished.load(Ordering::SeqCst) {
            return Ok(false);
        }
        let owned = self.provider.lock().map_err(|_| {
            Error::new(
                "ownership_conflict",
                "provider cancellation registry failed",
            )
        })?;
        for provider in owned.iter() {
            if !provider.exited()? {
                return Ok(false);
            }
        }
        Ok(true)
    }
    fn cancel(&self) -> Result<()> {
        self.requested.store(true, Ordering::SeqCst);
        self.notification.notify_waiters();
        let owned = self.provider.lock().map_err(|_| {
            Error::new(
                "ownership_conflict",
                "provider cancellation registry failed",
            )
        })?;
        let mut first_error = None;
        for provider in owned.iter() {
            if let Err(error) = provider.request_cancel() {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}
pub struct OwnedWorkerRun {
    operation: Uuid,
    registry: Arc<ExecutionRegistry>,
    join: Option<tokio::task::JoinHandle<Result<RunCompletion>>>,
}
impl OwnedWorkerRun {
    pub fn operation_id(&self) -> Uuid {
        self.operation
    }
    pub fn is_finished(&self) -> bool {
        self.join.as_ref().is_none_or(|join| join.is_finished())
    }
    pub fn cancel_fenced(&self, control: &ControlHost) -> Result<usize> {
        self.registry.cancel_fenced(control, self.operation)
    }
    pub async fn join(&mut self) -> Result<RunCompletion> {
        self.join
            .take()
            .ok_or_else(|| Error::new("ownership_conflict", "worker run was already joined"))?
            .await
            .map_err(|_| {
                Error::new(
                    "operation_uncertain",
                    "owned worker task exited without a complete receipt; reconcile retained state",
                )
            })?
    }
}

struct ProtocolGate {
    control: Arc<ControlHost>,
    permit: Permit,
    host: Arc<dyn WorkerHost>,
    authority: WorkerAuthority,
    resume_source: Option<ResumeStartupSource>,
    pending: Mutex<BTreeMap<u64, PendingProtocol>>,
}
/// A verified source remains immutable through resume and the first transmitted
/// model turn. Later legitimate worker writes do not retain this startup check.
pub(crate) struct ResumeStartupSource {
    enrollment: crate::housekeeping::EnrolledDirectoryReuse,
    first_turn_sent: AtomicBool,
}
impl ResumeStartupSource {
    pub(crate) fn new(enrollment: crate::housekeeping::EnrolledDirectoryReuse) -> Self {
        Self {
            enrollment,
            first_turn_sent: AtomicBool::new(false),
        }
    }
    fn before_request(&self, permit: &Permit, method: &str) -> Result<()> {
        if self.enrollment.operation_id() != permit.operation_id
            || matches!(method, "thread/start" | "review/start")
            || method == "thread/resume" && self.first_turn_sent.load(Ordering::SeqCst)
        {
            return Err(Error::new(
                "resume_incompatible",
                "resumed source differs from its admitted startup operation",
            ));
        }
        if matches!(method, "thread/resume" | "turn/start")
            && !self.first_turn_sent.load(Ordering::SeqCst)
        {
            self.enrollment.revalidate()?;
        }
        Ok(())
    }
    fn transmitted(&self, method: &str) {
        if method == "turn/start" {
            self.first_turn_sent.store(true, Ordering::SeqCst);
        }
    }
}

/// Shared by the real RequestGate and source/Control component fixtures. The
/// source check and actual frame write run inside the same serialized effect.
pub(crate) fn dispatch_protocol_effect(
    control: &ControlHost,
    permit: &Permit,
    request: MutationRequest,
    source: Option<&ResumeStartupSource>,
    fresh: impl FnOnce() -> Result<()>,
    transmit: &mut dyn FnMut() -> Result<()>,
) -> Result<InFlightEffect> {
    let method = request
        .kind
        .strip_prefix("codex/")
        .filter(|method| request.payload["method"].as_str() == Some(*method))
        .ok_or_else(|| {
            Error::new(
                "protocol_error",
                "provider effect method differs from its frame",
            )
        })?
        .to_owned();
    let (effect, ()) = control.begin_protocol_effect(
        request,
        permit,
        || {
            fresh()?;
            if let Some(source) = source {
                source.before_request(permit, &method)?;
            }
            Ok(())
        },
        |_| {
            transmit()?;
            if let Some(source) = source {
                source.transmitted(&method);
            }
            Ok(())
        },
    )?;
    Ok(effect)
}
struct PendingProtocol {
    method: String,
    effect: InFlightEffect,
}
impl ProtocolGate {
    fn fresh(&self) -> Result<()> {
        self.host.verify_owner(&self.permit.lease)?;
        crate::worker::effective_authority(
            self.host.as_ref(),
            self.permit.operation_id,
            &self.authority,
        )
        .map(|_| ())
    }
}
impl RequestGate for ProtocolGate {
    fn dispatch(
        &self,
        id: u64,
        method: &str,
        frame: &Value,
        transmit: &mut dyn FnMut() -> Result<()>,
    ) -> Result<()> {
        let request = MutationRequest {
            request_id: Uuid::new_v4(),
            runtime_id: self.permit.lease.runtime_id,
            actor: Actor {
                identity: format!("owner:{}", self.permit.lease.owner_id),
                principal: None,
                roles: Default::default(),
            },
            kind: format!("codex/{method}"),
            canonical_targets: vec![format!("codex-session:{}", self.permit.operation_id)],
            expected: BTreeMap::from([("rpc_id".into(), id.to_string())]),
            payload: frame.clone(),
        };
        let effect = dispatch_protocol_effect(
            &self.control,
            &self.permit,
            request,
            self.resume_source.as_ref(),
            || self.fresh(),
            transmit,
        )?;
        self.pending
            .lock()
            .map_err(|_| Error::new("ownership_conflict", "provider effect registry failed"))?
            .insert(
                id,
                PendingProtocol {
                    method: method.into(),
                    effect,
                },
            );
        Ok(())
    }
    fn observed(&self, id: u64, method: &str, result: &Result<Value>) -> Result<()> {
        let effect = self
            .pending
            .lock()
            .map_err(|_| Error::new("ownership_conflict", "provider effect registry failed"))?
            .remove(&id)
            .ok_or_else(|| {
                Error::new(
                    "ownership_conflict",
                    "provider response lacks its dispatched effect",
                )
            })?;
        if effect.method != method {
            return Err(Error::new(
                "ownership_conflict",
                "provider acknowledgement method differs from dispatched request",
            ));
        }
        let digest = match result {
            Ok(value) => canonical_digest(1, value)?,
            Err(error) => canonical_digest(1, error)?,
        };
        self.control.complete_effect(effect.effect,&self.permit,||self.host.verify_owner(&self.permit.lease),json!({"method":method,"rpc_id":id,"provider_acknowledged":result.is_ok(),"response_digest":digest}))?;
        Ok(())
    }
    fn uncertain(&self, _: u64, _: &str) -> Result<()> {
        self.control.fence(self.permit.operation_id)
    }
}

/// Sealed provider launch from an actual scoped worker. It retains its own
/// registry claim until `finish` observes the owned probe/provider handles.
pub struct WorkerProviderLaunch {
    input: ProviderLaunchInput,
    cancellation: Arc<Cancellation>,
    gate: Arc<ProtocolGate>,
    registry: Arc<ExecutionRegistry>,
    launched: AtomicBool,
    manifest_digest: String,
    max_turns: u32,
}
pub(crate) struct ProviderLaunchInput {
    pub control: Arc<ControlHost>,
    pub permit: Permit,
    pub processes: Arc<crate::process_records::ProcessJournal>,
    pub executable: BinaryIdentity,
    pub auth_home: PathBuf,
    pub permissions: PermissionConfiguration,
    pub isolation: ValidatedIsolation,
    pub timeout: Duration,
}
impl WorkerProviderLaunch {
    pub(crate) fn new(
        input: ProviderLaunchInput,
        contract: &WorkerContract,
        host: Arc<dyn WorkerHost>,
        registry: Arc<ExecutionRegistry>,
    ) -> Result<Self> {
        let expected =
            PermissionConfiguration::for_isolation(contract.isolation(), &input.auth_home)?;
        let manifest = &contract.instructions().manifest;
        let admitted = crate::control::RunReference::from_receipt(
            &input.control.operation(input.permit.operation_id)?,
        )?
        .ok_or_else(|| {
            Error::new(
                "admission_budget",
                "provider launch requires actual finite run admission",
            )
        })?;
        if input.timeout.is_zero()
            || input.timeout > Duration::from_secs(300)
            || input.permit.scope_digest != contract.scope_digest()
            || input.permit.work_key != contract.work_key()?
            || input.isolation.digest() != contract.isolation().digest()
            || canonical_digest(1, &input.permissions)? != canonical_digest(1, &expected)?
            || admitted.instruction_digest != contract.instructions().manifest_digest
            || admitted.workspace != manifest.workspace_id
            || admitted.account != contract.account_id()
            || admitted.profile_digest != canonical_digest(1, &manifest.profile)?
        {
            return Err(Error::new(
                "worker_binding",
                "provider launch differs from actual admitted contract",
            ));
        }
        let cancellation = Arc::new(Cancellation {
            requested: AtomicBool::new(false),
            task_finished: AtomicBool::new(false),
            notification: Notify::new(),
            provider: Mutex::new(vec![]),
        });
        let gate = Arc::new(ProtocolGate {
            control: input.control.clone(),
            permit: input.permit.clone(),
            host,
            authority: contract.authority().clone(),
            resume_source: None,
            pending: Mutex::new(BTreeMap::new()),
        });
        input
            .control
            .validate_permit(&input.permit, || gate.fresh())?;
        Ok(Self {
            input,
            cancellation,
            gate,
            registry,
            launched: AtomicBool::new(false),
            manifest_digest: contract.instructions().manifest_digest.clone(),
            max_turns: admitted.max_turns,
        })
    }
    pub fn manifest_digest(&self) -> &str {
        &self.manifest_digest
    }
    pub fn operation_id(&self) -> Uuid {
        self.input.permit.operation_id
    }
    pub fn max_turns(&self) -> u32 {
        self.max_turns
    }
    pub fn verify_binding(
        &self,
        work: &str,
        assembled: &instructions::AssembledInstructions,
        isolation: &ValidatedIsolation,
        permissions: &PermissionConfiguration,
        executable: &std::path::Path,
        auth_home: &std::path::Path,
    ) -> Result<()> {
        self.input.executable.revalidate(false)?;
        if work != self.input.permit.work_key
            || assembled.manifest_digest != self.manifest_digest
            || assembled.manifest.operation_id != self.operation_id()
            || isolation.digest() != self.input.isolation.digest()
            || canonical_digest(1, permissions)? != canonical_digest(1, &self.input.permissions)?
            || executable
                .canonicalize()
                .map_err(|e| Error::io("verify provider executable", e))?
                != self.input.executable.path
            || auth_home
                .canonicalize()
                .map_err(|e| Error::io("verify provider auth home", e))?
                != self.input.auth_home
        {
            return Err(Error::new(
                "worker_binding",
                "trial differs from its original scoped provider launch",
            ));
        }
        self.input
            .control
            .validate_permit(&self.input.permit, || self.gate.fresh())
    }
    pub async fn launch(&self) -> Result<RpcClient> {
        if self.launched.swap(true, Ordering::SeqCst) {
            return Err(Error::new(
                "operation_uncertain",
                "owned provider launch is single-use",
            ));
        }
        self.input.control.validate_permit(&self.input.permit, || {
            self.gate.fresh()?;
            self.cancellation.before_start()?;
            self.registry.register(
                self.input.permit.operation_id,
                Uuid::new_v4(),
                self.cancellation.clone(),
            )
        })?;
        let result = launch_provider(&self.input, &self.cancellation, self.gate.clone()).await;
        if result.is_err() {
            self.input.control.fence(self.input.permit.operation_id)?;
            self.registry
                .cancel_fenced(&self.input.control, self.input.permit.operation_id)?;
        }
        result
    }
    pub fn cancel(&self) -> Result<()> {
        self.input.control.fence(self.input.permit.operation_id)?;
        self.registry
            .cancel_fenced(&self.input.control, self.input.permit.operation_id)
            .map(|_| ())
    }
    /// Call after terminating/joining the returned RpcClient. This never turns a
    /// missing handle, open process or unknown descendant into confirmed exit.
    pub fn finish(&self) -> Result<()> {
        self.input.control.serialize_observation(|_| {
            self.cancellation
                .task_finished
                .store(true, Ordering::SeqCst);
            Ok(())
        })?;
        if !self.cancellation.exited()? {
            return Err(Error::new(
                "operation_uncertain",
                "owned provider process has not exited",
            ));
        }
        self.registry
            .observe_exited(self.input.permit.operation_id)?;
        Ok(())
    }
}
async fn launch_provider(
    input: &ProviderLaunchInput,
    cancel: &Arc<Cancellation>,
    gate: Arc<ProtocolGate>,
) -> Result<RpcClient> {
    let control = &input.control;
    let permit = &input.permit;
    let processes = &input.processes;
    let mut client = None;
    let mut probe = None;
    control.validate_permit(permit, || {
        gate.fresh()?;
        cancel.before_start()?;
        let guard = processes.begin(crate::process_records::SpawnBinding {
            operation_id: permit.operation_id,
            effect_id: Uuid::new_v4(),
            lease: Some(permit.lease.clone()),
            control_owner: None,
            check_bundle: None,
            work_key: permit.work_key.clone(),
            scope_digest: permit.scope_digest.clone(),
            cwd: input.isolation.plan().cwd.clone(),
            executable: input.executable.clone(),
            observed_executables: vec![],
            argv_digest: canonical_digest(1, &vec!["--version"])?,
            kind: "codex-version".into(),
        })?;
        let spawned = RpcClient::start_version_probe(&input.executable, Some(guard))?;
        cancel.install(spawned.cancellation())?;
        probe = Some(spawned);
        Ok(())
    })?;
    let verified = probe
        .ok_or_else(|| Error::new("ownership_conflict", "version probe gate did not run"))?
        .verify(input.timeout)
        .await?;
    control.validate_permit(permit, || {
        gate.fresh()?;
        cancel.before_start()?;
        let guard = processes.begin(crate::process_records::SpawnBinding {
            operation_id: permit.operation_id,
            effect_id: Uuid::new_v4(),
            lease: Some(permit.lease.clone()),
            control_owner: None,
            check_bundle: None,
            work_key: permit.work_key.clone(),
            scope_digest: permit.scope_digest.clone(),
            cwd: input.isolation.plan().cwd.clone(),
            executable: input.executable.clone(),
            observed_executables: vec![],
            argv_digest: canonical_digest(1, &RpcClient::app_server_argv(&input.permissions)?)?,
            kind: "codex-app-server".into(),
        })?;
        let spawned = RpcClient::start_verified_guarded(
            &verified,
            &input.auth_home,
            &input.permissions,
            input.timeout,
            guard,
        )?;
        cancel.install(spawned.cancellation().ok_or_else(|| {
            Error::new(
                "ownership_conflict",
                "native provider process lacks its owned cancellation handle",
            )
        })?)?;
        client = Some(spawned);
        Ok(())
    })?;
    let mut rpc = client
        .ok_or_else(|| Error::new("ownership_conflict", "provider spawn gate did not run"))?;
    rpc.bind_request_gate(gate)?;
    Ok(rpc)
}

/// The immutable, already admitted production hook lifecycle. Only a real
/// bound session can create it; evaluation callers cannot supply no-op hooks.
pub struct TrialLifecycle {
    permit: Permit,
    hooks: Arc<dyn RunHooks>,
    before_started: AtomicBool,
    before_finished: AtomicBool,
    after_started: AtomicBool,
    launched: AtomicBool,
    after_finished: AtomicBool,
    after_observation: Mutex<Option<Result<AfterHookObservation>>>,
    terminal: Mutex<Option<crate::evaluation::TrialExecutionObservation>>,
    after_bound: Duration,
    max_seconds: u64,
}
impl TrialLifecycle {
    pub fn operation_id(&self) -> Uuid {
        self.permit.operation_id
    }
    pub fn max_seconds(&self) -> u64 {
        self.max_seconds
    }
    pub(crate) fn execution_started(&self) -> bool {
        self.before_started.load(Ordering::SeqCst)
    }
    pub(crate) fn after_finished(&self) -> bool {
        self.after_finished.load(Ordering::SeqCst)
    }
    pub(crate) fn after_bound(&self) -> Duration {
        self.after_bound
    }
    pub(crate) fn after_result(&self) -> Result<Result<AfterHookObservation>> {
        self.after_observation
            .lock()
            .map_err(|_| Error::new("ownership_conflict", "trial hook observation lock failed"))?
            .clone()
            .ok_or_else(|| {
                Error::new(
                    "reconciliation_required",
                    "trial has no actual after-hook observation",
                )
            })
    }
    pub(crate) fn retain_execution(
        &self,
        observation: crate::evaluation::TrialExecutionObservation,
    ) -> Result<()> {
        if !self.execution_started()
            || !self.after_finished()
            || observation.state().binding.operation_id != self.permit.operation_id.to_string()
        {
            return Err(Error::new(
                "worker_binding",
                "trial execution lacks its actual lifecycle",
            ));
        }
        let mut terminal = self
            .terminal
            .lock()
            .map_err(|_| Error::new("ownership_conflict", "trial terminal lock failed"))?;
        if terminal.is_some() {
            return Err(Error::new(
                "ownership_conflict",
                "trial terminal observation is single-use",
            ));
        }
        *terminal = Some(observation);
        Ok(())
    }
    pub async fn before(&self) -> Result<()> {
        if self.before_started.swap(true, Ordering::SeqCst) {
            return Err(Error::new(
                "operation_uncertain",
                "trial before hook is single-use",
            ));
        }
        self.hooks.before(&self.permit).await?;
        self.before_finished.store(true, Ordering::SeqCst);
        Ok(())
    }
    pub async fn launched(&self) -> Result<()> {
        if !self.before_finished.load(Ordering::SeqCst)
            || self.launched.swap(true, Ordering::SeqCst)
        {
            return Err(Error::new(
                "ownership_conflict",
                "trial launch hook lacks its unique preparation",
            ));
        }
        self.hooks.launched(&self.permit).await
    }
    pub async fn after(&self, state: &SessionState) -> Result<()> {
        if !self.before_started.load(Ordering::SeqCst)
            || state.binding.operation_id != self.permit.operation_id.to_string()
            || self.after_started.swap(true, Ordering::SeqCst)
        {
            return Err(Error::new(
                "ownership_conflict",
                "trial after hook lacks its exact attempted session",
            ));
        }
        let observation = self.hooks.after(&self.permit, state).await;
        *self.after_observation.lock().map_err(|_| {
            Error::new("ownership_conflict", "trial hook observation lock failed")
        })? = Some(observation.clone());
        self.after_finished.store(true, Ordering::SeqCst);
        observation.map(|_| ())
    }
}
/// Consumes one actual prepared/admitted worker into the evaluator's ordinary
/// broker and guarded provider path. There is no raw-parts constructor.
pub struct AdmittedTrial {
    session: Option<BoundSession>,
    provider: WorkerProviderLaunch,
    lifecycle: TrialLifecycle,
    started: std::time::Instant,
}
impl AdmittedTrial {
    pub(crate) fn new(
        prepared: PreparedRun,
        permit: Permit,
        services: RunServices,
        options: RunOptions,
    ) -> Result<Self> {
        if options.resume.is_some() {
            return Err(Error::new(
                "evaluation_fidelity",
                "the evaluator cannot replace an admitted resumed session with a fresh provider thread",
            ));
        }
        let session = bind_session(prepared, permit, services, options)?;
        let provider = session.broker.provider_launch(
            session.processes.clone(),
            &session.prepared.executable.path,
            &session.prepared.auth_home,
            &session.prepared.permissions,
            Duration::from_millis(session.prepared.runtime.request_timeout_ms),
        )?;
        let lifecycle = TrialLifecycle {
            permit: session.permit.clone(),
            hooks: session.hooks.clone(),
            before_started: AtomicBool::new(false),
            before_finished: AtomicBool::new(false),
            after_started: AtomicBool::new(false),
            launched: AtomicBool::new(false),
            after_finished: AtomicBool::new(false),
            after_observation: Mutex::new(None),
            terminal: Mutex::new(None),
            after_bound: Duration::from_millis(
                session
                    .prepared
                    .contract
                    .workflow()
                    .front_matter
                    .hooks
                    .timeout_ms,
            )
            .min(Duration::from_secs(session.options.max_seconds)),
            max_seconds: session.options.max_seconds,
        };
        Ok(Self {
            session: Some(session),
            provider,
            lifecycle,
            started: std::time::Instant::now(),
        })
    }
    pub fn operation_id(&self) -> Uuid {
        self.lifecycle.operation_id()
    }
    pub fn max_seconds(&self) -> u64 {
        self.lifecycle.max_seconds
    }
    /// Rebinds only exact declared fixture data to this admitted production
    /// worker's owned resources. It does not start the worker or grant live use.
    pub fn bind_fixture(
        &self,
        variant: &crate::evaluation::Variant,
        fixture: crate::evaluation::PreparedFixture,
        report_root: &std::path::Path,
    ) -> Result<crate::evaluation::PreparedFixture> {
        let session = self
            .session
            .as_ref()
            .ok_or_else(|| Error::new("ownership_conflict", "trial has already been consumed"))?;
        if self.lifecycle.before_started.load(Ordering::SeqCst)
            || self.provider.launched.load(Ordering::SeqCst)
        {
            return Err(Error::new(
                "evaluation_fidelity",
                "cannot bind fixture after worker start",
            ));
        }
        fixture.bind_worker(variant, &session.prepared.contract, report_root)
    }
    pub fn operation<'a>(
        &'a mut self,
        fixture: &'a crate::evaluation::PreparedFixture,
        report_root: &'a std::path::Path,
    ) -> Result<crate::evaluation::TrialOperation<'a>> {
        let session = self
            .session
            .as_mut()
            .ok_or_else(|| Error::new("ownership_conflict", "trial has already been consumed"))?;
        Ok(crate::evaluation::TrialOperation {
            work_identity: &session.permit.work_key,
            fixture,
            assembled: session.prepared.contract.instructions(),
            isolation: &session.prepared.isolation,
            permissions: &session.prepared.permissions,
            tools: session.prepared.contract.tools(),
            executable: &session.prepared.executable.path,
            provider: &self.provider,
            lifecycle: &self.lifecycle,
            host: &mut session.broker,
            report_root,
        })
    }
    pub fn finish(mut self) -> Result<VerifiedTrialCompletion> {
        self.try_finish()
    }
    fn try_finish(&mut self) -> Result<VerifiedTrialCompletion> {
        if !self.lifecycle.execution_started() || !self.lifecycle.after_finished() {
            return Err(Error::new(
                "reconciliation_required",
                "trial has not joined its actual execution and hook lifecycle",
            ));
        }
        self.provider.finish()?;
        let session = self
            .session
            .as_mut()
            .ok_or_else(|| Error::new("ownership_conflict", "trial already consumed"))?;
        let after = self.lifecycle.after_result()?;
        let mut terminal = self
            .lifecycle
            .terminal
            .lock()
            .map_err(|_| Error::new("ownership_conflict", "trial terminal lock failed"))?;
        let observation = terminal.as_ref().ok_or_else(|| {
            Error::new(
                "reconciliation_required",
                "trial has no actual terminal execution observation",
            )
        })?;
        let state = session
            .broker
            .trial_checkpoint(observation.state(), observation.pending())?;
        if matches!(observation.outcome(), RunOutcome::Completed) {
            session.broker.completed_checkpoint()?;
        }
        session
            .registry
            .observe_exited(session.permit.operation_id)?;
        if session
            .registry
            .active_operations()?
            .contains(&session.permit.operation_id)
        {
            return Err(Error::new(
                "operation_uncertain",
                "trial retains an unconfirmed owned local effect",
            ));
        }
        // All fallible terminal validation is complete. Preserve the original
        // private observation across errors so owned cleanup can retry safely.
        let observation = terminal.take().expect("validated retained terminal");
        let (outcome, error, hook_error) = observation.into_result();
        let hook_error = hook_error.or_else(|| after.as_ref().err().cloned());
        drop(terminal);
        let validation_history = Some(session.broker.seal_validation_history());
        let validation_error = session.broker.validation_history_error();
        let checks = session.broker.take_verified_checks();
        let failed_checks = session.broker.take_failed_checks();
        let validation_plan = session.broker.take_validation_plan();
        let validation_attempts = session.broker.take_validation_attempts();
        let validation_artifacts = session.broker.take_validation_artifacts();
        let session = self.session.take().expect("validated retained session");
        Ok(VerifiedTrialCompletion {
            completion: RunCompletion {
                operation_id: session.permit.operation_id,
                state,
                outcome,
                error,
                elapsed_ms: self.started.elapsed().as_millis().min(u64::MAX as u128) as u64,
                hook_error,
                checks,
                failed_checks,
                validation_plan,
                validation_attempts,
                validation_artifacts,
                validation_error,
                validation_history,
                execution: session.execution,
                contract: Some(session.prepared.contract),
                local_exit_confirmed: true,
            },
        })
    }
}
impl Drop for AdmittedTrial {
    fn drop(&mut self) {
        if let Some(session) = &self.session {
            // Fence before cancelling any registered provider, hook or check.
            if session.control.fence(session.permit.operation_id).is_ok() {
                let _ = session
                    .registry
                    .cancel_fenced(&session.control, session.permit.operation_id);
                if !self.provider.launched.load(Ordering::SeqCst)
                    && !self.lifecycle.before_started.load(Ordering::SeqCst)
                    && session
                        .registry
                        .active_operations()
                        .is_ok_and(|active| !active.contains(&session.permit.operation_id))
                {
                    let _ = session.control.retire_local(
                        session.permit.operation_id,
                        json!({"outcome":"not_started","phase":"evaluation_admission_abandoned"}),
                    );
                }
            }
        }
    }
}
/// Only actual provider/hook completion and the bound broker's durable final
/// checkpoint can construct this retained result. Ordinary JSON cannot.
pub struct VerifiedTrialCompletion {
    completion: RunCompletion,
}
impl VerifiedTrialCompletion {
    pub(crate) fn into_completion(self) -> RunCompletion {
        self.completion
    }
}
struct BoundSession {
    prepared: PreparedRun,
    permit: Permit,
    broker: ScopedWorker,
    state: SessionState,
    options: RunOptions,
    hooks: Arc<dyn RunHooks>,
    processes: Arc<crate::process_records::ProcessJournal>,
    execution: Option<Arc<WorkerExecution>>,
    control: Arc<ControlHost>,
    host: Arc<dyn WorkerHost>,
    registry: Arc<ExecutionRegistry>,
    authority: WorkerAuthority,
}
fn bind_session(
    prepared: PreparedRun,
    permit: Permit,
    services: RunServices,
    options: RunOptions,
) -> Result<BoundSession> {
    if options.max_seconds == 0
        || options.max_seconds > 86_400
        || options.max_turns == 0
        || options.max_turns > prepared.runtime.max_turns
        || options.initial_input.trim().is_empty()
    {
        return Err(Error::new(
            "invalid_input",
            "worker needs explicit finite limits within configured bounds and nonempty input",
        ));
    }
    if tokio::runtime::Handle::try_current()
        .map_err(|_| Error::new("prerequisite", "worker needs the host executor"))?
        .runtime_flavor()
        != tokio::runtime::RuntimeFlavor::MultiThread
    {
        return Err(Error::new(
            "prerequisite",
            "worker dispatch needs the multithread host executor",
        ));
    }
    prepared.executable.revalidate(false)?;
    prepared.isolation.revalidate()?;
    let control = services.worker.control.clone();
    let run = crate::control::RunReference::from_receipt(&control.operation(permit.operation_id)?)?
        .ok_or_else(|| {
            Error::new(
                "admission_budget",
                "native worker needs its durable finite run admission",
            )
        })?;
    let manifest = &prepared.contract.instructions().manifest;
    if run.workspace != manifest.workspace_id
        || run.account != prepared.contract.account_id()
        || run.profile_digest != canonical_digest(1, &manifest.profile)?
        || run.instruction_digest != prepared.contract.instructions().manifest_digest
        || options.max_turns > run.max_turns
    {
        return Err(Error::new(
            "admission_budget",
            "worker profile, instructions or requested turns differ from original admitted budget",
        ));
    }
    let host = services.worker.host.clone();
    let authority = prepared.contract.authority().clone();
    let registry = services.worker.executions.clone();
    let execution = services.worker.execution.clone();
    if let Some(resume) = &options.resume {
        resume.verify(&permit, &control, &prepared.contract, execution.as_ref())?;
    }
    let mut broker = ScopedWorker::new(prepared.contract.clone(), permit.clone(), services.worker)?;
    broker.validate_live_operation(
        prepared.contract.instructions(),
        &prepared.isolation,
        prepared.contract.tools(),
        &permit.work_key,
    )?;
    let m = &prepared.contract.instructions().manifest;
    let binding = SessionBinding {
        schema_version: 1,
        account_id: prepared.contract.account_id().into(),
        work_identity: permit.work_key.clone(),
        operation_id: permit.operation_id.to_string(),
        generation: m.work_generation.clone(),
        instruction_digest: prepared.contract.instructions().manifest_digest.clone(),
        isolation_digest: prepared.isolation.digest().into(),
        profile: CodexProfile {
            model: m.profile.model.clone(),
            reasoning: m.profile.reasoning.clone(),
            speed: match m.profile.speed {
                Speed::Standard => "standard",
                Speed::Fast => "fast",
            }
            .into(),
        },
        cwd: m.cwd.clone(),
    };
    let mut state = SessionState::new(binding, options.max_turns)?;
    state.observe_correction(broker.correction_requirement().watermark);
    if let Some(resume) = &options.resume {
        broker.reconcile_prior(Some(&resume.prior_record()))?;
        state = resume.new_session(state.binding, options.max_turns)?;
    } else {
        broker.reconcile_prior(None)?;
    }
    broker.checkpoint(&state, None)?;
    Ok(BoundSession {
        prepared,
        permit,
        broker,
        state,
        options,
        hooks: services.hooks,
        processes: services.processes,
        execution,
        control,
        host,
        registry,
        authority,
    })
}
/// Consume the same admitted session binding used by production and evaluations.
pub fn run(
    prepared: PreparedRun,
    permit: Permit,
    services: RunServices,
    options: RunOptions,
) -> Result<OwnedWorkerRun> {
    let BoundSession {
        prepared,
        permit,
        broker,
        state,
        options,
        hooks,
        processes,
        execution,
        control,
        host,
        registry,
        authority,
    } = bind_session(prepared, permit, services, options)?;
    let cancellation = Arc::new(Cancellation {
        requested: AtomicBool::new(false),
        task_finished: AtomicBool::new(false),
        notification: Notify::new(),
        provider: Mutex::new(vec![]),
    });
    let gate = Arc::new(ProtocolGate {
        control: control.clone(),
        permit: permit.clone(),
        host: host.clone(),
        authority: authority.clone(),
        resume_source: options
            .resume
            .as_ref()
            .map(AdmittedSessionResume::startup_source),
        pending: Mutex::new(BTreeMap::new()),
    });
    let operation = permit.operation_id;
    let effect = Uuid::new_v4();
    let mut join = None;
    control.validate_permit(&permit, || {
        host.verify_owner(&permit.lease)?;
        crate::worker::effective_authority(host.as_ref(), permit.operation_id, &authority)?;
        registry.register(operation, effect, cancellation.clone())?;
        let registry = registry.clone();
        let control = control.clone();
        let permit = permit.clone();
        let task_cancellation = cancellation.clone();
        join = Some(tokio::spawn(async move {
            let finished = FinishTask(task_cancellation.clone());
            let mut result = execute(ExecutionTask {
                prepared,
                permit,
                execution,
                broker,
                state,
                options,
                hooks,
                processes,
                cancel: cancellation,
                gate,
                control,
                registry: registry.clone(),
            })
            .await;
            drop(finished);
            registry.observe_exited(operation)?;
            if let Ok(completion) = &mut result {
                completion.local_exit_confirmed = !registry.active_operations()?.contains(&operation);
                if !completion.local_exit_confirmed {
                    completion.outcome = RunOutcome::Interrupted;
                    completion.error = Some(Error::new("operation_uncertain", "owned worker ended with unconfirmed local effects; retain ownership until actual exit observation"));
                    completion.checks.clear();
                    completion.failed_checks.clear();
                }
            }
            result
        }));
        Ok(())
    })?;
    Ok(OwnedWorkerRun {
        operation,
        registry,
        join,
    })
}
pub(crate) trait CorrectionBroker: TrialHost {
    fn correction_waiting(&self) -> Result<bool>;
    fn pending_correction(
        &self,
    ) -> Result<Option<Arc<crate::corrections::VerifiedCorrectionTransition>>>;
    fn deliver_correction(
        &mut self,
        state: &SessionState,
        transition: Arc<crate::corrections::VerifiedCorrectionTransition>,
    ) -> Result<bool>;
    fn finalize_correction(&mut self, state: &mut SessionState) -> Result<()>;
}
impl CorrectionBroker for ScopedWorker {
    fn correction_waiting(&self) -> Result<bool> {
        ScopedWorker::correction_waiting(self)
    }
    fn pending_correction(
        &self,
    ) -> Result<Option<Arc<crate::corrections::VerifiedCorrectionTransition>>> {
        ScopedWorker::pending_correction(self)
    }
    fn deliver_correction(
        &mut self,
        state: &SessionState,
        transition: Arc<crate::corrections::VerifiedCorrectionTransition>,
    ) -> Result<bool> {
        ScopedWorker::deliver_correction(self, state, transition)
    }
    fn finalize_correction(&mut self, state: &mut SessionState) -> Result<()> {
        ScopedWorker::finalize_correction(self, state)
    }
}
/// One bounded current correction delivery, shared by the production runner
/// and actual evaluator. Unknown/rejected writes never become consumed input.
pub(crate) async fn advance_correction(
    rpc: &mut RpcClient,
    state: &mut SessionState,
    permissions: &PermissionConfiguration,
    broker: &mut dyn CorrectionBroker,
) -> Result<bool> {
    if broker.correction_waiting()? {
        return Ok(false);
    }
    let Some(transition) = broker.pending_correction()? else {
        return Ok(false);
    };
    state.observe_correction(transition.watermark());
    broker.correction_event(transition.watermark(), transition.input())?;
    if state.turn_id.is_some() {
        broker.checkpoint(state, Some("turn/steer"))?;
        let accepted = rpc
            .steer(state, transition.watermark(), transition.input(), broker)
            .await?;
        if accepted {
            broker.deliver_correction(state, transition)?;
        }
    } else {
        broker.checkpoint(state, Some("turn/start"))?;
        rpc.start_turn(state, permissions, transition.input(), broker)
            .await?;
        broker.deliver_correction(state, transition)?;
    }
    broker.finalize_correction(state)?;
    broker.checkpoint(state, None)?;
    Ok(true)
}
struct ExecutionTask {
    execution: Option<Arc<WorkerExecution>>,
    prepared: PreparedRun,
    permit: Permit,
    broker: ScopedWorker,
    state: SessionState,
    options: RunOptions,
    hooks: Arc<dyn RunHooks>,
    processes: Arc<crate::process_records::ProcessJournal>,
    cancel: Arc<Cancellation>,
    gate: Arc<ProtocolGate>,
    control: Arc<ControlHost>,
    registry: Arc<ExecutionRegistry>,
}
async fn execute(task: ExecutionTask) -> Result<RunCompletion> {
    let ExecutionTask {
        execution,
        prepared,
        permit,
        mut broker,
        mut state,
        options,
        hooks,
        processes,
        cancel,
        gate,
        control,
        registry,
    } = task;
    let start = tokio::time::Instant::now();
    let deadline = start + Duration::from_secs(options.max_seconds);
    broker.bind_inspection_deadline(deadline.into_std());
    let mut client = None;
    let mut core = Box::pin(async {
        hooks.before(&permit).await?;
        if cancel.requested.load(Ordering::SeqCst) {
            return Err(Error::new(
                "interrupted",
                "worker was fenced before provider startup",
            ));
        }
        client = Some(
            launch_provider(
                &ProviderLaunchInput {
                    control: control.clone(),
                    permit: permit.clone(),
                    processes: processes.clone(),
                    executable: prepared.executable.clone(),
                    auth_home: prepared.auth_home.clone(),
                    permissions: prepared.permissions.clone(),
                    isolation: prepared.isolation.clone(),
                    timeout: Duration::from_millis(prepared.runtime.request_timeout_ms),
                },
                &cancel,
                gate,
            )
            .await?,
        );
        let rpc = client.as_mut().expect("owned adapter initialized");
        rpc.initialize().await?;
        rpc.verify_account(&state.binding.account_id).await?;
        state
            .binding
            .profile
            .validate_catalog(&rpc.discover_models().await?)?;
        broker.rate_limits(&rpc.rate_limits().await?)?;
        rpc.verify_execution_configuration(&prepared.permissions, &prepared.isolation.plan().cwd)
            .await?;
        if let Some(resume) = &options.resume {
            broker.checkpoint(&state, Some("thread/resume"))?;
            rpc.resume_verified(
                &mut state,
                resume.bridge(),
                &prepared.isolation,
                &prepared.permissions,
                &mut broker,
            )
            .await?;
        } else {
            broker.checkpoint(&state, Some("thread/start"))?;
            rpc.start_thread(
                &mut state,
                &prepared.isolation,
                &prepared.permissions,
                &prepared.contract.instructions().prompt,
                prepared.contract.tools().definitions().to_vec(),
            )
            .await?;
        }
        broker.checkpoint(&state, None)?;
        hooks.launched(&permit).await?;
        let review = matches!(
            prepared.contract.instructions().manifest.role,
            AgentRole::Review | AgentRole::IndependentReview
        );
        broker.checkpoint(
            &state,
            Some(if review { "review/start" } else { "turn/start" }),
        )?;
        if review {
            rpc.review(&mut state, &options.initial_input, &mut broker)
                .await?;
        } else {
            rpc.start_turn(
                &mut state,
                &prepared.permissions,
                &options.initial_input,
                &mut broker,
            )
            .await?;
        }
        broker.checkpoint(&state, None)?;
        let correction_wake = broker.correction_wake()?;
        loop {
            if cancel.requested.load(Ordering::SeqCst) {
                return Err(Error::new("interrupted", "owned worker was cancelled"));
            }
            if state.intervention.is_some() {
                return Err(Error::new(
                    "provider_intervention",
                    "provider verification is required",
                ));
            }
            if state.input_required {
                return Err(Error::new(
                    "input_required",
                    "worker requested unavailable interactive input",
                ));
            }
            if advance_correction(rpc, &mut state, &prepared.permissions, &mut broker).await? {
                continue;
            }
            broker.finalize_correction(&mut state)?;
            broker.checkpoint(&state, None)?;
            if let Some(terminal) = &state.terminal
                && !broker.correction_waiting()?
            {
                return if terminal == "completed" && state.completion_is_current() {
                    Ok(())
                } else {
                    Err(Error::new(
                        "turn_incomplete",
                        "worker ended without a current complete turn",
                    ))
                };
            }
            if state.turn_id.is_none() && broker.correction_waiting()? {
                rpc.pump_waiting(
                    &mut state,
                    &mut broker,
                    deadline,
                    correction_wake.as_deref(),
                )
                .await?;
            } else {
                rpc.pump_or_wake(
                    &mut state,
                    &mut broker,
                    Duration::from_millis(prepared.runtime.stream_silence_ms),
                    Duration::from_millis(prepared.runtime.stall_ms),
                    correction_wake.as_deref(),
                )
                .await?;
            }
            broker.checkpoint(&state, None)?;
        }
    });
    let mut interrupted = false;
    let result = tokio::select! {result=&mut core=>result,_=cancel.notified()=>{interrupted=true;Err(Error::new("interrupted","owned worker cancellation requested"))},_=tokio::time::sleep_until(deadline)=>{interrupted=true;Err(Error::new("timeout","finite worker duration exhausted"))}};
    let result = if interrupted {
        control.fence(permit.operation_id)?;
        cancel.cancel()?;
        // Do not drop an in-flight tool/check callback. Its registered owned
        // process receives the same fenced cancellation, then it drains normally.
        registry.cancel_fenced(&control, permit.operation_id)?;
        match tokio::time::timeout(Duration::from_secs(15), &mut core).await {
            Ok(_) => result,
            Err(_) => Err(Error::new(
                "operation_uncertain",
                "worker callback did not drain after owned cancellation; preserve registered effects",
            )),
        }
    } else {
        result
    };
    drop(core);
    if result.is_err() {
        control.fence(permit.operation_id)?;
        registry.cancel_fenced(&control, permit.operation_id)?;
    }
    if let Some(rpc) = client.as_mut() {
        rpc.terminate().await?;
    }
    if interrupted {
        state.interruption("owned_cancellation");
    }
    broker.checkpoint(
        &state,
        if interrupted {
            Some("interrupted_unknown")
        } else {
            None
        },
    )?;
    let hook_bound =
        Duration::from_millis(prepared.contract.workflow().front_matter.hooks.timeout_ms)
            .min(Duration::from_secs(options.max_seconds));
    let mut after = Box::pin(hooks.after(&permit, &state));
    let hook_result = match tokio::time::timeout(hook_bound, &mut after).await {
        Ok(result) => result,
        Err(_) => {
            control.fence(permit.operation_id)?;
            registry.cancel_fenced(&control, permit.operation_id)?;
            let _ = tokio::time::timeout(Duration::from_secs(15), &mut after).await;
            Err(Error::new(
                "operation_uncertain",
                "after-run hook exceeded its finite bound; preserve its unknown attempt",
            ))
        }
    };
    drop(after);
    broker.record_after_hook(hook_result.as_ref())?;
    let hook_error = hook_result.err();
    let outcome = match &result {
        Ok(()) => RunOutcome::Completed,
        Err(_) if interrupted => RunOutcome::Interrupted,
        Err(_) if state.turns_started == 0 => RunOutcome::Blocked,
        Err(_) => RunOutcome::Failed,
    };
    let validation_history = Some(broker.seal_validation_history());
    let validation_error = broker.validation_history_error();
    Ok(RunCompletion {
        operation_id: permit.operation_id,
        state,
        outcome,
        error: result.err(),
        elapsed_ms: start.elapsed().as_millis().min(u64::MAX as u128) as u64,
        hook_error,
        checks: broker.take_verified_checks(),
        failed_checks: broker.take_failed_checks(),
        validation_plan: broker.take_validation_plan(),
        validation_attempts: broker.take_validation_attempts(),
        validation_artifacts: broker.take_validation_artifacts(),
        validation_error,
        validation_history,
        execution,
        contract: Some(prepared.contract.clone()),
        local_exit_confirmed: false,
    })
}

/// Read/coordination roles never execute repository hooks. This implementation
/// proves the immutable nonexecuting role. Final inapplicability is an
/// observation of the original admission and does not reopen a fenced permit.
pub struct NonExecutingRunHooks {
    contract: Arc<WorkerContract>,
    control: Arc<ControlHost>,
    host: Arc<dyn WorkerHost>,
}
impl NonExecutingRunHooks {
    pub fn new(
        contract: Arc<WorkerContract>,
        control: Arc<ControlHost>,
        host: Arc<dyn WorkerHost>,
    ) -> Result<Self> {
        if contract
            .instructions()
            .manifest
            .permissions
            .repository_code_execution
        {
            return Err(Error::new(
                "scope_violation",
                "executable worker roles require the real hook executor",
            ));
        }
        Ok(Self {
            contract,
            control,
            host,
        })
    }
    fn verify(&self, permit: &Permit) -> Result<()> {
        if permit.scope_digest != self.contract.scope_digest() {
            return Err(Error::new(
                "scope_violation",
                "hook boundary differs from admitted worker",
            ));
        }
        self.control.validate_permit(permit, || {
            self.host.verify_owner(&permit.lease)?;
            crate::worker::effective_authority(
                self.host.as_ref(),
                permit.operation_id,
                self.contract.authority(),
            )
            .map(|_| ())
        })
    }
}
impl RunHooks for NonExecutingRunHooks {
    fn before<'a>(&'a self, permit: &'a Permit) -> RunFuture<'a, ()> {
        Box::pin(async move { self.verify(permit) })
    }
    fn launched<'a>(&'a self, permit: &'a Permit) -> RunFuture<'a, ()> {
        Box::pin(async move { self.verify(permit) })
    }
    fn after<'a>(
        &'a self,
        permit: &'a Permit,
        state: &'a SessionState,
    ) -> RunFuture<'a, AfterHookObservation> {
        Box::pin(async move {
            verify_after_observation(&self.contract, &self.control, permit, state)?;
            // No repository hook is applicable to this immutable role. A
            // fenced owner may record that fact; this dispatches no effect.
            Ok(AfterHookObservation::NotApplicable)
        })
    }
}
pub struct ExecutorHookServices {
    pub contract: Arc<WorkerContract>,
    pub control: Arc<ControlHost>,
    pub host: Arc<dyn WorkerHost>,
    pub execution: Arc<crate::worker::WorkerExecution>,
    pub registry: Arc<ExecutionRegistry>,
    /// Retained private ledger from the owned directory's actual after_create
    /// phase. An absent/uninitialized ledger never fabricates creation success.
    pub records: crate::storage::PrivateDir,
    pub session_id: Uuid,
}
#[derive(Clone)]
pub struct ExecutorRunHooks {
    contract: Arc<WorkerContract>,
    control: Arc<ControlHost>,
    host: Arc<dyn WorkerHost>,
    execution: Arc<crate::worker::WorkerExecution>,
    registry: Arc<ExecutionRegistry>,
    records: Arc<crate::storage::PrivateDir>,
    ledger: Arc<tokio::sync::Mutex<crate::execution::HookLedger>>,
    session_id: Uuid,
    _lock: Arc<crate::storage::ExclusiveLock>,
}
impl ExecutorRunHooks {
    pub(crate) fn new_reused(
        input: ExecutorHookServices,
        enrollment: &crate::housekeeping::EnrolledDirectoryReuse,
    ) -> Result<Self> {
        if &input.execution.binding != enrollment.binding()
            || input.contract.instructions().manifest.operation_id != enrollment.operation_id()
        {
            return Err(Error::new(
                "hook_gate",
                "successor hook ledger differs from actual family enrollment",
            ));
        }
        let ledger = crate::execution::HookLedger::for_reused_execution(
            enrollment,
            input.contract.workflow(),
        )?;
        let lock = input.records.lock("hooks.lock")?;
        match input
            .records
            .read::<crate::execution::HookLedger>("hooks.json")
        {
            Err(error) if error.code == "record_missing" => {
                input.records.write("hooks.json", &ledger)?
            }
            Ok(_) => {
                return Err(Error::new(
                    "reconciliation_required",
                    "successor hooks already have retained history",
                ));
            }
            Err(error) => return Err(error),
        }
        drop(lock);
        Self::new(input)
    }
    pub fn new(input: ExecutorHookServices) -> Result<Self> {
        let contract = &input.contract;
        let root = input.records.path();
        if !contract
            .instructions()
            .manifest
            .role
            .permits_repository_execution()
            || input.session_id.is_nil()
            || input.execution.binding.operation_id != contract.instructions().manifest.operation_id
            || input.execution.binding.isolation_digest != contract.isolation().digest()
        {
            return Err(Error::new(
                "scope_violation",
                "hook executor requires the exact executable worker binding",
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
            if root.starts_with(grant) || grant.starts_with(root) {
                return Err(Error::new(
                    "scope_violation",
                    "hook ledger must be outside worker grants",
                ));
            }
        }
        if !contract
            .isolation()
            .plan()
            .forbidden_roots
            .iter()
            .any(|path| root.starts_with(path))
        {
            return Err(Error::new(
                "scope_violation",
                "hook ledger needs an explicit forbidden root",
            ));
        }
        let lock = input.records.lock("hooks.lock")?;
        let ledger: crate::execution::HookLedger = input.records.read("hooks.json")?;
        if ledger.binding_digest != canonical_digest(1, &input.execution.binding)?
            || !ledger.initialized
        {
            return Err(Error::new(
                "reconciliation_required",
                "owned directory creation hook is absent, unknown or bound to another operation",
            ));
        }
        Ok(Self {
            contract: input.contract,
            control: input.control,
            host: input.host,
            execution: input.execution,
            registry: input.registry,
            records: Arc::new(input.records),
            ledger: Arc::new(tokio::sync::Mutex::new(ledger)),
            session_id: input.session_id,
            _lock: Arc::new(lock),
        })
    }
    fn fresh(&self, permit: &Permit) -> Result<()> {
        if permit.operation_id != self.execution.binding.operation_id
            || permit.scope_digest != self.contract.scope_digest()
        {
            return Err(Error::new(
                "scope_violation",
                "hook permit differs from current worker",
            ));
        }
        self.host.verify_owner(&permit.lease)?;
        crate::worker::effective_authority(
            self.host.as_ref(),
            permit.operation_id,
            self.contract.authority(),
        )
        .map(|_| ())
    }
    fn executor(&self) -> crate::execution::LocalExecutor<'_> {
        crate::execution::LocalExecutor {
            isolation: self.contract.isolation(),
            git: &self.execution.git,
            boundary: self.execution.boundary.as_ref(),
            limits: self.execution.limits.clone(),
            ownership: &self.execution.ownership,
        }
    }
    async fn phase(&self, permit: &Permit, phase: crate::execution::HookPhase) -> Result<()> {
        let owned = self.clone();
        let permit = permit.clone();
        let handle = tokio::runtime::Handle::current();
        // Scoped Git preflight and source hashing are synchronous. Keep them on
        // a bounded owned task so local control remains responsive while waiting.
        tokio::task::spawn_blocking(move || handle.block_on(owned.phase_owned(&permit, phase)))
            .await
            .map_err(|_| Error::new("operation_uncertain", "owned hook dispatcher interrupted"))?
    }
    async fn phase_owned(&self, permit: &Permit, phase: crate::execution::HookPhase) -> Result<()> {
        let mut ledger = self.ledger.lock().await;
        let executor = self.executor();
        let prepared = executor.prepare_hook(
            &mut ledger,
            crate::execution::HookInvocation {
                binding: &self.execution.binding,
                workflow: self.contract.workflow(),
                phase,
                session: Some(self.session_id),
                eligible_cleanup_plan: None,
            },
            |ledger| self.records.write("hooks.json", ledger),
        )?;
        let request = MutationRequest {
            request_id: Uuid::new_v4(),
            runtime_id: permit.lease.runtime_id,
            actor: Actor {
                identity: format!("owner:{}", permit.lease.owner_id),
                principal: None,
                roles: Default::default(),
            },
            kind: "worker_hook".into(),
            canonical_targets: vec![format!("hook:{}:{phase:?}", self.session_id)],
            expected: BTreeMap::new(),
            payload: json!({"session":self.session_id,"phase":phase,"workflow":self.contract.workflow().digest}),
        };
        let (effect, running) = self.control.begin_effect(
            request,
            permit,
            || self.fresh(permit),
            |receipt| {
                executor.preflight_hook(&prepared)?;
                let reservation = self
                    .registry
                    .reserve(permit.operation_id, receipt.operation_id)?;
                let running = executor.start_hook(prepared)?;
                if let Some(cancel) = running.cancellation() {
                    reservation.attach(cancel)?;
                } else {
                    reservation.never_started();
                }
                Ok(running)
            },
        )?;
        let effect_id = effect.operation_id();
        let cancellation = running.cancellation();
        let result = executor
            .complete_hook(&mut ledger, running, |ledger| {
                self.records.write("hooks.json", ledger)
            })
            .await;
        if cancellation
            .as_ref()
            .map(|owned| owned.exited())
            .transpose()?
            .is_some_and(|exited| !exited)
        {
            return Err(Error::new(
                "operation_uncertain",
                "hook exit is unconfirmed; preserve its owned handle and sent receipt",
            ));
        }
        self.registry.remove(permit.operation_id, effect_id)?;
        let observed = match &result {
            Ok(result) => {
                json!({"hook":phase,"state":result.state,"diagnostic_only":result.diagnostic_only})
            }
            Err(error) => json!({"hook":phase,"error":error}),
        };
        self.control
            .complete_effect(effect, permit, || self.fresh(permit), observed)?;
        let result = result?;
        if !result.allows_next_phase {
            return Err(Error::new(
                "hook_failed",
                "configured hook failed or remains unknown; retained files require reconciliation",
            ));
        }
        Ok(())
    }
}
impl RunHooks for ExecutorRunHooks {
    fn before<'a>(&'a self, permit: &'a Permit) -> RunFuture<'a, ()> {
        Box::pin(async move {
            self.phase(permit, crate::execution::HookPhase::BeforeRun)
                .await
        })
    }
    fn launched<'a>(&'a self, permit: &'a Permit) -> RunFuture<'a, ()> {
        Box::pin(async move {
            self.control
                .validate_permit(permit, || self.fresh(permit))?;
            self.ledger
                .lock()
                .await
                .mark_launched(self.session_id, &mut |ledger| {
                    self.records.write("hooks.json", ledger)
                })
        })
    }
    fn after<'a>(
        &'a self,
        permit: &'a Permit,
        state: &'a SessionState,
    ) -> RunFuture<'a, AfterHookObservation> {
        Box::pin(async move {
            verify_after_observation(&self.contract, &self.control, permit, state)?;
            {
                let mut ledger = self.ledger.lock().await;
                if !ledger.sessions.contains_key(&self.session_id) {
                    return Ok(AfterHookObservation::Skipped {
                        reason: AfterHookSkip::SessionNotLaunched,
                    });
                }
                ledger.mark_ended(self.session_id, &mut |ledger| {
                    self.records.write("hooks.json", ledger)
                })?;
                if self
                    .contract
                    .workflow()
                    .front_matter
                    .hooks
                    .after_run
                    .is_none()
                {
                    self.execution
                        .ownership
                        .verify(&self.execution.binding, self.contract.isolation())?;
                    if ledger.binding_digest != canonical_digest(1, &self.execution.binding)? {
                        return Err(Error::new(
                            "execution_identity",
                            "after-hook ledger binding changed",
                        ));
                    }
                    let key = format!("AfterRun:{}", self.session_id);
                    if ledger.records.contains_key(&key) {
                        return Err(Error::new(
                            "reconciliation_required",
                            "after-hook observation already exists",
                        ));
                    }
                    let mut next = ledger.clone();
                    next.records.insert(
                        key,
                        crate::execution::HookRecord {
                            phase: crate::execution::HookPhase::AfterRun,
                            session: Some(self.session_id),
                            workflow_digest: self.contract.workflow().digest.clone(),
                            script_digest: None,
                            cleanup_plan: None,
                            state: crate::execution::HookState::Skipped,
                            exit_code: None,
                            timed_out: false,
                            stdout: String::new(),
                            stderr: String::new(),
                            error_code: None,
                            evidence_class: self.execution.boundary.evidence_class(),
                        },
                    );
                    self.records.write("hooks.json", &next)?;
                    *ledger = next;
                    return Ok(AfterHookObservation::Skipped {
                        reason: AfterHookSkip::HookNotConfigured,
                    });
                }
            }
            self.control
                .validate_permit(permit, || self.fresh(permit))
                .map_err(|error| {
                    Error::new(
                        "hook_not_run_fenced",
                        "configured after-run hook was not authorized to start",
                    )
                    .detail(json!({"cause":error.code}))
                })?;
            self.phase(permit, crate::execution::HookPhase::AfterRun)
                .await?;
            Ok(AfterHookObservation::Completed)
        })
    }
}

fn verify_after_observation(
    contract: &WorkerContract,
    control: &ControlHost,
    permit: &Permit,
    state: &SessionState,
) -> Result<()> {
    let admission = control.operation(permit.operation_id)?;
    let manifest = &contract.instructions().manifest;
    if permit.scope_digest != contract.scope_digest()
        || permit.work_key != contract.work_key()?
        || admission.kind != "admission"
        || admission.runtime_id != permit.lease.runtime_id
        || admission.owner_generation != permit.lease.owner_generation
        || admission.actor != format!("owner:{}", permit.lease.owner_id)
        || admission.canonical_targets != [permit.work_key.clone()]
        || admission.expected.get("scope") != Some(&permit.scope_digest)
        || state.binding.operation_id != permit.operation_id.to_string()
        || state.binding.account_id != contract.account_id()
        || state.binding.work_identity != permit.work_key
        || state.binding.generation != manifest.work_generation
        || state.binding.instruction_digest != contract.instructions().manifest_digest
        || state.binding.cwd != manifest.cwd
        || state.binding.isolation_digest != contract.isolation().digest()
    {
        return Err(Error::new(
            "worker_binding",
            "after-hook observation differs from its original session admission",
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "worker_protocol_tests.rs"]
mod protocol_tests;

#[cfg(test)]
mod trial_lifecycle_tests {
    use super::*;
    use crate::control::{Lease, Lifecycle};
    use crate::storage::PrivateDir;
    use std::collections::BTreeSet;
    use std::sync::atomic::AtomicUsize;

    struct FailedHooks {
        after: AtomicUsize,
    }
    impl RunHooks for FailedHooks {
        fn before<'a>(&'a self, _: &'a Permit) -> RunFuture<'a, ()> {
            Box::pin(async { Err(Error::new("hook_failed", "synthetic component before hook")) })
        }
        fn launched<'a>(&'a self, _: &'a Permit) -> RunFuture<'a, ()> {
            Box::pin(async { Err(Error::new("test_unexpected", "component must not launch")) })
        }
        fn after<'a>(
            &'a self,
            _: &'a Permit,
            _: &'a SessionState,
        ) -> RunFuture<'a, AfterHookObservation> {
            Box::pin(async move {
                self.after.fetch_add(1, Ordering::SeqCst);
                Err(Error::new("hook_failed", "synthetic component after hook"))
            })
        }
    }
    #[tokio::test]
    async fn failed_before_hook_still_runs_exact_after_once_without_terminal_capability() {
        // Real Control admission; hooks are explicit deterministic component
        // inputs. No provider/owned exit/native completion is manufactured.
        let root = tempfile::tempdir().unwrap();
        let runtime = Uuid::new_v4();
        let control = ControlHost::setup(
            PrivateDir::create(&root.path().canonicalize().unwrap().join("control")).unwrap(),
            runtime,
            Uuid::new_v4(),
        )
        .unwrap();
        control
            .lifecycle(
                MutationRequest {
                    request_id: Uuid::new_v4(),
                    runtime_id: runtime,
                    actor: Actor {
                        identity: "fixture".into(),
                        principal: None,
                        roles: BTreeSet::from(["lifecycle".into()]),
                    },
                    kind: "start".into(),
                    canonical_targets: vec![runtime.to_string()],
                    expected: BTreeMap::new(),
                    payload: Value::Null,
                },
                Lifecycle::Start,
                0,
            )
            .unwrap();
        let lease = Lease {
            runtime_id: runtime,
            owner_generation: 1,
            owner_id: Uuid::new_v4(),
        };
        control.attach_reconciled(lease.clone()).unwrap();
        let permit = control
            .admit(
                &lease,
                Uuid::new_v4(),
                "component-work".into(),
                kegari_digest::bytes_digest(b"scope"),
                || Ok(()),
            )
            .unwrap();
        let hooks = Arc::new(FailedHooks {
            after: AtomicUsize::new(0),
        });
        let lifecycle = TrialLifecycle {
            permit: permit.clone(),
            hooks: hooks.clone(),
            before_started: AtomicBool::new(false),
            before_finished: AtomicBool::new(false),
            after_started: AtomicBool::new(false),
            launched: AtomicBool::new(false),
            after_finished: AtomicBool::new(false),
            after_observation: Mutex::new(None),
            terminal: Mutex::new(None),
            after_bound: Duration::from_secs(1),
            max_seconds: 1,
        };
        let mut state = SessionState::new(
            SessionBinding {
                schema_version: 1,
                account_id: "account".into(),
                work_identity: permit.work_key.clone(),
                operation_id: Uuid::new_v4().to_string(),
                generation: kegari_digest::bytes_digest(b"generation"),
                instruction_digest: kegari_digest::bytes_digest(b"instructions"),
                isolation_digest: kegari_digest::bytes_digest(b"isolation"),
                profile: CodexProfile {
                    model: "model".into(),
                    reasoning: "high".into(),
                    speed: "standard".into(),
                },
                cwd: root.path().canonicalize().unwrap(),
            },
            1,
        )
        .unwrap();
        assert!(lifecycle.after(&state).await.is_err());
        assert_eq!(hooks.after.load(Ordering::SeqCst), 0);
        assert_eq!(lifecycle.before().await.unwrap_err().code, "hook_failed");
        assert!(lifecycle.after(&state).await.is_err());
        assert_eq!(hooks.after.load(Ordering::SeqCst), 0);
        state.binding.operation_id = permit.operation_id.to_string();
        assert_eq!(
            lifecycle.after(&state).await.unwrap_err().code,
            "hook_failed"
        );
        assert!(lifecycle.after_finished());
        assert_eq!(hooks.after.load(Ordering::SeqCst), 1);
        assert!(
            lifecycle
                .after_observation
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .is_err()
        );
        assert!(lifecycle.after(&state).await.is_err());
        assert!(lifecycle.terminal.lock().unwrap().is_none());
        assert_eq!(hooks.after.load(Ordering::SeqCst), 1);
        assert_ne!(
            control.operation(permit.operation_id).unwrap().phase,
            crate::control::ReceiptPhase::Reconciled
        );
    }
}
