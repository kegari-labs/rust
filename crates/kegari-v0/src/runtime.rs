//! Host composition state. All worker callbacks consume current provider proofs
//! and actual process registration; worker summaries never establish delivery.
use crate::authority::IssueKey;
use crate::control::ControlHost;
use crate::control::Lease;
use crate::control::Permit;
use crate::control::RunReference;
use crate::observations::ObservationCache;
use crate::platform::Registration;
use crate::reconcile::DependencyOutcomes;
use crate::reconcile::GatePurpose;
use crate::reconcile::HistoryRow;
use crate::reconcile::ParentDelivery;
pub use crate::runtime_actions::{RuntimeAction, WorkerAction};
pub use crate::runtime_claims::{VerifiedAuthorizedOutcome, VerifiedCoordinationOutcome};
use crate::runtime_host::ConfigurationControl;
use crate::runtime_host::RuntimeDependencies;
use crate::runtime_host::RuntimeInspection;
use crate::scheduler::Quota;
use crate::worker::ExecutionRegistry;
use crate::worker::OutcomeSink;
use crate::worker::WorkerAuthority;
use crate::worker::WorkerContract;
use crate::worker::WorkerHost;
use crate::worker::WorkerOutcome;
use crate::worker_run::OwnedWorkerRun;
use crate::worker_run::RunCompletion;
use chrono::{Duration, Utc};
use kegari_digest::{bytes_digest, canonical_digest};
use kegari_storage::PrivateDir;
use kegari_types::error::{Error, Result};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};
use uuid::Uuid;
#[path = "runtime_evaluation.rs"]
mod evaluation;
pub(crate) use evaluation::{VerifiedEvaluationExit, VerifiedEvaluationSource};
#[path = "runtime_corrections.rs"]
mod correction_transitions;
#[path = "runtime_paired_recovery.rs"]
mod paired_recovery;
#[path = "runtime_parent_retirement.rs"]
mod parent_retirement;
#[path = "runtime_worker_boundary.rs"]
mod worker_boundary;
pub(crate) use parent_retirement::ParentRunRegistrationInput;
#[path = "runtime_parent_reassembly.rs"]
mod parent_reassembly;
#[path = "runtime_released.rs"]
mod released;
#[path = "runtime_recovery.rs"]
mod session_recovery;
#[path = "runtime_resume.rs"]
mod session_resume;
pub(crate) use session_recovery::recovery_wait as session_recovery_wait;
pub(crate) use session_recovery::{SessionRecoveryAction, SessionRecoveryStage};

#[derive(Clone)]
struct RegisteredRun {
    permit: Permit,
    contract: Arc<WorkerContract>,
    budget: RunReference,
    _active: Option<crate::observations::ActiveSnapshot>,
    last_observation: Option<Value>,
    joined: bool,
    parent_origin: Option<Arc<crate::delivery::PreparedParentRunOrigin>>,
    parent_joined: Option<parent_retirement::JoinedParentRetirementCandidate>,
}
#[derive(Default)]
struct State {
    runs: BTreeMap<Uuid, RegisteredRun>,
    prepared_active: BTreeMap<Uuid, crate::observations::ActiveSnapshot>,
    delivery: BTreeMap<IssueKey, ParentDelivery>,
    delivery_snapshots: BTreeMap<IssueKey, crate::delivery::VerifiedDeliverySnapshot>,
    released_completions: BTreeMap<Uuid, crate::runtime_delivery::VerifiedReleaseCompletion>,
    released_recovery_errors: BTreeMap<Uuid, Error>,
    released_inventory_error: Option<Error>,
    known_release_completion_anchors:
        BTreeMap<Uuid, crate::runtime_delivery::ReleaseCompletionAnchor>,
    no_release_completions: BTreeMap<Uuid, crate::runtime_delivery::VerifiedNoReleaseCompletion>,
    no_release_recovery_errors: BTreeMap<Uuid, Error>,
    known_no_release_seals: BTreeMap<Uuid, crate::runtime_delivery::VerifiedNoReleaseCompletion>,
    no_release_sealed_operations: BTreeSet<Uuid>,
    no_release_inventory_error: Option<Error>,
    archived_delivery: BTreeMap<(IssueKey, String), ParentDelivery>,
    explicit_corrections: BTreeMap<Uuid, crate::decisions::VerifiedExplicitCorrection>,
    retry_grants: BTreeMap<Uuid, crate::decisions::VerifiedRetryGrant>,
    unfinished_targets: BTreeMap<String, Arc<crate::delivery::UnfinishedChildWork>>,
    retry_errors: BTreeMap<String, Error>,
    schedule_blockers: BTreeMap<String, Error>,
    completion_errors: BTreeMap<Uuid, Error>,
    parent_retirement_errors: BTreeMap<Uuid, Error>,
    parent_retirement_seals: BTreeMap<Uuid, String>,
    claim_errors: BTreeMap<Uuid, Error>,
    delivery_errors: BTreeMap<String, Error>,
    outcomes: DependencyOutcomes,
    history: Vec<HistoryRow>,
    release_details: BTreeMap<(Uuid, IssueKey), Value>,
    quotas: BTreeMap<String, Quota>,
    rate_observations: BTreeMap<String, Value>,
    resume_proofs: BTreeMap<Uuid, String>,
    admitted_resumes: BTreeMap<Uuid, Arc<crate::worker_run::AdmittedSessionResume>>,
    admitted_rounds: BTreeMap<Uuid, Arc<crate::delivery::VerifiedReviewRoundPlan>>,
    correction_inputs: BTreeMap<(Uuid, u64), (String, String)>,
    correction_transitions: BTreeMap<Uuid, Arc<crate::corrections::VerifiedCorrectionTransition>>,
    correction_errors: BTreeMap<Uuid, Error>,
    correction_wakes: BTreeMap<Uuid, Arc<tokio::sync::Notify>>,
    provider_holds: BTreeMap<Uuid, crate::codex::SessionState>,
    completions: BTreeMap<Uuid, Arc<RunCompletion>>,
    session_recovery: BTreeMap<String, session_recovery::RecoveryEntry>,
    session_recovery_errors: BTreeMap<Uuid, Error>,
}
/// Shared authority and evidence projection for native runtime phases. The
/// orchestration driver registers exact admitted contracts before launching and
/// removes a local claim only after a joined owned worker confirms exit.
pub struct RuntimeCoordinator {
    runtime_id: Uuid,
    pub(crate) configuration: Arc<dyn ConfigurationControl>,
    pub(crate) control: Arc<ControlHost>,
    pub(crate) registration: Arc<Registration>,
    pub(crate) observations: Arc<ObservationCache>,
    pub(crate) executions: Arc<ExecutionRegistry>,
    pub(crate) services: RuntimeDependencies,
    claims: PrivateDir,
    workers: Mutex<BTreeMap<Uuid, OwnedWorkerRun>>,
    state: Mutex<State>,
    evaluations: evaluation::Evaluations,
}
impl RuntimeCoordinator {
    pub fn invalidate_delivery(&self, parent: &IssueKey, error: Error) -> Result<()> {
        invalidate_delivery(&self.control, &self.state, parent, error)
    }
    /// Read current completed classifier outputs after actual local exit. Other
    /// roles and stale generations cannot become correction publication tasks.
    pub fn pending_coordination_outcomes(&self) -> Result<Vec<VerifiedCoordinationOutcome>> {
        let projection = self.observations.admission_snapshot()?;
        let control = self.control.snapshot()?;
        let outcomes = self.dependency_outcomes()?;
        let mut ready = Vec::new();
        let mut errors = BTreeMap::new();
        for receipt in self.control.operations()? {
            let Some(run) = RunReference::from_receipt(&receipt)? else {
                continue;
            };
            if !run.phase.phase.starts_with("correction_classification_")
                || receipt.phase != crate::control::ReceiptPhase::Reconciled
                || receipt
                    .observed_result
                    .as_ref()
                    .and_then(|r| r.get("local_execution"))
                    .and_then(Value::as_str)
                    != Some("exited")
                || control
                    .active_operations
                    .iter()
                    .any(|p| p.work_key == run.phase.work)
                || control
                    .unresolved_operations
                    .values()
                    .any(|targets| targets.contains(&run.phase.work))
            {
                continue;
            }
            let Some(workspace) = projection
                .workspaces
                .get(&run.workspace)
                .filter(|w| w.complete)
            else {
                continue;
            };
            let Some(parent) = workspace
                .parents
                .values()
                .find(|p| p.key.identity().is_ok_and(|k| k == run.phase.work))
            else {
                continue;
            };
            if parent.corrections.pending_ids().is_empty() {
                continue;
            }
            let observed = (|| {
                let current = parent.fresh_gate(
                    None,
                    GatePurpose::Coordination,
                    &outcomes,
                    Utc::now(),
                    Duration::seconds(30),
                )?;
                if canonical_digest(1, &current.work().generation)? != run.phase.generation {
                    return Ok(vec![]);
                }
                self.require_provider_session_clear(&run.account, &run.phase.work)?;
                let records = PrivateDir::open(
                    &self
                        .registration
                        .root()
                        .path()
                        .join("worker-records")
                        .join(receipt.operation_id.to_string()),
                )?;
                let mut claims = Vec::new();
                for proposal in crate::runtime_claims::proposal_ids(&records, &receipt)? {
                    let claim = self.coordination_outcome(receipt.operation_id, proposal)?;
                    if claim.outcome().claim.get("kind").and_then(Value::as_str)
                        != Some("correction_classification")
                    {
                        continue;
                    }
                    if claim
                        .original_authority()
                        .compare_current(&current, Utc::now(), Duration::seconds(30))
                        .is_ok()
                    {
                        claims.push(claim);
                    }
                }
                Ok(claims)
            })();
            match observed {
                Ok(claims) => ready.extend(claims),
                Err(error) => {
                    errors.insert(receipt.operation_id, error);
                }
            }
        }
        self.state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "claim recovery lock failed"))?
            .claim_errors = errors;
        Ok(ready)
    }
    /// Replace readiness only with a freshly hydrated provider/artifact snapshot.
    /// Empty verified snapshots deliberately remove stale cached success.
    pub fn retain_delivery_snapshot(
        &self,
        snapshot: crate::delivery::VerifiedDeliverySnapshot,
    ) -> Result<()> {
        let parent = IssueKey::new(
            &snapshot.work().linear_endpoint,
            snapshot.work().linear_workspace,
            snapshot.work().parent,
        )?;
        self.control.serialize_observation(|_| {
            let config = self.configuration.snapshot()?;
            let projection = self.observations.admission_snapshot()?;
            let expected = snapshot.authority();
            self.configuration
                .authority_fences()?
                .check_workspace(expected.workspace_id())?;
            if snapshot.work().child.is_some()
                || snapshot.work() != expected.work()
                || expected.config_revision() != crate::config::config_revision(&config)?
                || snapshot.observed_at() > Utc::now() + Duration::seconds(5)
                || Utc::now() - snapshot.observed_at() > Duration::seconds(30)
            {
                return Err(Error::new(
                    "stale_result",
                    "delivery snapshot is not current parent evidence",
                ));
            }
            let mut state = self.state.lock().map_err(|_| {
                Error::new("runtime_unavailable", "delivery proof cache lock failed")
            })?;
            let (_, outcomes) = effective_evidence(
                &state,
                &projection,
                &crate::config::config_revision(&config)?,
                &self.configuration.authority_fences()?,
                &config,
                &self.observations,
            );
            let current = projection
                .parent(expected.workspace_id(), snapshot.work().parent)?
                .fresh_gate(
                    None,
                    expected.purpose(),
                    &outcomes,
                    Utc::now(),
                    Duration::seconds(30),
                )?;
            expected.compare_current(&current, Utc::now(), Duration::seconds(30))?;
            state.delivery_errors.remove(&parent.identity()?);
            if let Some(proof) = snapshot.parent_delivery().no_release_required() {
                let record = proof.record();
                let current_parent = projection.parent(expected.workspace_id(), record.work.parent)?;
                let links = snapshot.no_release_link().map(|url| vec![url.to_owned()]).unwrap_or_default();
                if !state.history.iter().any(|row| row.parent == parent
                    && row.parent_reference == current_parent.parent_reference
                    && row.generation == record.work.generation && row.outcome == "no-release-required"
                    && row.links == links)
                {
                    state.history.push(HistoryRow {
                        parent_reference: current_parent.parent_reference.clone(), parent: parent.clone(),
                        outcome: "no-release-required".into(), generation: record.work.generation.clone(),
                        completed_at: record.recorded_at, links,
                    });
                }
                state.release_details.insert((expected.workspace_id(), parent.clone()), json!({
                    "outcome":"no-release-required", "record":record, "record_digest":proof.digest(),
                    "publication":snapshot.no_release_link(), "production_changes":false,
                }));
            }
            state.delivery_snapshots.insert(parent, snapshot);
            // A replaced, missing or stale provider generation cannot continue
            // satisfying another parent's dependencies through an old cache.
            state.delivery_snapshots.retain(|_, proof| {
                projection
                    .workspaces
                    .get(&proof.authority().workspace_id())
                    .filter(|workspace| workspace.complete)
                    .and_then(|workspace| workspace.parents.get(&proof.work().parent))
                    .is_some_and(|parent| parent.work.as_ref() == Some(proof.work()))
                    && proof.authority().config_revision() == projection.config_revision
                    && Utc::now() - proof.observed_at() <= Duration::seconds(30)
            });
            let mut delivery = BTreeMap::new();
            let mut outcomes = DependencyOutcomes::default();
            for (parent, proof) in &state.delivery_snapshots {
                delivery.insert(parent.clone(), proof.parent_delivery().clone());
                outcomes.merge(proof.outcomes());
            }
            state.delivery = delivery;
            state.outcomes = outcomes;
            Ok(())
        })
    }
    pub fn authorized_outcome(
        &self,
        operation: Uuid,
        proposal: Uuid,
    ) -> Result<VerifiedAuthorizedOutcome> {
        let admission = self.control.operation(operation)?;
        let records = PrivateDir::open(
            &self
                .registration
                .root()
                .path()
                .join("worker-records")
                .join(operation.to_string()),
        )?;
        crate::runtime_claims::recover_authorized(
            &self.services.key,
            &self.claims,
            &records,
            self.runtime_id,
            admission,
            proposal,
        )
    }
    pub fn ready_actions(&self) -> Result<Vec<RuntimeAction>> {
        let config = self.configuration.snapshot()?;
        let projection = self.observations.admission_snapshot()?;
        self.refresh_retry_references(&projection)?;
        let (outcomes, delivery, grants) = {
            let state = self
                .state
                .lock()
                .map_err(|_| Error::new("runtime_unavailable", "action projection lock failed"))?;
            let (delivery, outcomes) = effective_evidence(
                &state,
                &projection,
                &crate::config::config_revision(&config)?,
                &self.configuration.authority_fences()?,
                &config,
                &self.observations,
            );
            (outcomes, delivery, state.retry_grants.clone())
        };
        let selected = crate::runtime_actions::select(
            crate::runtime_actions::SelectionInput {
                config: &config,
                projection: &projection,
                outcomes: &outcomes,
                delivery: &delivery,
                grants: &grants,
            },
            |phase, workspace, account| self.next_attempt(phase, workspace, account),
        )?;
        self.state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "action projection lock failed"))?
            .schedule_blockers = selected.blockers;
        Ok(selected.actions)
    }
    /// Recover host-recorded provenance without adopting a worker's completion
    /// claim as workflow success or granting a fresh mutation permit.
    pub fn coordination_outcome(
        &self,
        operation: Uuid,
        proposal: Uuid,
    ) -> Result<VerifiedCoordinationOutcome> {
        let admission = self.control.operation(operation)?;
        let records = PrivateDir::open(
            &self
                .registration
                .root()
                .path()
                .join("worker-records")
                .join(operation.to_string()),
        )?;
        crate::runtime_claims::recover_coordination(
            &self.services.key,
            &self.claims,
            &records,
            self.runtime_id,
            admission,
            proposal,
        )
    }
    pub fn new(services: RuntimeDependencies) -> Result<Arc<Self>> {
        let config = services.configuration.snapshot()?;
        if services.control.snapshot()?.intent.runtime_id != config.runtime.id
            || services.observations.snapshot()?.runtime_id != config.runtime.id
        {
            return Err(Error::new(
                "identity_mismatch",
                "runtime services identify different registrations",
            ));
        }
        let claims = services.registration.root().child("worker-claims")?;
        Ok(Arc::new(Self {
            runtime_id: config.runtime.id,
            configuration: services.configuration.clone(),
            control: services.control.clone(),
            registration: services.registration.clone(),
            observations: services.observations.clone(),
            executions: Arc::new(ExecutionRegistry::default()),
            services,
            claims,
            workers: Mutex::new(BTreeMap::new()),
            state: Mutex::new(State::default()),
            evaluations: evaluation::Evaluations::default(),
        }))
    }
    pub fn next_attempt(
        &self,
        phase: &crate::scheduler::PhaseKey,
        workspace: Uuid,
        account: &str,
    ) -> Result<crate::runtime_schedule::RunSchedule> {
        self.plan_attempt(phase, workspace, account, None, None)
    }
    /// Revalidate the exact selected cause under the preparation admission lock.
    /// The supplied snapshot avoids reentering ControlHost; no permit exists yet.
    pub(crate) fn plan_worker_preparation(
        &self,
        request: &crate::runtime_workers::LaunchRequest,
        workspace: Uuid,
        account: &str,
        snapshot: &crate::control::ControlSnapshot,
    ) -> Result<crate::runtime_schedule::RunSchedule> {
        request.validate_binding()?;
        self.check_evaluation_worker(
            snapshot,
            request.operation_id,
            &request.authority,
            &request.phase,
            request.max_turns,
        )?;
        self.registration.verify_active_owner(&request.lease)?;
        let current = self.current_admission_authority(&request.authority)?;
        request.authority.compare(&current)?;
        if authority_settings(&current).0 != workspace {
            return Err(Error::new(
                "worker_binding",
                "preparation workspace changed",
            ));
        }
        if let WorkerAuthority::Authorized(gate) = &current {
            let grants = crate::runtime_workflow::current_retry_grants(
                &self.configuration.snapshot()?,
                &self.observations.admission_snapshot()?,
                gate.work(),
                &self.control.operations()?,
                &self.services.key,
            )?;
            if grants.iter().any(|grant| grant.phase() == &request.phase) {
                return Err(Error::new(
                    "stale_generation",
                    "preparation budget was superseded by an explicit retry grant",
                ));
            }
        }
        if let Some(proof) = request
            .delivery
            .as_ref()
            .and_then(|a| a.parent_reassembly())
        {
            return self.schedule_parent_reassembly(proof, workspace, account, Some(snapshot));
        }
        let cause = request_readmission(request)?;
        self.plan_attempt_context(
            &request.phase,
            workspace,
            account,
            WorkerPlanning {
                readmission: cause,
                round: request.review_round.as_deref(),
            },
            Some(snapshot),
        )
    }
    pub fn next_attempt_for_unfinished(
        &self,
        proof: &crate::delivery::UnfinishedChildWork,
    ) -> Result<crate::runtime_schedule::RunSchedule> {
        self.next_attempt_for_readmission(WorkerReadmission::Unfinished(proof))
    }
    pub fn next_attempt_for_source_advanced(
        &self,
        proof: &crate::delivery::VerifiedSourceAdvanced,
    ) -> Result<crate::runtime_schedule::RunSchedule> {
        self.next_attempt_for_readmission(WorkerReadmission::SourceAdvanced(proof))
    }
    pub fn next_attempt_for_retried_unfinished(
        &self,
        proof: &crate::delivery::RetriedUnfinishedWork,
    ) -> Result<crate::runtime_schedule::RunSchedule> {
        self.next_attempt_for_readmission(WorkerReadmission::Retried(proof))
    }
    pub fn next_attempt_for_review_retry(
        &self,
        proof: &crate::delivery::ReviewRetryWork,
    ) -> Result<crate::runtime_schedule::RunSchedule> {
        self.next_attempt_for_readmission(WorkerReadmission::ReviewRetry(proof))
    }
    pub(crate) fn next_attempt_for_validation_retry(
        &self,
        request: &crate::runtime_workers::LaunchRequest,
    ) -> Result<crate::runtime_schedule::RunSchedule> {
        request.validate_binding()?;
        let proof = request.validation_retry.as_deref().ok_or_else(|| {
            Error::new(
                "worker_binding",
                "validation retry scheduling requires its actual opaque cause",
            )
        })?;
        self.next_attempt_for_readmission(WorkerReadmission::ValidationRetry(proof))
    }
    fn next_attempt_for_readmission(
        &self,
        proof: WorkerReadmission<'_>,
    ) -> Result<crate::runtime_schedule::RunSchedule> {
        let config = self.configuration.snapshot()?;
        let workspace = config
            .workspaces
            .values()
            .find(|workspace| workspace.id == proof.authority().workspace_id())
            .ok_or_else(|| Error::new("scope_violation", "readmission workspace disappeared"))?;
        let account = &config.connections[&workspace.codex_connection].account_id;
        self.plan_attempt(proof.phase(), workspace.id, account, Some(proof), None)
    }
    pub(crate) fn next_attempt_for_review_round(
        &self,
        request: &crate::runtime_workers::LaunchRequest,
    ) -> Result<crate::runtime_schedule::RunSchedule> {
        request.validate_binding()?;
        let round = request.review_round.as_deref().ok_or_else(|| {
            Error::new(
                "worker_binding",
                "round planning requires the actual opaque round plan",
            )
        })?;
        let workspace = round.authority().workspace_id();
        let config = self.configuration.snapshot()?;
        let ws = config
            .workspaces
            .values()
            .find(|ws| ws.id == workspace)
            .ok_or_else(|| Error::new("scope_violation", "round workspace disappeared"))?;
        self.plan_attempt_context(
            &request.phase,
            workspace,
            &config.connections[&ws.codex_connection].account_id,
            WorkerPlanning {
                readmission: request_readmission(request)?,
                round: Some(round),
            },
            None,
        )
    }
    fn schedule_review_round(
        &self,
        limits: &kegari_config::RuntimeConfig,
        receipts: &[crate::control::Receipt],
        round: &crate::delivery::VerifiedReviewRoundPlan,
        readmission: Option<WorkerReadmission<'_>>,
    ) -> Result<crate::runtime_schedule::RunSchedule> {
        let config = self.configuration.snapshot()?;
        // A paired repair ignores an unrelated implementation-captured review
        // cap, but must still fit the current configured review namespace.
        if round.reference().ordinal > config.runtime.max_review_rounds {
            return Err(Error::new(
                "review_rounds_exhausted",
                "round exceeds the current configured review bound",
            ));
        }

        let current = self
            .current_admission_authority(&WorkerAuthority::Authorized(round.authority().clone()))?;
        WorkerAuthority::Authorized(round.authority().clone()).compare(&current)?;
        let grants = crate::runtime_workflow::current_retry_grants(
            &config,
            &self.observations.admission_snapshot()?,
            round.authority().work(),
            receipts,
            &self.services.key,
        )?;
        round.verify_joined(&|operation| self.retained_completion(operation))?;
        round.verify_resumed_bridges(&|operation| self.retained_session_resume(operation))?;
        let cause = round_run_cause(readmission)?;
        crate::runtime_schedule::plan_review_round(crate::runtime_schedule::RoundScheduleInput {
            runtime: limits,
            phase: round.subject_phase(),
            workspace: round.authority().workspace_id(),
            receipts,
            grants: &grants,
            proof: round,
            cause: &cause,
            review_retry: match readmission {
                Some(WorkerReadmission::ReviewRetry(proof)) => Some(proof),
                _ => None,
            },
            now: Utc::now(),
        })
    }
    fn plan_attempt(
        &self,
        phase: &crate::scheduler::PhaseKey,
        workspace: Uuid,
        account: &str,
        readmission: Option<WorkerReadmission<'_>>,
        locked_snapshot: Option<&crate::control::ControlSnapshot>,
    ) -> Result<crate::runtime_schedule::RunSchedule> {
        self.plan_attempt_context(
            phase,
            workspace,
            account,
            WorkerPlanning {
                readmission,
                round: None,
            },
            locked_snapshot,
        )
    }
    fn plan_attempt_context(
        &self,
        phase: &crate::scheduler::PhaseKey,
        workspace: Uuid,
        account: &str,
        planning: WorkerPlanning<'_>,
        locked_snapshot: Option<&crate::control::ControlSnapshot>,
    ) -> Result<crate::runtime_schedule::RunSchedule> {
        let WorkerPlanning { readmission, round } = planning;
        if round.is_none()
            && match readmission {
                Some(WorkerReadmission::Unfinished(proof)) => proof.requires_review_round(),
                Some(WorkerReadmission::SourceAdvanced(proof)) => proof.requires_review_round(),
                Some(WorkerReadmission::ReviewRetry(proof)) => proof.requires_review_round(),
                _ => false,
            }
        {
            return Err(Error::new(
                "worker_binding",
                "actual common review lineage requires its opaque round plan",
            ));
        }
        let config = self.configuration.snapshot()?;
        let ws = config
            .workspaces
            .values()
            .find(|item| item.id == workspace)
            .ok_or_else(|| {
                Error::new(
                    "scope_violation",
                    "worker workspace is no longer configured",
                )
            })?;
        if config.connections[&ws.codex_connection].account_id != account {
            return Err(Error::new(
                "identity_mismatch",
                "worker account differs from the selected workspace connection",
            ));
        }
        let observed;
        let state = match locked_snapshot {
            Some(state) => state,
            None => {
                observed = self.control.snapshot()?;
                &observed
            }
        };
        if state
            .active_operations
            .iter()
            .any(|permit| permit.work_key == phase.work)
            || state
                .unresolved_operations
                .values()
                .any(|targets| targets.contains(&phase.work))
        {
            return Err(Error::new(
                "reconciliation_required",
                "canonical work has an active or unresolved operation",
            ));
        }
        let receipts = self.control.operations()?;
        if phase.phase == "child_local_validation" && readmission.is_none() {
            let repeated = receipts
                .iter()
                .try_fold(false, |seen, receipt| -> Result<bool> {
                    Ok(seen
                        || RunReference::from_receipt(receipt)?
                            .is_some_and(|run| run.phase == *phase))
                })?;
            if phase.retry_grant.is_some() || repeated {
                return Err(Error::new(
                    "validation_retry_required",
                    "repeated validation requires its exact source advancement or failed-session retry capability",
                ));
            }
        }
        let mut limits = config.runtime.clone();
        self.require_provider_session_clear(account, &phase.work)?;
        if let Some(error) = self
            .state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "retry proof lock failed"))?
            .retry_errors
            .get(&phase.work)
        {
            return Err(error.clone());
        }
        if let Some(grant_id) = phase.retry_grant {
            let state = self
                .state
                .lock()
                .map_err(|_| Error::new("runtime_unavailable", "retry proof lock failed"))?;
            let grant = state.retry_grants.get(&grant_id).cloned().ok_or_else(|| {
                Error::new(
                    "reconciliation_required",
                    "retry phase has no verified published grant",
                )
            })?;
            drop(state);
            if grant.phase_key() != *phase
                || grant.record().workspace_id != workspace
                || grant.record().runtime_id != config.runtime.id
            {
                return Err(Error::new(
                    "scope_violation",
                    "retry grant differs from selected work phase",
                ));
            }
            let projection = self.observations.admission_snapshot()?;
            let observed = crate::runtime_workflow::current_retry_grants(
                &config,
                &projection,
                grant.work(),
                &receipts,
                &self.services.key,
            )?;
            crate::runtime_workflow::require_current_grant(&grant, &observed)?;
            if phase.phase == "child_codex_review"
                && round.is_none()
                && !matches!(readmission, Some(WorkerReadmission::ReviewRetry(_)))
                && !matches!(readmission, Some(WorkerReadmission::SourceAdvanced(proof)) if proof.has_granted_review())
            {
                return Err(Error::new(
                    "review_retry_required",
                    "granted review requires its exact retained failed-session and source assignment",
                ));
            }
            if crate::runtime_schedule::grant_requires_unfinished(grant.phase(), phase, &receipts)?
                && !matches!(readmission, Some(WorkerReadmission::Retried(_)))
            {
                return Err(Error::new(
                    "unfinished_retry_required",
                    "completed budget retry requires its verified retained repair assignment",
                ));
            }
            if let Some(WorkerReadmission::Retried(proof)) = readmission {
                proof.verify_current(&receipts, &observed)?;
            }
            let parent = projection.parent(workspace, grant.work().parent)?;
            let current = grant
                .work()
                .child
                .and_then(|child| parent.children.get(&child))
                .and_then(|child| child.work.as_ref())
                .or_else(|| {
                    if grant.work().child.is_none() {
                        parent.work.as_ref()
                    } else {
                        None
                    }
                });
            if current != Some(grant.work()) {
                return Err(Error::new(
                    "stale_result",
                    "retry grant no longer binds current work",
                ));
            }
            limits.max_attempts_per_phase = limits
                .max_attempts_per_phase
                .min(grant.new_budget().max_attempts);
            limits.max_turns = limits.max_turns.min(grant.new_budget().max_turns);
            limits.max_review_rounds = limits
                .max_review_rounds
                .min(grant.new_budget().max_review_rounds);
        }
        crate::runtime_schedule::require_provider_clear(
            config.runtime.id,
            account,
            &phase.work,
            &receipts,
        )?;
        if let Some(proof) = readmission {
            proof.verify_inventory(&receipts)?;
            let current_grants = crate::runtime_workflow::current_retry_grants(
                &config,
                &self.observations.admission_snapshot()?,
                proof.authority().work(),
                &receipts,
                &self.services.key,
            )?;
            if current_grants.iter().any(|grant| grant.phase() == phase) {
                return Err(Error::new(
                    "stale_generation",
                    "readmission budget was superseded by a current published grant",
                ));
            }
            if let WorkerReadmission::Retried(proof) = proof {
                self.verify_unfinished_target(proof.original(), &receipts)?;
            }
            match proof {
                WorkerReadmission::ReviewRetry(proof) => {
                    proof.verify_current(&receipts, &current_grants)?
                }
                WorkerReadmission::ValidationRetry(proof) => {
                    proof.verify_current(&receipts, &current_grants)?;
                    proof.verify_joined(&|id| self.retained_completion(id))?;
                    proof.verify_resumed_bridges(&|id| self.retained_session_resume(id))?;
                }
                WorkerReadmission::Unfinished(proof) => {
                    proof.verify_current(&receipts, &current_grants)?;
                    proof.verify_resumed_bridges(&|id| self.retained_session_resume(id))?;
                }
                WorkerReadmission::SourceAdvanced(proof) => {
                    proof.verify_current(&receipts, &current_grants)?;
                    proof.verify_joined(&|id| self.retained_completion(id))?;
                    proof.verify_resumed_bridges(&|id| self.retained_session_resume(id))?;
                }
                WorkerReadmission::Retried(proof) => {
                    proof.verify_current(&receipts, &current_grants)?;
                    proof.verify_resumed_bridges(&|id| self.retained_session_resume(id))?;
                }
            }
            if proof.phase() != phase
                || proof.authority().workspace_id() != workspace
                || proof.authority().work_key()? != phase.work
            {
                return Err(Error::new(
                    "worker_binding",
                    "readmission differs from its original phase and current child",
                ));
            }
            self.current_admission_authority(&WorkerAuthority::Authorized(
                proof.authority().clone(),
            ))?;
            let failure = receipts
                .iter()
                .find(|receipt| receipt.operation_id == proof.cause_operation())
                .ok_or_else(|| {
                    Error::new(
                        "reconciliation_required",
                        "readmission lost its actual source or failure admission",
                    )
                })?;
            if canonical_digest(1, failure)? != proof.cause_receipt_digest() {
                return Err(Error::new(
                    "stale_result",
                    "readmission no longer matches its exact cause receipt",
                ));
            }
            let predecessor = receipts
                .iter()
                .find(|receipt| receipt.operation_id == proof.predecessor())
                .ok_or_else(|| {
                    Error::new(
                        "reconciliation_required",
                        "readmission predecessor is unavailable",
                    )
                })?;
            let prior_run = RunReference::from_receipt(predecessor)?.ok_or_else(|| {
                Error::new("worker_binding", "readmission predecessor lacks a run")
            })?;
            if prior_run.account != account {
                return Err(Error::new(
                    "identity_mismatch",
                    "readmission cannot replace the original account",
                ));
            }
            proof.source().revalidate_filesystem()?;
            let completed = self
                .retained_completion(proof.completed_operation())?
                .ok_or_else(|| {
                    Error::new(
                        "reconciliation_required",
                        "readmission lost its actual joined completion",
                    )
                })?;
            match proof {
                WorkerReadmission::ReviewRetry(proof) => proof.verify_completion(&completed)?,
                WorkerReadmission::ValidationRetry(proof) => proof.verify_completion(&completed)?,
                _ => require_joined_readmission(&completed)?,
            }
            if let Some(round) = round {
                return self.schedule_review_round(&limits, &receipts, round, Some(proof));
            }
            match proof {
                WorkerReadmission::Unfinished(proof) => crate::runtime_schedule::plan_unfinished(
                    &limits,
                    phase,
                    workspace,
                    &receipts,
                    &continuation_reference(proof),
                    Utc::now(),
                ),
                WorkerReadmission::SourceAdvanced(proof) => {
                    crate::runtime_schedule::plan_source_advanced(
                        &limits,
                        phase,
                        workspace,
                        &receipts,
                        &source_advanced_reference(proof),
                        Utc::now(),
                    )
                }
                WorkerReadmission::ReviewRetry(proof) => {
                    crate::runtime_schedule::plan_review_retry(
                        &limits,
                        phase,
                        workspace,
                        &receipts,
                        proof,
                        Utc::now(),
                    )
                }
                WorkerReadmission::ValidationRetry(proof) => {
                    crate::runtime_schedule::plan_validation_retry(
                        &limits,
                        phase,
                        workspace,
                        &receipts,
                        proof,
                        Utc::now(),
                    )
                }
                WorkerReadmission::Retried(_) => {
                    let now = Utc::now();
                    let not_before = readmission_spacing(predecessor, failure, now)?;
                    let mut schedule =
                        crate::runtime_schedule::plan(&limits, phase, workspace, &receipts, now)?;
                    schedule.not_before = schedule.not_before.max(not_before);
                    Ok(schedule)
                }
            }
        } else if let Some(round) = round {
            self.schedule_review_round(&limits, &receipts, round, None)
        } else {
            crate::runtime_schedule::plan(&limits, phase, workspace, &receipts, Utc::now())
        }
    }
    pub(crate) fn retain_retry_grant(
        &self,
        grant: crate::decisions::VerifiedRetryGrant,
    ) -> Result<()> {
        if grant.record().runtime_id != self.runtime_id {
            return Err(Error::new(
                "identity_mismatch",
                "retry belongs to another runtime",
            ));
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "retry proof lock failed"))?;
        let id = grant.record().operation_id;
        if state
            .retry_grants
            .get(&id)
            .is_some_and(|prior| prior.digest() != grant.digest())
        {
            return Err(Error::new(
                "ownership_conflict",
                "retry grant publication changed",
            ));
        }
        state.retry_grants.insert(id, grant);
        self.services.wake.notify_one();
        Ok(())
    }
    /// Cache-only accessors are also used inside the serialized decision gate.
    /// They must never reacquire ControlHost or perform provider reads.
    pub fn current_delivery(&self, parent: &IssueKey) -> Result<ParentDelivery> {
        let projection = self.observations.admission_snapshot()?;
        let config = self.configuration.snapshot()?;
        let revision = crate::config::config_revision(&config)?;
        let state = self
            .state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "evidence cache lock failed"))?;
        Ok(effective_evidence(
            &state,
            &projection,
            &revision,
            &self.configuration.authority_fences()?,
            &config,
            &self.observations,
        )
        .0
        .remove(parent)
        .unwrap_or_default())
    }
    pub fn dependency_outcomes(&self) -> Result<DependencyOutcomes> {
        let projection = self.observations.admission_snapshot()?;
        let config = self.configuration.snapshot()?;
        let revision = crate::config::config_revision(&config)?;
        let state = self
            .state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "dependency cache lock failed"))?;
        Ok(effective_evidence(
            &state,
            &projection,
            &revision,
            &self.configuration.authority_fences()?,
            &config,
            &self.observations,
        )
        .1)
    }
    pub(crate) fn retain_acceptance(
        &self,
        parent: &IssueKey,
        acceptance: crate::evidence::VerifiedAcceptance,
    ) -> Result<()> {
        let work = &acceptance.acceptance().work;
        if acceptance.acceptance().runtime_id != self.runtime_id
            || work.child.is_some()
            || IssueKey::new(&work.linear_endpoint, work.linear_workspace, work.parent)? != *parent
        {
            return Err(Error::new(
                "identity_mismatch",
                "acceptance differs from its parent/runtime",
            ));
        }
        let projection = self.observations.admission_snapshot()?;
        let config = self.configuration.snapshot()?;
        let revision = crate::config::config_revision(&config)?;
        let fences = self.configuration.authority_fences()?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "acceptance cache lock failed"))?;
        let proof = state.delivery_snapshots.get_mut(parent).ok_or_else(|| {
            Error::new(
                "evidence_missing",
                "parent evidence has not been freshly hydrated",
            )
        })?;
        if !evidence_projection_current(
            proof.authority(),
            proof.observed_at(),
            &projection,
            &revision,
            &fences,
            Utc::now(),
        ) {
            return Err(Error::new(
                "stale_result",
                "acceptance cannot update a stale or permission-fenced evidence snapshot",
            ));
        }
        // This records a verified decision against the existing evidence token;
        // it never refreshes the provider observation's age.
        proof.acceptance(acceptance)?;
        let delivery = proof.parent_delivery().clone();
        state.delivery.insert(parent.clone(), delivery);
        drop(state);
        self.services.wake.notify_one();
        Ok(())
    }
    pub(crate) fn retain_carried_acceptance(
        &self,
        parent: &IssueKey,
        carry: crate::delivery::PublishedCarry,
    ) -> Result<()> {
        self.control.serialize_observation(|_| {
            let projection = self.observations.admission_snapshot()?;
            let config = self.configuration.snapshot()?;
            // Local runtime workspace and provider workspace are distinct.
            let workspace = carry.workspace_id();
            let current = projection.parent(workspace, parent.issue)?;
            if current.key != *parent {
                return Err(Error::new(
                    "identity_mismatch",
                    "carry parent identity differs",
                ));
            }
            carry.verify_current_projection(&config, current)?;
            let fences = self.configuration.authority_fences()?;
            let mut state = self
                .state
                .lock()
                .map_err(|_| Error::new("runtime_unavailable", "carry cache lock failed"))?;
            let snapshot = state.delivery_snapshots.get_mut(parent).ok_or_else(|| {
                Error::new(
                    "evidence_missing",
                    "carry needs actual freshly hydrated parent evidence",
                )
            })?;
            if !evidence_projection_current(
                snapshot.authority(),
                snapshot.observed_at(),
                &projection,
                &crate::config::config_revision(&config)?,
                &fences,
                Utc::now(),
            ) {
                return Err(Error::new(
                    "stale_result",
                    "carry cannot refresh stale current evidence",
                ));
            }
            snapshot.carried_acceptance(carry)?;
            let delivery = snapshot.parent_delivery().clone();
            state.delivery.insert(parent.clone(), delivery);
            Ok(())
        })
    }
    /// Attach freshly verified read-only release readiness to the same current
    /// parent snapshot. This grants no dispatch authority and cannot renew the
    /// snapshot's provider observation time.
    pub(crate) fn retain_release_eligibility(
        &self,
        parent: &IssueKey,
        eligibility: crate::release::VerifiedReleaseEligibility,
    ) -> Result<()> {
        let projection = self.observations.admission_snapshot()?;
        let config = self.configuration.snapshot()?;
        let revision = crate::config::config_revision(&config)?;
        let fences = self.configuration.authority_fences()?;
        let now = Utc::now();
        if eligibility.checked_at() > now + Duration::seconds(5)
            || now - eligibility.checked_at() > Duration::seconds(30)
        {
            return Err(Error::new(
                "stale_result",
                "release eligibility observation expired",
            ));
        }
        let mut state = self.state.lock().map_err(|_| {
            Error::new(
                "runtime_unavailable",
                "release eligibility cache lock failed",
            )
        })?;
        let snapshot = state.delivery_snapshots.get_mut(parent).ok_or_else(|| {
            Error::new(
                "evidence_missing",
                "release readiness needs the freshly hydrated parent snapshot",
            )
        })?;
        let work = snapshot.work();
        if work.child.is_some()
            || IssueKey::new(&work.linear_endpoint, work.linear_workspace, work.parent)? != *parent
            || !evidence_projection_current(
                snapshot.authority(),
                snapshot.observed_at(),
                &projection,
                &revision,
                &fences,
                now,
            )
        {
            return Err(Error::new(
                "stale_result",
                "release readiness cannot update stale or permission-fenced evidence",
            ));
        }
        snapshot.release(eligibility)?;
        let delivery = snapshot.parent_delivery().clone();
        state.delivery.insert(parent.clone(), delivery);
        drop(state);
        self.services.wake.notify_one();
        Ok(())
    }
    pub(crate) fn retain_correction(
        &self,
        correction: crate::decisions::VerifiedExplicitCorrection,
    ) -> Result<()> {
        if correction.record().runtime_id != self.runtime_id {
            return Err(Error::new(
                "identity_mismatch",
                "correction belongs to another runtime",
            ));
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "correction cache lock failed"))?;
        let id = correction.record().operation_id;
        if let Some(prior) = state.explicit_corrections.get(&id) {
            if prior.body_digest() != correction.body_digest()
                || prior.publication().record_id != correction.publication().record_id
            {
                return Err(Error::new(
                    "ownership_conflict",
                    "correction publication changed",
                ));
            }
            return Ok(());
        }
        let parent = correction.record().parent.clone();
        state.delivery_snapshots.remove(&parent);
        let mut outcomes = DependencyOutcomes::default();
        for proof in state.delivery_snapshots.values() {
            outcomes.merge(proof.outcomes());
        }
        state.outcomes = outcomes;
        if let Some(prior) = state.delivery.remove(&parent) {
            let mut remaining = ParentDelivery::default();
            for child in prior.verified_children().values() {
                remaining.child(child.clone())?;
            }
            let token = prior
                .verified_parent()
                .map(|p| p.token().to_owned())
                .unwrap_or_else(|| correction.body_digest().to_owned());
            state
                .archived_delivery
                .insert((parent.clone(), token), prior);
            state.delivery.insert(parent, remaining);
        }
        state.explicit_corrections.insert(id, correction);
        self.services.wake.notify_one();
        Ok(())
    }
    /// Retain only an actual delivery-produced obligation. Publication or a
    /// worker summary cannot construct this capability. The producing host
    /// phase may still be retiring; selection separately requires idle work.
    pub(crate) fn retain_unfinished_target(
        &self,
        proof: Arc<crate::delivery::UnfinishedChildWork>,
    ) -> Result<()> {
        self.control.serialize_observation(|_| {
            self.verify_unfinished_target(&proof, &self.control.operations()?)?;
            let mut state = self
                .state
                .lock()
                .map_err(|_| Error::new("runtime_unavailable", "unfinished target lock failed"))?;
            if !state.unfinished_targets.contains_key(proof.digest())
                && state.unfinished_targets.len() >= 4096
            {
                return Err(Error::new(
                    "resource_limit",
                    "unfinished target retention reached its finite bound",
                ));
            }
            state
                .unfinished_targets
                .insert(proof.digest().into(), proof);
            Ok(())
        })
    }
    pub(crate) fn verified_unfinished_targets(
        &self,
        work: &crate::evidence::WorkIdentity,
    ) -> Result<Vec<Arc<crate::delivery::UnfinishedChildWork>>> {
        // DecisionManager also calls this within its serialized publication
        // gate. It checks live ownership/holds using the supplied snapshot;
        // this capability reader must never reenter ControlHost's mutex.
        let receipts = self.control.operations()?;
        let proofs: Vec<_> = self
            .state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "unfinished target lock failed"))?
            .unfinished_targets
            .values()
            .filter(|proof| proof.authority().work() == work)
            .cloned()
            .collect();
        let mut current = Vec::new();
        for proof in proofs {
            match self.verify_unfinished_target(&proof, &receipts) {
                Ok(()) => current.push(proof),
                // A newer actual result may obsolete a retained historical
                // obligation. It supplies no current retry authority.
                Err(error) if error.code == "stale_result" => (),
                Err(error) => return Err(error),
            }
        }
        Ok(current)
    }
    fn verify_unfinished_target(
        &self,
        proof: &crate::delivery::UnfinishedChildWork,
        receipts: &[crate::control::Receipt],
    ) -> Result<()> {
        if proof.predecessor_receipt().runtime_id != self.runtime_id {
            return Err(Error::new(
                "identity_mismatch",
                "unfinished target belongs to another runtime",
            ));
        }
        proof.verify_retry_basis(receipts)?;
        self.current_admission_authority(&WorkerAuthority::Authorized(proof.authority().clone()))?;
        let config = self.configuration.snapshot()?;
        let grants = crate::runtime_workflow::current_retry_grants(
            &config,
            &self.observations.admission_snapshot()?,
            proof.authority().work(),
            receipts,
            &self.services.key,
        )?;
        proof.verify_current(receipts, &grants)?;
        let workspace = config
            .workspaces
            .values()
            .find(|w| w.id == proof.authority().workspace_id())
            .ok_or_else(|| Error::new("scope_violation", "unfinished workspace disappeared"))?;
        let account = &config.connections[&workspace.codex_connection].account_id;
        if account != &proof.reference().account {
            return Err(Error::new(
                "identity_mismatch",
                "unfinished target account changed",
            ));
        }
        self.require_provider_session_clear(account, &proof.phase().work)?;
        crate::runtime_schedule::require_provider_clear(
            self.runtime_id,
            account,
            &proof.phase().work,
            receipts,
        )?;
        for operation in [proof.predecessor(), proof.failure_operation()] {
            let completed = self.retained_completion(operation)?.ok_or_else(|| {
                Error::new(
                    "reconciliation_required",
                    "unfinished target lost its actual joined completion",
                )
            })?;
            require_joined_readmission(&completed)?;
        }
        Ok(())
    }
    pub(crate) fn retry_target(
        &self,
        work: &crate::evidence::WorkIdentity,
    ) -> Result<crate::decisions::RetryTarget> {
        self.retry_target_except(work, None)
    }
    pub(crate) fn published_retry_target(
        &self,
        grant: &crate::decisions::VerifiedRetryGrant,
    ) -> Result<crate::decisions::RetryTarget> {
        self.retry_target_except(grant.work(), Some(grant))
    }
    fn retry_target_except(
        &self,
        work: &crate::evidence::WorkIdentity,
        published: Option<&crate::decisions::VerifiedRetryGrant>,
    ) -> Result<crate::decisions::RetryTarget> {
        let receipts = self.control.operations()?;
        let canonical = IssueKey::new(
            &work.linear_endpoint,
            work.linear_workspace,
            work.child.unwrap_or(work.parent),
        )?
        .identity()?;
        // A poll can cache a recovery error after the provider publishes the
        // comment but before its local acknowledgement. Finalizing that exact
        // publication reparses all current comments and receipts below; a stale
        // summary must not override that complete fresh evidence. Ordinary new
        // retry selection retains the conservative cached-error gate.
        if published.is_none()
            && let Some(error) = self
                .state
                .lock()
                .map_err(|_| Error::new("runtime_unavailable", "retry cache lock failed"))?
                .retry_errors
                .get(&canonical)
        {
            return Err(error.clone());
        }
        let config = self.configuration.snapshot()?;
        let grants = crate::runtime_workflow::current_retry_grants(
            &config,
            &self.observations.admission_snapshot()?,
            work,
            &receipts,
            &self.services.key,
        )?;
        for receipt in &receipts {
            if let Some(run) = RunReference::from_receipt(receipt)?
                && run.phase.work == canonical
            {
                self.require_provider_session_clear(&run.account, &canonical)?;
            }
        }
        if let Some(published) = published {
            return crate::runtime_workflow::published_retry_target(
                &config, published, &grants, &receipts,
            );
        }
        let unfinished = self.verified_unfinished_targets(work)?;
        crate::runtime_workflow::select_retry_target_with_unfinished(
            self.runtime_id,
            work,
            &receipts,
            &grants,
            &unfinished,
            &self.configuration.snapshot()?.runtime,
        )
    }
    /// Rebuild retry capabilities from current provider records. This is also
    /// used while STOPPED; it never reserves a run or enlarges a phase budget.
    pub fn refresh_retry_references(
        &self,
        projection: &crate::reconcile::RuntimeProjection,
    ) -> Result<()> {
        let config = self.configuration.snapshot()?;
        if projection.runtime_id != self.runtime_id
            || projection.config_revision != crate::config::config_revision(&config)?
        {
            return Err(Error::new(
                "stale_config",
                "retry recovery projection is not current",
            ));
        }
        let receipts = self.control.operations()?;
        let mut grants = BTreeMap::new();
        let mut errors = BTreeMap::new();
        let mut observed_work = std::collections::BTreeSet::new();
        let mut complete_workspaces = std::collections::BTreeSet::new();
        for workspace in projection.workspaces.values().filter(|w| w.complete) {
            complete_workspaces.insert(workspace.workspace_id);
            let configured = config
                .workspaces
                .values()
                .find(|w| w.id == workspace.workspace_id)
                .ok_or_else(|| Error::new("identity_mismatch", "retry workspace was removed"))?;
            let writer = &config.connections[&configured.linear_connection].account_id;
            for parent in workspace.parents.values() {
                for work in parent.work.iter().chain(
                    parent
                        .children
                        .values()
                        .filter_map(|child| child.work.as_ref()),
                ) {
                    let target = work.child.unwrap_or(work.parent);
                    let canonical =
                        IssueKey::new(&work.linear_endpoint, work.linear_workspace, target)?
                            .identity()?;
                    observed_work.insert(canonical.clone());
                    let Some(observed) = parent.observations.get(&target) else {
                        continue;
                    };
                    for comment in observed.comments.iter().filter(|c| {
                        c.body.starts_with("<!-- kegari:v0:operator_retry -->")
                            && c.user_id.map(|id| id.to_string()).as_deref() == Some(writer)
                    }) {
                        let recovered = crate::runtime_workflow::recover_retry_comment(
                            self.runtime_id,
                            workspace.workspace_id,
                            writer,
                            work,
                            comment,
                            &receipts,
                            &self.services.key,
                        );
                        match recovered {
                            Ok(grant) => {
                                let id = grant.record().operation_id;
                                if grants.insert(id, grant).is_some() {
                                    errors.insert(canonical.clone(),Error::new("ownership_conflict","multiple current publications claim the same retry grant"));
                                }
                            }
                            // Valid historical grants for older generations are
                            // not new authority and must not block a correction.
                            Err(error) => {
                                let old = crate::evidence::decode_signed_publication::<
                                    crate::decisions::RetryGrant,
                                >(
                                    &comment.body, "operator_retry", &self.services.key
                                )
                                .ok();
                                if old.as_ref().is_some_and(|signed| {
                                    signed.payload.work != *work
                                        && signed.payload.work.child == work.child
                                        && signed.payload.work.parent == work.parent
                                        && signed.payload.work.linear_endpoint
                                            == work.linear_endpoint
                                        && signed.payload.work.linear_workspace
                                            == work.linear_workspace
                                }) {
                                    continue;
                                }
                                errors.insert(canonical.clone(), error);
                            }
                        }
                    }
                }
            }
        }
        let revision = canonical_digest(1, projection)?;
        self.control.serialize_observation(|_| {
            if crate::config::config_revision(&self.configuration.snapshot()?)?
                != crate::config::config_revision(&config)?
                || canonical_digest(1, &self.observations.admission_snapshot()?)? != revision
            {
                return Err(Error::new(
                    "stale_result",
                    "provider projection changed during retry recovery",
                ));
            }
            let mut state = self.state.lock().map_err(|_| {
                Error::new("runtime_unavailable", "retry recovery cache lock failed")
            })?;
            state
                .retry_grants
                .retain(|_, g| !complete_workspaces.contains(&g.record().workspace_id));
            state.retry_grants.extend(grants);
            state
                .retry_errors
                .retain(|work, _| !observed_work.contains(work));
            state.retry_errors.extend(errors);
            Ok(())
        })
    }
    fn require_provider_session_clear(&self, account: &str, work: &str) -> Result<()> {
        let state = self
            .state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "provider hold lock failed"))?;
        for (id, session) in &state.provider_holds {
            if session.intervention.is_some()
                && (session.binding.account_id == account || session.binding.work_identity == work)
            {
                return Err(Error::new("provider_intervention", "provider resolution remains unverified for this account/work")
                    .detail(json!({"operation_id":id,"account":session.binding.account_id,"provider_signal":session.intervention})));
            }
            if session.input_required && session.binding.work_identity == work {
                return Err(Error::new(
                    "provider_input_required",
                    "prior provider input requirement remains unresolved",
                )
                .detail(json!({"operation_id":id})));
            }
        }
        Ok(())
    }
    /// Read only the protected, bound journals for actual durable admissions.
    /// Missing/corrupt records remain recovery failures; no empty-registry proof
    /// or later ordinary checkpoint can erase an earlier provider hold.
    pub fn recover_provider_holds(&self) -> Result<()> {
        let mut recovered = Vec::new();
        for receipt in self.control.operations()? {
            if RunReference::from_receipt(&receipt)?.is_none() {
                continue;
            }
            if receipt.runtime_id != self.runtime_id {
                return Err(Error::new(
                    "record_corrupt",
                    "provider journal receipt belongs to another runtime",
                ));
            }
            let records = PrivateDir::open(
                &self
                    .registration
                    .root()
                    .path()
                    .join("worker-records")
                    .join(receipt.operation_id.to_string()),
            )?;
            if let Some(session) = crate::worker::recover_session(&records, &receipt)?
                && (session.state.intervention.is_some() || session.state.input_required)
            {
                recovered.push((receipt.operation_id, session.state));
            }
        }
        self.control.serialize_observation(|_| {
            let mut state = self
                .state
                .lock()
                .map_err(|_| Error::new("runtime_unavailable", "provider hold lock failed"))?;
            for (operation, session) in recovered {
                retain_provider_hold(&mut state.provider_holds, operation, &session)?;
            }
            Ok(())
        })
    }
    /// Advisory preflight before preparing an owned directory. It writes no
    /// receipt and reserves no capacity; `admit_worker` still performs the final
    /// serialized check against current authority and every admitted session.
    pub fn check_worker_capacity(
        &self,
        workspace: Uuid,
        account: &str,
        phase: &crate::scheduler::PhaseKey,
        normalized_state: &str,
    ) -> Result<()> {
        if phase.work.is_empty() || phase.phase.is_empty() {
            return Err(Error::invalid(
                "capacity preflight requires a canonical work phase",
            ));
        }
        crate::evidence::validate_digest(&phase.generation)?;
        self.configuration
            .authority_fences()?
            .check_workspace(workspace)?;
        self.require_provider_session_clear(account, &phase.work)?;
        let config = self.configuration.snapshot()?;
        let active =
            crate::runtime_schedule::occupying_runs(self.runtime_id, &self.control.operations()?)?;
        let quota = self
            .state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "quota state lock failed"))?
            .quotas
            .get(account)
            .cloned();
        crate::runtime_schedule::check_capacity(
            &config,
            &active,
            crate::runtime_schedule::CapacityRequest {
                workspace,
                account,
                normalized_state,
            },
            quota.as_ref(),
        )
    }
    pub fn admit_worker(
        &self,
        lease: &Lease,
        input: crate::control::RunAdmission,
        contract: &WorkerContract,
    ) -> Result<Permit> {
        self.admit_worker_inner(lease, input, contract, None, None)
    }
    pub(crate) fn admit_unfinished_worker(
        &self,
        lease: &Lease,
        input: crate::control::RunAdmission,
        contract: &WorkerContract,
        proof: &crate::delivery::UnfinishedChildWork,
    ) -> Result<Permit> {
        self.admit_worker_inner(
            lease,
            input,
            contract,
            Some(WorkerReadmission::Unfinished(proof)),
            None,
        )
    }
    pub(crate) fn admit_source_advanced_worker(
        &self,
        lease: &Lease,
        input: crate::control::RunAdmission,
        contract: &WorkerContract,
        proof: &crate::delivery::VerifiedSourceAdvanced,
    ) -> Result<Permit> {
        self.admit_worker_inner(
            lease,
            input,
            contract,
            Some(WorkerReadmission::SourceAdvanced(proof)),
            None,
        )
    }
    pub(crate) fn admit_retried_unfinished_worker(
        &self,
        lease: &Lease,
        input: crate::control::RunAdmission,
        contract: &WorkerContract,
        proof: &crate::delivery::RetriedUnfinishedWork,
    ) -> Result<Permit> {
        self.admit_worker_inner(
            lease,
            input,
            contract,
            Some(WorkerReadmission::Retried(proof)),
            None,
        )
    }
    pub(crate) fn admit_review_retry_worker(
        &self,
        lease: &Lease,
        input: crate::control::RunAdmission,
        contract: &WorkerContract,
        proof: &crate::delivery::ReviewRetryWork,
    ) -> Result<Permit> {
        self.admit_worker_inner(
            lease,
            input,
            contract,
            Some(WorkerReadmission::ReviewRetry(proof)),
            None,
        )
    }
    pub(crate) fn admit_validation_retry_worker(
        &self,
        lease: &Lease,
        input: crate::control::RunAdmission,
        contract: &WorkerContract,
        proof: &crate::delivery::ValidationRetryWork,
    ) -> Result<Permit> {
        self.admit_worker_inner(
            lease,
            input,
            contract,
            Some(WorkerReadmission::ValidationRetry(proof)),
            None,
        )
    }
    fn admit_worker_inner(
        &self,
        lease: &Lease,
        input: crate::control::RunAdmission,
        contract: &WorkerContract,
        readmission: Option<WorkerReadmission<'_>>,
        session_resume: Option<(
            &crate::worker_run::VerifiedSessionResume,
            &crate::housekeeping::VerifiedDirectoryReuse,
        )>,
    ) -> Result<Permit> {
        self.admit_worker_context(
            lease,
            input,
            contract,
            WorkerAdmissionContext {
                planning: WorkerPlanning {
                    readmission,
                    round: None,
                },
                session_resume,
                paired_resume: None,
                preparation: None,
                parent_reassembly: None,
            },
        )
    }
    pub(crate) fn admit_review_round_worker(
        &self,
        input: crate::control::RunAdmission,
        contract: &WorkerContract,
        request: &crate::runtime_workers::LaunchRequest,
        preparation: &crate::control::Receipt,
    ) -> Result<Permit> {
        request.validate_binding()?;
        let round = request.review_round.as_deref().ok_or_else(|| {
            Error::new(
                "worker_binding",
                "round admission requires its actual opaque plan",
            )
        })?;
        if input.request_id != request.operation_id || input.reference.phase != request.phase {
            return Err(Error::new(
                "worker_binding",
                "round request differs from actual run admission",
            ));
        }
        request.authority.compare(contract.authority())?;
        self.admit_worker_context(
            &request.lease,
            input,
            contract,
            WorkerAdmissionContext {
                planning: WorkerPlanning {
                    readmission: request_readmission(request)?,
                    round: Some(round),
                },
                session_resume: None,
                paired_resume: None,
                parent_reassembly: None,
                preparation: Some(crate::control::ReviewReceiptReference {
                    operation: preparation.operation_id,
                    receipt_digest: canonical_digest(1, preparation)?,
                }),
            },
        )
    }
    fn admit_worker_context(
        &self,
        lease: &Lease,
        input: crate::control::RunAdmission,
        contract: &WorkerContract,
        context: WorkerAdmissionContext<'_>,
    ) -> Result<Permit> {
        let WorkerAdmissionContext {
            planning,
            session_resume,
            paired_resume,
            preparation,
            parent_reassembly,
        } = context;
        let WorkerPlanning { readmission, round } = planning;
        if parent_reassembly.is_some()
            && (readmission.is_some()
                || round.is_some()
                || session_resume.is_some()
                || preparation.is_none())
        {
            return Err(Error::new(
                "worker_binding",
                "parent reassembly has conflicting cause or lacks exact preparation",
            ));
        }

        if round.is_some() && matches!(readmission, Some(WorkerReadmission::ValidationRetry(_))) {
            return Err(Error::new(
                "worker_binding",
                "validation retry cannot allocate or replace a review round",
            ));
        }
        if round.is_some() && (session_resume.is_some() || preparation.is_none()) {
            return Err(Error::new(
                "worker_binding",
                "round admission lost its exact preparation",
            ));
        }
        if readmission.is_some() && session_resume.is_some() {
            return Err(Error::new(
                "worker_binding",
                "conflicting session admission causes",
            ));
        }
        if paired_resume.is_some()
            && (session_resume.is_none()
                || round.is_some()
                || readmission.is_some()
                || preparation.is_some())
        {
            return Err(Error::new(
                "worker_binding",
                "paired resume must retain only its actual admitted bridge cause",
            ));
        }
        let reference = input.reference.clone();
        let resume_reference = session_resume
            .map(|(proof, directory)| {
                let mut reference = self.session_resume_reference(proof, directory)?;
                if let Some(paired) = paired_resume {
                    reference.review_rounds_consumed = reference
                        .review_rounds_consumed
                        .max(paired.plan().reference().ordinal);
                }
                Ok(reference)
            })
            .transpose()?;
        if input.work_key != contract.work_key()?
            || input.scope_digest != contract.scope_digest()
            || reference.phase.work != input.work_key
            || reference.workspace != contract.instructions().manifest.workspace_id
            || reference.account != contract.account_id()
            || reference.instruction_digest != contract.instructions().manifest_digest
            || reference.profile_digest
                != canonical_digest(1, &contract.instructions().manifest.profile)?
        {
            return Err(Error::new(
                "worker_binding",
                "admission capacity/budget differs from assembled worker contract",
            ));
        }
        let (workspace, revision, workflow) = authority_settings(contract.authority());
        let active = self
            .observations
            .capture_active(workspace, revision, workflow)?;
        let evaluation_operation = input.request_id;
        let final_gate = |snapshot: &crate::control::ControlSnapshot| {
            self.check_evaluation_worker(
                snapshot,
                evaluation_operation,
                contract.authority(),
                &reference.phase,
                reference.max_turns,
            )?;
            self.check_evaluation_context_contract(contract)?;
            self.registration.verify_active_owner(lease)?;
            if let Some(proof) = parent_reassembly {
                proof.verify_contract(contract)?;
                let schedule = self.schedule_parent_reassembly(
                    proof,
                    reference.workspace,
                    &reference.account,
                    Some(snapshot),
                )?;
                if reference.phase != *proof.phase()
                    || reference.attempt != schedule.attempt
                    || reference.max_attempts > schedule.max_attempts
                    || reference.max_turns > schedule.max_turns
                    || reference.max_review_rounds > schedule.max_review_rounds
                {
                    return Err(Error::new(
                        "admission_budget",
                        "parent reassembly exceeds original exact phase bounds",
                    ));
                }
            }
            if let Some(round) = round {
                round.verify_contract(contract)?;
                if round.subject_phase() != &reference.phase {
                    return Err(Error::new(
                        "worker_binding",
                        "round admission changes phase",
                    ));
                }
                WorkerAuthority::Authorized(round.authority().clone())
                    .compare(contract.authority())?;
            }
            if let Some(proof) = readmission {
                proof.verify_contract(contract)?;
                if proof.phase() != &reference.phase {
                    return Err(Error::new(
                        "worker_binding",
                        "readmission cannot replace its original phase",
                    ));
                }
                WorkerAuthority::Authorized(proof.authority().clone())
                    .compare(contract.authority())?;
                let schedule = self.plan_attempt_context(
                    &reference.phase,
                    reference.workspace,
                    &reference.account,
                    WorkerPlanning {
                        readmission: Some(proof),
                        round,
                    },
                    Some(snapshot),
                )?;
                if reference.attempt != schedule.attempt
                    || reference.max_attempts > schedule.max_attempts
                    || reference.max_turns > schedule.max_turns
                    || reference.max_review_rounds > schedule.max_review_rounds
                {
                    return Err(Error::new(
                        "admission_budget",
                        "readmission exceeds original finite phase bounds",
                    ));
                }
            }
            if let Some((proof, directory)) = session_resume {
                let schedule = match paired_resume {
                    Some(paired) => self
                        .plan_paired_session_resume(contract, proof, directory, paired, snapshot)?,
                    None => self.plan_session_resume(contract, proof, directory, snapshot)?,
                };
                if reference.phase != proof.recovery().observation().retained().reference().phase
                    || reference.attempt != schedule.attempt
                    || reference.max_attempts > schedule.max_attempts
                    || reference.max_turns > schedule.max_turns
                    || reference.max_review_rounds > schedule.max_review_rounds
                {
                    return Err(Error::new(
                        "admission_budget",
                        "session resume exceeds its exact original finite phase",
                    ));
                }
            }
            if readmission.is_none() && session_resume.is_none() && parent_reassembly.is_none() {
                let schedule = self.plan_attempt_context(
                    &reference.phase,
                    reference.workspace,
                    &reference.account,
                    WorkerPlanning {
                        readmission: None,
                        round,
                    },
                    Some(snapshot),
                )?;
                if reference.attempt != schedule.attempt
                    || reference.max_attempts > schedule.max_attempts
                    || reference.max_turns > schedule.max_turns
                    || reference.max_review_rounds > schedule.max_review_rounds
                {
                    return Err(Error::new(
                        "admission_budget",
                        "admission exceeds its current exact bounded trigger",
                    ));
                }
            }
            self.require_provider_session_clear(&reference.account, &reference.phase.work)?;
            self.current_admission_authority(contract.authority())?;
            let config = self.configuration.snapshot()?;
            let projection = self.observations.admission_snapshot()?;
            if let WorkerAuthority::Authorized(gate) = contract.authority() {
                let observed = crate::runtime_workflow::current_retry_grants(
                    &config,
                    &projection,
                    gate.work(),
                    &self.control.operations()?,
                    &self.services.key,
                )?;
                if observed
                    .iter()
                    .any(|grant| grant.phase() == &reference.phase)
                {
                    return Err(Error::new(
                        "stale_generation",
                        "phase budget was superseded by an explicit retry grant",
                    ));
                }
                if let Some(id) = reference.phase.retry_grant {
                    let retained = self
                        .state
                        .lock()
                        .map_err(|_| Error::new("runtime_unavailable", "retry proof lock failed"))?
                        .retry_grants
                        .get(&id)
                        .cloned()
                        .ok_or_else(|| {
                            Error::new(
                                "reconciliation_required",
                                "retry admission lacks retained verified grant",
                            )
                        })?;
                    let current =
                        crate::runtime_workflow::require_current_grant(&retained, &observed)?;
                    if crate::runtime_schedule::grant_requires_unfinished(
                        current.phase(),
                        &reference.phase,
                        &self.control.operations()?,
                    )? && !matches!(readmission, Some(WorkerReadmission::Retried(_)))
                        && paired_resume.is_none()
                    {
                        return Err(Error::new(
                            "unfinished_retry_required",
                            "completed budget cannot be admitted without its sealed unfinished source and task",
                        ));
                    }
                    if current.phase_key() != reference.phase
                        || reference.max_attempts > current.new_budget().max_attempts
                        || reference.max_turns > current.new_budget().max_turns
                        || reference.max_review_rounds > current.new_budget().max_review_rounds
                    {
                        return Err(Error::new(
                            "admission_budget",
                            "retry admission exceeds its current exact published budget",
                        ));
                    }
                }
            }
            let expected_state = match contract.authority() {
                WorkerAuthority::Authorized(gate) => {
                    if gate.config_revision() != crate::config::config_revision(&config)? {
                        return Err(Error::new(
                            "stale_config",
                            "new worker preparation predates current configuration",
                        ));
                    }
                    let parent = projection.parent(gate.workspace_id(), gate.work().parent)?;
                    let issue = if let Some(child) = gate.work().child {
                        parent
                            .children
                            .get(&child)
                            .and_then(|child| child.issue.as_ref())
                    } else {
                        parent.issue.as_ref()
                    }
                    .ok_or_else(|| {
                        Error::new("incomplete_snapshot", "worker state is unavailable")
                    })?;
                    issue.state_name.trim().to_lowercase()
                }
                WorkerAuthority::Independent(gate) => {
                    if gate.config_revision() != crate::config::config_revision(&config)? {
                        return Err(Error::new(
                            "stale_config",
                            "new review preparation predates current configuration",
                        ));
                    }
                    "independent_review".into()
                }
            };
            if reference.normalized_state.as_deref() != Some(expected_state.as_str()) {
                return Err(Error::new(
                    "stale_result",
                    "worker state changed before capacity reservation",
                ));
            }
            let active = crate::runtime_schedule::occupying_runs(
                self.runtime_id,
                &self.control.operations()?,
            )?;
            let quota = self
                .state
                .lock()
                .map_err(|_| Error::new("runtime_unavailable", "quota state lock failed"))?
                .quotas
                .get(&reference.account)
                .cloned();
            crate::runtime_schedule::ensure_capacity(&config, &active, &reference, quota.as_ref())
        };
        let permit = if let Some(paired) = paired_resume {
            let (proof, directory) = session_resume
                .ok_or_else(|| paired_recovery::paired_error("paired bridge disappeared"))?;
            let provenance = crate::control::RoundResumeProvenance {
                round: paired.plan().reference().clone(),
                resume: paired.resume_reference(proof, directory, &self.control.operations()?)?,
            };
            self.control.admit_round_run_resumed_observed(
                lease,
                input,
                resume_reference.ok_or_else(|| {
                    paired_recovery::paired_error("paired resume reference disappeared")
                })?,
                provenance,
                final_gate,
            )?
        } else if let Some(proof) = parent_reassembly {
            self.control.admit_parent_reassembly_run_observed(
                lease,
                input,
                proof.reference().clone(),
                preparation
                    .ok_or_else(|| Error::new("worker_binding", "parent preparation missing"))?,
                final_gate,
            )?
        } else if let Some(round) = round {
            self.control.admit_round_run_observed(
                lease,
                input,
                crate::control::ReviewRoundRunProvenance {
                    round: round.reference().clone(),
                    preparation: preparation
                        .ok_or_else(|| Error::new("worker_binding", "round preparation missing"))?,
                    cause: round_run_cause(readmission)?,
                },
                final_gate,
            )?
        } else if let Some(resume) = resume_reference {
            self.control
                .admit_run_resumed_observed(lease, input, resume, final_gate)?
        } else {
            match readmission {
                Some(WorkerReadmission::Unfinished(proof)) => {
                    self.control.admit_run_continued_observed(
                        lease,
                        input,
                        continuation_reference(proof),
                        final_gate,
                    )?
                }
                Some(WorkerReadmission::SourceAdvanced(proof)) => {
                    self.control.admit_run_source_advanced_observed(
                        lease,
                        input,
                        source_advanced_reference(proof),
                        final_gate,
                    )?
                }
                Some(WorkerReadmission::Retried(_))
                | Some(WorkerReadmission::ReviewRetry(_))
                | Some(WorkerReadmission::ValidationRetry(_))
                | None => self.control.admit_run_observed(lease, input, final_gate)?,
            }
        };
        self.state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "active snapshot registration failed"))?
            .prepared_active
            .insert(permit.operation_id, active);
        Ok(permit)
    }
    pub fn register_run(&self, permit: Permit, contract: Arc<WorkerContract>) -> Result<()> {
        self.register_run_inner(permit, contract, None)
    }
    fn register_run_inner(
        &self,
        permit: Permit,
        contract: Arc<WorkerContract>,
        parent_origin: Option<Arc<crate::delivery::PreparedParentRunOrigin>>,
    ) -> Result<()> {
        let budget = RunReference::from_receipt(&self.control.operation(permit.operation_id)?)?
            .ok_or_else(|| {
                Error::new(
                    "record_corrupt",
                    "worker admission has no durable phase budget",
                )
            })?;
        if budget.account != contract.account_id()
            || budget.workspace != contract.instructions().manifest.workspace_id
            || budget.instruction_digest != contract.instructions().manifest_digest
            || budget.profile_digest
                != canonical_digest(1, &contract.instructions().manifest.profile)?
            || permit.scope_digest != contract.scope_digest()
            || permit.work_key != contract.work_key()?
            || budget.phase.work != permit.work_key
        {
            return Err(Error::new(
                "worker_binding",
                "worker differs from its immutable admitted account, context or budget",
            ));
        }
        let active = self
            .state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "active snapshot registration failed"))?
            .prepared_active
            .remove(&permit.operation_id)
            .ok_or_else(|| {
                Error::new(
                    "ownership_conflict",
                    "worker admission has no retained settings snapshot",
                )
            })?;
        self.control.validate_permit(&permit, || {
            self.registration.verify_active_owner(&permit.lease)?;
            self.retained_authority(contract.authority())?;
            Ok(())
        })?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "worker registration lock failed"))?;
        if state.runs.contains_key(&permit.operation_id) {
            return Err(Error::new(
                "ownership_conflict",
                "worker operation is already registered",
            ));
        }
        state
            .correction_wakes
            .insert(permit.operation_id, Arc::new(tokio::sync::Notify::new()));
        state.runs.insert(
            permit.operation_id,
            RegisteredRun {
                permit,
                contract,
                budget,
                _active: Some(active),
                last_observation: None,
                joined: false,
                parent_origin,
                parent_joined: None,
            },
        );
        Ok(())
    }
    /// Track only the owned handle returned for an already registered admission.
    /// No serialized process/session identifier can become an owned handle.
    pub fn track_run(&self, run: OwnedWorkerRun) -> Result<()> {
        let id = run.operation_id();
        if !self
            .state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "worker state lock failed"))?
            .runs
            .contains_key(&id)
        {
            return Err(Error::new(
                "ownership_conflict",
                "owned worker has no registered admission",
            ));
        }
        let mut workers = self
            .workers
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "worker handle lock failed"))?;
        if workers.contains_key(&id) {
            return Err(Error::new(
                "ownership_conflict",
                "owned worker is already tracked",
            ));
        }
        workers.insert(id, run);
        self.services.wake.notify_one();
        Ok(())
    }
    /// Join only finished tasks; never hold state/control locks over worker waits.
    /// Retains the actual sealed check handles before returning shared immutable
    /// observations to the driver. No JSON roundtrip can recover these proofs.
    pub async fn collect_finished(&self) -> Result<Vec<Arc<RunCompletion>>> {
        let finished = {
            let mut workers = self
                .workers
                .lock()
                .map_err(|_| Error::new("runtime_unavailable", "worker handle lock failed"))?;
            let ids: Vec<_> = workers
                .iter()
                .filter(|(_, run)| run.is_finished())
                .map(|(id, _)| *id)
                .collect();
            ids.into_iter()
                .filter_map(|id| workers.remove(&id))
                .collect::<Vec<_>>()
        };
        let mut completed = Vec::new();
        for mut run in finished {
            let id = run.operation_id();
            match run.join().await {
                Ok(completion) => {
                    let observed = completion.observation();
                    let completion = Arc::new(completion);
                    let mut state = self.state.lock().map_err(|_| {
                        Error::new("runtime_unavailable", "worker state lock failed")
                    })?;
                    let tracked = state.runs.get_mut(&id).ok_or_else(|| {
                        Error::new(
                            "ownership_conflict",
                            "joined worker registration disappeared",
                        )
                    })?;
                    let parent_join = tracked
                        .parent_origin
                        .as_ref()
                        .map(|origin| {
                            Self::parent_join_candidate(
                                origin,
                                &tracked.contract,
                                &tracked.permit,
                                &tracked.budget,
                                &completion,
                            )
                        })
                        .transpose();
                    let parent_error = match parent_join {
                        Ok(candidate) => {
                            tracked.parent_joined = candidate;
                            None
                        }
                        Err(error) => Some(error),
                    };
                    tracked.last_observation = Some(observed.clone());
                    tracked.joined = true;
                    state.completions.insert(id, completion.clone());
                    if let Some(error) = &parent_error {
                        state.parent_retirement_errors.insert(id, error.clone());
                    }
                    drop(state);
                    if parent_error.is_some() {
                        // Preserve the actual join for diagnostics, but never
                        // retire a registration using a differently bound result.
                        if let Err(error) = self.control.fence(id).and_then(|()| {
                            self.executions.cancel_fenced(&self.control, id).map(|_| ())
                        }) {
                            self.state
                                .lock()
                                .map_err(|_| {
                                    Error::new(
                                        "runtime_unavailable",
                                        "parent join fence diagnostics failed",
                                    )
                                })?
                                .completion_errors
                                .insert(id, error);
                        }
                        completed.push(completion);
                        continue;
                    }
                    // Persist/retain the actual joined capabilities first. A
                    // failed receipt retirement must not drop this result or
                    // prevent collection of other already finished workers.
                    if completion.local_exit_confirmed {
                        let retired = self.control.retire_local(id, observed);
                        let mut state = self.state.lock().map_err(|_| {
                            Error::new("runtime_unavailable", "worker state lock failed")
                        })?;
                        match retired {
                            Ok(_) => {
                                if let Some(tracked) = state.runs.get_mut(&id) {
                                    tracked._active = None;
                                }
                            }
                            Err(error) => {
                                state.completion_errors.insert(id, error);
                            }
                        }
                    } else if let Err(error) = self
                        .control
                        .fence(id)
                        .and_then(|()| self.executions.cancel_fenced(&self.control, id).map(|_| ()))
                    {
                        self.state
                            .lock()
                            .map_err(|_| {
                                Error::new("runtime_unavailable", "completion error lock failed")
                            })?
                            .completion_errors
                            .insert(id, error);
                    }
                    completed.push(completion);
                }
                Err(error) => {
                    // A joined outer task does not prove that every local child
                    // exited. Preserve the permit and every unresolved receipt.
                    let mut state = self.state.lock().map_err(|_| {
                        Error::new("runtime_unavailable", "worker state lock failed")
                    })?;
                    let tracked = state.runs.get_mut(&id).ok_or_else(|| {
                        Error::new(
                            "ownership_conflict",
                            "failed worker registration disappeared",
                        )
                    })?;
                    tracked.last_observation = Some(
                        json!({"outcome":"uncertain","error":error,"joined":true,"local_exit_confirmed":false}),
                    );
                    tracked.joined = true;
                    drop(state);
                    if let Err(error) = self
                        .control
                        .fence(id)
                        .and_then(|()| self.executions.cancel_fenced(&self.control, id).map(|_| ()))
                    {
                        self.state
                            .lock()
                            .map_err(|_| {
                                Error::new("runtime_unavailable", "completion error lock failed")
                            })?
                            .completion_errors
                            .insert(id, error);
                    }
                }
            }
        }
        self.observe_parent_retirements()?;
        Ok(completed)
    }
    /// Accept only the same admitted trial's final broker checkpoint and actual
    /// owned provider/hook exit capability, never caller-supplied completion JSON.
    pub fn finish_trial(
        &self,
        verified: crate::worker_run::VerifiedTrialCompletion,
    ) -> Result<Arc<RunCompletion>> {
        self.finish_trial_completion(
            &crate::worker_run::RetainedTrialCompletion::from_verified(verified),
            false,
        )
    }
    // Private actual-capability caller may retry only the identical retained Arc.
    // A retirement/storage error must not destroy the original terminal evidence.
    fn finish_trial_completion(
        &self,
        verified: &crate::worker_run::RetainedTrialCompletion,
        retry: bool,
    ) -> Result<Arc<RunCompletion>> {
        let completion = verified.completion().clone();
        let id = completion.operation_id;
        let observed = completion.observation();
        self.control.serialize_observation(|_| {
            let mut state = self
                .state
                .lock()
                .map_err(|_| Error::new("runtime_unavailable", "trial completion lock failed"))?;
            if let Some(previous) = state.completions.get(&id) {
                return if retry && Arc::ptr_eq(previous, &completion) {
                    Ok(())
                } else {
                    Err(Error::new(
                        "ownership_conflict",
                        "trial completion already retained",
                    ))
                };
            }
            let run = state.runs.get_mut(&id).ok_or_else(|| {
                Error::new("ownership_conflict", "trial admission is not registered")
            })?;
            let contract = completion
                .contract
                .as_ref()
                .ok_or_else(|| Error::new("worker_binding", "trial lost original contract"))?;
            if !Arc::ptr_eq(&run.contract, contract)
                || !completion.local_exit_confirmed
                || completion.state.binding.operation_id != id.to_string()
                || matches!(completion.outcome, crate::worker_run::RunOutcome::Completed)
                    && !completion.state.completion_is_current()
            {
                return Err(Error::new(
                    "worker_binding",
                    "trial completion does not bind its original registered worker",
                ));
            }
            run.last_observation = Some(observed.clone());
            run.joined = true;
            state.completions.insert(id, completion.clone());
            Ok(())
        })?;
        let retirement = if retry {
            self.control.operation(id).and_then(|actual| {
                if actual.phase == crate::control::ReceiptPhase::Reconciled {
                    let registered = self
                        .state
                        .lock()
                        .map_err(|_| Error::new("runtime_unavailable", "trial retry lock failed"))?
                        .runs
                        .get(&id)
                        .cloned()
                        .ok_or_else(|| {
                            Error::new("worker_binding", "trial retry lost admission")
                        })?;
                    evaluation::closed_model_observation(
                        &actual,
                        &registered.permit,
                        &registered.budget,
                        &observed,
                    )?
                    .then_some(actual)
                    .ok_or_else(|| {
                        Error::new(
                            "reconciliation_required",
                            "trial retirement remains uncertain",
                        )
                    })
                } else {
                    self.control.retire_local(id, observed)
                }
            })
        } else {
            self.control.retire_local(id, observed)
        };
        let retirement = if retry {
            retirement.and_then(|receipt| {
                let registered = self
                    .state
                    .lock()
                    .map_err(|_| {
                        Error::new(
                            "runtime_unavailable",
                            "trial retirement validation lock failed",
                        )
                    })?
                    .runs
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| {
                        Error::new("worker_binding", "trial retirement lost admission")
                    })?;
                if evaluation::closed_model_observation(
                    &receipt,
                    &registered.permit,
                    &registered.budget,
                    &completion.observation(),
                )? {
                    Ok(receipt)
                } else {
                    Err(Error::new(
                        "reconciliation_required",
                        "trial retains unresolved effects",
                    ))
                }
            })
        } else {
            retirement
        };
        if let Err(error) = retirement {
            self.state
                .lock()
                .map_err(|_| Error::new("runtime_unavailable", "trial retirement lock failed"))?
                .completion_errors
                .insert(id, error.clone());
            return Err(error);
        }
        if let Some(run) = self
            .state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "trial retention lock failed"))?
            .runs
            .get_mut(&id)
        {
            run._active = None;
        }
        Ok(completion)
    }
    pub fn retained_completion(&self, operation: Uuid) -> Result<Option<Arc<RunCompletion>>> {
        Ok(self
            .state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "completion proof lock failed"))?
            .completions
            .get(&operation)
            .cloned())
    }
    /// Retry receipt retirement only when the actual joined adapter confirmed
    /// complete local exit. Empty registries or group-only observations cannot
    /// upgrade an uncertain completion into descendant-exit proof.
    pub fn observe_local_exits(&self) -> Result<()> {
        // Already-retired history writes retry independently of another run
        // whose receipt retirement may still fail later in this loop.
        self.observe_parent_retirements()?;
        let candidates: Vec<_> = self
            .state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "worker state lock failed"))?
            .runs
            .iter()
            .filter(|(_, run)| {
                run.joined && (run.parent_origin.is_none() || run.parent_joined.is_some())
            })
            .map(|(id, run)| (*id, run.last_observation.clone()))
            .collect();
        for (id, observation) in candidates {
            if self
                .retained_completion(id)?
                .is_none_or(|result| !result.local_exit_confirmed)
            {
                continue;
            }
            self.executions.observe_exited(id)?;
            if self.executions.active_operations()?.contains(&id) {
                continue;
            }
            if self
                .control
                .snapshot()?
                .active_operations
                .iter()
                .any(|permit| permit.operation_id == id)
            {
                self.control.retire_local(
                    id,
                    observation.ok_or_else(|| {
                        Error::new("record_corrupt", "joined completion observation is missing")
                    })?,
                )?;
                if let Some(run) = self
                    .state
                    .lock()
                    .map_err(|_| Error::new("runtime_unavailable", "worker state lock failed"))?
                    .runs
                    .get_mut(&id)
                {
                    run._active = None;
                }
            }
        }
        self.observe_parent_retirements()?;
        Ok(())
    }
    /// New sessions never inherit another operation's permission to drain.
    /// Callers must supply a new-operation or coordination authority and the
    /// complete current selection is checked again at serialized admission.
    fn current_admission_authority(&self, expected: &WorkerAuthority) -> Result<WorkerAuthority> {
        let config = self.configuration.snapshot()?;
        let workspace = match expected {
            WorkerAuthority::Authorized(g) => g.workspace_id(),
            WorkerAuthority::Independent(g) => g.workspace_id(),
        };
        self.configuration
            .authority_fences()?
            .check_workspace(workspace)?;
        let projection = self.observations.admission_snapshot()?;
        if projection.config_revision != crate::config::config_revision(&config)? {
            return Err(Error::new(
                "stale_config",
                "provider observation predates current configuration",
            ));
        }
        fresh_admission_authority(expected, &projection, &self.dependency_outcomes()?)
    }
    pub(crate) fn retained_authority(&self, expected: &WorkerAuthority) -> Result<WorkerAuthority> {
        match expected {
            WorkerAuthority::Authorized(gate) => {
                let outcomes = self.dependency_outcomes()?;
                self.observations
                    .active_gate(gate, &outcomes)
                    .map(WorkerAuthority::Authorized)
            }
            WorkerAuthority::Independent(gate) => self
                .observations
                .active_independent_gate(gate)
                .map(WorkerAuthority::Independent),
        }
    }
    pub fn inspection(&self) -> Result<RuntimeInspection> {
        let projection = self.observations.snapshot()?;
        let config = self.configuration.snapshot()?;
        let revision = crate::config::config_revision(&config)?;
        let active = self
            .control
            .snapshot()?
            .active_operations
            .into_iter()
            .map(|permit| permit.operation_id)
            .collect::<std::collections::BTreeSet<_>>();
        let state = self
            .state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "runtime projection lock failed"))?;
        let activities = state
            .runs
            .values()
            .filter(|run| active.contains(&run.permit.operation_id))
            .filter_map(|run| match run.contract.authority() {
                WorkerAuthority::Authorized(g) => Some(crate::reconcile::Activity {
                    parent: IssueKey::new(
                        &g.work().linear_endpoint,
                        g.work().linear_workspace,
                        g.work().parent,
                    )
                    .ok()?,
                    child: g.work().child,
                    operation_id: run.permit.operation_id,
                    phase: run.budget.phase.phase.clone(),
                    queued: false,
                }),
                WorkerAuthority::Independent(_) => None,
            })
            .collect();
        let (delivery, outcomes) = effective_evidence(
            &state,
            &projection,
            &revision,
            &self.configuration.authority_fences()?,
            &config,
            &self.observations,
        );
        Ok(RuntimeInspection {
            projection,
            delivery,
            outcomes,
            activities,
            history: state.history.clone(),
            release_details: state.release_details.clone(),
            release_history: Vec::new(),
            no_release_history: Vec::new(),
        })
    }
    pub fn account_observations(&self) -> Result<Value> {
        let config = self.configuration.snapshot()?;
        let observed = self.observations.admission_snapshot();
        let fences = self.configuration.authority_fences()?;
        let state = self
            .state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "account metrics lock failed"))?;
        let (_, released_errors) =
            released::effective_outcomes(&state, &config, observed.clone(), &fences);
        let (_, no_release_errors) =
            released::effective_no_release_outcomes(&state, &config, observed, &fences);
        let holds=state.provider_holds.iter().map(|(id,session)| json!({"operation_id":id,"account":session.binding.account_id,"work":session.binding.work_identity,"intervention":session.intervention,"input_required":session.input_required})).collect::<Vec<_>>();
        Ok(
            json!({"quota":state.quotas,"observations":state.rate_observations,"provider_holds":holds,"retry_recovery":state.retry_errors,"session_recovery":state.session_recovery_errors,"scheduling":state.schedule_blockers,"worker_completion":state.completion_errors,"parent_retirement":state.parent_retirement_errors,"worker_claims":state.claim_errors,"correction_transitions":state.correction_errors,"delivery":state.delivery_errors,"released_dependencies":released_errors,"no_release_dependencies":no_release_errors,"no_release_inventory":state.no_release_inventory_error}),
        )
    }
    pub fn execution_registry(&self) -> Arc<ExecutionRegistry> {
        self.executions.clone()
    }
}

/// Failed provider reads revoke readiness in the same order as admission and
/// successful hydration. Otherwise a final gate could inspect the old cache
/// and commit admission after a concurrent invalidation was already published.
fn invalidate_delivery(
    control: &ControlHost,
    state: &Mutex<State>,
    parent: &IssueKey,
    error: Error,
) -> Result<()> {
    let identity = parent.identity()?;
    control.serialize_observation(|_| {
        let mut state = state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "delivery proof cache lock failed"))?;
        state.delivery_snapshots.remove(parent);
        state.delivery.remove(parent);
        state.delivery_errors.insert(identity, error);
        Ok(())
    })
}
impl OutcomeSink for RuntimeCoordinator {
    fn record(&self, outcome: &WorkerOutcome) -> Result<String> {
        let run = self
            .state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "worker state lock failed"))?
            .runs
            .get(&outcome.operation_id)
            .cloned()
            .ok_or_else(|| {
                Error::new("worker_binding", "outcome has no admitted runtime worker")
            })?;
        self.control.validate_permit(&run.permit, || {
            self.registration.verify_active_owner(&run.permit.lease)?;
            crate::worker::effective_authority(
                self,
                outcome.operation_id,
                run.contract.authority(),
            )?;
            let transition = self.current_correction_transition(outcome.operation_id)?;
            let effective = transition
                .as_ref()
                .map(|proof| WorkerAuthority::Authorized(proof.current_authority().clone()))
                .unwrap_or_else(|| run.contract.authority().clone());
            let manifest = &run.contract.instructions().manifest;
            if run.joined
                || !matches!(outcome.schema_version, 1 | 2)
                || outcome.schema_version == 1
                    && (transition.is_some() || outcome.processed_correction.is_some())
                || outcome.proposal_id.is_nil()
                || outcome.runtime_id != run.permit.lease.runtime_id
                || outcome.work.as_ref() != effective.work()
                || outcome.fresh_gate_digest != effective.digest()?
                || outcome.instruction_digest != run.contract.instructions().manifest_digest
                || outcome.tool_contract_digest != run.contract.tools().digest()
                || canonical_digest(crate::instructions::ASSEMBLY_SCHEMA, &outcome.instructions)?
                    != outcome.instruction_digest
                || outcome.instructions.operation_id != run.permit.operation_id
                || outcome.session.binding.operation_id != run.permit.operation_id.to_string()
                || outcome.session.binding.instruction_digest != outcome.instruction_digest
                || outcome.session.binding.work_identity != run.permit.work_key
                || outcome.session.binding.generation != manifest.work_generation
                || outcome.session.binding.account_id != run.contract.account_id()
                || outcome.session.max_turns > run.budget.max_turns
                || outcome.session.turns_started > run.budget.max_turns
                || !outcome.session.completion_is_current()
                || outcome.session.terminal.as_deref() != Some("completed")
                || outcome
                    .session
                    .completed_turns
                    .get(&outcome.completed_turn)
                    .map(String::as_str)
                    != Some("completed")
                || outcome.session.intervention.is_some()
                || outcome.session.input_required
                || outcome.session.turn_id.is_some()
                || outcome.schema_version == 2
                    && outcome.session.last_completed_turn.as_deref()
                        != Some(&outcome.completed_turn)
            {
                return Err(Error::new(
                    "worker_binding",
                    "outcome differs from its current admitted completed session",
                ));
            }
            let expected_correction = transition
                .as_ref()
                .map(|proof| crate::worker::ProcessedCorrection {
                    watermark: proof.watermark(),
                    input_digest: proof.input_digest(),
                    transition_digest: Some(proof.digest().into()),
                })
                .or_else(|| match &effective {
                    WorkerAuthority::Authorized(gate) => Some(crate::worker::ProcessedCorrection {
                        watermark: gate.correction_watermark(),
                        input_digest: manifest.context_digest.clone(),
                        transition_digest: None,
                    }),
                    WorkerAuthority::Independent(_) => None,
                });
            if outcome.schema_version == 2
                && (outcome
                    .processed_correction
                    .as_ref()
                    .is_some_and(|reported| Some(reported) != expected_correction.as_ref())
                    || expected_correction.as_ref().is_some_and(|expected| {
                        expected.watermark > 0
                            && outcome.processed_correction.as_ref() != Some(expected)
                    }))
            {
                return Err(Error::new(
                    "worker_binding",
                    "outcome does not report the exact current processed correction input",
                ));
            }
            match run.contract.authority() {
                WorkerAuthority::Authorized(_) if outcome.independent_review.is_some() => {
                    return Err(Error::new(
                        "worker_binding",
                        "authorized work cannot become an independent review",
                    ));
                }
                WorkerAuthority::Independent(g)
                    if outcome.independent_review.as_ref().is_none_or(|target| {
                        target.repository.id != g.repository_id()
                            || target.pull_request != g.pull_request().number
                            || target.head != g.pull_request().head_sha
                            || target.base != g.pull_request().base_sha
                    }) =>
                {
                    return Err(Error::new(
                        "worker_binding",
                        "independent outcome differs from the exact selected PR",
                    ));
                }
                _ => {}
            }
            let authority = match &effective {
                WorkerAuthority::Authorized(gate) => serde_json::to_value(gate),
                WorkerAuthority::Independent(gate) => serde_json::to_value(gate),
            }
            .map_err(|_| Error::invalid("pending worker authority encoding failed"))?;
            let payload = json!({"schema_version":1,"outcome":outcome,"authority":authority});
            let signed = self.services.key.sign("pending_worker_claim", &payload)?;
            let bytes = serde_json::to_vec(&signed)
                .map_err(|_| Error::invalid("pending worker claim encoding failed"))?;
            if bytes.len() > 8 * 1024 * 1024 {
                return Err(Error::new(
                    "bounded_input",
                    "pending worker claim exceeds artifact limit",
                ));
            }
            let leaf = format!("pending-worker-{}.json", outcome.proposal_id);
            let _lock = self.claims.section_lock("pending-worker.lock")?;
            match self.claims.read_bytes(&leaf, 8 * 1024 * 1024) {
                Ok(prior) if prior == bytes => {}
                Ok(_) => {
                    return Err(Error::new(
                        "operation_conflict",
                        "worker proposal identity was reused for different content",
                    ));
                }
                Err(error) if error.code == "record_missing" => {
                    self.claims.write_bytes(&leaf, &bytes)?
                }
                Err(error) => return Err(error),
            }
            Ok(())
        })?;
        Ok(format!(
            "pending-worker:{}:{}",
            outcome.operation_id, outcome.proposal_id
        ))
    }
}

fn authority_settings(authority: &WorkerAuthority) -> (Uuid, &str, &str) {
    match authority {
        WorkerAuthority::Authorized(gate) => (
            gate.workspace_id(),
            gate.config_revision(),
            gate.workflow_digest(),
        ),
        WorkerAuthority::Independent(gate) => (
            gate.workspace_id(),
            gate.config_revision(),
            gate.workflow_digest(),
        ),
    }
}
fn retain_provider_hold(
    holds: &mut BTreeMap<Uuid, crate::codex::SessionState>,
    operation: Uuid,
    session: &crate::codex::SessionState,
) -> Result<()> {
    if session.binding.operation_id != operation.to_string()
        || session
            .intervention
            .as_ref()
            .is_some_and(|signal| signal.account_id != session.binding.account_id)
    {
        return Err(Error::new(
            "worker_binding",
            "provider signal belongs to another operation/account",
        ));
    }
    if let Some(prior) = holds.get(&operation) {
        if prior.binding != session.binding {
            return Err(Error::new(
                "worker_binding",
                "provider hold cannot move to another session binding",
            ));
        }
        let mut retained = session.clone();
        if prior.intervention.is_some() {
            retained.intervention = prior.intervention.clone();
        }
        retained.input_required |= prior.input_required;
        holds.insert(operation, retained);
    } else if session.intervention.is_some() || session.input_required {
        holds.insert(operation, session.clone());
    }
    Ok(())
}
impl WorkerHost for RuntimeCoordinator {
    fn observe_independent_broker(
        &self,
        control: &Arc<ControlHost>,
        permit: &Permit,
        contract: &Arc<WorkerContract>,
        inventory: &crate::worker::BrokerInventory,
    ) -> Result<crate::worker::BrokerControlBatch> {
        self.observe_registered_independent_broker(control, permit, contract, inventory)
    }
    fn observe_independent_boundary(
        &self,
        control: &Arc<ControlHost>,
        permit: &Permit,
        contract: &Arc<WorkerContract>,
    ) -> Result<crate::evaluation::IndependentBoundaryIdentity> {
        self.observe_registered_independent_boundary(control, permit, contract)
    }
    fn observed_session(
        &self,
        operation: Uuid,
        session: &crate::codex::SessionState,
    ) -> Result<()> {
        self.control.serialize_observation(|_| {
            let mut state = self
                .state
                .lock()
                .map_err(|_| Error::new("runtime_unavailable", "provider session lock failed"))?;
            let run = state.runs.get(&operation).ok_or_else(|| {
                Error::new(
                    "worker_binding",
                    "provider session has no admitted contract",
                )
            })?;
            if session.schema_version != 1
                || session.binding.operation_id != operation.to_string()
                || session.binding.account_id != run.contract.account_id()
                || session.binding.work_identity != run.permit.work_key
                || session.binding.instruction_digest != run.budget.instruction_digest
                || session.binding.generation
                    != run.contract.instructions().manifest.work_generation
                || session.turns_started > run.budget.max_turns
                || session.max_turns > run.budget.max_turns
            {
                return Err(Error::new(
                    "worker_binding",
                    "provider checkpoint differs from admitted contract",
                ));
            }
            retain_provider_hold(&mut state.provider_holds, operation, session)?;
            self.services.wake.notify_one();
            Ok(())
        })
    }
    fn current_authority(&self, expected: &WorkerAuthority) -> Result<WorkerAuthority> {
        let expected_digest = expected.digest()?;
        let registered = {
            let state = self.state.lock().map_err(|_| {
                Error::new("runtime_unavailable", "worker registration lock failed")
            })?;
            state.runs.iter().any(|(operation, run)| {
                run._active.is_some()
                    && !run.joined
                    && (run.contract.authority().digest().as_ref() == Ok(&expected_digest)
                        || state
                            .correction_transitions
                            .get(operation)
                            .is_some_and(|transition| {
                                transition.current_authority().digest().as_ref()
                                    == Ok(&expected_digest)
                            }))
            })
        };
        if registered {
            return self.retained_authority(expected);
        }
        self.current_admission_authority(expected)
    }
    fn correction_waiting(&self, operation: Uuid) -> Result<bool> {
        self.waiting_for_correction(operation)
    }
    fn correction_transition(
        &self,
        operation: Uuid,
    ) -> Result<Option<Arc<crate::corrections::VerifiedCorrectionTransition>>> {
        self.current_correction_transition(operation)
    }
    fn correction_wake(&self, operation: Uuid) -> Result<Option<Arc<tokio::sync::Notify>>> {
        Ok(self
            .state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "correction notification lock failed"))?
            .correction_wakes
            .get(&operation)
            .cloned())
    }
    fn verify_owner(&self, lease: &Lease) -> Result<()> {
        self.registration.verify_active_owner(lease)
    }
    fn rate_limits(&self, operation: Uuid, observed: &Value) -> Result<()> {
        if serde_json::to_vec(observed)
            .map_err(|_| Error::invalid("rate observation encoding"))?
            .len()
            > 65_536
        {
            return Err(Error::new(
                "bounded_input",
                "account rate observation exceeds 64KiB",
            ));
        }
        let account = self
            .state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "account metrics lock failed"))?
            .runs
            .get(&operation)
            .ok_or_else(|| {
                Error::new(
                    "ownership_conflict",
                    "rate observation has no registered worker",
                )
            })?
            .contract
            .account_id()
            .to_owned();
        record_account_limits(&self.control, &self.state, &account, observed)
    }
    fn reconcile_prior(&self, operation: Uuid, prior: Option<&Value>) -> Result<()> {
        let state = self
            .state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "recovery proof lock failed"))?;
        if !state.runs.contains_key(&operation) {
            return Err(Error::new(
                "ownership_conflict",
                "recovery has no admitted registered worker",
            ));
        }
        if let Some(prior) = prior
            && state.resume_proofs.get(&operation) != Some(&canonical_digest(1, prior)?)
        {
            return Err(Error::new(
                "reconciliation_required",
                "retained session lacks a matching provider and directory recovery proof",
            ));
        }
        Ok(())
    }
    fn correction_event(&self, operation: Uuid, watermark: u64, input: &str) -> Result<String> {
        let state = self
            .state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "correction proof lock failed"))?;
        let (digest, reference) = state
            .correction_inputs
            .get(&(operation, watermark))
            .ok_or_else(|| {
                Error::new(
                    "correction_provenance_unavailable",
                    "correction has not passed ordinary host intake and scoped routing",
                )
            })?;
        if *digest != bytes_digest(input.as_bytes()) {
            return Err(Error::new(
                "stale_result",
                "correction input differs from its current published scoped instruction",
            ));
        }
        Ok(reference.clone())
    }
    fn observed_state(&self, operation: Uuid) -> Result<Value> {
        let projection = self.observations.admission_snapshot()?;
        let config = self.configuration.snapshot()?;
        let revision = crate::config::config_revision(&config)?;
        let state = self
            .state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "workflow observation lock failed"))?;
        let run = state.runs.get(&operation).ok_or_else(|| {
            Error::new(
                "ownership_conflict",
                "state request has no registered worker",
            )
        })?;
        let (delivery, _) = effective_evidence(
            &state,
            &projection,
            &revision,
            &self.configuration.authority_fences()?,
            &config,
            &self.observations,
        );
        let evidence = if let Some(work) = run.contract.authority().work() {
            let parent = IssueKey::new(&work.linear_endpoint, work.linear_workspace, work.parent)?;
            delivery.get(&parent).and_then(|delivery|work.child.and_then(|child|delivery.verified_children().get(&child))).filter(|proof|&proof.evidence().work==work).map(|proof|json!({"verified":true,"digest":proof.digest(),"evidence":proof.evidence()}))
        } else {
            None
        };
        Ok(
            json!({"operation_id":operation,"phase":run.budget.phase.phase,"attempt":run.budget.attempt,"attempt_bound":run.budget.max_attempts,"completed":evidence.is_some(),"evidence":evidence,"last_observation":run.last_observation}),
        )
    }
}
fn evidence_projection_current(
    gate: &crate::reconcile::FreshGate,
    observed_at: chrono::DateTime<Utc>,
    projection: &crate::reconcile::RuntimeProjection,
    revision: &str,
    fences: &crate::configuration::AuthorityFences,
    now: chrono::DateTime<Utc>,
) -> bool {
    if fences.check_workspace(gate.workspace_id()).is_err()
        || projection.config_revision != revision
        || gate.config_revision() != revision
        || observed_at > now + Duration::seconds(5)
        || now - observed_at > Duration::seconds(30)
    {
        return false;
    }
    projection
        .workspaces
        .get(&gate.workspace_id())
        .filter(|w| w.complete)
        .and_then(|w| w.parents.get(&gate.work().parent))
        .is_some_and(|parent| {
            parent.work.as_ref() == Some(gate.work())
                && parent.authorization_published
                && parent.source_digest == gate.sources_digest()
                && parent.required_children == *gate.required_children()
                && parent
                    .corrections
                    .gates_current(gate.correction_watermark())
                && parent.blockers.iter().all(|b| b.code == "excluded")
                && parent.observed_at <= now + Duration::seconds(5)
                && now - parent.observed_at <= Duration::seconds(30)
        })
}
fn effective_evidence(
    state: &State,
    projection: &crate::reconcile::RuntimeProjection,
    revision: &str,
    fences: &crate::configuration::AuthorityFences,
    config: &kegari_config::Config,
    observations: &ObservationCache,
) -> (BTreeMap<IssueKey, ParentDelivery>, DependencyOutcomes) {
    let mut delivery = BTreeMap::new();
    let mut outcomes = DependencyOutcomes::default();
    let observed = observations.admission_snapshot();
    let (released, _) = released::effective_outcomes(state, config, observed.clone(), fences);
    let (no_release, no_release_errors) =
        released::effective_no_release_outcomes(state, config, observed, fences);
    let now = Utc::now();
    for (parent, proof) in &state.delivery_snapshots {
        if let Some(current) = proof.parent_delivery().no_release_required() {
            let operation = current.record().operation_id;
            // A current in-memory proof cannot substitute for a failed seal
            // once that exact operation has actually entered durable history.
            // Pending/unsupported never-sealed outcomes keep their existing
            // current-publication semantics; they gain no historical credit.
            if state.no_release_inventory_error.is_some()
                || (state.no_release_sealed_operations.contains(&operation)
                    && no_release_errors.contains_key(&operation))
            {
                continue;
            }
        }
        if let Some(carry) = proof.parent_delivery().published_carry()
            && projection
                .parent(proof.authority().workspace_id(), proof.work().parent)
                .and_then(|parent| carry.verify_current_projection(config, parent))
                .is_err()
        {
            continue;
        }
        if evidence_projection_current(
            proof.authority(),
            proof.observed_at(),
            projection,
            revision,
            fences,
            now,
        ) {
            delivery.insert(parent.clone(), proof.parent_delivery().clone());
            outcomes.merge(proof.outcomes());
        }
    }
    outcomes.merge(&released);
    outcomes.merge(&no_release);
    (delivery, outcomes)
}
fn fresh_admission_authority(
    expected: &WorkerAuthority,
    projection: &crate::reconcile::RuntimeProjection,
    outcomes: &DependencyOutcomes,
) -> Result<WorkerAuthority> {
    let workspace = match expected {
        WorkerAuthority::Authorized(gate) => gate.workspace_id(),
        WorkerAuthority::Independent(gate) => gate.workspace_id(),
    };
    if !projection
        .workspaces
        .get(&workspace)
        .is_some_and(|w| w.complete)
    {
        return Err(Error::new(
            "incomplete_snapshot",
            "new admission requires a complete selected workspace observation",
        ));
    }
    match expected {
        WorkerAuthority::Authorized(gate) => {
            if gate.purpose() == GatePurpose::DrainExisting {
                return Err(Error::new(
                    "scope_violation",
                    "a draining operation cannot authorize a new session",
                ));
            }
            let parent = projection.parent(gate.workspace_id(), gate.work().parent)?;
            if !parent.included {
                return Err(Error::new(
                    "excluded",
                    "parent no longer matches selection for a new session",
                ));
            }
            let current = parent.fresh_gate(
                gate.work().child,
                gate.purpose(),
                outcomes,
                Utc::now(),
                Duration::seconds(30),
            )?;
            gate.compare_current(&current, Utc::now(), Duration::seconds(30))?;
            Ok(WorkerAuthority::Authorized(current))
        }
        WorkerAuthority::Independent(gate) => {
            let current = projection.independent_gate(
                gate.workspace_id(),
                gate.repository_id(),
                gate.pull_request().number,
                Utc::now(),
                Duration::seconds(30),
            )?;
            gate.compare_current(&current, Utc::now(), Duration::seconds(30))?;
            Ok(WorkerAuthority::Independent(current))
        }
    }
}
fn record_account_limits(
    control: &ControlHost,
    state: &Mutex<State>,
    account: &str,
    observed: &Value,
) -> Result<()> {
    control.serialize_observation(|_| {
        let mut state = state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "account metrics lock failed"))?;
        state
            .quotas
            .insert(account.into(), observed_quota(observed, Utc::now()));
        state.rate_observations.insert(
            account.into(),
            json!({"observed_at":Utc::now(),"limits":observed}),
        );
        Ok(())
    })
}
fn observed_quota(value: &Value, now: chrono::DateTime<Utc>) -> Quota {
    let limits = value
        .get("rateLimitsByLimitId")
        .and_then(Value::as_object)
        .map(|map| map.values().collect::<Vec<_>>())
        .unwrap_or_else(|| value.get("rateLimits").into_iter().collect());
    let windows: Vec<_> = limits
        .iter()
        .flat_map(|limit| [limit.get("primary"), limit.get("secondary")])
        .flatten()
        .filter(|window| !window.is_null())
        .collect();
    if windows.is_empty() {
        return Quota::Unknown;
    }
    let mut reset = None;
    let mut exhausted = false;
    let mut unknown = false;
    let mut exhausted_without_reset = false;
    for window in &windows {
        match window.get("usedPercent").and_then(Value::as_f64) {
            Some(used) if used.is_finite() && (0.0..=100.0).contains(&used) => {
                if used >= 100.0 {
                    exhausted = true;
                    if let Some(at) = window
                        .get("resetsAt")
                        .and_then(Value::as_i64)
                        .and_then(|at| chrono::DateTime::from_timestamp(at, 0))
                    {
                        reset = Some(reset.map_or(at, |old: chrono::DateTime<Utc>| old.max(at)));
                    } else {
                        exhausted_without_reset = true;
                    }
                }
            }
            _ => unknown = true,
        }
    }
    if exhausted {
        Quota::Exhausted {
            observed_at: now,
            reset_at: if exhausted_without_reset { None } else { reset },
        }
    } else if unknown {
        Quota::Unknown
    } else {
        Quota::Available { observed_at: now }
    }
}

fn require_joined_readmission(completed: &RunCompletion) -> Result<()> {
    if !completed.local_exit_confirmed
        || completed.hook_error.is_some()
        || completed.error.is_some()
        || !matches!(completed.outcome, crate::worker_run::RunOutcome::Completed)
        || completed.state.turn_id.is_some()
        || completed.state.unresolved_turn.is_some()
        || completed.state.terminal.as_deref() != Some("completed")
        || completed.state.intervention.is_some()
        || completed.state.input_required
    {
        return Err(Error::new(
            "reconciliation_required",
            "readmission requires a joined completed session without unresolved provider or hook state",
        ));
    }
    Ok(())
}

fn readmission_spacing(
    predecessor: &crate::control::Receipt,
    cause: &crate::control::Receipt,
    now: chrono::DateTime<Utc>,
) -> Result<chrono::DateTime<Utc>> {
    let not_before = predecessor.updated_at.max(cause.updated_at) + Duration::seconds(1);
    if now < not_before {
        return Err(Error::new(
            "retry_wait",
            "unfinished retry must wait at least one second after both actual exits",
        )
        .detail(json!({"not_before":not_before})));
    }
    Ok(not_before)
}

#[derive(Clone, Copy)]
struct WorkerPlanning<'a> {
    readmission: Option<WorkerReadmission<'a>>,
    round: Option<&'a crate::delivery::VerifiedReviewRoundPlan>,
}
struct WorkerAdmissionContext<'a> {
    planning: WorkerPlanning<'a>,
    session_resume: Option<(
        &'a crate::worker_run::VerifiedSessionResume,
        &'a crate::housekeeping::VerifiedDirectoryReuse,
    )>,
    paired_resume: Option<&'a crate::delivery::VerifiedPairedRecoveryCandidate>,
    preparation: Option<crate::control::ReviewReceiptReference>,
    parent_reassembly: Option<&'a crate::delivery::ParentReassemblyWork>,
}
fn request_readmission(
    request: &crate::runtime_workers::LaunchRequest,
) -> Result<Option<WorkerReadmission<'_>>> {
    match (
        &request.unfinished,
        &request.source_advanced,
        &request.retried_unfinished,
        &request.review_retry,
        &request.validation_retry,
    ) {
        (Some(p), None, None, None, None) => Ok(Some(WorkerReadmission::Unfinished(p))),
        (None, Some(p), None, None, None) => Ok(Some(WorkerReadmission::SourceAdvanced(p))),
        (None, None, Some(p), None, None) => Ok(Some(WorkerReadmission::Retried(p))),
        (None, None, None, Some(p), None) => Ok(Some(WorkerReadmission::ReviewRetry(p))),
        (None, None, None, None, Some(p)) => Ok(Some(WorkerReadmission::ValidationRetry(p))),
        (None, None, None, None, None) => Ok(None),
        _ => Err(Error::new(
            "worker_binding",
            "conflicting round readmission causes",
        )),
    }
}
fn round_run_cause(
    readmission: Option<WorkerReadmission<'_>>,
) -> Result<crate::control::ReviewRoundRunCause> {
    use crate::control::ReviewRoundRunCause;
    match readmission {
        None | Some(WorkerReadmission::ReviewRetry(_)) => Ok(ReviewRoundRunCause::Review),
        Some(WorkerReadmission::Unfinished(p)) => {
            Ok(ReviewRoundRunCause::PairedRepair(continuation_reference(p)))
        }
        Some(WorkerReadmission::SourceAdvanced(p)) => Ok(ReviewRoundRunCause::AfterRepair(
            source_advanced_reference(p),
        )),
        Some(WorkerReadmission::ValidationRetry(_)) => Err(Error::new(
            "worker_binding",
            "validation retry preserves source lineage without allocating a review round",
        )),
        Some(WorkerReadmission::Retried(_)) => Err(Error::new(
            "worker_binding",
            "new implementation grant needs its own round lineage",
        )),
    }
}

#[derive(Clone, Copy)]
enum WorkerReadmission<'a> {
    Unfinished(&'a crate::delivery::UnfinishedChildWork),
    SourceAdvanced(&'a crate::delivery::VerifiedSourceAdvanced),
    Retried(&'a crate::delivery::RetriedUnfinishedWork),
    ReviewRetry(&'a crate::delivery::ReviewRetryWork),
    ValidationRetry(&'a crate::delivery::ValidationRetryWork),
}
impl<'a> WorkerReadmission<'a> {
    fn authority(self) -> &'a crate::reconcile::FreshGate {
        match self {
            Self::Unfinished(p) => p.authority(),
            Self::SourceAdvanced(p) => p.authority(),
            Self::Retried(p) => p.authority(),
            Self::ReviewRetry(p) => p.authority(),
            Self::ValidationRetry(p) => p.authority(),
        }
    }
    fn phase(self) -> &'a crate::scheduler::PhaseKey {
        match self {
            Self::Unfinished(p) => p.phase(),
            Self::SourceAdvanced(p) => p.phase(),
            Self::Retried(p) => p.phase(),
            Self::ReviewRetry(p) => p.phase(),
            Self::ValidationRetry(p) => p.phase(),
        }
    }
    fn predecessor(self) -> Uuid {
        match self {
            Self::Unfinished(p) => p.predecessor(),
            Self::SourceAdvanced(p) => p.predecessor(),
            Self::Retried(p) => p.original().predecessor(),
            Self::ReviewRetry(p) => p.predecessor(),
            Self::ValidationRetry(p) => p.predecessor(),
        }
    }
    fn cause_operation(self) -> Uuid {
        match self {
            Self::Unfinished(p) => p.failure_operation(),
            Self::SourceAdvanced(p) => p.source_operation(),
            Self::Retried(p) => p.original().failure_operation(),
            Self::ReviewRetry(p) => p.predecessor(),
            Self::ValidationRetry(p) => p.predecessor(),
        }
    }
    fn cause_receipt_digest(self) -> &'a str {
        match self {
            Self::Unfinished(p) => p.failure_receipt_digest(),
            Self::SourceAdvanced(p) => p.source_receipt_digest(),
            Self::Retried(p) => p.original().failure_receipt_digest(),
            Self::ReviewRetry(p) => p.predecessor_receipt_digest(),
            Self::ValidationRetry(p) => p.predecessor_receipt_digest(),
        }
    }
    fn completed_operation(self) -> Uuid {
        match self {
            Self::Unfinished(p) => p.predecessor(),
            Self::SourceAdvanced(p) => p.source_operation(),
            Self::Retried(p) => p.original().predecessor(),
            Self::ReviewRetry(p) => p.predecessor(),
            Self::ValidationRetry(p) => p.predecessor(),
        }
    }
    fn source(self) -> &'a crate::delivery::DeliverySource {
        match self {
            Self::Unfinished(p) => p.source(),
            Self::SourceAdvanced(p) => p.source(),
            Self::Retried(p) => p.source(),
            Self::ReviewRetry(p) => p.source(),
            Self::ValidationRetry(p) => p.source(),
        }
    }
    fn verify_contract(self, contract: &WorkerContract) -> Result<()> {
        match self {
            Self::Unfinished(p) => p.verify_contract(contract),
            Self::SourceAdvanced(p) => p.verify_contract(contract),
            Self::Retried(p) => p.verify_contract(contract),
            Self::ReviewRetry(p) => p.verify_contract(contract),
            Self::ValidationRetry(p) => p.verify_contract(contract),
        }
    }
    fn verify_inventory(self, receipts: &[crate::control::Receipt]) -> Result<()> {
        match self {
            Self::Unfinished(proof) => proof.verify_failure_inventory(receipts),
            Self::SourceAdvanced(proof) => proof.verify_predecessor_inventory(receipts),
            Self::Retried(proof) => proof.original().verify_failure_inventory(receipts),
            Self::ReviewRetry(proof) => proof.verify_inventory(receipts),
            Self::ValidationRetry(proof) => proof.verify_inventory(receipts),
        }
    }
}
fn source_advanced_reference(
    proof: &crate::delivery::VerifiedSourceAdvanced,
) -> crate::control::SourceAdvancedReference {
    crate::control::SourceAdvancedReference {
        predecessor: proof.predecessor(),
        predecessor_receipt_digest: proof.predecessor_receipt_digest().into(),
        source_operation: proof.source_operation(),
        source_receipt_digest: proof.source_receipt_digest().into(),
        trigger_digest: proof.digest().into(),
        progress_digest: proof.progress_digest().into(),
        review_rounds_consumed: proof.review_rounds_consumed(),
    }
}

fn continuation_reference(
    proof: &crate::delivery::UnfinishedChildWork,
) -> crate::control::RunContinuationReference {
    crate::control::RunContinuationReference {
        predecessor: proof.predecessor(),
        predecessor_receipt_digest: proof.predecessor_receipt_digest().into(),
        trigger_digest: proof.digest().into(),
        progress_digest: proof.progress_digest().into(),
        failure_operation: proof.failure_operation(),
        review_rounds_consumed: proof.review_rounds_consumed(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_retry_preserves_delay_after_both_actual_local_exits() {
        let (_temp, host, lease) = admission_host();
        let first = host
            .admit(
                &lease,
                Uuid::new_v4(),
                "fixture-first".into(),
                canonical_digest(1, &"scope").unwrap(),
                || Ok(()),
            )
            .unwrap();
        host.retire_local(first.operation_id, json!({"fixture":"no child process"}))
            .unwrap();
        let first = host.operation(first.operation_id).unwrap();
        let last = host
            .admit(
                &lease,
                Uuid::new_v4(),
                "fixture-last".into(),
                canonical_digest(1, &"scope").unwrap(),
                || Ok(()),
            )
            .unwrap();
        host.retire_local(last.operation_id, json!({"fixture":"no child process"}))
            .unwrap();
        let last = host.operation(last.operation_id).unwrap();
        let exit = first.updated_at.max(last.updated_at);
        assert_eq!(
            readmission_spacing(&first, &last, exit + Duration::milliseconds(999))
                .unwrap_err()
                .code,
            "retry_wait"
        );
        assert_eq!(
            readmission_spacing(&first, &last, exit + Duration::seconds(1)).unwrap(),
            exit + Duration::seconds(1)
        );
    }
    #[test]
    fn unfinished_selection_requires_joined_terminal_session_without_blockers() {
        use crate::codex::SessionState;
        use kegari_codex::{CodexProfile, ProviderIntervention, SessionBinding};
        let operation = Uuid::from_u128(44);
        let digest = canonical_digest(1, &"fixture").unwrap();
        let mut session = SessionState::new(
            SessionBinding {
                schema_version: 1,
                account_id: "fixture".into(),
                work_identity: "fixture-child".into(),
                operation_id: operation.to_string(),
                generation: digest.clone(),
                instruction_digest: digest.clone(),
                isolation_digest: digest.clone(),
                profile: CodexProfile {
                    model: "fixture".into(),
                    reasoning: "high".into(),
                    speed: "standard".into(),
                },
                cwd: "/fixture".into(),
            },
            3,
        )
        .unwrap();
        session.terminal = Some("completed".into());
        // Diagnostic state exercises the predicate only: this deliberately has
        // no source/contract/check capabilities and cannot mint unfinished work.
        let completion = || RunCompletion {
            operation_id: operation,
            state: session.clone(),
            outcome: crate::worker_run::RunOutcome::Completed,
            error: None,
            elapsed_ms: 1,
            hook_error: None,
            checks: vec![],
            failed_checks: vec![],
            validation_plan: None,
            validation_artifacts: vec![],
            validation_attempts: vec![],
            validation_error: None,
            validation_history: None,
            execution: None,
            contract: None,
            local_exit_confirmed: true,
        };
        require_joined_readmission(&completion()).unwrap();
        for case in 0..8 {
            let mut value = completion();
            match case {
                0 => value.local_exit_confirmed = false,
                1 => value.hook_error = Some(Error::new("operation_uncertain", "hook pending")),
                2 => value.error = Some(Error::new("transport_closed", "missing final reply")),
                3 => value.outcome = crate::worker_run::RunOutcome::Failed,
                4 => value.state.turn_id = Some("still-active".into()),
                5 => value.state.terminal = None,
                6 => value.state.input_required = true,
                _ => {
                    value.state.intervention = Some(ProviderIntervention {
                        schema_version: 1,
                        account_id: "fixture".into(),
                        thread_id: "thread".into(),
                        turn_id: "turn".into(),
                        source_method: "turn/completed".into(),
                        reason_code: "cyberPolicy".into(),
                        required_resolution: "provider resolution".into(),
                        observed_at: Utc::now().to_rfc3339(),
                        source_digest: digest.clone(),
                    })
                }
            }
            assert_eq!(
                require_joined_readmission(&value).unwrap_err().code,
                "reconciliation_required"
            );
        }
    }
    fn admission_host() -> (tempfile::TempDir, Arc<ControlHost>, Lease) {
        // This fixture exercises actual serialized admission and durable
        // receipts, without claiming native orchestration/process ownership.
        let temp = tempfile::tempdir().unwrap();
        let runtime = Uuid::from_u128(1);
        let host = Arc::new(
            ControlHost::setup(
                PrivateDir::create(&temp.path().canonicalize().unwrap().join("control")).unwrap(),
                runtime,
                Uuid::new_v4(),
            )
            .unwrap(),
        );
        host.lifecycle(
            crate::control::MutationRequest {
                request_id: Uuid::new_v4(),
                runtime_id: runtime,
                actor: kegari_control_api::identity::Actor {
                    identity: "fixture".into(),
                    principal: None,
                    roles: ["lifecycle".into()].into(),
                },
                kind: "lifecycle".into(),
                canonical_targets: vec![runtime.to_string()],
                expected: BTreeMap::new(),
                payload: Value::Null,
            },
            crate::control::Lifecycle::Start,
            0,
        )
        .unwrap();
        let lease = Lease {
            runtime_id: runtime,
            owner_generation: 1,
            owner_id: Uuid::new_v4(),
        };
        host.attach_reconciled(lease.clone()).unwrap();
        (temp, host, lease)
    }
    #[test]
    fn filter_exclusion_drains_only_existing_admission_and_never_creates_another() {
        let (_temp, host, lease) = admission_host();
        let (mut projection, coordination) = cache_projection_fixture();
        let outcomes = DependencyOutcomes::default();
        let original = projection
            .parent(coordination.workspace_id(), coordination.work().parent)
            .unwrap()
            .fresh_gate(
                None,
                GatePurpose::NewOperation,
                &outcomes,
                Utc::now(),
                Duration::seconds(30),
            )
            .unwrap();
        let authority = WorkerAuthority::Authorized(original.clone());
        let scope = canonical_digest(1, &"fixture scope").unwrap();
        let admitted = host
            .admit_observed(
                &lease,
                Uuid::new_v4(),
                authority.work_key().unwrap(),
                scope.clone(),
                |_| fresh_admission_authority(&authority, &projection, &outcomes).map(|_| ()),
            )
            .unwrap();
        let parent = projection
            .workspaces
            .get_mut(&original.workspace_id())
            .unwrap()
            .parents
            .get_mut(&original.work().parent)
            .unwrap();
        parent.included = false;
        parent.blockers.push(crate::reconcile::Blocker {
            code: "excluded".into(),
            reason: "actual parent selector changed".into(),
            affected: parent.key.clone(),
            impacted_gates: vec!["admission".into()],
            last_verified_evidence: None,
            attempted_recovery: "fresh provider read".into(),
            next_action: "preserve current work and drain".into(),
            prerequisite: None,
        });
        // The config and work generation did not change, and the existing
        // admitted operation still has its legitimate bounded drain path.
        host.validate_permit(&admitted, || {
            parent
                .retain_active_gate(&original, &outcomes, Utc::now(), Duration::seconds(30))
                .map(|_| ())
        })
        .unwrap();
        host.retire_local(
            admitted.operation_id,
            json!({"fixture":"no subprocess launched"}),
        )
        .unwrap();
        for expected in [authority, WorkerAuthority::Authorized(coordination)] {
            let request = Uuid::new_v4();
            let error = host
                .admit_observed(
                    &lease,
                    request,
                    expected.work_key().unwrap(),
                    scope.clone(),
                    |_| fresh_admission_authority(&expected, &projection, &outcomes).map(|_| ()),
                )
                .unwrap_err();
            assert_eq!(error.code, "excluded");
            assert_eq!(host.operation(request).unwrap_err().code, "record_missing");
        }
        let draining = projection
            .parent(original.workspace_id(), original.work().parent)
            .unwrap()
            .fresh_gate(
                None,
                GatePurpose::DrainExisting,
                &outcomes,
                Utc::now(),
                Duration::seconds(30),
            )
            .unwrap();
        assert_eq!(
            fresh_admission_authority(
                &WorkerAuthority::Authorized(draining),
                &projection,
                &outcomes
            )
            .unwrap_err()
            .code,
            "scope_violation"
        );
        assert!(host.snapshot().unwrap().active_operations.is_empty());
    }
    #[test]
    fn failed_delivery_observation_cannot_overtake_serialized_admission() {
        let (_temp, host, lease) = admission_host();
        let state = Arc::new(Mutex::new(State::default()));
        let parent = IssueKey::new(
            "https://api.linear.app/graphql",
            Uuid::from_u128(2),
            Uuid::from_u128(3),
        )
        .unwrap();
        let identity = parent.identity().unwrap();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (continue_tx, continue_rx) = std::sync::mpsc::channel();
        let admission = {
            let host = host.clone();
            let state = state.clone();
            let identity = identity.clone();
            let lease = lease.clone();
            std::thread::spawn(move || {
                host.admit_observed(
                    &lease,
                    Uuid::new_v4(),
                    identity,
                    canonical_digest(1, &"fixture scope").unwrap(),
                    |_| {
                        assert!(state.lock().unwrap().delivery_errors.is_empty());
                        entered_tx.send(()).unwrap();
                        continue_rx
                            .recv_timeout(std::time::Duration::from_secs(5))
                            .unwrap();
                        // No provider evidence is synthesized here: this test
                        // checks actual cache/admission ordering only.
                        Ok(())
                    },
                )
            })
        };
        entered_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        let (attempt_tx, attempt_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let invalidation = {
            let host = host.clone();
            let state = state.clone();
            std::thread::spawn(move || {
                attempt_tx.send(()).unwrap();
                let result = invalidate_delivery(
                    &host,
                    &state,
                    &parent,
                    Error::new("provider_unavailable", "fixture provider read failed"),
                );
                done_tx.send(()).unwrap();
                result
            })
        };
        attempt_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        assert!(matches!(
            done_rx.recv_timeout(std::time::Duration::from_millis(100)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ));
        assert!(state.lock().unwrap().delivery_errors.is_empty());
        continue_tx.send(()).unwrap();
        let permit = admission.join().unwrap().unwrap();
        invalidation.join().unwrap().unwrap();
        host.retire_local(permit.operation_id, json!({"fixture":"no process"}))
            .unwrap();
        let request = Uuid::new_v4();
        let error = host
            .admit_observed(
                &lease,
                request,
                identity.clone(),
                canonical_digest(1, &"fixture scope").unwrap(),
                |_| Err(state.lock().unwrap().delivery_errors[&identity].clone()),
            )
            .unwrap_err();
        assert_eq!(error.code, "provider_unavailable");
        assert_eq!(host.operation(request).unwrap_err().code, "record_missing");
        assert!(host.snapshot().unwrap().active_operations.is_empty());
    }
    #[test]
    fn quota_observation_and_admission_have_one_serialized_order() {
        let (_temp, host, lease) = admission_host();
        let state = Arc::new(Mutex::new(State::default()));
        record_account_limits(
            &host,
            &state,
            "fixture-account",
            &json!({"rateLimits":{"primary":{"usedPercent":20}}}),
        )
        .unwrap();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (continue_tx, continue_rx) = std::sync::mpsc::channel();
        let first = {
            let host = host.clone();
            let state = state.clone();
            let lease = lease.clone();
            std::thread::spawn(move || {
                host.admit_observed(
                    &lease,
                    Uuid::new_v4(),
                    "fixture-first".into(),
                    canonical_digest(1, &"scope").unwrap(),
                    |_| {
                        assert!(matches!(
                            state.lock().unwrap().quotas["fixture-account"],
                            Quota::Available { .. }
                        ));
                        entered_tx.send(()).unwrap();
                        continue_rx
                            .recv_timeout(std::time::Duration::from_secs(5))
                            .unwrap();
                        Ok(())
                    },
                )
            })
        };
        entered_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        let (attempt_tx, attempt_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let observation = {
            let host = host.clone();
            let state = state.clone();
            std::thread::spawn(move || {
                attempt_tx.send(()).unwrap();
                let result = record_account_limits(
                    &host,
                    &state,
                    "fixture-account",
                    &json!({"rateLimits":{"primary":{"usedPercent":100}}}),
                );
                done_tx.send(()).unwrap();
                result
            })
        };
        attempt_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        assert!(matches!(
            done_rx.recv_timeout(std::time::Duration::from_millis(100)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ));
        assert!(matches!(
            state.lock().unwrap().quotas["fixture-account"],
            Quota::Available { .. }
        ));
        continue_tx.send(()).unwrap();
        first.join().unwrap().unwrap();
        observation.join().unwrap().unwrap();
        assert!(matches!(
            state.lock().unwrap().quotas["fixture-account"],
            Quota::Exhausted { .. }
        ));
        let request = Uuid::new_v4();
        let error = host
            .admit_observed(
                &lease,
                request,
                "fixture-second".into(),
                canonical_digest(1, &"scope").unwrap(),
                |_| {
                    if matches!(
                        state.lock().unwrap().quotas["fixture-account"],
                        Quota::Exhausted { .. }
                    ) {
                        Err(Error::new(
                            "quota_wait",
                            "observed exhaustion precedes admission",
                        ))
                    } else {
                        Ok(())
                    }
                },
            )
            .unwrap_err();
        assert_eq!(error.code, "quota_wait");
        assert_eq!(host.operation(request).unwrap_err().code, "record_missing");
        assert_eq!(host.snapshot().unwrap().active_operations.len(), 1);
    }
    #[test]
    fn provider_hold_cannot_be_cleared_by_later_success_or_transplanted_binding() {
        use crate::codex::SessionState;
        use kegari_codex::{CodexProfile, ProviderIntervention, SessionBinding};
        let operation = Uuid::from_u128(1);
        let digest = canonical_digest(1, &"fixture").unwrap();
        let mut session = SessionState::new(
            SessionBinding {
                schema_version: 1,
                account_id: "account".into(),
                work_identity: digest.clone(),
                operation_id: operation.to_string(),
                generation: digest.clone(),
                instruction_digest: digest.clone(),
                isolation_digest: digest.clone(),
                profile: CodexProfile {
                    model: "fixture-model".into(),
                    reasoning: "high".into(),
                    speed: "standard".into(),
                },
                cwd: "/tmp/fixture".into(),
            },
            20,
        )
        .unwrap();
        let mut holds = BTreeMap::new();
        retain_provider_hold(&mut holds, operation, &session).unwrap();
        assert!(holds.is_empty());
        session.intervention = Some(ProviderIntervention {
            schema_version: 1,
            account_id: "account".into(),
            thread_id: "thread".into(),
            turn_id: "turn".into(),
            source_method: "turn/completed".into(),
            reason_code: "verification_required".into(),
            required_resolution: "Verified provider resolution".into(),
            observed_at: Utc::now().to_rfc3339(),
            source_digest: digest,
        });
        session.input_required = true;
        retain_provider_hold(&mut holds, operation, &session).unwrap();
        session.intervention = None;
        session.input_required = false;
        session.terminal = Some("completed".into());
        retain_provider_hold(&mut holds, operation, &session).unwrap();
        assert!(holds[&operation].intervention.is_some());
        assert!(holds[&operation].input_required);
        session.binding.account_id = "replacement-account".into();
        assert_eq!(
            retain_provider_hold(&mut holds, operation, &session)
                .unwrap_err()
                .code,
            "worker_binding"
        );
        assert_eq!(holds[&operation].binding.account_id, "account");
    }
    #[test]
    fn quota_uses_every_observed_window_and_never_invents_a_reset() {
        let now = Utc::now();
        let limits = json!({"rateLimitsByLimitId":{"a":{"primary":{"usedPercent":100,"resetsAt":1800000000},"secondary":{"usedPercent":100,"resetsAt":1800003600}},"b":{"primary":{"usedPercent":null}}}});
        assert!(
            matches!(observed_quota(&limits,now),Quota::Exhausted{reset_at:Some(at),..} if at.timestamp()==1800003600)
        );
        let limits = json!({"rateLimitsByLimitId":{"a":{"primary":{"usedPercent":100,"resetsAt":1800000000}},"b":{"primary":{"usedPercent":100}}}});
        assert!(matches!(
            observed_quota(&limits, now),
            Quota::Exhausted { reset_at: None, .. }
        ));
        for limits in [
            json!({}),
            json!({"rateLimits":{"primary":{"usedPercent":null}}}),
            json!({"rateLimits":{"primary":{"usedPercent":-1}}}),
        ] {
            assert!(matches!(observed_quota(&limits, now), Quota::Unknown));
        }
        assert!(matches!(
            observed_quota(
                &json!({"rateLimits":{"primary":{"usedPercent":99.5},"secondary":null}}),
                now
            ),
            Quota::Available { .. }
        ));
    }
    fn cache_projection_fixture() -> (
        crate::reconcile::RuntimeProjection,
        crate::reconcile::FreshGate,
    ) {
        use crate::{
            authority::Corrections,
            evidence::{Generation, WorkIdentity},
            reconcile::{ParentProjection, RuntimeProjection, WorkspaceProjection},
        };
        use kegari_config::{Profile, ReleaseMode, Speed};
        use kegari_digest::bytes_digest;
        let runtime = Uuid::from_u128(1);
        let workspace = Uuid::from_u128(2);
        let parent_id = Uuid::from_u128(3);
        let digest = bytes_digest(b"synthetic current provider projection");
        let corrections = Corrections::default();
        let work = WorkIdentity {
            linear_endpoint: "https://linear.invalid/graphql".into(),
            linear_workspace: Uuid::from_u128(4),
            parent: parent_id,
            child: None,
            generation: Generation {
                authorization: digest.clone(),
                definition: digest.clone(),
                scope: digest.clone(),
                correction: corrections.generation().unwrap(),
            },
        };
        let profile = Profile {
            tool: "codex".into(),
            model: "fixture-model".into(),
            reasoning: "high".into(),
            speed: Speed::Standard,
        };
        let now = Utc::now();
        let parent = ParentProjection {
            workspace_id: workspace,
            workspace_name: "fixture".into(),
            parent_reference: "fixture:TEST-3".into(),
            key: IssueKey::new(&work.linear_endpoint, work.linear_workspace, parent_id).unwrap(),
            issue: None,
            included: true,
            observed_at: now,
            required_children: std::collections::BTreeSet::from([Uuid::from_u128(6)]),
            children: BTreeMap::new(),
            observations: BTreeMap::new(),
            metadata: None,
            authorization_candidate: None,
            authorization_published: true,
            corrections,
            observation_revision: 1,
            disappeared_children: BTreeMap::new(),
            work: Some(work),
            blockers: vec![],
            prerequisites: Default::default(),
            source_digest: digest.clone(),
            config_revision: digest.clone(),
            workflow_digest: digest.clone(),
            coordination_profile: profile,
            drain_config_revision: digest.clone(),
            release_mode: ReleaseMode::Manual,
        };
        let gate = parent
            .fresh_gate(
                None,
                GatePurpose::Coordination,
                &DependencyOutcomes::default(),
                now,
                Duration::seconds(30),
            )
            .unwrap();
        let projection = RuntimeProjection {
            runtime_id: runtime,
            config_revision: digest.clone(),
            observed_at: now,
            workspaces: BTreeMap::from([(
                workspace,
                WorkspaceProjection {
                    workspace_id: workspace,
                    workspace_name: "fixture".into(),
                    complete: true,
                    observed_at: now,
                    parents: BTreeMap::from([(parent_id, parent)]),
                    repositories: BTreeMap::new(),
                    independent_jobs: vec![],
                    error: None,
                    config_revision: digest.clone(),
                    workflow_digest: digest,
                    independent_review_profile: None,
                },
            )]),
        };
        (projection, gate)
    }
    #[test]
    fn current_permission_fence_revokes_cached_dependency_before_refresh_or_revision_change() {
        let (projection, gate) = cache_projection_fixture();
        let now = Utc::now();
        let mut fences = crate::configuration::AuthorityFences::default();
        assert!(evidence_projection_current(
            &gate,
            now,
            &projection,
            &projection.config_revision,
            &fences,
            now
        ));
        fences.workspaces.insert(Uuid::from_u128(99));
        assert!(evidence_projection_current(
            &gate,
            now,
            &projection,
            &projection.config_revision,
            &fences,
            now
        ));
        fences.workspaces.insert(gate.workspace_id());
        // Identical last-good config and complete snapshot cannot restore a
        // separately persisted permission restriction on this prerequisite.
        assert!(!evidence_projection_current(
            &gate,
            now,
            &projection,
            &projection.config_revision,
            &fences,
            now
        ));
    }
    #[test]
    fn stale_or_incomplete_delivery_observations_cannot_satisfy_current_evidence() {
        let (projection, gate) = cache_projection_fixture();
        let now = Utc::now();
        let fences = crate::configuration::AuthorityFences::default();
        for scenario in [
            "expired_proof",
            "future_proof",
            "incomplete",
            "stale_parent",
            "source_changed",
            "authorization_removed",
            "pending_correction",
            "inventory_changed",
            "config_changed",
        ] {
            let mut current = projection.clone();
            let mut observed = now;
            let mut revision = projection.config_revision.clone();
            let workspace = current.workspaces.get_mut(&gate.workspace_id()).unwrap();
            let parent = workspace.parents.get_mut(&gate.work().parent).unwrap();
            match scenario {
                "expired_proof" => observed = now - Duration::seconds(31),
                "future_proof" => observed = now + Duration::seconds(6),
                "incomplete" => workspace.complete = false,
                "stale_parent" => parent.observed_at = now - Duration::seconds(31),
                "source_changed" => {
                    parent.source_digest = kegari_digest::bytes_digest(b"edited native record")
                }
                "authorization_removed" => parent.authorization_published = false,
                "pending_correction" => {
                    let actor = Uuid::from_u128(5);
                    parent
                        .corrections
                        .observe(
                            crate::authority::CommentRevision {
                                id: Uuid::from_u128(7),
                                author: Some(actor),
                                body_digest: kegari_digest::bytes_digest(
                                    b"pending native feedback",
                                ),
                                edited_at: None,
                                observed_revision: 2,
                            },
                            &std::collections::BTreeSet::from([actor]),
                            &Default::default(),
                            None,
                        )
                        .unwrap();
                }
                "inventory_changed" => {
                    parent.required_children.insert(Uuid::from_u128(8));
                }
                "config_changed" => revision = kegari_digest::bytes_digest(b"new configuration"),
                _ => unreachable!(),
            }
            assert!(
                !evidence_projection_current(&gate, observed, &current, &revision, &fences, now),
                "{scenario}"
            );
        }
    }
}
