//! Native scheduler driver. Owned launch tasks survive pause/force until their
//! actual completion is joined; recovery preserves uncertain process/effect holds.
use crate::{
    config::Config,
    control::{ControlSnapshot, Intent, Lease, ReceiptPhase},
    digest::canonical_digest,
    error::{Error, Result},
    preparation::AuthorizationPublisher,
    process_records::{ProcessJournal, ProcessRecovery},
    runtime::{RuntimeAction, RuntimeCoordinator, WorkerAction},
    runtime_workers::{self, LaunchRequest},
    scheduler::{Limits, Scheduler},
    storage::PrivateDir,
    supervisor::{DrainState, SupervisorDriver, SupervisorFuture},
    worker::WorkerHost,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::task::JoinHandle;
use uuid::Uuid;

/// Concrete delivery services schedule bounded owned phases. Target-specific
/// prerequisite failures remain diagnostics; global ownership/storage failures
/// are returned. Recovery and maintenance never create new autonomous effects.
pub trait RuntimeProgression: Send + Sync {
    fn reconcile(&self) -> SupervisorFuture<'_, ()>;
    fn advance<'a>(&'a self, lease: &'a Lease) -> SupervisorFuture<'a, ()>;
    fn maintain(&self) -> SupervisorFuture<'_, ()>;
    fn cancel_fenced<'a>(&'a self, operations: &'a [Uuid]) -> SupervisorFuture<'a, ()>;
}
pub struct RuntimeDriverServices {
    pub runtime: Arc<RuntimeCoordinator>,
    pub delivery: Arc<dyn RuntimeProgression>,
    pub corrections: Arc<crate::corrections::CorrectionPublisher>,
    pub processes: Arc<ProcessJournal>,
}

#[derive(Clone, Copy)]
pub struct DriverLimits {
    /// A finite host bound in addition to the original admitted turn count.
    pub worker_seconds: u64,
    pub authorization_tasks: usize,
}
impl DriverLimits {
    fn validate(self) -> Result<Self> {
        if self.worker_seconds == 0
            || self.worker_seconds > 86_400
            || self.authorization_tasks == 0
            || self.authorization_tasks > 64
        {
            return Err(Error::invalid(
                "driver duration and authorization concurrency must be finite",
            ));
        }
        Ok(self)
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DispatchState {
    Dispatched,
    ModelOwned,
    Returned,
    Blocked,
    WorkerJoined,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DispatchRecord {
    schema_version: u32,
    #[serde(default)]
    correction: Option<CorrectionReference>,
    #[serde(default)]
    metadata: Option<MetadataDispatch>,
    runtime: Uuid,
    action: String,
    operation: Uuid,
    work: String,
    phase: String,
    lease: Lease,
    state: DispatchState,
    observed_at: chrono::DateTime<Utc>,
    error: Option<Error>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorrectionReference {
    worker_operation: Uuid,
    proposal: Uuid,
    digest: String,
    #[serde(default)]
    attempt: usize,
    #[serde(default)]
    stage_limit: usize,
    #[serde(default)]
    stages: BTreeMap<Uuid, String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MetadataDispatch {
    base: String,
    publication: Uuid,
    attempt: u32,
    max_attempts: u32,
    observed_admissions: std::collections::BTreeSet<Uuid>,
}
struct PendingTask {
    record: DispatchRecord,
    worker: bool,
    join: JoinHandle<Result<()>>,
}
struct DriverState {
    scheduler: Scheduler,
    tasks: BTreeMap<Uuid, PendingTask>,
    records: BTreeMap<String, DispatchRecord>,
    recovery: Vec<ProcessRecovery>,
    recovery_errors: BTreeMap<Uuid, Error>,
    recovery_cursor: Option<Uuid>,
    last_refresh: Option<tokio::time::Instant>,
    last_error: Option<Error>,
    dispatch_errors: BTreeMap<String, Error>,
}
pub struct RuntimeDriver {
    runtime: Arc<RuntimeCoordinator>,
    publisher: Arc<AuthorizationPublisher>,
    delivery: Arc<dyn RuntimeProgression>,
    corrections: Arc<crate::corrections::CorrectionPublisher>,
    processes: Arc<ProcessJournal>,
    records: Arc<PrivateDir>,
    limits: DriverLimits,
    state: Mutex<DriverState>,
    cycle: tokio::sync::Mutex<()>,
}
impl RuntimeDriver {
    pub fn new(services: RuntimeDriverServices, limits: DriverLimits) -> Result<Arc<Self>> {
        let runtime = services.runtime;
        let limits = limits.validate()?;
        let config = runtime.configuration.snapshot()?;
        let records = Arc::new(runtime.registration.root().child("driver-dispatch")?);
        let mut retained = BTreeMap::new();
        for (index, entry) in std::fs::read_dir(records.path())
            .map_err(|error| Error::io("enumerate driver records", error))?
            .enumerate()
        {
            if index >= 65_536 {
                return Err(Error::new(
                    "record_limit",
                    "driver dispatch history exceeds bounded inventory",
                ));
            }
            let entry = entry.map_err(|error| Error::io("read driver record entry", error))?;
            let leaf = entry
                .file_name()
                .into_string()
                .map_err(|_| Error::new("record_corrupt", "invalid driver record name"))?;
            if !leaf.ends_with(".json") {
                return Err(Error::new(
                    "record_corrupt",
                    "unfinished driver record requires recovery",
                ));
            }
            let record: DispatchRecord = records.read_bounded(&leaf, 64 * 1024)?;
            if record.schema_version != 1
                || record.runtime != config.runtime.id
                || leaf != record_file(&record.action)?
                || record.operation.is_nil()
                || record.lease.runtime_id != record.runtime
            {
                return Err(Error::new(
                    "record_corrupt",
                    "driver record identity differs",
                ));
            }
            retained.insert(record.action.clone(), record);
        }
        let owner = runtime.clone();
        let publisher = Arc::new(AuthorizationPublisher::with_inputs(
            runtime.configuration.clone(),
            runtime.control.clone(),
            runtime.observations.clone(),
            runtime.services.secrets.clone(),
            runtime.services.key.clone(),
            runtime.services.inputs.clone(),
            Arc::new(move |lease| owner.verify_owner(lease)),
        ));
        let processes = services.processes;
        Ok(Arc::new(Self {
            runtime,
            publisher,
            delivery: services.delivery,
            corrections: services.corrections,
            processes,
            records,
            limits,
            state: Mutex::new(DriverState {
                scheduler: Scheduler::new(scheduler_limits(&config))?,
                tasks: BTreeMap::new(),
                records: retained,
                recovery: vec![],
                recovery_errors: BTreeMap::new(),
                recovery_cursor: None,
                last_refresh: None,
                last_error: None,
                dispatch_errors: BTreeMap::new(),
            }),
            cycle: tokio::sync::Mutex::new(()),
        }))
    }
    fn state(&self) -> Result<std::sync::MutexGuard<'_, DriverState>> {
        self.state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "driver state lock failed"))
    }
    pub fn diagnostics(&self) -> Result<Value> {
        let state = self.state()?;
        Ok(
            json!({"pending_launches":state.tasks.len(),"allocations":state.scheduler.running(),"dispatches":state.records.values().collect::<Vec<_>>(),"process_recovery":state.recovery,"receipt_recovery":state.recovery_errors,"dispatch_errors":state.dispatch_errors,"error":state.last_error,"completion":"dispatch_and_process_observations_are_not_delivery_evidence"}),
        )
    }
    fn fresh(&self, lease: &Lease) -> Result<()> {
        self.runtime.verify_owner(lease)?;
        let snapshot = self.runtime.control.snapshot()?;
        if snapshot.intent.desired != Intent::Running
            || !snapshot.admission_open
            || !snapshot.reconciled
            || snapshot.service_stopping
        {
            return Err(Error::new(
                "admission_closed",
                "driver cannot schedule outside current RUNNING admission",
            ));
        }
        Ok(())
    }
    fn recover_processes(&self) -> Result<()> {
        let receipts = self.runtime.control.operations()?;
        let by_operation: BTreeMap<_, _> = receipts
            .iter()
            .map(|receipt| (receipt.operation_id, receipt))
            .collect();
        let snapshot = self.runtime.control.snapshot()?;
        let mut pending = Vec::new();
        for operation in self.processes.operations()? {
            let observed = self.processes.observe(operation)?;
            if observed.is_empty() {
                continue;
            }
            let receipt = by_operation.get(&operation).ok_or_else(|| {
                Error::new(
                    "record_corrupt",
                    "native process record lacks its durable admission",
                )
            })?;
            for process in &observed {
                validate_process_admission(process, receipt)?;
            }
            if matches!(receipt.phase, ReceiptPhase::Prepared | ReceiptPhase::Sent) {
                if !snapshot.unresolved_operations.contains_key(&operation)
                    && !snapshot
                        .active_operations
                        .iter()
                        .any(|permit| permit.operation_id == operation)
                {
                    return Err(Error::new(
                        "record_corrupt",
                        "unfinished process admission lost its canonical hold",
                    ));
                }
                pending.extend(observed);
            } else if observed.iter().any(|process| {
                !matches!(
                    process.state,
                    crate::process_records::RecoveryState::GroupExitRecordedTreeUnverified
                )
            }) {
                return Err(Error::new(
                    "ownership_conflict",
                    "terminal admission conflicts with unconfirmed recorded process exit",
                ));
            }
        }
        self.state()?.recovery = pending;
        Ok(())
    }
    async fn recover_authorizations(&self) -> Result<()> {
        self.runtime.registration.verify_control_owner()?;
        let snapshot = self.runtime.control.snapshot()?;
        let mut pending: Vec<_> = self
            .runtime
            .control
            .operations()?
            .into_iter()
            .filter(|receipt| {
                matches!(receipt.phase, ReceiptPhase::Prepared | ReceiptPhase::Sent)
                    && receipt.expected.get("preparation").map(String::as_str)
                        == Some("parent_authorization")
                    && !snapshot.active_operations.iter().any(|permit| {
                        receipt.expected.get("admission_operation")
                            == Some(&permit.operation_id.to_string())
                    })
            })
            .map(|receipt| receipt.operation_id)
            .collect();
        pending.sort();
        let cursor = self.state()?.recovery_cursor;
        pending.sort_by_key(|operation| {
            (
                cursor.is_some_and(|cursor| *operation <= cursor),
                *operation,
            )
        });
        let active: std::collections::BTreeSet<_> = pending.iter().copied().collect();
        self.state()?
            .recovery_errors
            .retain(|operation, _| active.contains(operation));
        for operation in pending.into_iter().take(self.limits.authorization_tasks) {
            let registration = self.runtime.registration.clone();
            let result = self
                .publisher
                .recover_receipt(operation, move || registration.verify_control_owner())
                .await;
            let mut state = self.state()?;
            state.recovery_cursor = Some(operation);
            match result {
                Ok(_) => {
                    state.recovery_errors.remove(&operation);
                }
                Err(error) => {
                    state.recovery_errors.insert(operation, error);
                }
            }
            // A missing/ambiguous provider record retains its exact receipt hold.
            // It cannot prevent unrelated, independently authorized scheduling.
        }
        Ok(())
    }
    async fn recover_corrections(&self) -> Result<()> {
        let pending = {
            let state = self.state()?;
            let mut records = state
                .records
                .values()
                .filter(|record| {
                    record.correction.is_some()
                        && !matches!(record.state, DispatchState::Returned)
                        && !state.tasks.contains_key(&record.operation)
                })
                .cloned()
                .collect::<Vec<_>>();
            records.sort_by_key(|record| (record.observed_at, record.operation));
            records.truncate(self.limits.authorization_tasks);
            records
        };
        for mut record in pending {
            let reference = record
                .correction
                .as_ref()
                .expect("filtered correction reference");
            let observed = async {
                let outcome = self
                    .runtime
                    .coordination_outcome(reference.worker_operation, reference.proposal)?;
                if outcome.digest() != reference.digest {
                    return Err(Error::new(
                        "record_corrupt",
                        "correction recovery refers to another completed proposal",
                    ));
                }
                self.corrections.recover(&outcome).await
            }
            .await;
            record.observed_at = Utc::now();
            match observed {
                Ok(Some(_)) => {
                    record.state = DispatchState::Returned;
                    record.error = None;
                }
                Ok(None) => {
                    record.state = DispatchState::Blocked;
                    record.error = Some(Error::new(
                        "correction_pending",
                        "existing publication prefix is observed; final classification still requires current RUNNING authority",
                    ));
                }
                Err(error) => {
                    record.state = DispatchState::Blocked;
                    record.error = Some(error);
                }
            }
            self.record_observation(&record)?;
        }
        Ok(())
    }
    async fn refresh(&self, force: bool) -> Result<()> {
        let config = self.runtime.configuration.snapshot()?;
        let due = force
            || self
                .state()?
                .last_refresh
                .is_none_or(|last| last.elapsed() >= Duration::from_millis(config.runtime.poll_ms));
        if due {
            self.runtime.observations.refresh().await?;
            self.runtime.recover_provider_holds()?;
            self.recover_authorizations().await?;
            self.recover_corrections().await?;
            self.state()?.last_refresh = Some(tokio::time::Instant::now());
        }
        Ok(())
    }
    fn record(&self, record: &DispatchRecord) -> Result<()> {
        self.records.write(&record_file(&record.action)?, record)?;
        self.state()?
            .records
            .insert(record.action.clone(), record.clone());
        Ok(())
    }
    fn launch_worker(&self, lease: &Lease, action: WorkerAction) -> Result<Uuid> {
        self.fresh(lease)?;
        let key = worker_action_key(&action)?;
        if self.state()?.records.contains_key(&key) {
            return Err(Error::new(
                "dispatch_retained",
                "this exact action already has a retained dispatch; reconcile it or supply a new verified trigger",
            ));
        }
        let operation = Uuid::new_v4();
        let record = DispatchRecord {
            schema_version: 1,
            correction: None,
            metadata: None,
            runtime: lease.runtime_id,
            action: key,
            operation,
            work: action.ready.canonical_key.clone(),
            phase: action.phase.phase.clone(),
            lease: lease.clone(),
            state: DispatchState::Dispatched,
            observed_at: Utc::now(),
            error: None,
        };
        self.record(&record)?;
        let runtime = self.runtime.clone();
        let lease = lease.clone();
        let bound = self.limits.worker_seconds;
        let handle = tokio::runtime::Handle::current();
        // Synchronous owned Git metadata runs on a blocking worker, never the
        // control listener's executor thread. Every subprocess still gates itself.
        let join = tokio::task::spawn_blocking(move || {
            handle.block_on(async move {
                let result = async {
                    let source =
                        runtime_workers::resolve_source(&runtime, &action.authority).await?;
                    runtime_workers::launch(
                        runtime.clone(),
                        LaunchRequest {
                            lease,
                            operation_id: operation,
                            authority: action.authority,
                            phase: action.phase,
                            source,
                            delivery: None,
                            max_seconds: bound,
                            max_turns: action.schedule.max_turns,
                        },
                    )
                    .await?;
                    Ok(())
                }
                .await;
                runtime.services.wake.notify_one();
                result
            })
        });
        self.state()?.tasks.insert(
            operation,
            PendingTask {
                record,
                worker: true,
                join,
            },
        );
        Ok(operation)
    }
    fn launch_authorization(
        &self,
        lease: &Lease,
        gate: crate::reconcile::AuthorizationGate,
    ) -> Result<()> {
        self.fresh(lease)?;
        let work = gate.work_key()?;
        let config = self.runtime.configuration.snapshot()?;
        let planned = (|| {
            let publication = self.publisher.next_publication(&gate)?;
            let base = canonical_digest(
                1,
                &json!({"kind":"authorization","workspace":gate.workspace_id(),"snapshot":gate.snapshot(),"config":gate.config_revision(),"workflow":gate.workflow_digest(),"publication":publication}),
            )?;
            let snapshot = canonical_digest(1, gate.snapshot())?;
            let state = self.state()?;
            preadmission_plan(
                &state.records,
                &base,
                publication,
                &snapshot,
                &self.runtime.control.operations()?,
                &config.runtime,
                Utc::now(),
            )
        })();
        let metadata = match planned {
            Ok(value) => value,
            Err(error) => {
                self.state()?.dispatch_errors.insert(work, error);
                return Ok(());
            }
        };
        let action = canonical_digest(
            1,
            &json!({"metadata":metadata.base,"attempt":metadata.attempt}),
        )?;
        let state = self.state()?;
        if state.tasks.values().any(|task| task.record.work == work)
            || state.tasks.values().filter(|task| !task.worker).count()
                >= self.limits.authorization_tasks
        {
            return Ok(());
        }
        drop(state);
        self.state()?.dispatch_errors.remove(&work);
        let operation = Uuid::new_v4();
        let record = DispatchRecord {
            schema_version: 1,
            correction: None,
            metadata: Some(metadata),
            runtime: lease.runtime_id,
            action,
            operation,
            work,
            phase: "authorization".into(),
            lease: lease.clone(),
            state: DispatchState::Dispatched,
            observed_at: Utc::now(),
            error: None,
        };
        self.record(&record)?;
        let publisher = self.publisher.clone();
        let lease = lease.clone();
        let wake = self.runtime.services.wake.clone();
        let join = tokio::spawn(async move {
            let result = publisher.publish(&lease, gate).await.map(|_| ());
            wake.notify_one();
            result
        });
        self.state()?.tasks.insert(
            operation,
            PendingTask {
                record,
                worker: false,
                join,
            },
        );
        Ok(())
    }
    fn record_observation(&self, record: &DispatchRecord) -> Result<()> {
        let mut state = self.state()?;
        persist_observation(&self.records, &mut state.records, record)
    }
    async fn launch_correction(
        &self,
        lease: &Lease,
        completed: crate::runtime::VerifiedCoordinationOutcome,
    ) -> Result<()> {
        self.fresh(lease)?;
        let work = completed.original_authority().work_key()?;
        {
            let state = self.state()?;
            if state.tasks.values().any(|task| task.record.work == work)
                || state.tasks.values().filter(|task| !task.worker).count()
                    >= self.limits.authorization_tasks
            {
                return Ok(());
            }
        }
        // Only exact observed prefix progress can authorize another dispatch.
        // Missing or ambiguous existing stages remain held, including on restart.
        let progress = match self.corrections.recover_progress(&completed).await {
            Ok(progress) => progress,
            Err(error) => {
                self.state()?.dispatch_errors.insert(work, error);
                return Ok(());
            }
        };
        if progress.publication().is_some() {
            return Ok(());
        }
        let planned = {
            let state = self.state()?;
            correction_continuation(
                &state.records,
                completed.digest(),
                progress.stages(),
                progress.stage_limit(),
            )
        };
        let attempt = match planned {
            Ok(attempt) => attempt,
            Err(error) => {
                self.state()?.dispatch_errors.insert(work, error);
                return Ok(());
            }
        };
        self.fresh(lease)?;
        let action = canonical_digest(
            1,
            &json!({"kind":"correction_publication","work":work,"outcome":completed.digest(),"attempt":attempt,"prefix":progress.fingerprint()?}),
        )?;
        self.state()?.dispatch_errors.remove(&work);
        let operation = Uuid::new_v4();
        let record = DispatchRecord {
            schema_version: 1,
            runtime: lease.runtime_id,
            action,
            operation,
            work,
            phase: "correction_publication".into(),
            lease: lease.clone(),
            state: DispatchState::Dispatched,
            observed_at: Utc::now(),
            error: None,
            metadata: None,
            correction: Some(CorrectionReference {
                worker_operation: completed.outcome().operation_id,
                proposal: completed.outcome().proposal_id,
                digest: completed.digest().into(),
                attempt,
                stage_limit: progress.stage_limit(),
                stages: progress.stages().clone(),
            }),
        };
        self.record(&record)?;
        let publisher = self.corrections.clone();
        let lease = lease.clone();
        let wake = self.runtime.services.wake.clone();
        let join = tokio::spawn(async move {
            let result = publisher.publish(&lease, &completed).await.map(|_| ());
            wake.notify_one();
            result
        });
        self.state()?.tasks.insert(
            operation,
            PendingTask {
                record,
                worker: false,
                join,
            },
        );
        Ok(())
    }
    async fn drain_finished(&self) -> Result<()> {
        let finished = {
            let mut state = self.state()?;
            let ids: Vec<_> = state
                .tasks
                .iter()
                .filter(|(_, task)| task.join.is_finished())
                .map(|(id, _)| *id)
                .collect();
            ids.into_iter()
                .filter_map(|id| state.tasks.remove(&id))
                .collect::<Vec<_>>()
        };
        let mut first_error = None;
        for task in finished {
            let result = task
                .join
                .await
                .map_err(|_| {
                    Error::new(
                        "operation_uncertain",
                        "owned launch task ended without a complete observation",
                    )
                })
                .and_then(|value| value);
            let mut record = task.record;
            record.observed_at = Utc::now();
            record.state = match (&result, task.worker) {
                (Ok(_), true) => DispatchState::ModelOwned,
                (Ok(_), false) => DispatchState::Returned,
                (Err(_), _) => DispatchState::Blocked,
            };
            record.error = result.err();
            // Retain each joined result in memory before attempting persistence.
            // A failed disk write must not discard the remaining owned joins.
            retain_error(&mut first_error, self.record_observation(&record));
            if record.error.is_some() {
                match self.runtime.control.snapshot() {
                    Ok(snapshot) => {
                        for permit in snapshot
                            .active_operations
                            .iter()
                            .filter(|permit| permit.work_key == record.work)
                        {
                            // Cancellation is authorized only after durable fencing.
                            match self.runtime.control.fence(permit.operation_id) {
                                Ok(()) => retain_error(
                                    &mut first_error,
                                    self.runtime
                                        .executions
                                        .cancel_fenced(&self.runtime.control, permit.operation_id)
                                        .map(|_| ()),
                                ),
                                Err(error) => {
                                    first_error.get_or_insert(error);
                                }
                            }
                        }
                    }
                    Err(error) => {
                        first_error.get_or_insert(error);
                    }
                }
            }
            if task.worker && record.error.is_some() {
                let release = (|| {
                    let snapshot = self.runtime.control.snapshot()?;
                    if !snapshot
                        .active_operations
                        .iter()
                        .any(|permit| permit.work_key == record.work)
                        && !snapshot
                            .unresolved_operations
                            .values()
                            .any(|targets| targets.contains(&record.work))
                    {
                        self.state()?.scheduler.finish(record.operation)?;
                    }
                    Ok(())
                })();
                retain_error(&mut first_error, release);
            }
        }
        match self.runtime.collect_finished().await {
            Ok(completions) => {
                for completion in completions {
                    let observed = (|| {
                        let record = self
                            .state()?
                            .records
                            .values()
                            .find(|record| record.operation == completion.operation_id)
                            .cloned();
                        if let Some(mut record) = record {
                            record.state = DispatchState::WorkerJoined;
                            record.error = completion.error.clone();
                            record.observed_at = Utc::now();
                            self.record_observation(&record)?;
                        }
                        Ok(())
                    })();
                    retain_error(&mut first_error, observed);
                    let release =
                        (|| {
                            if completion.local_exit_confirmed
                                && self.state()?.scheduler.running().iter().any(|allocation| {
                                    allocation.operation == completion.operation_id
                                })
                            {
                                self.state()?.scheduler.finish(completion.operation_id)?;
                            }
                            Ok(())
                        })();
                    retain_error(&mut first_error, release);
                }
            }
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
        retain_error(&mut first_error, self.runtime.observe_local_exits());
        self.state()?.last_error = first_error.clone();
        first_error.map_or(Ok(()), Err)
    }
    fn drain_state(&self, snapshot: &ControlSnapshot) -> Result<DrainState> {
        let state = self.state()?;
        Ok(drain_observation(
            snapshot,
            state.tasks.len(),
            self.runtime.executions.active_operations()?.len(),
            state.recovery.len(),
        ))
    }
}
fn correction_continuation(
    records: &BTreeMap<String, DispatchRecord>,
    digest: &str,
    stages: &BTreeMap<Uuid, String>,
    stage_limit: usize,
) -> Result<usize> {
    if stage_limit == 0 || stage_limit > 16_385 || stages.len() > stage_limit {
        return Err(Error::new(
            "record_corrupt",
            "correction stage bound is invalid",
        ));
    }
    let mut history = records
        .values()
        .filter_map(|record| {
            record
                .correction
                .as_ref()
                .filter(|reference| reference.digest == digest)
                .map(|reference| (reference, record))
        })
        .collect::<Vec<_>>();
    history.sort_by_key(|(reference, _)| reference.attempt);
    if history
        .iter()
        .map(|(reference, _)| reference.attempt)
        .ne(1..=history.len())
    {
        return Err(Error::new(
            "record_corrupt",
            "correction dispatch attempts have a gap or duplicate",
        ));
    }
    if let Some((prior, record)) = history.last() {
        if !matches!(record.state, DispatchState::Blocked)
            || prior.stage_limit != stage_limit
            || history.len() >= stage_limit
        {
            return Err(Error::new(
                "dispatch_retained",
                "correction continuation is complete, in flight or exhausted",
            ));
        }
        if stages.len() <= prior.stages.len()
            || prior
                .stages
                .iter()
                .any(|(operation, digest)| stages.get(operation) != Some(digest))
        {
            return Err(Error::new(
                "correction_pending",
                "no newly verified publication prefix authorizes continuation",
            ));
        }
    }
    Ok(history.len() + 1)
}
fn preadmission_plan(
    records: &BTreeMap<String, DispatchRecord>,
    base: &str,
    publication: Uuid,
    snapshot: &str,
    receipts: &[crate::control::Receipt],
    config: &crate::config::RuntimeConfig,
    now: chrono::DateTime<Utc>,
) -> Result<MetadataDispatch> {
    let observed_admissions = receipts
        .iter()
        .filter_map(
            |receipt| match crate::control::MetadataReference::from_receipt(receipt) {
                Ok(Some(reference))
                    if reference.purpose == "parent_authorization"
                        && reference.generation == snapshot =>
                {
                    Some(Ok(receipt.operation_id))
                }
                Ok(_) => None,
                Err(error) => Some(Err(error)),
            },
        )
        .collect::<Result<std::collections::BTreeSet<_>>>()?;
    let mut history = records
        .values()
        .filter_map(|record| {
            record
                .metadata
                .as_ref()
                .filter(|metadata| metadata.base == base)
                .map(|metadata| (metadata, record))
        })
        .collect::<Vec<_>>();
    history.sort_by_key(|(metadata, _)| metadata.attempt);
    if history
        .iter()
        .map(|(metadata, _)| metadata.attempt)
        .ne(1..=history.len() as u32)
    {
        return Err(Error::new(
            "record_corrupt",
            "pre-admission metadata attempts have a gap or duplicate",
        ));
    }
    let cap = history
        .iter()
        .map(|(metadata, _)| metadata.max_attempts)
        .fold(config.max_attempts_per_phase, u32::min);
    let attempt = history.len() as u32 + 1;
    if let Some((prior, record)) = history.last() {
        if attempt > cap {
            return Err(Error::new(
                "attempts_exhausted",
                "finite metadata preparation attempts exhausted",
            ));
        }
        if prior.publication != publication
            || prior.observed_admissions != observed_admissions
            || receipts
                .iter()
                .any(|receipt| receipt.operation_id == publication)
        {
            return Err(Error::new(
                "reconciliation_required",
                "metadata admission or provider effect exists; its durable planner owns retries",
            ));
        }
        if !matches!(record.state, DispatchState::Blocked) {
            return Err(Error::new(
                "dispatch_retained",
                "prior metadata dispatch has no terminal failed preparation observation",
            ));
        }
        let error = record
            .error
            .as_ref()
            .filter(|error| error.retryable)
            .ok_or_else(|| {
                Error::new(
                    "dispatch_retained",
                    "metadata preparation requires a changed prerequisite or verified trigger",
                )
            })?;
        let delay = error
            .details
            .get("retry_after_ms")
            .and_then(Value::as_u64)
            .into_iter()
            .chain(
                error
                    .details
                    .pointer("/rate_limit/retry_after_ms")
                    .and_then(Value::as_u64),
            )
            .max();
        let mut retry_at = crate::scheduler::retry_at(
            record.observed_at,
            prior.attempt,
            config.retry_cap_ms,
            delay,
        )?;
        if let Some(reset) = error
            .details
            .pointer("/rate_limit/reset_at")
            .filter(|value| !value.is_null())
        {
            let reset: chrono::DateTime<Utc> = serde_json::from_value(reset.clone())
                .map_err(|_| Error::new("record_corrupt", "provider retry reset is malformed"))?;
            retry_at = retry_at.max(reset);
        }
        if now < retry_at {
            return Err(Error::new(
                "retry_wait",
                "metadata preparation is waiting for bounded backoff",
            )
            .detail(json!({"retry_at":retry_at})));
        }
    }
    Ok(MetadataDispatch {
        base: base.into(),
        publication,
        attempt,
        max_attempts: cap,
        observed_admissions,
    })
}
fn retain_error(first: &mut Option<Error>, result: Result<()>) {
    if let Err(error) = result {
        first.get_or_insert(error);
    }
}
fn persist_observation(
    directory: &PrivateDir,
    retained: &mut BTreeMap<String, DispatchRecord>,
    record: &DispatchRecord,
) -> Result<()> {
    retained.insert(record.action.clone(), record.clone());
    directory.write(&record_file(&record.action)?, record)
}
fn drain_observation(
    snapshot: &ControlSnapshot,
    owned_tasks: usize,
    owned_process_operations: usize,
    unresolved_process_records: usize,
) -> DrainState {
    DrainState {
        local_clear: owned_tasks == 0
            && owned_process_operations == 0
            && unresolved_process_records == 0
            && snapshot.active_operations.is_empty()
            && snapshot.pending_local_processes.is_empty(),
        remote_reconciled: snapshot.unresolved_operations.is_empty()
            && snapshot.pending_external_effects.is_empty(),
    }
}
impl SupervisorDriver for RuntimeDriver {
    fn reconcile_before_start(&self) -> SupervisorFuture<'_, ()> {
        Box::pin(async move {
            let _cycle = self.cycle.lock().await;
            self.runtime.registration.verify_control_owner()?;
            self.runtime.recover_provider_holds()?;
            self.recover_processes()?;
            self.refresh(true).await?;
            self.delivery.reconcile().await
        })
    }
    fn recover<'a>(&'a self, lease: &'a Lease) -> SupervisorFuture<'a, ()> {
        Box::pin(async move {
            let _cycle = self.cycle.lock().await;
            self.runtime.verify_owner(lease)?;
            self.runtime.recover_provider_holds()?;
            self.recover_processes()?;
            self.refresh(true).await?;
            self.delivery.reconcile().await
        })
    }
    fn poll<'a>(&'a self, lease: &'a Lease, _sequence: u64) -> SupervisorFuture<'a, ()> {
        Box::pin(async move {
            let _cycle = self.cycle.lock().await;
            self.drain_finished().await?;
            self.fresh(lease)?;
            self.refresh(false).await?;
            self.runtime
                .refresh_active_corrections(&self.corrections)
                .await?;
            self.delivery.advance(lease).await?;
            for completed in self.runtime.pending_coordination_outcomes()? {
                self.launch_correction(lease, completed).await?;
            }
            let config = self.runtime.configuration.snapshot()?;
            let actions = self.runtime.ready_actions()?;
            let mut workers = BTreeMap::new();
            for action in actions {
                match action {
                    RuntimeAction::Authorize(gate) => self.launch_authorization(lease, *gate)?,
                    RuntimeAction::Worker(action) => {
                        workers.insert(action.ready.canonical_key.clone(), *action);
                    }
                }
            }
            let selected = workers.keys().cloned().collect();
            let mut scheduler = {
                let mut state = self.state()?;
                state.scheduler.replace_limits(scheduler_limits(&config))?;
                state.scheduler.retain_pending(&selected);
                for action in workers.values() {
                    if !state.scheduler.running().iter().any(|allocation| {
                        allocation.work.canonical_key == action.ready.canonical_key
                    }) && !state.records.contains_key(&worker_action_key(action)?)
                    {
                        state.scheduler.enqueue(action.ready.clone())?;
                    }
                }
                std::mem::replace(
                    &mut state.scheduler,
                    Scheduler::new(scheduler_limits(&config))?,
                )
            };
            let scheduled = (|| {
                while scheduler
                    .admit_next(Utc::now(), |ready| {
                        self.launch_worker(
                            lease,
                            workers
                                .get(&ready.canonical_key)
                                .ok_or_else(|| {
                                    Error::new("stale_result", "selected work disappeared")
                                })?
                                .clone(),
                        )
                    })?
                    .is_some()
                {}
                Ok(())
            })();
            self.state()?.scheduler = scheduler;
            scheduled
        })
    }
    fn maintain<'a>(
        &'a self,
        _lease: &'a Lease,
        _snapshot: &'a ControlSnapshot,
    ) -> SupervisorFuture<'a, DrainState> {
        Box::pin(async move {
            let _cycle = self.cycle.lock().await;
            self.drain_finished().await?;
            self.delivery.maintain().await?;
            self.recover_processes()?;
            self.refresh(false).await?;
            self.runtime
                .refresh_active_corrections(&self.corrections)
                .await?;
            self.drain_state(&self.runtime.control.snapshot()?)
        })
    }
    fn cancel_fenced<'a>(&'a self, operations: &'a [Uuid]) -> SupervisorFuture<'a, ()> {
        Box::pin(async move {
            self.runtime.registration.verify_control_owner()?;
            let mut error = None;
            for operation in operations {
                if let Err(current) = self
                    .runtime
                    .executions
                    .cancel_fenced(&self.runtime.control, *operation)
                {
                    error.get_or_insert(current);
                }
            }
            retain_error(&mut error, self.delivery.cancel_fenced(operations).await);
            error.map_or(Ok(()), Err)
        })
    }
    fn exited<'a>(&'a self, _lease: &'a Lease) -> SupervisorFuture<'a, DrainState> {
        Box::pin(async move {
            let _cycle = self.cycle.lock().await;
            self.drain_finished().await?;
            self.delivery.maintain().await?;
            self.recover_processes()?;
            self.refresh(false).await?;
            self.drain_state(&self.runtime.control.snapshot()?)
        })
    }
}
fn record_file(action: &str) -> Result<String> {
    crate::evidence::validate_digest(action)?;
    Ok(format!(
        "{}.json",
        action
            .rsplit(':')
            .next()
            .ok_or_else(|| Error::invalid("action digest missing"))?
    ))
}

fn validate_process_admission(
    process: &ProcessRecovery,
    receipt: &crate::control::Receipt,
) -> Result<()> {
    let binding = &process.binding;
    let cleanup = binding
        .control_owner
        .as_ref()
        .is_some_and(|owner| owner.cleanup_receipt.is_some());
    let authority_matches = match (&binding.lease, &binding.control_owner) {
        (Some(lease), None) => {
            receipt.kind == "admission"
                && receipt.runtime_id == lease.runtime_id
                && receipt.owner_generation == lease.owner_generation
                && receipt.actor == format!("owner:{}", lease.owner_id)
        }
        (None, Some(owner)) if cleanup => {
            receipt.kind == "cleanup"
                && receipt.runtime_id == owner.runtime_id
                && owner.cleanup_receipt.as_ref() == Some(&receipt.payload_digest)
                && receipt.expected.get("source_operation").is_some_and(|id| {
                    owner.work_key == format!("cleanup:{id}")
                        && receipt.canonical_targets.contains(id)
                })
                && receipt
                    .canonical_targets
                    .contains(&owner.runtime_id.to_string())
                && receipt
                    .expected
                    .get("cleanup_plan")
                    .is_some_and(|digest| crate::evidence::validate_digest(digest).is_ok())
        }
        (None, Some(owner)) => {
            receipt.kind == "host_metadata"
                && receipt.runtime_id == owner.runtime_id
                && receipt.owner_generation == owner.owner_generation
                && receipt.actor == crate::host_process::host_actor(owner)?
                && receipt.expected.get("host_identity") == Some(&canonical_digest(1, owner)?)
        }
        _ => false,
    };
    if !authority_matches
        || receipt.operation_id != binding.operation_id
        || (!cleanup
            && (receipt.canonical_targets != [binding.work_key.clone()]
                || receipt.expected.get("scope") != Some(&binding.scope_digest)))
        || process.effect_id != binding.effect_id
        || process.operation_id != binding.operation_id
    {
        return Err(Error::new(
            "record_corrupt",
            "retained process does not match its exact immutable admission",
        ));
    }
    Ok(())
}
fn worker_action_key(action: &WorkerAction) -> Result<String> {
    preparation_identity(action)
}
/// Stable dispatch identity. Refresh timestamps, source observations and the
/// runner's own comments do not authorize another preparation at the same phase.
pub fn preparation_identity(action: &WorkerAction) -> Result<String> {
    let authority = match &action.authority {
        crate::worker::WorkerAuthority::Authorized(gate) => {
            json!({"workspace":gate.workspace_id(),"work":gate.work(),"config":gate.config_revision(),"workflow":gate.workflow_digest(),"profile":gate.profile(),"scope":gate.child_scope_digest(),"children":gate.required_children(),"dependencies":gate.dependency_digest(),"purpose":gate.purpose()})
        }
        crate::worker::WorkerAuthority::Independent(gate) => {
            json!({"workspace":gate.workspace_id(),"repository":gate.repository_id(),"review":gate.review_identity(),"config":gate.config_revision(),"workflow":gate.workflow_digest(),"profile":gate.profile(),"head_ref":gate.pull_request().head_ref,"base_ref":gate.pull_request().base_ref,"author":gate.pull_request().author_id})
        }
    };
    canonical_digest(
        1,
        &json!({"kind":"worker","authority":authority,"phase":action.phase,"attempt":action.schedule.attempt}),
    )
}
fn scheduler_limits(config: &Config) -> Limits {
    Limits {
        global: config.runtime.max_agents as usize,
        accounts: config
            .account_limits()
            .into_iter()
            .map(|(account, limit)| (account, limit as usize))
            .collect(),
        workspaces: config
            .workspaces
            .values()
            .map(|workspace| (workspace.id, workspace.max_agents as usize))
            .collect(),
        states: config
            .workspaces
            .values()
            .flat_map(|workspace| {
                workspace
                    .max_agents_by_state
                    .iter()
                    .map(move |(state, limit)| {
                        ((workspace.id, state.trim().to_lowercase()), *limit as usize)
                    })
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        control::ControlHost,
        digest::bytes_digest,
        platform::BinaryIdentity,
        process_records::{RecoveryState, SpawnBinding},
    };
    fn id(value: u128) -> Uuid {
        Uuid::from_u128(value)
    }
    #[test]
    fn recovered_process_requires_exact_admission_and_never_uses_empty_registry_as_exit() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let host = ControlHost::setup(
            PrivateDir::create(&root.join("control")).unwrap(),
            id(1),
            id(99),
        )
        .unwrap();
        let snapshot = host.snapshot().unwrap();
        assert!(drain_observation(&snapshot, 0, 0, 0).local_clear);
        assert!(
            !drain_observation(&snapshot, 0, 0, 1).local_clear,
            "retained unknown process must block drain even with no live in-memory handle"
        );
        assert!(
            !drain_observation(&snapshot, 1, 0, 0).local_clear,
            "source/preparation future remains owned until joined"
        );
        let binding = SpawnBinding {
            operation_id: id(2),
            effect_id: id(3),
            lease: Some(Lease {
                runtime_id: id(1),
                owner_generation: 1,
                owner_id: id(4),
            }),
            control_owner: None,
            work_key: "canonical-work".into(),
            scope_digest: bytes_digest(b"scope"),
            cwd: root,
            executable: BinaryIdentity::inspect(std::path::Path::new("/bin/sleep"), false).unwrap(),
            observed_executables: vec![],
            argv_digest: bytes_digest(b"argv"),
            kind: "fixture".into(),
        };
        let process = ProcessRecovery {
            operation_id: id(2),
            effect_id: id(3),
            binding: binding.clone(),
            state: RecoveryState::GroupExitRecordedTreeUnverified,
            process: None,
            all_descendants_exited: false,
        };
        let mut receipt = crate::control::Receipt {
            operation_id: id(2),
            request_id: id(2),
            runtime_id: id(1),
            owner_generation: 1,
            actor: "owner:00000000-0000-0000-0000-000000000004".into(),
            kind: "admission".into(),
            canonical_targets: vec![binding.work_key.clone()],
            payload_digest: bytes_digest(b"fixture"),
            expected: BTreeMap::from([("scope".into(), binding.scope_digest.clone())]),
            phase: ReceiptPhase::Sent,
            remote_ids: BTreeMap::new(),
            observed_result: None,
            cancellation: "none".into(),
            updated_at: Utc::now(),
        };
        validate_process_admission(&process, &receipt).unwrap();
        receipt.canonical_targets = vec!["other-work".into()];
        assert_eq!(
            validate_process_admission(&process, &receipt)
                .unwrap_err()
                .code,
            "record_corrupt"
        );
        receipt.canonical_targets = vec![binding.work_key];
        receipt.owner_generation = 2;
        assert_eq!(
            validate_process_admission(&process, &receipt)
                .unwrap_err()
                .code,
            "record_corrupt"
        );
    }
    #[test]
    fn all_join_observations_survive_a_failed_persistence_attempt() {
        let temp = tempfile::tempdir().unwrap();
        let directory =
            PrivateDir::create(&temp.path().canonicalize().unwrap().join("results")).unwrap();
        std::fs::remove_dir(directory.path()).unwrap();
        let mut retained = BTreeMap::new();
        let mut first_error = None;
        for operation in [id(8), id(9)] {
            let record = DispatchRecord {
                schema_version: 1,
                correction: None,
                metadata: None,
                runtime: id(1),
                action: bytes_digest(operation.as_bytes()),
                operation,
                work: operation.to_string(),
                phase: "implementation".into(),
                lease: Lease {
                    runtime_id: id(1),
                    owner_generation: 1,
                    owner_id: id(2),
                },
                state: DispatchState::Blocked,
                observed_at: Utc::now(),
                error: Some(Error::new(
                    "operation_uncertain",
                    "actual joined task failed",
                )),
            };
            retain_error(
                &mut first_error,
                persist_observation(&directory, &mut retained, &record),
            );
        }
        assert!(first_error.is_some());
        assert_eq!(retained.len(), 2);
        assert!(retained.values().all(|record| {
            record
                .error
                .as_ref()
                .is_some_and(|error| error.code == "operation_uncertain")
        }));
        assert!(
            !directory.path().exists(),
            "failed storage is not silently recreated"
        );
    }
    fn failed_dispatch(metadata: Option<MetadataDispatch>) -> DispatchRecord {
        let mut error = Error::new("provider_transport", "fixture failed before admission");
        error.retryable = true;
        DispatchRecord {
            schema_version: 1,
            correction: None,
            metadata,
            runtime: id(1),
            action: bytes_digest(b"attempt1"),
            operation: id(9),
            work: "work".into(),
            phase: "authorization".into(),
            lease: Lease {
                runtime_id: id(1),
                owner_generation: 1,
                owner_id: id(2),
            },
            state: DispatchState::Blocked,
            observed_at: Utc::now(),
            error: Some(error),
        }
    }
    #[test]
    fn metadata_preparation_retry_is_finite_backed_off_and_never_replays_dispatch() {
        let mut config: crate::config::RuntimeConfig = serde_json::from_value(
            json!({"id":id(1),"host_id":"fixture","max_agents":1,"max_attempts_per_phase":2}),
        )
        .unwrap();
        let now = Utc::now();
        let base = bytes_digest(b"base");
        let mut records = BTreeMap::new();
        let first =
            preadmission_plan(&records, &base, id(3), "snapshot", &[], &config, now).unwrap();
        assert_eq!(first.attempt, 1);
        let mut record = failed_dispatch(Some(first));
        record.observed_at = now;
        records.insert(record.action.clone(), record.clone());
        assert_eq!(
            preadmission_plan(&records, &base, id(3), "snapshot", &[], &config, now)
                .unwrap_err()
                .code,
            "retry_wait"
        );
        record.error.as_mut().unwrap().details = json!({"rate_limit":{"retry_after_ms":120_000,"reset_at":now+chrono::Duration::minutes(3)}});
        records.insert(record.action.clone(), record.clone());
        assert_eq!(
            preadmission_plan(
                &records,
                &base,
                id(3),
                "snapshot",
                &[],
                &config,
                now + chrono::Duration::minutes(2)
            )
            .unwrap_err()
            .code,
            "retry_wait"
        );
        let later = now + chrono::Duration::minutes(4);
        let next =
            preadmission_plan(&records, &base, id(3), "snapshot", &[], &config, later).unwrap();
        assert_eq!(next.attempt, 2);
        record.state = DispatchState::Dispatched;
        records.insert(record.action.clone(), record.clone());
        assert_eq!(
            preadmission_plan(&records, &base, id(3), "snapshot", &[], &config, later)
                .unwrap_err()
                .code,
            "dispatch_retained"
        );
        record.state = DispatchState::Blocked;
        record.error.as_mut().unwrap().retryable = false;
        records.insert(record.action.clone(), record.clone());
        assert_eq!(
            preadmission_plan(&records, &base, id(3), "snapshot", &[], &config, later)
                .unwrap_err()
                .code,
            "dispatch_retained"
        );
        record.error.as_mut().unwrap().retryable = true;
        records.insert(record.action.clone(), record.clone());
        let receipt = crate::control::Receipt {
            operation_id: id(3),
            request_id: id(3),
            runtime_id: id(1),
            owner_generation: 1,
            actor: "fixture".into(),
            kind: "provider_write".into(),
            canonical_targets: vec!["work".into()],
            payload_digest: bytes_digest(b"payload"),
            expected: BTreeMap::new(),
            phase: ReceiptPhase::Sent,
            remote_ids: BTreeMap::new(),
            observed_result: None,
            cancellation: "none".into(),
            updated_at: now,
        };
        assert_eq!(
            preadmission_plan(
                &records,
                &base,
                id(3),
                "snapshot",
                &[receipt],
                &config,
                later
            )
            .unwrap_err()
            .code,
            "reconciliation_required"
        );
        let mut second = failed_dispatch(Some(next));
        second.action = bytes_digest(b"attempt2");
        records.insert(second.action.clone(), second);
        config.max_attempts_per_phase = 8;
        assert_eq!(
            preadmission_plan(&records, &base, id(3), "snapshot", &[], &config, later)
                .unwrap_err()
                .code,
            "attempts_exhausted"
        );
    }
    #[test]
    fn correction_continuation_requires_new_exact_prefix_and_retains_original_finite_bound() {
        let digest = bytes_digest(b"outcome");
        let mut records = BTreeMap::new();
        assert_eq!(
            correction_continuation(&records, &digest, &BTreeMap::new(), 3).unwrap(),
            1
        );
        let mut record = failed_dispatch(None);
        record.correction = Some(CorrectionReference {
            worker_operation: id(3),
            proposal: id(4),
            digest: digest.clone(),
            attempt: 1,
            stage_limit: 3,
            stages: BTreeMap::new(),
        });
        records.insert(record.action.clone(), record.clone());
        assert_eq!(
            correction_continuation(&records, &digest, &BTreeMap::new(), 3)
                .unwrap_err()
                .code,
            "correction_pending"
        );
        let mut prefix = BTreeMap::from([(id(20), bytes_digest(b"native1"))]);
        assert_eq!(
            correction_continuation(&records, &digest, &prefix, 3).unwrap(),
            2
        );
        let mut second = record.clone();
        second.action = bytes_digest(b"attempt2");
        second.correction.as_mut().unwrap().attempt = 2;
        second.correction.as_mut().unwrap().stages = prefix.clone();
        records.insert(second.action.clone(), second);
        assert_eq!(
            correction_continuation(&records, &digest, &prefix, 3)
                .unwrap_err()
                .code,
            "correction_pending"
        );
        prefix.insert(id(21), bytes_digest(b"native2"));
        assert_eq!(
            correction_continuation(&records, &digest, &prefix, 3).unwrap(),
            3
        );
        assert_eq!(
            correction_continuation(&records, &digest, &prefix, 4)
                .unwrap_err()
                .code,
            "dispatch_retained"
        );
        prefix.insert(id(20), bytes_digest(b"edited"));
        assert_eq!(
            correction_continuation(&records, &digest, &prefix, 3)
                .unwrap_err()
                .code,
            "correction_pending"
        );
    }
    #[test]
    fn scheduler_reload_keeps_running_reservation_and_fair_workspace_rotation() {
        use crate::scheduler::{ReadyWork, Trigger, WorkKind};
        let limits = Limits {
            global: 2,
            accounts: BTreeMap::from([("account".into(), 2)]),
            workspaces: BTreeMap::from([(id(1), 2), (id(2), 2)]),
            states: BTreeMap::new(),
        };
        let mut scheduler = Scheduler::new(limits.clone()).unwrap();
        for (work, workspace) in [("a", id(1)), ("b", id(1)), ("c", id(2))] {
            scheduler
                .enqueue(ReadyWork {
                    canonical_key: work.into(),
                    workspace,
                    account: "account".into(),
                    generation: bytes_digest(work.as_bytes()),
                    state: "ready".into(),
                    priority: None,
                    created_at: None,
                    kind: WorkKind::Child,
                    trigger: Some(Trigger::InitialAuthorization),
                    not_before: Utc::now(),
                    gate_blockers: vec![],
                })
                .unwrap();
        }
        let first = scheduler
            .admit_next(Utc::now(), |_| Ok(id(10)))
            .unwrap()
            .unwrap();
        assert_eq!(first.work.workspace, id(1));
        let mut reduced = limits.clone();
        reduced.global = 1;
        scheduler.replace_limits(reduced).unwrap();
        assert_eq!(scheduler.running().len(), 1);
        assert!(
            scheduler
                .admit_next(Utc::now(), |_| panic!(
                    "reduced cap must preserve active reservation"
                ))
                .unwrap()
                .is_none()
        );
        scheduler.finish(id(10)).unwrap();
        scheduler.replace_limits(limits).unwrap();
        assert_eq!(
            scheduler
                .admit_next(Utc::now(), |_| Ok(id(11)))
                .unwrap()
                .unwrap()
                .work
                .workspace,
            id(2)
        );
    }
}
