//! Owned driver tasks outlive individual supervisor polling futures. Handles
//! stay in their slots until awaited; an interrupted waiter cannot abort a phase
//! or claim that its process/effects have exited. Host destruction still leaves
//! durable process/effect journals unresolved; it is never a successful join.
use super::*;
use crate::{observations::ObservationCache, reconcile::RuntimeProjection};
use std::future::Future;
use tokio::{sync::Notify, time::Instant};

const OBSERVATION_BOUND: Duration = Duration::from_secs(30);

pub(crate) fn observation_cadence(poll_ms: u64) -> Duration {
    Duration::from_millis(poll_ms.clamp(1, 10_000))
}

#[derive(Default)]
struct ObservationSlot {
    task: Option<JoinHandle<Result<RuntimeProjection>>>,
    started: Option<Instant>,
}

#[derive(Default, Clone, Serialize)]
pub(crate) struct ObservationTaskStatus {
    pub pending: bool,
    pub joined: u64,
    pub error: Option<Error>,
}

pub(crate) enum ObservationTick {
    Idle,
    Started,
    Pending,
    Joined(Result<RuntimeProjection>),
}

/// The production read-only observation task owner, also usable by component
/// fixtures. Neither a tick nor an observation can admit work or run recovery
/// mutations. The cache itself marks bounded read failures unavailable.
pub(crate) struct ObservationMaintenance {
    cache: Arc<ObservationCache>,
    wake: Arc<Notify>,
    completed: Arc<Notify>,
    slot: tokio::sync::Mutex<ObservationSlot>,
    status: Mutex<ObservationTaskStatus>,
}
impl ObservationMaintenance {
    pub(crate) fn new(cache: Arc<ObservationCache>, wake: Arc<Notify>) -> Self {
        Self {
            cache,
            wake,
            completed: Arc::new(Notify::new()),
            slot: Default::default(),
            status: Default::default(),
        }
    }

    pub(crate) fn status(&self) -> Result<ObservationTaskStatus> {
        self.status
            .lock()
            .map(|status| status.clone())
            .map_err(|_| Error::new("runtime_unavailable", "observation task status lock failed"))
    }

    pub(crate) fn pending(&self) -> Result<bool> {
        Ok(self.status()?.pending)
    }

    pub(crate) async fn tick(&self, poll_ms: u64, allow_start: bool) -> Result<ObservationTick> {
        let Ok(mut slot) = self.slot.try_lock() else {
            return Ok(ObservationTick::Pending);
        };
        if let Some(task) = &mut slot.task {
            if !task.is_finished() {
                return Ok(ObservationTick::Pending);
            }
            let result = self.observe_join(task).await;
            slot.task = None;
            self.record_join(&result)?;
            return Ok(ObservationTick::Joined(result));
        }
        if !allow_start
            || slot
                .started
                .is_some_and(|started| started.elapsed() < observation_cadence(poll_ms))
        {
            return Ok(ObservationTick::Idle);
        }
        let cache = self.cache.clone();
        let wake = self.wake.clone();
        let completed = self.completed.clone();
        slot.started = Some(Instant::now());
        self.status
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "observation task status lock failed"))?
            .pending = true;
        slot.task = Some(tokio::spawn(async move {
            let result = cache.refresh_bounded(OBSERVATION_BOUND).await;
            completed.notify_one();
            wake.notify_one();
            result
        }));
        Ok(ObservationTick::Started)
    }

    /// Borrow the stored handle throughout the await. Dropping this join future
    /// leaves the actual task and its handle owned by this same slot.
    pub(crate) async fn join(&self) -> Result<Option<RuntimeProjection>> {
        let mut slot = self.slot.lock().await;
        let Some(task) = &mut slot.task else {
            return Ok(None);
        };
        let result = self.observe_join(task).await;
        slot.task = None;
        self.record_join(&result)?;
        result.map(Some)
    }

    async fn observe_join(
        &self,
        task: &mut JoinHandle<Result<RuntimeProjection>>,
    ) -> Result<RuntimeProjection> {
        match task.await {
            Ok(result) => result,
            Err(_) => {
                let error = Error::new(
                    "observation_unavailable",
                    "owned provider observation task did not complete",
                );
                self.cache.record_refresh_failure(&error)?;
                Err(error)
            }
        }
    }

    fn record_join(&self, result: &Result<RuntimeProjection>) -> Result<()> {
        let mut status = self.status.lock().map_err(|_| {
            Error::new("runtime_unavailable", "observation task status lock failed")
        })?;
        status.pending = false;
        status.joined = status.joined.saturating_add(1);
        status.error = result.as_ref().err().cloned();
        Ok(())
    }

    /// Keep the same observation owner active while a finite admitted phase is
    /// pending. Every observation is joined, including after the phase fails.
    /// This wrapper is itself owned by the driver's retained progression task.
    pub(crate) async fn maintain_while<F: Future>(
        &self,
        poll_ms: u64,
        phase: F,
    ) -> (F::Output, Result<()>) {
        tokio::pin!(phase);
        let mut first_error = None;
        loop {
            match self.tick(poll_ms, true).await {
                Ok(ObservationTick::Joined(result)) => {
                    if let Err(error) = result {
                        first_error.get_or_insert(error);
                    }
                    continue;
                }
                Err(error) => {
                    first_error.get_or_insert(error);
                }
                _ => {}
            }
            let wait = self
                .slot
                .try_lock()
                .ok()
                .map(|slot| {
                    if slot.task.is_some() {
                        observation_cadence(poll_ms)
                    } else {
                        slot.started
                            .map(|at| observation_cadence(poll_ms).saturating_sub(at.elapsed()))
                            .unwrap_or_default()
                    }
                })
                .unwrap_or_else(|| observation_cadence(poll_ms))
                .max(Duration::from_millis(1));
            tokio::select! {
                result = &mut phase => {
                    if let Err(error) = self.join().await {
                        first_error.get_or_insert(error);
                    }
                    return (result, first_error.map_or(Ok(()), Err));
                }
                _ = self.completed.notified() => {}
                _ = tokio::time::sleep(wait) => {}
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ProgressionMode {
    Advance,
    Maintain,
}

struct ProgressionTask {
    mode: ProgressionMode,
    lease: Lease,
    join: JoinHandle<Result<()>>,
}

#[derive(Default)]
pub(super) struct ProgressionSlot {
    task: Option<ProgressionTask>,
    requested_advance: Option<Lease>,
    last_maintenance: Option<(Lease, u64, Instant)>,
    error: Option<Error>,
    ready_advance: Option<Lease>,
}

#[derive(Default, Clone, Serialize)]
pub(super) struct ProgressionTaskStatus {
    pub(super) pending: bool,
    mode: Option<ProgressionMode>,
    lease: Option<Lease>,
    joined: u64,
    error: Option<Error>,
}

impl ProgressionSlot {
    async fn join_finished(&mut self) -> Option<(ProgressionMode, Lease, Result<()>)> {
        let task = self.task.as_mut()?;
        if !task.join.is_finished() {
            return None;
        }
        let result = (&mut task.join)
            .await
            .map_err(|_| {
                Error::new(
                    "operation_uncertain",
                    "owned delivery progression task did not complete; preserve effects",
                )
            })
            .and_then(|value| value);
        let mode = task.mode;
        let owner = task.lease.clone();
        self.task = None;
        self.error = result.as_ref().err().cloned();
        if result.is_err() {
            self.requested_advance = None;
            self.ready_advance = None;
        } else if mode == ProgressionMode::Advance {
            self.ready_advance = Some(owner.clone());
        }
        Some((mode, owner, result))
    }

    fn select(
        &mut self,
        lease: &Lease,
        snapshot: &ControlSnapshot,
        maintenance: bool,
        has_work: bool,
        cadence: Duration,
    ) -> Option<ProgressionMode> {
        if self.task.is_some() {
            return None;
        }
        let running = snapshot.intent.desired == Intent::Running
            && snapshot.admission_open
            && !snapshot.service_stopping;
        if self
            .requested_advance
            .as_ref()
            .is_some_and(|requested| requested != lease)
            || !running
        {
            self.requested_advance = None;
        }
        if self
            .ready_advance
            .as_ref()
            .is_some_and(|ready| ready != lease)
            || !running
        {
            self.ready_advance = None;
        }
        if self.ready_advance.is_some() {
            return None;
        }
        if self.requested_advance.take().is_some() {
            return Some(ProgressionMode::Advance);
        }
        if !maintenance {
            return None;
        }
        let due = self
            .last_maintenance
            .as_ref()
            .is_none_or(|(owner, generation, at)| {
                owner != lease
                    || *generation != snapshot.intent.generation
                    || ((running || has_work) && at.elapsed() >= cadence)
            });
        if !due {
            return None;
        }
        self.last_maintenance = Some((lease.clone(), snapshot.intent.generation, Instant::now()));
        Some(ProgressionMode::Maintain)
    }
}

/// Recovery writes share the progression lane; observation tasks never call these.
pub(super) struct RecoveryMaintenance {
    runtime: Arc<RuntimeCoordinator>,
    publisher: Arc<AuthorizationPublisher>,
    corrections: Arc<crate::corrections::CorrectionPublisher>,
    records: Arc<PrivateDir>,
    limits: DriverLimits,
    state: Arc<Mutex<DriverState>>,
}
impl RecoveryMaintenance {
    fn state(&self) -> Result<std::sync::MutexGuard<'_, DriverState>> {
        self.state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "driver state lock failed"))
    }
    fn record_observation(&self, record: &DispatchRecord) -> Result<()> {
        let mut state = self.state()?;
        persist_observation(&self.records, &mut state.records, record)?;
        drop(state);
        emit_dispatch(&self.runtime, record);
        Ok(())
    }
    pub(super) async fn recover_authorizations(&self) -> Result<()> {
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
    pub(super) async fn recover_corrections(&self) -> Result<()> {
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
}

impl RuntimeDriver {
    pub(super) fn recovery_maintenance(&self) -> RecoveryMaintenance {
        RecoveryMaintenance {
            runtime: self.runtime.clone(),
            publisher: self.publisher.clone(),
            corrections: self.corrections.clone(),
            records: self.records.clone(),
            limits: self.limits,
            state: self.state.clone(),
        }
    }

    pub(super) fn progression_status(&self) -> Result<ProgressionTaskStatus> {
        self.progression_status
            .lock()
            .map(|status| status.clone())
            .map_err(|_| Error::new("runtime_unavailable", "progression status lock failed"))
    }

    pub(super) fn drain_without_observer(&self, snapshot: &ControlSnapshot) -> Result<bool> {
        let state = self.state()?;
        Ok(drain_observation(
            snapshot,
            state.tasks.len() + usize::from(self.progression_status()?.pending),
            self.runtime.executions.active_operations()?.len(),
            state.recovery.len(),
        )
        .local_clear)
    }

    pub(super) async fn tick_observations(&self, allow_start: bool) -> Result<()> {
        let poll_ms = self.runtime.configuration.snapshot()?.runtime.poll_ms;
        if let ObservationTick::Joined(result) =
            self.observations.tick(poll_ms, allow_start).await?
        {
            match result {
                Ok(projection) => {
                    self.runtime.refresh_retry_references(&projection)?;
                    self.runtime.recover_provider_holds()?;
                }
                Err(error) => {
                    self.state()?.last_error = Some(error.clone());
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    pub(super) async fn request_advance(&self, lease: &Lease) {
        let mut slot = self.progression.lock().await;
        if slot.ready_advance.as_ref() != Some(lease)
            && !slot
                .task
                .as_ref()
                .is_some_and(|task| task.mode == ProgressionMode::Advance && &task.lease == lease)
        {
            slot.requested_advance = Some(lease.clone());
        }
    }

    pub(super) async fn take_completed_advance(&self, lease: &Lease) -> bool {
        let mut slot = self.progression.lock().await;
        if slot.ready_advance.as_ref() == Some(lease) {
            slot.ready_advance = None;
            true
        } else {
            false
        }
    }

    pub(super) async fn tick_progression(&self, lease: &Lease, maintenance: bool) -> Result<()> {
        let mut slot = self.progression.lock().await;
        if let Some((_, _, result)) = slot.join_finished().await {
            {
                let mut status = self.progression_status.lock().map_err(|_| {
                    Error::new("runtime_unavailable", "progression status lock failed")
                })?;
                status.pending = false;
                status.joined = status.joined.saturating_add(1);
                status.error = slot.error.clone();
            }
            result?;
        }
        let snapshot = self.runtime.control.snapshot()?;
        let config = self.runtime.configuration.snapshot()?;
        let has_work = {
            let state = self.state()?;
            !drain_observation(
                &snapshot,
                state.tasks.len(),
                self.runtime.executions.active_operations()?.len(),
                state.recovery.len(),
            )
            .local_clear
        };
        let Some(mode) = slot.select(
            lease,
            &snapshot,
            maintenance,
            has_work,
            observation_cadence(config.runtime.poll_ms),
        ) else {
            return Ok(());
        };
        if mode == ProgressionMode::Advance {
            self.fresh(lease)?;
            if let Some(error) = self.observations.status()?.error {
                return Err(error);
            }
            self.runtime.observations.admission_snapshot()?;
        } else {
            self.runtime.registration.verify_control_owner()?;
        }
        let delivery = self.delivery.clone();
        let runtime = self.runtime.clone();
        let corrections = self.corrections.clone();
        let recovery = self.recovery_maintenance();
        let owner = lease.clone();
        let observer = self.observations.clone();
        let wake = self.runtime.services.wake.clone();
        let poll_ms = config.runtime.poll_ms;
        {
            let mut status = self
                .progression_status
                .lock()
                .map_err(|_| Error::new("runtime_unavailable", "progression status lock failed"))?;
            status.pending = true;
            status.mode = Some(mode);
            status.lease = Some(owner.clone());
        }
        slot.task = Some(ProgressionTask {
            mode,
            lease: owner.clone(),
            join: tokio::spawn(async move {
                let phase = async {
                    if mode == ProgressionMode::Advance {
                        runtime.verify_owner(&owner)?;
                        runtime.refresh_active_corrections(&corrections).await?;
                        delivery.advance(&owner).await
                    } else {
                        runtime.registration.verify_control_owner()?;
                        let maintained = delivery.maintain().await;
                        let recovered = async {
                            recovery.recover_authorizations().await?;
                            recovery.recover_corrections().await?;
                            runtime.refresh_active_corrections(&corrections).await
                        }
                        .await;
                        maintained.and(recovered)
                    }
                };
                let (result, observed) = observer.maintain_while(poll_ms, phase).await;
                wake.notify_one();
                result.and(observed)
            }),
        });
        Ok(())
    }
}

#[cfg(test)]
#[path = "runtime_driver_tasks_tests.rs"]
mod tests;
