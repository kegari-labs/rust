//! Host composition state. All worker callbacks consume current provider proofs
//! and actual process registration; worker summaries never establish delivery.
use crate::{
    app::{ConfigurationControl, RuntimeInspection},
    authority::IssueKey,
    control::{ControlHost, Lease, Permit, RunReference},
    digest::{bytes_digest, canonical_digest},
    error::{Error, Result},
    host::RuntimeDependencies,
    observations::ObservationCache,
    platform::Registration,
    reconcile::{DependencyOutcomes, GatePurpose, HistoryRow, ParentDelivery},
    scheduler::Quota,
    storage::PrivateDir,
    worker::{
        ExecutionRegistry, OutcomeSink, WorkerAuthority, WorkerContract, WorkerHost, WorkerOutcome,
    },
    worker_run::{OwnedWorkerRun, RunCompletion},
};
use chrono::{Duration, Utc};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

#[derive(Clone)]
struct RegisteredRun {
    permit: Permit,
    contract: Arc<WorkerContract>,
    budget: RunReference,
    _active: Option<crate::observations::ActiveSnapshot>,
    last_observation: Option<Value>,
    joined: bool,
}
#[derive(Default)]
struct State {
    runs: BTreeMap<Uuid, RegisteredRun>,
    prepared_active: BTreeMap<Uuid, crate::observations::ActiveSnapshot>,
    delivery: BTreeMap<IssueKey, ParentDelivery>,
    archived_delivery: BTreeMap<(IssueKey, String), ParentDelivery>,
    explicit_corrections: BTreeMap<Uuid, crate::decisions::VerifiedExplicitCorrection>,
    retry_grants: BTreeMap<Uuid, crate::decisions::VerifiedRetryGrant>,
    retry_errors: BTreeMap<String, Error>,
    outcomes: DependencyOutcomes,
    history: Vec<HistoryRow>,
    release_details: BTreeMap<IssueKey, Value>,
    quotas: BTreeMap<String, Quota>,
    rate_observations: BTreeMap<String, Value>,
    resume_proofs: BTreeMap<Uuid, String>,
    correction_inputs: BTreeMap<(Uuid, u64), (String, String)>,
    provider_holds: BTreeMap<Uuid, crate::codex::SessionState>,
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
}
impl RuntimeCoordinator {
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
        }))
    }
    pub fn next_attempt(
        &self,
        phase: &crate::scheduler::PhaseKey,
        workspace: Uuid,
        account: &str,
    ) -> Result<crate::runtime_schedule::RunSchedule> {
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
        let state = self.control.snapshot()?;
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
        crate::runtime_schedule::plan(&limits, phase, workspace, &receipts, Utc::now())
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
        Ok(self
            .state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "evidence cache lock failed"))?
            .delivery
            .get(parent)
            .cloned()
            .unwrap_or_default())
    }
    pub fn dependency_outcomes(&self) -> Result<DependencyOutcomes> {
        Ok(self
            .state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "dependency cache lock failed"))?
            .outcomes
            .clone())
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
        self.state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "acceptance cache lock failed"))?
            .delivery
            .get_mut(parent)
            .ok_or_else(|| Error::new("evidence_missing", "parent evidence has not been hydrated"))?
            .acceptance(acceptance)?;
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
    pub(crate) fn retry_target(
        &self,
        work: &crate::evidence::WorkIdentity,
    ) -> Result<crate::decisions::RetryTarget> {
        let receipts = self.control.operations()?;
        let canonical = IssueKey::new(
            &work.linear_endpoint,
            work.linear_workspace,
            work.child.unwrap_or(work.parent),
        )?
        .identity()?;
        if let Some(error) = self
            .state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "retry cache lock failed"))?
            .retry_errors
            .get(&canonical)
        {
            return Err(error.clone());
        }
        let grants = crate::runtime_workflow::current_retry_grants(
            &self.configuration.snapshot()?,
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
        crate::runtime_workflow::select_retry_target(self.runtime_id, work, &receipts, &grants)
    }
    /// Rebuild retry capabilities from current provider records. This is also
    /// used while STOPPED; it never reserves a run or enlarges a phase budget.
    pub fn refresh_retry_references(
        &self,
        projection: &crate::reconcile::RuntimeProjection,
    ) -> Result<()> {
        let config = self.configuration.snapshot()?;
        if projection.runtime_id != self.runtime_id
            || projection.config_revision != config.revision()?
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
            if self.configuration.snapshot()?.revision()? != config.revision()?
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
    pub fn admit_worker(
        &self,
        lease: &Lease,
        input: crate::control::RunAdmission,
        contract: &WorkerContract,
    ) -> Result<Permit> {
        let reference = input.reference.clone();
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
        let permit = self.control.admit_run_observed(lease, input, |snapshot| {
            self.registration.verify_active_owner(lease)?;
            self.require_provider_session_clear(&reference.account, &reference.phase.work)?;
            self.current_authority(contract.authority())?;
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
                    if gate.config_revision() != config.revision()? {
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
                    if gate.config_revision() != config.revision()? {
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
            let active = snapshot
                .active_operations
                .iter()
                .map(|permit| {
                    RunReference::from_receipt(&self.control.operation(permit.operation_id)?)
                })
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            let quota = self
                .state
                .lock()
                .map_err(|_| Error::new("runtime_unavailable", "quota state lock failed"))?
                .quotas
                .get(&reference.account)
                .cloned();
            crate::runtime_schedule::ensure_capacity(&config, &active, &reference, quota.as_ref())
        })?;
        self.state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "active snapshot registration failed"))?
            .prepared_active
            .insert(permit.operation_id, active);
        Ok(permit)
    }
    pub fn register_run(&self, permit: Permit, contract: Arc<WorkerContract>) -> Result<()> {
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
        state.runs.insert(
            permit.operation_id,
            RegisteredRun {
                permit,
                contract,
                budget,
                _active: Some(active),
                last_observation: None,
                joined: false,
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
    /// Returned checks remain sealed local observations, not child completion.
    pub async fn collect_finished(&self) -> Result<Vec<RunCompletion>> {
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
                    if completion.local_exit_confirmed {
                        self.control.retire_local(id, observed.clone())?;
                    }
                    let mut state = self.state.lock().map_err(|_| {
                        Error::new("runtime_unavailable", "worker state lock failed")
                    })?;
                    let tracked = state.runs.get_mut(&id).ok_or_else(|| {
                        Error::new(
                            "ownership_conflict",
                            "joined worker registration disappeared",
                        )
                    })?;
                    tracked.last_observation = Some(observed);
                    tracked.joined = true;
                    if completion.local_exit_confirmed {
                        tracked._active = None;
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
                }
            }
        }
        Ok(completed)
    }
    /// Re-observe handles after cancellation/timeout. A previously reported
    /// uncertain task stays blocked until every actual registered handle exits.
    pub fn observe_local_exits(&self) -> Result<()> {
        let candidates: Vec<_> = self
            .state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "worker state lock failed"))?
            .runs
            .iter()
            .filter(|(_, run)| run.joined)
            .map(|(id, run)| (*id, run.last_observation.clone()))
            .collect();
        for (id, observation) in candidates {
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
                    json!({"owned_handles":"observed_exited","prior":observation}),
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
        Ok(())
    }
    pub(crate) fn retained_authority(&self, expected: &WorkerAuthority) -> Result<WorkerAuthority> {
        match expected {
            WorkerAuthority::Authorized(gate) => {
                let outcomes = self
                    .state
                    .lock()
                    .map_err(|_| Error::new("runtime_unavailable", "dependency proof lock failed"))?
                    .outcomes
                    .clone();
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
        Ok(RuntimeInspection {
            projection,
            delivery: state.delivery.clone(),
            outcomes: state.outcomes.clone(),
            activities,
            history: state.history.clone(),
            release_details: state.release_details.clone(),
        })
    }
    pub fn account_observations(&self) -> Result<Value> {
        let state = self
            .state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "account metrics lock failed"))?;
        let holds=state.provider_holds.iter().map(|(id,session)| json!({"operation_id":id,"account":session.binding.account_id,"work":session.binding.work_identity,"intervention":session.intervention,"input_required":session.input_required})).collect::<Vec<_>>();
        Ok(
            json!({"quota":state.quotas,"observations":state.rate_observations,"provider_holds":holds,"retry_recovery":state.retry_errors}),
        )
    }
    pub fn execution_registry(&self) -> Arc<ExecutionRegistry> {
        self.executions.clone()
    }
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
            self.current_authority(run.contract.authority())?;
            let manifest = &run.contract.instructions().manifest;
            if run.joined
                || outcome.schema_version != 1
                || outcome.proposal_id.is_nil()
                || outcome.runtime_id != run.permit.lease.runtime_id
                || outcome.work.as_ref() != run.contract.authority().work()
                || outcome.fresh_gate_digest != run.contract.authority().digest()?
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
            {
                return Err(Error::new(
                    "worker_binding",
                    "outcome differs from its current admitted completed session",
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
            let payload = serde_json::to_value(outcome)
                .map_err(|_| Error::invalid("pending worker claim encoding failed"))?;
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
            let _lock = self.claims.lock("pending-worker.lock")?;
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
        let registered = self
            .state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "worker registration lock failed"))?
            .runs
            .values()
            .any(|run| {
                run._active.is_some()
                    && run.contract.authority().digest().as_ref() == Ok(&expected_digest)
            });
        if registered {
            return self.retained_authority(expected);
        }
        let config = self.configuration.snapshot()?;
        let workspace = match expected {
            WorkerAuthority::Authorized(g) => g.workspace_id(),
            WorkerAuthority::Independent(g) => g.workspace_id(),
        };
        self.configuration
            .authority_fences()?
            .check_workspace(workspace)?;
        let projection = self.observations.admission_snapshot()?;
        if projection.config_revision != config.revision()? {
            return Err(Error::new(
                "stale_config",
                "provider observation predates current configuration",
            ));
        }
        match expected {
            WorkerAuthority::Authorized(g) => {
                let outcomes = self
                    .state
                    .lock()
                    .map_err(|_| Error::new("runtime_unavailable", "dependency proof lock failed"))?
                    .outcomes
                    .clone();
                let purpose = if g.purpose() == GatePurpose::Coordination {
                    GatePurpose::Coordination
                } else {
                    GatePurpose::DrainExisting
                };
                let current = projection
                    .parent(g.workspace_id(), g.work().parent)?
                    .fresh_gate(
                        g.work().child,
                        purpose,
                        &outcomes,
                        Utc::now(),
                        Duration::seconds(30),
                    )?;
                g.compare_current(&current, Utc::now(), Duration::seconds(30))?;
                Ok(WorkerAuthority::Authorized(current))
            }
            WorkerAuthority::Independent(g) => {
                let current = projection.independent_gate(
                    g.workspace_id(),
                    g.repository_id(),
                    g.pull_request().number,
                    Utc::now(),
                    Duration::seconds(30),
                )?;
                g.compare_current(&current, Utc::now(), Duration::seconds(30))?;
                Ok(WorkerAuthority::Independent(current))
            }
        }
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
        let mut state = self
            .state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "account metrics lock failed"))?;
        let account = state
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
        let quota = observed_quota(observed, Utc::now());
        state.quotas.insert(account.clone(), quota);
        state
            .rate_observations
            .insert(account, json!({"observed_at":Utc::now(),"limits":observed}));
        Ok(())
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
        let evidence = if let Some(work) = run.contract.authority().work() {
            let parent = IssueKey::new(&work.linear_endpoint, work.linear_workspace, work.parent)?;
            state.delivery.get(&parent).and_then(|delivery|work.child.and_then(|child|delivery.verified_children().get(&child))).filter(|proof|&proof.evidence().work==work).map(|proof|json!({"verified":true,"digest":proof.digest(),"evidence":proof.evidence()}))
        } else {
            None
        };
        Ok(
            json!({"operation_id":operation,"phase":run.budget.phase.phase,"attempt":run.budget.attempt,"attempt_bound":run.budget.max_attempts,"completed":evidence.is_some(),"evidence":evidence,"last_observation":run.last_observation}),
        )
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn provider_hold_cannot_be_cleared_by_later_success_or_transplanted_binding() {
        use crate::codex::{CodexProfile, ProviderIntervention, SessionBinding, SessionState};
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
}
