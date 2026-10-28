//! Native orchestration ownership and finite scheduling opportunities. The driver
//! is mandatory: no successful empty recovery or permissive scheduler is supplied.
use crate::{
    control::{ControlHost, ControlSnapshot, Intent, Lease},
    orchestration::RunningOrchestration,
    platform::{BinaryIdentity, HostIdentity, Registration},
};
use chrono::{DateTime, Utc};
use kegari_types::error::{Error, Result};
use serde::Serialize;
use std::{
    collections::BTreeSet,
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::{Notify, watch},
    time::{Instant, MissedTickBehavior, timeout},
};
use uuid::Uuid;

pub type SupervisorFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DrainState {
    pub local_clear: bool,
    pub remote_reconciled: bool,
}
/// Methods operate outside the control mutex and must use actual owned workers,
/// provider facts and receipt recovery. poll schedules bounded operations; it
/// never waits for the lifetime of a model turn or repository command.
pub trait SupervisorDriver: Send + Sync {
    fn reconcile_before_start(&self) -> SupervisorFuture<'_, ()>;
    fn recover<'a>(&'a self, lease: &'a Lease) -> SupervisorFuture<'a, ()>;
    fn poll<'a>(&'a self, lease: &'a Lease, sequence: u64) -> SupervisorFuture<'a, ()>;
    fn maintain<'a>(
        &'a self,
        lease: &'a Lease,
        snapshot: &'a ControlSnapshot,
    ) -> SupervisorFuture<'a, DrainState>;
    /// Called once per generation/operation after durable fencing. Cancellation
    /// results must remain pending/unknown until actually observed.
    fn cancel_fenced<'a>(&'a self, operations: &'a [Uuid]) -> SupervisorFuture<'a, ()>;
    fn exited<'a>(&'a self, lease: &'a Lease) -> SupervisorFuture<'a, DrainState>;
}
#[derive(Clone, Copy, Debug)]
pub struct SupervisorLimits {
    pub heartbeat: Duration,
    pub driver_step: Duration,
    pub shutdown: Duration,
}
impl SupervisorLimits {
    pub fn validate(&self) -> Result<()> {
        if self.heartbeat.is_zero()
            || self.heartbeat >= Duration::from_secs(10)
            || self.driver_step.is_zero()
            || self.shutdown.is_zero()
            || self.driver_step > Duration::from_secs(3600)
            || self.shutdown > Duration::from_secs(60)
        {
            return Err(Error::invalid(
                "supervisor limits must be finite; heartbeat must stay below host silence",
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SupervisorState {
    Stopped,
    Paused,
    Starting,
    Recovering,
    Running,
    Draining,
    Blocked,
    ServiceStopping,
}
#[derive(Clone, Debug, Serialize)]
pub struct SupervisorStatus {
    pub state: SupervisorState,
    pub lease: Option<Lease>,
    pub last_cycle: u64,
    pub error: Option<Error>,
    pub observed_at: DateTime<Utc>,
}
impl SupervisorStatus {
    fn new(state: SupervisorState) -> Self {
        Self {
            state,
            lease: None,
            last_cycle: 0,
            error: None,
            observed_at: Utc::now(),
        }
    }
}
/// This interface permits deterministic process-channel fixtures; production
/// construction below always installs NativeFactory with actual OS ownership.
pub trait SupervisedProcess: Send {
    fn lease(&self) -> &Lease;
    fn activate<'a>(&'a mut self, recovery: SupervisorFuture<'a, ()>) -> SupervisorFuture<'a, ()>;
    fn refresh(&mut self) -> SupervisorFuture<'_, ()>;
    fn next_cycle(&mut self) -> SupervisorFuture<'_, u64>;
    fn exited(&mut self) -> Result<bool>;
    fn shutdown(&mut self, force: bool) -> SupervisorFuture<'_, ()>;
    fn retire_failed(&mut self) -> SupervisorFuture<'_, ()>;
    fn retire(&self, observed: DrainState) -> Result<()>;
}
pub trait ProcessFactory: Send + Sync {
    fn spawn(&self) -> SupervisorFuture<'_, Box<dyn SupervisedProcess>>;
}
struct NativeFactory {
    registration: Arc<Registration>,
    control: Arc<ControlHost>,
    host: HostIdentity,
    binary: BinaryIdentity,
    poll_ms: u64,
}
struct NativeProcess {
    running: RunningOrchestration,
    registration: Arc<Registration>,
}
impl ProcessFactory for NativeFactory {
    fn spawn(&self) -> SupervisorFuture<'_, Box<dyn SupervisedProcess>> {
        Box::pin(async move {
            let running = RunningOrchestration::spawn(
                self.registration.clone(),
                self.control.clone(),
                &self.host,
                &self.binary,
                self.poll_ms,
            )
            .await?;
            Ok(Box::new(NativeProcess {
                running,
                registration: self.registration.clone(),
            }) as Box<dyn SupervisedProcess>)
        })
    }
}
impl SupervisedProcess for NativeProcess {
    fn lease(&self) -> &Lease {
        self.running.lease()
    }
    fn activate<'a>(&'a mut self, recovery: SupervisorFuture<'a, ()>) -> SupervisorFuture<'a, ()> {
        Box::pin(self.running.activate(recovery))
    }
    fn refresh(&mut self) -> SupervisorFuture<'_, ()> {
        Box::pin(self.running.refresh_intent())
    }
    fn next_cycle(&mut self) -> SupervisorFuture<'_, u64> {
        Box::pin(self.running.next_cycle())
    }
    fn exited(&mut self) -> Result<bool> {
        self.running.exited()
    }
    fn shutdown(&mut self, force: bool) -> SupervisorFuture<'_, ()> {
        Box::pin(self.running.shutdown(force))
    }
    fn retire_failed(&mut self) -> SupervisorFuture<'_, ()> {
        Box::pin(self.running.retire_failed())
    }
    fn retire(&self, observed: DrainState) -> Result<()> {
        self.registration.reconcile_exited(
            self.running.lease(),
            observed.local_clear,
            observed.remote_reconciled,
        )
    }
}
pub struct NativeSupervisor {
    pub registration: Arc<Registration>,
    pub control: Arc<ControlHost>,
    pub host: HostIdentity,
    pub binary: BinaryIdentity,
    pub poll_ms: u64,
    pub driver: Arc<dyn SupervisorDriver>,
    pub wake: Arc<Notify>,
    pub limits: SupervisorLimits,
}
pub struct Supervisor {
    control: Arc<ControlHost>,
    factory: Arc<dyn ProcessFactory>,
    driver: Arc<dyn SupervisorDriver>,
    wake: Arc<Notify>,
    limits: SupervisorLimits,
    status: watch::Sender<SupervisorStatus>,
    shutdown: HostShutdown,
}
/// Host process interruption preserves desired orchestration intent. It is not
/// an operator stop and never writes the explicit service-stop reservation.
#[derive(Clone)]
pub struct HostShutdown {
    requested: Arc<AtomicBool>,
    wake: Arc<Notify>,
    prerequisites_ready: Arc<AtomicBool>,
    control_wake: Arc<Notify>,
}
impl HostShutdown {
    /// Retry only a pre-spawn prerequisite failure. This never resets an actual
    /// owner failure, attempt budget, unknown effect or intervention hold.
    pub fn prerequisites_ready(&self) {
        self.prerequisites_ready.store(true, Ordering::SeqCst);
        self.control_wake.notify_waiters();
    }
    pub fn request(&self) {
        self.requested.store(true, Ordering::SeqCst);
        self.wake.notify_waiters();
    }
    fn requested(&self) -> bool {
        self.requested.load(Ordering::SeqCst)
    }
    async fn notified(&self) {
        loop {
            let next = self.wake.notified();
            if self.requested() {
                return;
            }
            next.await;
        }
    }
}
impl Supervisor {
    pub fn native(input: NativeSupervisor) -> Result<Self> {
        let factory = Arc::new(NativeFactory {
            registration: input.registration,
            control: input.control.clone(),
            host: input.host,
            binary: input.binary,
            poll_ms: input.poll_ms,
        });
        Self::with_process_factory(
            input.control,
            factory,
            input.driver,
            input.wake,
            input.limits,
        )
    }
    /// Injected process fixtures never establish native qualification. Actual
    /// worker/delivery gates independently recheck OS registration ownership.
    pub fn with_process_factory(
        control: Arc<ControlHost>,
        factory: Arc<dyn ProcessFactory>,
        driver: Arc<dyn SupervisorDriver>,
        wake: Arc<Notify>,
        limits: SupervisorLimits,
    ) -> Result<Self> {
        limits.validate()?;
        let (status, _) = watch::channel(SupervisorStatus::new(SupervisorState::Stopped));
        Ok(Self {
            control,
            factory,
            driver,
            wake: wake.clone(),
            limits,
            status,
            shutdown: HostShutdown {
                requested: Arc::new(AtomicBool::new(false)),
                wake: Arc::new(Notify::new()),
                prerequisites_ready: Arc::new(AtomicBool::new(false)),
                control_wake: wake,
            },
        })
    }
    pub fn observe(&self) -> watch::Receiver<SupervisorStatus> {
        self.status.subscribe()
    }
    pub fn shutdown_handle(&self) -> HostShutdown {
        self.shutdown.clone()
    }
    fn publish(
        &self,
        state: SupervisorState,
        lease: Option<&Lease>,
        cycle: u64,
        error: Option<Error>,
    ) {
        self.status.send_replace(SupervisorStatus {
            state,
            lease: lease.cloned(),
            last_cycle: cycle,
            error,
            observed_at: Utc::now(),
        });
    }
    async fn bounded<T>(&self, future: SupervisorFuture<'_, T>) -> Result<T> {
        timeout(self.limits.driver_step, future)
            .await
            .map_err(|_| {
                Error::new(
                    "timeout",
                    "supervisor driver step exceeded its bound; reconciliation is required",
                )
            })?
    }
    async fn wait_change(&self) {
        tokio::select! {_ = self.shutdown.notified()=>{},_ = self.wake.notified()=>{},_ = tokio::time::sleep(self.limits.heartbeat)=>{}}
    }
    /// A failed owner is not silently restarted in a tight loop. Its durable
    /// suspension must be reconciled by the driver and a new operator command
    /// observed before another start attempt under the same saved RUNNING intent.
    pub async fn run(self) -> Result<()> {
        let mut blocked_command = None;
        let mut prerequisite_failure = false;
        let mut cancelled = BTreeSet::new();
        loop {
            if self.shutdown.requested() {
                return Ok(());
            }
            let snapshot = self.control.snapshot()?;
            if let Err(error) = self.cancel_new_fences(&mut cancelled).await {
                self.publish(SupervisorState::Blocked, None, 0, Some(error));
            }
            if prerequisite_failure
                && self
                    .shutdown
                    .prerequisites_ready
                    .swap(false, Ordering::SeqCst)
            {
                blocked_command = None;
                prerequisite_failure = false;
            }
            if snapshot.service_stopping {
                self.publish(SupervisorState::ServiceStopping, None, 0, None);
                return Ok(());
            }
            if snapshot.intent.desired != Intent::Running
                || blocked_command == Some(snapshot.intent.last_command_id)
            {
                if blocked_command != Some(snapshot.intent.last_command_id) {
                    self.publish(
                        if snapshot.intent.desired == Intent::Paused {
                            SupervisorState::Paused
                        } else {
                            SupervisorState::Stopped
                        },
                        None,
                        0,
                        None,
                    );
                }
                self.wait_change().await;
                continue;
            }
            let command = snapshot.intent.last_command_id;
            self.publish(SupervisorState::Starting, None, 0, None);
            let recovery = tokio::select! { _=self.shutdown.notified()=>return Ok(()), result=self.bounded(self.driver.reconcile_before_start())=>result };
            if let Err(error) = recovery {
                self.publish(SupervisorState::Blocked, None, 0, Some(error));
                blocked_command = Some(command);
                prerequisite_failure = true;
                continue;
            }
            prerequisite_failure = false;
            // Recheck after the asynchronous read-only recovery. An operator may
            // have stopped/paused while prerequisites were being inspected.
            let current = self.control.snapshot()?;
            if current.intent.desired != Intent::Running
                || current.service_stopping
                || self.shutdown.requested()
            {
                continue;
            }
            let mut process = match timeout(self.limits.shutdown, self.factory.spawn()).await {
                Ok(Ok(process)) => process,
                result => {
                    let error = match result {
                        Ok(Err(error)) => error,
                        _ => Error::new(
                            "timeout",
                            "orchestration startup exceeded its bound; preserve pending ownership",
                        ),
                    };
                    self.publish(SupervisorState::Blocked, None, 0, Some(error));
                    blocked_command = Some(command);
                    continue;
                }
            };
            let lease = process.lease().clone();
            self.publish(SupervisorState::Recovering, Some(&lease), 0, None);
            let activated = tokio::select! { _=self.shutdown.notified()=>Err(Error::new("host_shutdown", "control host interruption requires owned work reconciliation")), result=timeout(
                self.limits.driver_step,
                process.activate(self.driver.recover(&lease)),
            )=>result.map_err(|_|Error::new(
                    "timeout",
                    "orchestration recovery exceeded its bound",
                )).and_then(|value|value) };
            let result = match activated {
                Ok(()) => self.run_owner(process.as_mut(), &mut cancelled).await,
                Err(error) => Err(error),
            };
            if let Err(error) = result {
                // Suspension closes/fences admission durably before the process
                // handle is released. No operator intent is changed here.
                if self.control.snapshot()?.orchestration_alive {
                    self.control.suspend_owner(&lease, &error.code)?;
                }
                self.publish(SupervisorState::Blocked, Some(&lease), 0, Some(error));
                blocked_command = Some(self.control.snapshot()?.intent.last_command_id);
                self.observe_failed(
                    process.as_mut(),
                    &mut cancelled,
                    blocked_command.expect("failure command bound"),
                )
                .await?;
            } else {
                blocked_command = None;
            }
        }
    }
    async fn run_owner(
        &self,
        process: &mut dyn SupervisedProcess,
        cancelled: &mut BTreeSet<(u64, Uuid)>,
    ) -> Result<()> {
        let lease = process.lease().clone();
        let mut heartbeat = tokio::time::interval(self.limits.heartbeat);
        heartbeat.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut last_cycle = 0;
        loop {
            if self.shutdown.requested() {
                return Err(Error::new(
                    "host_shutdown",
                    "control host interruption requires owned work reconciliation",
                ));
            }
            let snapshot = self.control.snapshot()?;
            if process.exited()? {
                if snapshot.intent.desired != Intent::Stopped {
                    self.control.suspend_owner(&lease, "control_unavailable")?;
                }
                self.control.orchestration_exited(&lease)?;
                return Err(Error::new(
                    "control_unavailable",
                    "owned orchestration exited unexpectedly; preserved work requires reconciliation",
                ));
            }
            self.cancel_new_fences(cancelled).await?;
            let observed = self
                .with_heartbeats(process, self.driver.maintain(&lease, &snapshot), cancelled)
                .await?;
            let current = self.control.snapshot()?;
            let force = !current.active_operations.is_empty()
                && current
                    .active_operations
                    .iter()
                    .all(|permit| current.fenced_operations.contains(&permit.operation_id));
            if current.intent.desired == Intent::Stopped
                && observed.local_clear
                && (current.active_operations.is_empty() || force)
            {
                timeout(self.limits.shutdown, process.shutdown(force))
                    .await
                    .map_err(|_| {
                        Error::new(
                            "stop_in_progress",
                            "owned process shutdown exceeded its bound",
                        )
                    })??;
                let settled = self.bounded(self.driver.exited(&lease)).await?;
                process.retire(settled)?;
                self.publish(SupervisorState::Stopped, None, last_cycle, None);
                return Ok(());
            }
            self.publish(
                match current.intent.desired {
                    Intent::Running => SupervisorState::Running,
                    Intent::Paused => SupervisorState::Paused,
                    Intent::Stopped => SupervisorState::Draining,
                },
                Some(&lease),
                last_cycle,
                None,
            );
            tokio::select! {
                biased;
                _=self.shutdown.notified()=>return Err(Error::new("host_shutdown", "control host interruption requires owned work reconciliation")),
                _=self.wake.notified()=>process.refresh().await?,
                _=heartbeat.tick()=>process.refresh().await?,
                cycle=process.next_cycle()=>match cycle {
                    Ok(sequence)=>{
                        if sequence<=last_cycle{return Err(Error::new("ownership_conflict","repeated supervisor cycle sequence"));}
                        last_cycle=sequence;
                        let snapshot=self.control.snapshot()?;
                        if snapshot.intent.desired==Intent::Running && snapshot.admission_open {
                            self.with_heartbeats(process,self.driver.poll(&lease,sequence), cancelled).await?;
                        }
                    }
                    Err(error) if error.code=="admission_closed"=>(),
                    Err(error)=>return Err(error),
                }
            }
        }
    }
    /// Driver work may await a provider, but ownership heartbeats and lifecycle
    /// observations remain responsive. Calls still have one finite deadline.
    async fn with_heartbeats<T>(
        &self,
        process: &mut dyn SupervisedProcess,
        future: SupervisorFuture<'_, T>,
        cancelled: &mut BTreeSet<(u64, Uuid)>,
    ) -> Result<T> {
        let mut future = future;
        let deadline = Instant::now() + self.limits.driver_step;
        let mut heartbeat = tokio::time::interval(self.limits.heartbeat);
        heartbeat.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _=self.shutdown.notified()=>return Err(Error::new("host_shutdown", "control host interruption requires owned work reconciliation")),
                result=&mut future=>return result,
                _=tokio::time::sleep_until(deadline)=>return Err(Error::new("timeout","supervisor driver step exceeded its bound; retain unknown effects")),
                _=self.wake.notified()=>{process.refresh().await?;self.cancel_new_fences(cancelled).await?;},
                _=heartbeat.tick()=>{process.refresh().await?;self.cancel_new_fences(cancelled).await?;},
            }
        }
    }
    async fn observe_failed(
        &self,
        process: &mut dyn SupervisedProcess,
        cancelled: &mut BTreeSet<(u64, Uuid)>,
        failed_command: Uuid,
    ) -> Result<()> {
        let lease = process.lease().clone();
        if let Err(error) = self.cancel_new_fences(cancelled).await {
            self.publish(SupervisorState::Blocked, Some(&lease), 0, Some(error));
        }
        // This retires only the scheduling child after admission is closed; it
        // never adopts a process by PID or claims worker/remote completion.
        if let Err(error) = timeout(self.limits.shutdown, process.retire_failed())
            .await
            .map_err(|_| {
                Error::new(
                    "stop_in_progress",
                    "failed scheduling process remains unconfirmed",
                )
            })
            .and_then(|value| value)
        {
            self.publish(SupervisorState::Blocked, Some(&lease), 0, Some(error));
        }
        let mut retired = false;
        let mut host_exit_deadline = None;
        loop {
            if self.shutdown.requested() && host_exit_deadline.is_none() {
                host_exit_deadline = Some(Instant::now() + self.limits.shutdown);
            }
            if host_exit_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                return Err(Error::new(
                    "operation_uncertain",
                    "host exit reached its bound with owned work unconfirmed; retain recovery holds",
                ));
            }
            let snapshot = self.control.snapshot()?;
            if let Err(error) = self.cancel_new_fences(cancelled).await {
                self.publish(SupervisorState::Blocked, Some(&lease), 0, Some(error));
            }
            if process.exited()? {
                if self.control.snapshot()?.orchestration_alive {
                    self.control.orchestration_exited(&lease)?;
                }
                if !retired {
                    match self.bounded(self.driver.exited(&lease)).await {
                        Ok(observed) if observed.local_clear && self.shutdown.requested() => {
                            // Unknown remote effects and owner metadata remain for
                            // recovery. Local exit is the host shutdown boundary.
                            return Ok(());
                        }
                        Ok(observed) if observed.local_clear && observed.remote_reconciled => {
                            match process.retire(observed) {
                                Ok(()) => retired = true,
                                Err(error) => self.publish(
                                    SupervisorState::Blocked,
                                    Some(&lease),
                                    0,
                                    Some(error),
                                ),
                            };
                        }
                        Ok(_) => (),
                        Err(error) => {
                            self.publish(SupervisorState::Blocked, Some(&lease), 0, Some(error))
                        }
                    }
                }
            }
            if retired
                && (snapshot.service_stopping || snapshot.intent.last_command_id != failed_command)
            {
                return Ok(());
            }
            if self.shutdown.requested() {
                tokio::time::sleep(self.limits.heartbeat).await;
            } else {
                self.wait_change().await;
            }
        }
    }
    async fn cancel_new_fences(&self, cancelled: &mut BTreeSet<(u64, Uuid)>) -> Result<()> {
        let snapshot = self.control.snapshot()?;
        let pending: Vec<_> = snapshot
            .fenced_operations
            .iter()
            .map(|operation| (snapshot.intent.generation, *operation))
            .filter(|key| !cancelled.contains(key))
            .collect();
        if pending.is_empty() {
            return Ok(());
        }
        if snapshot.persistence_error.is_some() {
            return Err(Error::new(
                "control_failed",
                "intent persistence is unconfirmed; cancellation is not authorized",
            ));
        }
        cancelled.extend(pending.iter().copied());
        let ids: Vec<_> = pending.into_iter().map(|(_, id)| id).collect();
        timeout(
            self.limits.driver_step.min(Duration::from_secs(5)),
            self.driver.cancel_fenced(&ids),
        )
        .await
        .map_err(|_| {
            Error::new(
                "operation_uncertain",
                "owned cancellation exceeded its bound; reconcile without repeating",
            )
        })?
    }
}
