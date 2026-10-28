//! Same-process recovery selection. Durable records identify attempts; only the
//! actual joined completion and live opaque readback/source proofs authorize
//! progression. No cold receipt becomes a worker or provider capability.
use super::*;
use crate::{
    control::{
        ControlSnapshot, Intent, Receipt, ReceiptPhase, SessionRecoveryPreparationReference,
    },
    git::{Git, GitLimits, HostGitExecutionScope, VerifiedReusableSource},
    host_process::{HostProcessServices, LocalProcessPermit, MetadataRequest},
    process_records::ProcessJournal,
    runtime_schedule::RunSchedule,
    runtime_workers::{LaunchRequest, SourceSelection},
    worker_run::{
        RecoveryPreparation, RecoveryProviderLaunch, RunOutcome, VerifiedProviderRecovery,
    },
};
use kegari_config::Config;
use kegari_context::instructions::AgentRole;
use kegari_scheduler::{ReadyWork, Trigger, WorkKind};
use kegari_types::run::RunReference;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum SessionRecoveryStage {
    Provider { attempt: u32 },
    Source,
    Resume,
}
impl SessionRecoveryStage {
    pub(crate) fn model(self) -> bool {
        self == Self::Resume
    }
}
#[derive(Clone)]
pub(crate) struct SessionRecoveryAction {
    pub(crate) key: String,
    pub(crate) candidate: String,
    pub(crate) predecessor: Uuid,
    pub(crate) operation: Uuid,
    pub(crate) stage: SessionRecoveryStage,
    pub(crate) ready: ReadyWork,
}
#[derive(Clone)]
struct Candidate {
    previous: Arc<RunCompletion>,
    retained: crate::worker_run::RetainedResumeSession,
    config: Arc<Config>,
    records: PathBuf,
    key: String,
    model_operation: Uuid,
    paired: Option<Arc<crate::delivery::VerifiedPairedRecoveryCandidate>>,
}
pub(super) struct RecoveryEntry {
    candidate: Candidate,
    provider: Option<VerifiedProviderRecovery>,
    source: Option<VerifiedReusableSource>,
    in_flight: Option<SessionRecoveryStage>,
    completed: bool,
    error: Option<Error>,
}
impl RecoveryEntry {
    fn new(candidate: Candidate) -> Self {
        Self {
            candidate,
            provider: None,
            source: None,
            in_flight: None,
            completed: false,
            error: None,
        }
    }
}
struct CandidatePlan {
    model: RunSchedule,
    metadata_attempt: u32,
    metadata_cap: u32,
    authority: WorkerAuthority,
    ready: ReadyWork,
}
struct RecoveryStageContext<'a> {
    lease: &'a Lease,
    action: &'a SessionRecoveryAction,
    candidate: &'a Candidate,
    provider: Option<&'a VerifiedProviderRecovery>,
    source: Option<&'a VerifiedReusableSource>,
    processes: Arc<ProcessJournal>,
    worker_seconds: u64,
}

impl RuntimeCoordinator {
    /// Implementation recovery preserves ordinary or actual registered paired
    /// causes. Validation/review/parent phases require their own stage integration.
    pub(crate) fn ready_session_recoveries(&self) -> Result<Vec<SessionRecoveryAction>> {
        let completed = self
            .state
            .lock()
            .map_err(|_| lock_error())?
            .completions
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let mut actions = Vec::new();
        let mut errors = BTreeMap::new();
        for previous in completed {
            if !candidate_shape(&previous) {
                continue;
            }
            let predecessor = previous.operation_id;
            let observed = (|| {
                let candidate = self.recovery_candidate(previous)?;
                {
                    let mut state = self.state.lock().map_err(|_| lock_error())?;
                    if state.session_recovery.len() >= 65_536
                        && !state.session_recovery.contains_key(&candidate.key)
                    {
                        return Err(Error::new(
                            "record_limit",
                            "same-process recovery inventory exceeds its bound",
                        ));
                    }
                    state
                        .session_recovery
                        .entry(candidate.key.clone())
                        .or_insert_with(|| RecoveryEntry::new(candidate.clone()));
                }
                let (provider, source, in_flight, finished, error) = {
                    let state = self.state.lock().map_err(|_| lock_error())?;
                    let entry = &state.session_recovery[&candidate.key];
                    (
                        entry.provider.clone(),
                        entry.source.clone(),
                        entry.in_flight,
                        entry.completed,
                        entry.error.clone(),
                    )
                };
                if in_flight.is_some() || finished {
                    return Ok(None);
                }
                let snapshot = self.control.snapshot()?;
                let plan =
                    self.plan_recovery_candidate(&candidate, provider.as_ref(), &snapshot)?;
                let stage = if provider.is_none() {
                    SessionRecoveryStage::Provider {
                        attempt: plan.metadata_attempt,
                    }
                } else if source.is_none() {
                    SessionRecoveryStage::Source
                } else {
                    SessionRecoveryStage::Resume
                };
                if let Some(error) = error
                    && !recovery_wait(&error)
                    && !matches!(stage, SessionRecoveryStage::Provider { .. })
                {
                    return Err(error);
                }
                self.check_worker_capacity(
                    candidate.retained.reference().workspace,
                    &candidate.retained.reference().account,
                    &candidate.retained.reference().phase,
                    &plan.ready.state,
                )?;
                let operation = stage_operation(&candidate, stage, plan.metadata_cap)?;
                // A durable successful or uncertain stage without its actual
                // retained capability is not permission to reconstruct/replay.
                if self
                    .control
                    .operations()?
                    .iter()
                    .any(|receipt| receipt.operation_id == operation)
                {
                    return Err(Error::new(
                        "reconciliation_required",
                        "recovery stage has a retained receipt but no consumable same-process capability",
                    ));
                }
                Ok(Some(SessionRecoveryAction {
                    key: canonical_digest(1, &json!({"candidate":candidate.key,"stage":stage}))?,
                    candidate: candidate.key.clone(),
                    predecessor,
                    operation,
                    stage,
                    ready: plan.ready,
                }))
            })();
            match observed {
                Ok(Some(action)) => actions.push(action),
                Ok(None) => (),
                Err(error) => {
                    errors.insert(predecessor, error);
                }
            }
        }
        let mut claims = std::collections::BTreeSet::new();
        if actions
            .iter()
            .any(|action| !claims.insert(action.ready.canonical_key.clone()))
        {
            return Err(Error::new(
                "ownership_conflict",
                "multiple recovery candidates claim the same canonical work",
            ));
        }
        self.state
            .lock()
            .map_err(|_| lock_error())?
            .session_recovery_errors = errors;
        Ok(actions)
    }

    fn recovery_candidate(&self, previous: Arc<RunCompletion>) -> Result<Candidate> {
        let contract = previous
            .contract
            .as_ref()
            .ok_or_else(|| incompatible("joined contract missing"))?;
        if self
            .configuration
            .snapshot()?
            .workspaces
            .values()
            .find(|w| w.id == contract.instructions().manifest.workspace_id)
            .is_some_and(|w| w.evaluation.is_some())
        {
            return Err(incompatible(
                "evaluation trial requires explicit inspection; automatic session recovery is disabled",
            ));
        }
        let receipt = self.control.operation(previous.operation_id)?;
        let reference = crate::control::run_reference_from_receipt(&receipt)?
            .ok_or_else(|| incompatible("joined admission missing"))?;
        if reference.phase.phase != "implementation"
            || reference.phase.work != contract.work_key()?
        {
            return Err(incompatible(
                "automatic recovery is limited to child implementation",
            ));
        }
        let records = self
            .registration
            .root()
            .path()
            .join("worker-records")
            .join(previous.operation_id.to_string());
        let directory = PrivateDir::open(&records)?;
        let retained = crate::worker::recover_session(&directory, &receipt)?
            .ok_or_else(|| incompatible("protected final session missing"))?
            .resume_candidate()?;
        if canonical_digest(1, retained.state())? != canonical_digest(1, &previous.state)? {
            return Err(incompatible(
                "joined state differs from protected final session",
            ));
        }
        let config = Arc::new(self.configuration.snapshot()?);
        let paired = if let Some(plan) = self.retained_review_round(previous.operation_id)? {
            let current = self.current_admission_authority(contract.authority())?;
            let WorkerAuthority::Authorized(current) = &current else {
                return Err(incompatible(
                    "paired recovery requires current child authority",
                ));
            };
            let receipts = self.control.operations()?;
            let grants = crate::runtime_workflow::current_retry_grants(
                &config,
                &self.observations.admission_snapshot()?,
                current.work(),
                &receipts,
                &self.services.key,
            )?;
            let current_gate = |gate: &crate::reconcile::FreshGate| {
                let expected = WorkerAuthority::Authorized(gate.clone());
                expected.compare(&self.current_admission_authority(&expected)?)
            };
            let joined = |id| self.retained_completion(id);
            let resumed = |id| self.retained_session_resume(id);
            Some(Arc::new(
                crate::delivery::VerifiedPairedRecoveryCandidate::observe(
                    &self.control,
                    crate::delivery::PairedRecoveryInput {
                        plan,
                        failed: previous.clone(),
                        protected: retained.clone(),
                        current,
                        config: &config,
                        grants: &grants,
                    },
                    &crate::delivery::PairedRecoveryReaders {
                        current_gate: &current_gate,
                        joined: &joined,
                        resumed: &resumed,
                    },
                )?,
            ))
        } else {
            if receipt.expected.contains_key("review_round") {
                return Err(incompatible(
                    "round recovery lost its actual registered plan",
                ));
            }
            None
        };
        let mut identity = json!({"predecessor":previous.operation_id,"candidate":retained.digest(),"receipt":retained.admission_digest(),"config":crate::config::config_revision(&config)?,"contract":contract.scope_digest()});
        if let Some(paired) = &paired {
            identity["paired_recovery"] = json!(paired.digest());
        }
        let key = canonical_digest(1, &identity)?;
        Ok(Candidate {
            previous,
            retained,
            config,
            records,
            model_operation: recovery_id(&key, "model")?,
            paired,
            key,
        })
    }

    fn plan_recovery_candidate(
        &self,
        candidate: &Candidate,
        provider: Option<&VerifiedProviderRecovery>,
        snapshot: &ControlSnapshot,
    ) -> Result<CandidatePlan> {
        let previous = self
            .retained_completion(candidate.previous.operation_id)?
            .ok_or_else(|| incompatible("actual joined completion was lost"))?;
        if !Arc::ptr_eq(&previous, &candidate.previous) || !candidate_shape(&previous) {
            return Err(incompatible(
                "recovery no longer owns its actual joined completion",
            ));
        }
        let contract = previous
            .contract
            .as_ref()
            .ok_or_else(|| incompatible("joined contract missing"))?;
        let config = self.configuration.snapshot()?;
        if crate::config::config_revision(&config)?
            != crate::config::config_revision(&candidate.config)?
        {
            return Err(incompatible("recovery configuration changed"));
        }
        let authority = self.current_admission_authority(contract.authority())?;
        contract.authority().compare(&authority)?;
        let WorkerAuthority::Authorized(gate) = &authority else {
            return Err(incompatible("recovery requires authorized child work"));
        };
        let run = candidate.retained.reference();
        let receipts = self.control.operations()?;
        candidate.retained.verify_prior_receipts(&receipts)?;
        self.require_provider_session_clear(&run.account, &run.phase.work)?;
        crate::runtime_schedule::require_provider_clear(
            self.runtime_id,
            &run.account,
            &run.phase.work,
            &receipts,
        )?;
        if snapshot
            .active_operations
            .iter()
            .any(|permit| permit.work_key == run.phase.work)
            || snapshot
                .unresolved_operations
                .values()
                .any(|targets| targets.contains(&run.phase.work))
            || snapshot.pending_local_processes.iter().any(|operation| {
                receipts.iter().any(|receipt| {
                    receipt.operation_id == *operation
                        && receipt.canonical_targets.contains(&run.phase.work)
                })
            })
        {
            return Err(Error::new(
                "reconciliation_required",
                "recovery work has an active or unresolved owner/effect",
            ));
        }
        let projection = self.observations.admission_snapshot()?;
        let grants = crate::runtime_workflow::current_retry_grants(
            &config,
            &projection,
            gate.work(),
            &receipts,
            &self.services.key,
        )?;
        if grants.iter().any(|grant| grant.phase() == &run.phase) {
            return Err(incompatible(
                "recovery budget was superseded by an operator grant",
            ));
        }
        let mut limits = config.runtime.clone();
        if let Some(id) = run.phase.retry_grant {
            let grant = grants
                .iter()
                .find(|grant| grant.record().operation_id == id && grant.phase_key() == run.phase)
                .ok_or_else(|| {
                    incompatible("recovery original retry grant is no longer current")
                })?;
            crate::runtime_workflow::require_current_grant(grant, &grants)?;
            limits.max_attempts_per_phase = limits
                .max_attempts_per_phase
                .min(grant.new_budget().max_attempts);
            limits.max_turns = limits.max_turns.min(grant.new_budget().max_turns);
            limits.max_review_rounds = limits
                .max_review_rounds
                .min(grant.new_budget().max_review_rounds);
        }
        if let Some(paired) = &candidate.paired {
            self.verify_paired_recovery(paired, gate, &config, &receipts, &grants)?;
            if let Some(provider) = provider {
                paired.verify_provider(provider, &receipts)?;
            }
        }
        let model = recovery_model_plan(
            self.runtime_id,
            OriginalRecoveryHistory {
                reference: candidate.retained.reference(),
                predecessor: candidate.previous.operation_id,
                receipt_digest: candidate.retained.admission_digest(),
            },
            &receipts,
            &limits,
            Utc::now(),
        )?;
        let (metadata_attempt, metadata_cap, metadata_time) = metadata_plan(
            &candidate.retained,
            provider,
            &receipts,
            &limits,
            Utc::now(),
        )?;
        let child = gate
            .work()
            .child
            .ok_or_else(|| incompatible("recovery has no child"))?;
        let issue = projection
            .parent(run.workspace, gate.work().parent)?
            .children
            .get(&child)
            .and_then(|child| child.issue.as_ref())
            .ok_or_else(|| incompatible("current child observation is incomplete"))?;
        let ready = ReadyWork {
            canonical_key: run.phase.work.clone(),
            workspace: run.workspace,
            account: run.account.clone(),
            generation: run.phase.generation.clone(),
            state: issue.state_name.trim().to_lowercase(),
            priority: issue.priority.and_then(|p| i32::try_from(p).ok()),
            created_at: issue.created_at,
            kind: WorkKind::Child,
            trigger: Some(Trigger::ScheduledRetry),
            not_before: model.not_before.max(metadata_time),
            gate_blockers: vec![],
        };
        Ok(CandidatePlan {
            model,
            metadata_attempt,
            metadata_cap,
            authority,
            ready,
        })
    }

    pub(crate) async fn advance_session_recovery(
        self: &Arc<Self>,
        lease: &Lease,
        action: &SessionRecoveryAction,
        processes: Arc<ProcessJournal>,
        worker_seconds: u64,
    ) -> Result<()> {
        let (candidate, provider, source) = {
            let mut state = self.state.lock().map_err(|_| lock_error())?;
            let entry = state
                .session_recovery
                .get_mut(&action.candidate)
                .ok_or_else(|| incompatible("same-process recovery capability absent"))?;
            if entry.in_flight.is_some() || entry.completed {
                return Err(Error::new(
                    "operation_active",
                    "recovery stage already owned",
                ));
            }
            entry.in_flight = Some(action.stage);
            (
                entry.candidate.clone(),
                entry.provider.clone(),
                entry.source.clone(),
            )
        };
        let result = self
            .perform_recovery_stage(RecoveryStageContext {
                lease,
                action,
                candidate: &candidate,
                provider: provider.as_ref(),
                source: source.as_ref(),
                processes,
                worker_seconds,
            })
            .await;
        let mut state = self.state.lock().map_err(|_| lock_error())?;
        let entry = state
            .session_recovery
            .get_mut(&action.candidate)
            .ok_or_else(|| incompatible("recovery cache disappeared during owned stage"))?;
        entry.in_flight = None;
        match result {
            Ok(StageProof::Provider(proof)) => {
                entry.provider = Some(*proof);
                entry.error = None;
                Ok(())
            }
            Ok(StageProof::Source(proof)) => {
                entry.source = Some(*proof);
                entry.error = None;
                Ok(())
            }
            Ok(StageProof::Model) => {
                entry.completed = true;
                entry.error = None;
                Ok(())
            }
            Err(error) => {
                entry.error = Some(error.clone());
                Err(error)
            }
        }
    }

    async fn perform_recovery_stage(
        self: &Arc<Self>,
        context: RecoveryStageContext<'_>,
    ) -> Result<StageProof> {
        let RecoveryStageContext {
            lease,
            action,
            candidate,
            provider,
            source,
            processes,
            worker_seconds,
        } = context;
        self.verify_owner(lease)?;
        let snapshot = self.control.snapshot()?;
        running(&snapshot)?;
        let plan = self.plan_recovery_candidate(candidate, provider, &snapshot)?;
        if stage_operation(candidate, action.stage, plan.metadata_cap)? != action.operation
            || action.predecessor != candidate.previous.operation_id
        {
            return Err(incompatible("recovery action identity changed"));
        }
        self.check_worker_capacity(
            candidate.retained.reference().workspace,
            &candidate.retained.reference().account,
            &candidate.retained.reference().phase,
            &plan.ready.state,
        )?;
        let contract = candidate
            .previous
            .contract
            .as_ref()
            .ok_or_else(|| incompatible("joined contract missing"))?;
        let execution = candidate
            .previous
            .execution
            .as_ref()
            .ok_or_else(|| incompatible("joined source missing"))?;
        match action.stage {
            SessionRecoveryStage::Provider { attempt } => {
                if provider.is_some() || attempt != plan.metadata_attempt {
                    return Err(incompatible("provider metadata stage changed"));
                }
                let preparation = RecoveryPreparation {
                    config: candidate.config.clone(),
                    contract: contract.clone(),
                    candidate: candidate.retained.clone(),
                    records: PrivateDir::open(&candidate.records)?,
                    lease: lease.clone(),
                    control: self.control.clone(),
                    host: self.clone(),
                    processes,
                    executions: self.executions.clone(),
                    timeout: std::time::Duration::from_millis(
                        candidate.config.runtime.request_timeout_ms.min(300_000),
                    ),
                    attempt,
                    max_attempts: plan.metadata_cap,
                };
                let current = |snapshot: &ControlSnapshot| {
                    running(snapshot)?;
                    self.verify_owner(lease)?;
                    let current = self.plan_recovery_candidate(candidate, None, snapshot)?;
                    require_plan(&plan, &current)?;
                    Ok(())
                };
                let launch = match &candidate.paired {
                    Some(paired) => RecoveryProviderLaunch::admit_round_checked(
                        preparation,
                        crate::control::RoundRecoveryProvenance {
                            round: paired.plan().reference().clone(),
                            recovery: paired.provenance().clone(),
                        },
                        current,
                    )?,
                    None => RecoveryProviderLaunch::admit_checked(preparation, current)?,
                };
                if launch.operation_id() != action.operation {
                    return Err(incompatible("metadata preparation identity changed"));
                }
                launch
                    .recover()
                    .await
                    .map(Box::new)
                    .map(StageProof::Provider)
            }
            SessionRecoveryStage::Source => {
                let provider =
                    provider.ok_or_else(|| incompatible("source stage lacks provider readback"))?;
                if source.is_some() {
                    return Err(incompatible("source stage already has its proof"));
                }
                let root = self.services.housekeeping.retained_execution_read_root(
                    &candidate.config,
                    &execution.binding,
                    contract.isolation(),
                    &execution.ownership,
                    contract.workflow(),
                )?;
                let binary =
                    kegari_platform::BinaryIdentity::inspect(&execution.git_executable, true)?;
                let scope = LocalProcessPermit::metadata_observed(
                    HostProcessServices {
                        control: self.control.clone(),
                        registration: self.registration.clone(),
                        configuration: self.configuration.clone(),
                        processes,
                        executions: self.executions.clone(),
                    },
                    MetadataRequest {
                        request_id: action.operation,
                        work_key: candidate.retained.reference().phase.work.clone(),
                        roots: vec![root],
                        git_binary: binary,
                    },
                    |snapshot| {
                        running(snapshot)?;
                        self.verify_owner(lease)?;
                        let current =
                            self.plan_recovery_candidate(candidate, Some(provider), snapshot)?;
                        require_plan(&plan, &current)
                    },
                )?;
                let observed = (|| {
                    let git = Git::from_pinned_executable(&execution.git_executable)?
                        .with_host_execution_scope(HostGitExecutionScope {
                            limits: GitLimits::default(),
                            authority: scope.clone(),
                        })?;
                    git.verify_reusable_execution(
                        &execution.ownership,
                        &execution.binding,
                        contract.isolation(),
                    )
                })();
                let summary = match &observed {
                    Ok(proof) => json!({"source_reuse":proof.digest()}),
                    Err(error) => json!({"source_reuse_error":error}),
                };
                scope.finish(summary)?;
                observed.map(Box::new).map(StageProof::Source)
            }
            SessionRecoveryStage::Resume => {
                let provider =
                    provider.ok_or_else(|| incompatible("resume has no actual provider proof"))?;
                let source =
                    source.ok_or_else(|| incompatible("resume has no actual source proof"))?;
                source.revalidate_filesystem()?;
                let reference = candidate.retained.reference();
                let request = LaunchRequest {
                    lease: lease.clone(),
                    operation_id: candidate.model_operation,
                    authority: plan.authority,
                    phase: reference.phase.clone(),
                    source: Some(SourceSelection {
                        repository: execution.binding.repository.id.clone(),
                        revision: execution.binding.revision.clone(),
                    }),
                    delivery: candidate
                        .paired
                        .as_ref()
                        .map(|paired| paired.plan().assignment().clone()),
                    unfinished: None,
                    source_advanced: None,
                    retried_unfinished: None,
                    validation_retry: None,
                    review_retry: None,
                    review_round: None,
                    max_seconds: worker_seconds,
                    max_turns: plan.model.max_turns,
                };
                let worker = match &candidate.paired {
                    Some(paired) => {
                        crate::runtime_workers::prepare_paired_resumed(
                            self.clone(),
                            request,
                            provider.clone(),
                            source.clone(),
                            paired.clone(),
                        )
                        .await?
                    }
                    None => {
                        crate::runtime_workers::prepare_resumed(
                            self.clone(),
                            request,
                            candidate.previous.operation_id,
                            provider.clone(),
                            source.clone(),
                        )
                        .await?
                    }
                };
                worker.run()?;
                Ok(StageProof::Model)
            }
        }
    }
}

enum StageProof {
    Provider(Box<VerifiedProviderRecovery>),
    Source(Box<VerifiedReusableSource>),
    Model,
}
fn candidate_shape(completion: &RunCompletion) -> bool {
    completion.local_exit_confirmed
        && matches!(completion.outcome, RunOutcome::Failed)
        && completion
            .error
            .as_ref()
            .is_some_and(|error| error.code == "turn_incomplete")
        && completion.hook_error.is_none()
        && completion.checks.is_empty()
        && completion.failed_checks.is_empty()
        && completion.execution.is_some()
        && completion.contract.as_ref().is_some_and(|contract| {
            matches!(
                contract.instructions().manifest.role,
                AgentRole::Implementation | AgentRole::Testing | AgentRole::Documentation
            ) && contract.instructions().manifest.phase == "implementation"
                && contract.workflow().front_matter.hooks.before_run.is_none()
                && contract.workflow().front_matter.hooks.after_run.is_none()
        })
}
fn running(snapshot: &ControlSnapshot) -> Result<()> {
    if snapshot.intent.desired != Intent::Running
        || !snapshot.admission_open
        || !snapshot.reconciled
        || snapshot.service_stopping
    {
        return Err(Error::new(
            "admission_closed",
            "new recovery stages require current RUNNING admission",
        ));
    }
    Ok(())
}
fn require_plan(original: &CandidatePlan, current: &CandidatePlan) -> Result<()> {
    if original.model.attempt != current.model.attempt
        || original.model.max_attempts != current.model.max_attempts
        || original.model.max_turns != current.model.max_turns
        || original.model.max_review_rounds != current.model.max_review_rounds
        || original.metadata_attempt != current.metadata_attempt
        || original.metadata_cap != current.metadata_cap
    {
        return Err(incompatible("recovery plan changed before admission"));
    }
    Ok(())
}
fn stage_operation(candidate: &Candidate, stage: SessionRecoveryStage, cap: u32) -> Result<Uuid> {
    match stage {
        SessionRecoveryStage::Provider { attempt } => SessionRecoveryPreparationReference {
            predecessor: candidate.previous.operation_id,
            predecessor_receipt_digest: candidate.retained.admission_digest().into(),
            candidate_digest: candidate.retained.digest().into(),
            workspace: candidate.retained.reference().workspace,
            account: candidate.retained.reference().account.clone(),
            phase: candidate.retained.reference().phase.clone(),
            attempt,
            max_attempts: cap,
        }
        .operation_id(),
        SessionRecoveryStage::Source => recovery_id(&candidate.key, "source"),
        SessionRecoveryStage::Resume => Ok(candidate.model_operation),
    }
}
fn recovery_id(candidate: &str, purpose: &str) -> Result<Uuid> {
    let digest = canonical_digest(
        1,
        &json!({"kind":"native_session_recovery","candidate":candidate,"purpose":purpose}),
    )?;
    let bytes = hex::decode(digest.rsplit(':').next().unwrap_or_default())
        .map_err(|_| incompatible("recovery identity digest invalid"))?;
    let mut id: [u8; 16] = bytes[..16]
        .try_into()
        .map_err(|_| incompatible("recovery identity invalid"))?;
    id[6] = (id[6] & 0x0f) | 0x80;
    id[8] = (id[8] & 0x3f) | 0x80;
    Ok(Uuid::from_bytes(id))
}
pub(crate) fn recovery_wait(error: &Error) -> bool {
    matches!(
        error.code.as_str(),
        "admission_closed"
            | "retry_wait"
            | "continuation_wait"
            | "capacity_wait"
            | "quota_wait"
            | "rate_limited"
    )
}
fn lock_error() -> Error {
    Error::new("runtime_unavailable", "session recovery cache lock failed")
}
fn incompatible(reason: &str) -> Error {
    Error::new("resume_incompatible", reason)
}

// Plain planner inputs are not a resume capability. Production callers derive
// them from the protected candidate and separately require all opaque proofs.
struct OriginalRecoveryHistory<'a> {
    reference: &'a RunReference,
    predecessor: Uuid,
    receipt_digest: &'a str,
}
fn recovery_model_plan(
    runtime: Uuid,
    candidate: OriginalRecoveryHistory<'_>,
    receipts: &[Receipt],
    limits: &kegari_config::RuntimeConfig,
    now: chrono::DateTime<Utc>,
) -> Result<RunSchedule> {
    let original = candidate.reference;
    let mut history = BTreeMap::new();
    let mut caps = (
        limits.max_attempts_per_phase,
        limits.max_turns,
        limits.max_review_rounds,
    );
    for receipt in receipts {
        if let Some(run) = crate::control::run_reference_from_receipt(receipt)?
            && run.phase == original.phase
        {
            if receipt.runtime_id != runtime
                || run.workspace != original.workspace
                || run.account != original.account
                || run.profile_digest != original.profile_digest
                || history.insert(run.attempt, receipt).is_some()
            {
                return Err(incompatible("original phase history changed identity"));
            }
            caps.0 = caps.0.min(run.max_attempts);
            caps.1 = caps.1.min(run.max_turns);
            caps.2 = caps.2.min(run.max_review_rounds);
        }
    }
    let (attempt, prior) = history
        .last_key_value()
        .ok_or_else(|| incompatible("original phase history is absent"))?;
    if history.keys().copied().ne(1..=history.len() as u32)
        || prior.operation_id != candidate.predecessor
        || canonical_digest(1, prior)? != candidate.receipt_digest
    {
        return Err(incompatible(
            "candidate is not the exact latest original phase receipt",
        ));
    }
    let observed = prior.observed_result.as_ref();
    if prior.phase != ReceiptPhase::Reconciled
        || observed.is_none_or(|value| {
            value["local_execution"] != "exited"
                || value["unresolved_effects"]
                    .as_array()
                    .is_none_or(|values| !values.is_empty())
                || value["observation"]["outcome"] != "failed"
                || value["observation"]["local_exit_confirmed"] != true
                || value["observation"]["error"]["code"] != "turn_incomplete"
                || !value["observation"]["hook_error"].is_null()
        })
    {
        return Err(incompatible(
            "candidate is not an actually exited closed terminal failure",
        ));
    }
    if *attempt >= caps.0 || caps.1 == 0 || caps.2 == 0 {
        return Err(Error::new(
            "attempts_exhausted",
            "recovery retains original finite model bounds",
        ));
    }
    let failures = history
        .values()
        .rev()
        .take_while(|receipt| {
            receipt
                .observed_result
                .as_ref()
                .is_some_and(|v| v["observation"]["outcome"] == "failed")
        })
        .count()
        .max(1) as u32;
    let not_before = kegari_scheduler::retry_at(
        prior.updated_at,
        failures,
        limits.retry_cap_ms,
        observed
            .and_then(|v| v.pointer("/observation/error/details/retry_after_ms"))
            .and_then(Value::as_u64),
    )?
    .max(prior.updated_at + Duration::seconds(1));
    if now < not_before {
        return Err(
            Error::new("retry_wait", "recovery waits for original failure backoff")
                .detail(json!({"not_before":not_before})),
        );
    }
    Ok(RunSchedule {
        attempt: attempt + 1,
        max_attempts: caps.0,
        max_turns: caps.1,
        max_review_rounds: caps.2,
        trigger: Trigger::ScheduledRetry,
        not_before,
    })
}

fn metadata_plan(
    candidate: &crate::worker_run::RetainedResumeSession,
    proof: Option<&VerifiedProviderRecovery>,
    receipts: &[Receipt],
    limits: &kegari_config::RuntimeConfig,
    now: chrono::DateTime<Utc>,
) -> Result<(u32, u32, chrono::DateTime<Utc>)> {
    let predecessor: Uuid = candidate
        .state()
        .binding
        .operation_id
        .parse()
        .map_err(|_| incompatible("candidate operation is invalid"))?;
    let mut history = BTreeMap::new();
    let mut cap = limits
        .max_attempts_per_phase
        .min(candidate.reference().max_attempts);
    for receipt in receipts {
        if let Some(reference) = SessionRecoveryPreparationReference::from_receipt(receipt)?
            && reference.predecessor() == predecessor
        {
            if reference.candidate_digest() != candidate.digest()
                || reference.phase() != &candidate.reference().phase
                || reference.account() != candidate.reference().account
                || reference.workspace() != candidate.reference().workspace
                || history.insert(reference.attempt, receipt).is_some()
            {
                return Err(incompatible("metadata history changed original candidate"));
            }
            cap = cap.min(reference.max_attempts);
        }
    }
    if history.keys().copied().ne(1..=history.len() as u32) {
        return Err(incompatible("metadata attempt inventory has gaps"));
    }
    if let Some(proof) = proof {
        let actual = history
            .values()
            .find(|receipt| receipt.operation_id == proof.preparation().operation_id)
            .ok_or_else(|| incompatible("cached metadata proof has no actual completed receipt"))?;
        if proof.observation().retained().digest() != candidate.digest()
            || canonical_digest(1, actual)? != canonical_digest(1, proof.preparation())?
            || actual.phase != ReceiptPhase::Reconciled
            || actual
                .observed_result
                .as_ref()
                .is_none_or(|value| value["observation"]["metadata_recovery"] != "verified")
        {
            return Err(incompatible(
                "cached metadata proof no longer matches its actual completion",
            ));
        }
        return Ok((0, cap, actual.updated_at));
    }
    let mut not_before = chrono::DateTime::<Utc>::UNIX_EPOCH;
    for (attempt, previous) in &history {
        let value = previous.observed_result.as_ref();
        if previous.phase != ReceiptPhase::Reconciled
            || value.is_none_or(|value| {
                value["local_execution"] != "exited"
                    || value["unresolved_effects"]
                        .as_array()
                        .is_none_or(|effects| !effects.is_empty())
                    || value["observation"]["metadata_recovery"] != "failed_no_effect"
                    || value["observation"]["retryable"] != true
            })
        {
            return Err(Error::new(
                "reconciliation_required",
                "metadata stage is successful without its live proof, unknown, or not retryable; it cannot be replayed",
            ));
        }
        not_before = not_before.max(kegari_scheduler::retry_at(
            previous.updated_at,
            *attempt,
            limits.retry_cap_ms,
            value
                .and_then(|value| value.pointer("/observation/error/details/retry_after_ms"))
                .and_then(Value::as_u64),
        )?);
    }
    let attempt = u32::try_from(history.len())
        .map_err(|_| incompatible("metadata attempt overflow"))?
        .checked_add(1)
        .ok_or_else(|| incompatible("metadata attempt overflow"))?;
    if attempt > cap {
        return Err(Error::new(
            "attempts_exhausted",
            "metadata recovery exhausted its original finite budget",
        ));
    }
    if now < not_before {
        return Err(Error::new(
            "retry_wait",
            "metadata recovery waits for bounded failure backoff",
        )
        .detail(json!({"not_before":not_before})));
    }
    Ok((attempt, cap, not_before))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::Lifecycle;
    use crate::control::MutationRequest;
    use crate::control::RunAdmission;
    use kegari_control_api::identity::Actor;

    #[test]
    fn metadata_selection_preserves_model_caps_latest_receipt_and_generic_retry_denial() {
        let temp = tempfile::tempdir().unwrap();
        let runtime = Uuid::new_v4();
        let host = ControlHost::setup(
            PrivateDir::create(&temp.path().canonicalize().unwrap().join("control")).unwrap(),
            runtime,
            Uuid::new_v4(),
        )
        .unwrap();
        host.lifecycle(
            MutationRequest {
                request_id: Uuid::new_v4(),
                runtime_id: runtime,
                actor: Actor {
                    identity: "planner-component".into(),
                    principal: None,
                    roles: ["lifecycle".into()].into(),
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
        host.attach_reconciled(lease.clone()).unwrap();
        let reference = RunReference {
            workspace: Uuid::new_v4(),
            account: "account".into(),
            normalized_state: Some("in progress".into()),
            phase: kegari_types::run::PhaseKey {
                work: "work".into(),
                generation: bytes_digest(b"generation"),
                phase: "implementation".into(),
                retry_grant: None,
            },
            profile_digest: bytes_digest(b"profile"),
            instruction_digest: bytes_digest(b"instructions"),
            max_attempts: 3,
            max_turns: 5,
            max_review_rounds: 2,
            attempt: 1,
        };
        let permit = host
            .admit_run(
                &lease,
                RunAdmission {
                    request_id: Uuid::new_v4(),
                    work_key: reference.phase.work.clone(),
                    scope_digest: bytes_digest(b"scope"),
                    reference: reference.clone(),
                },
                || Ok(()),
            )
            .unwrap();
        // Actual admission/retirement tests the planner only. There is no joined
        // provider, protected candidate or opaque recovery capability here.
        let prior = host.retire_local(permit.operation_id, json!({"outcome":"failed","local_exit_confirmed":true,"hook_error":null,"error":Error::new("turn_incomplete","terminal failure"),"session":{"terminal":"failed","input_required":false,"intervention":null}})).unwrap();
        let digest = canonical_digest(1, &prior).unwrap();
        let mut limits: kegari_config::RuntimeConfig = serde_json::from_value(json!({"id":runtime,"host_id":"fixture","max_agents":1,"max_attempts_per_phase":9,"max_turns":99,"max_review_rounds":9})).unwrap();
        let plan = |receipts: &[Receipt], limits: &kegari_config::RuntimeConfig, now| {
            recovery_model_plan(
                runtime,
                OriginalRecoveryHistory {
                    reference: &reference,
                    predecessor: permit.operation_id,
                    receipt_digest: &digest,
                },
                receipts,
                limits,
                now,
            )
        };
        let receipts = host.operations().unwrap();
        assert_eq!(
            plan(&receipts, &limits, prior.updated_at).unwrap_err().code,
            "retry_wait"
        );
        let due = prior.updated_at + Duration::seconds(20);
        let selected = plan(&receipts, &limits, due).unwrap();
        assert_eq!(
            (
                selected.attempt,
                selected.max_attempts,
                selected.max_turns,
                selected.max_review_rounds
            ),
            (2, 3, 5, 2)
        );
        assert_eq!(
            crate::runtime_schedule::plan(
                &limits,
                &reference.phase,
                reference.workspace,
                &receipts,
                due
            )
            .unwrap_err()
            .code,
            "persistent_blocker",
            "metadata selection must not authorize a generic new model attempt"
        );
        limits.max_turns = 2;
        assert_eq!(plan(&receipts, &limits, due).unwrap().max_turns, 2);
        limits.max_attempts_per_phase = 1;
        assert_eq!(
            plan(&receipts, &limits, due).unwrap_err().code,
            "attempts_exhausted"
        );
        limits.max_attempts_per_phase = 9;
        let mut changed = receipts.clone();
        changed
            .iter_mut()
            .find(|r| r.operation_id == permit.operation_id)
            .unwrap()
            .updated_at += Duration::milliseconds(1);
        assert_eq!(
            plan(&changed, &limits, due).unwrap_err().code,
            "resume_incompatible",
            "whole retained predecessor receipt must remain exact"
        );
        let mut next = reference.clone();
        next.attempt = 2;
        let next = host
            .admit_run(
                &lease,
                RunAdmission {
                    request_id: Uuid::new_v4(),
                    work_key: reference.phase.work.clone(),
                    scope_digest: bytes_digest(b"scope"),
                    reference: next,
                },
                || Ok(()),
            )
            .unwrap();
        assert_eq!(
            plan(&host.operations().unwrap(), &limits, due)
                .unwrap_err()
                .code,
            "resume_incompatible",
            "a later actual admission supersedes the candidate before any metadata launch"
        );
        host.retire_local(next.operation_id, json!({"component_cleanup":true}))
            .unwrap();
    }
}
