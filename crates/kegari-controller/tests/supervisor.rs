use kegari_control_api::identity::Actor;
use kegari_controller::supervisor::*;
use kegari_storage::PrivateDir;
use kegari_types::error::{Error, Result};
use kegari_v0::{
    control::*,
    runtime_host::{DrainState, SupervisorDriver, SupervisorFuture},
};
use serde_json::json;
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Notify, mpsc, watch};
use uuid::Uuid;

struct Probe {
    spawns: AtomicUsize,
    refreshes: AtomicUsize,
    retired: AtomicUsize,
    shutdown: AtomicUsize,
    alive: AtomicBool,
    sequence: Mutex<Option<mpsc::UnboundedReceiver<u64>>>,
}
struct Factory {
    host: Arc<ControlHost>,
    probe: Arc<Probe>,
}
struct Process {
    host: Arc<ControlHost>,
    probe: Arc<Probe>,
    lease: Lease,
    cycles: mpsc::UnboundedReceiver<u64>,
}
impl ProcessFactory for Factory {
    fn spawn(&self) -> SupervisorFuture<'_, Box<dyn SupervisedProcess>> {
        Box::pin(async move {
            self.probe.spawns.fetch_add(1, Ordering::SeqCst);
            self.probe.alive.store(true, Ordering::SeqCst);
            let lease = Lease {
                runtime_id: self.host.snapshot()?.intent.runtime_id,
                owner_generation: 1,
                owner_id: Uuid::new_v4(),
            };
            let cycles = self
                .probe
                .sequence
                .lock()
                .unwrap()
                .take()
                .ok_or_else(|| Error::new("fixture", "unexpected second spawn"))?;
            Ok(Box::new(Process {
                host: self.host.clone(),
                probe: self.probe.clone(),
                lease,
                cycles,
            }) as Box<dyn SupervisedProcess>)
        })
    }
}
impl SupervisedProcess for Process {
    fn lease(&self) -> &Lease {
        &self.lease
    }
    fn activate<'a>(&'a mut self, recovery: SupervisorFuture<'a, ()>) -> SupervisorFuture<'a, ()> {
        Box::pin(async move {
            recovery.await?;
            self.host.attach_reconciled(self.lease.clone())
        })
    }
    fn refresh(&mut self) -> SupervisorFuture<'_, ()> {
        Box::pin(async move {
            self.probe.refreshes.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }
    fn next_cycle(&mut self) -> SupervisorFuture<'_, u64> {
        Box::pin(async move {
            self.cycles
                .recv()
                .await
                .ok_or_else(|| Error::new("control_unavailable", "fixture channel closed"))
        })
    }
    fn exited(&mut self) -> Result<bool> {
        Ok(!self.probe.alive.load(Ordering::SeqCst))
    }
    fn shutdown(&mut self, _force: bool) -> SupervisorFuture<'_, ()> {
        Box::pin(async move {
            assert_eq!(self.host.snapshot()?.intent.desired, Intent::Stopped);
            self.probe.shutdown.fetch_add(1, Ordering::SeqCst);
            self.probe.alive.store(false, Ordering::SeqCst);
            self.host.orchestration_exited(&self.lease)
        })
    }
    fn retire_failed(&mut self) -> SupervisorFuture<'_, ()> {
        Box::pin(async move {
            let snapshot = self.host.snapshot()?;
            assert!(!snapshot.admission_open);
            self.probe.alive.store(false, Ordering::SeqCst);
            if snapshot.orchestration_alive {
                self.host.orchestration_exited(&self.lease)?;
            }
            Ok(())
        })
    }
    fn retire(&self, state: DrainState) -> Result<()> {
        assert!(state.local_clear && state.remote_reconciled);
        self.probe.retired.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
struct Driver {
    host: Arc<ControlHost>,
    polls: AtomicUsize,
    starts: AtomicUsize,
    recovers: AtomicUsize,
    cancels: AtomicUsize,
    block: AtomicBool,
    fail_recovery: AtomicBool,
    fail_prerequisite: AtomicBool,
    defer_cancel: AtomicBool,
    release: Notify,
}
impl SupervisorDriver for Driver {
    fn reconcile_before_start(&self) -> SupervisorFuture<'_, ()> {
        Box::pin(async move {
            self.starts.fetch_add(1, Ordering::SeqCst);
            if self.fail_prerequisite.load(Ordering::SeqCst) {
                return Err(Error::new("missing_secret", "fixture key unavailable"));
            }
            Ok(())
        })
    }
    fn recover<'a>(&'a self, _: &'a Lease) -> SupervisorFuture<'a, ()> {
        Box::pin(async move {
            self.recovers.fetch_add(1, Ordering::SeqCst);
            if self.fail_recovery.load(Ordering::SeqCst) {
                Err(Error::new(
                    "reconciliation_required",
                    "fixture unresolved prior effect",
                ))
            } else {
                Ok(())
            }
        })
    }
    fn poll<'a>(&'a self, _: &'a Lease, _: u64) -> SupervisorFuture<'a, ()> {
        Box::pin(async move {
            self.polls.fetch_add(1, Ordering::SeqCst);
            if self.block.load(Ordering::SeqCst) {
                self.release.notified().await;
            }
            Ok(())
        })
    }
    fn maintain<'a>(
        &'a self,
        _: &'a Lease,
        _: &'a ControlSnapshot,
    ) -> SupervisorFuture<'a, DrainState> {
        Box::pin(async move {
            Ok(DrainState {
                local_clear: self.host.snapshot()?.active_operations.is_empty(),
                remote_reconciled: true,
            })
        })
    }
    fn cancel_fenced<'a>(&'a self, operations: &'a [Uuid]) -> SupervisorFuture<'a, ()> {
        Box::pin(async move {
            let snapshot = self.host.snapshot()?;
            assert!(snapshot.persistence_error.is_none());
            self.cancels.fetch_add(1, Ordering::SeqCst);
            if self.defer_cancel.load(Ordering::SeqCst) {
                return Err(Error::new(
                    "operation_uncertain",
                    "fixture local exit is unconfirmed",
                ));
            }
            for id in operations {
                assert!(snapshot.fenced_operations.contains(id));
                self.host
                    .finish(*id, json!({"cancelled":true,"observed":true}))?;
            }
            self.release.notify_waiters();
            Ok(())
        })
    }
    fn exited<'a>(&'a self, _: &'a Lease) -> SupervisorFuture<'a, DrainState> {
        Box::pin(async move {
            Ok(DrainState {
                local_clear: self.host.snapshot()?.active_operations.is_empty(),
                remote_reconciled: true,
            })
        })
    }
}
struct Fixture {
    _temp: tempfile::TempDir,
    host: Arc<ControlHost>,
    probe: Arc<Probe>,
    driver: Arc<Driver>,
    wake: Arc<Notify>,
    cycles: mpsc::UnboundedSender<u64>,
    status: watch::Receiver<SupervisorStatus>,
    task: tokio::task::JoinHandle<Result<()>>,
    shutdown: HostShutdown,
}
fn command(host: &ControlHost) -> MutationRequest {
    MutationRequest {
        request_id: Uuid::new_v4(),
        runtime_id: host.snapshot().unwrap().intent.runtime_id,
        actor: Actor {
            identity: "operator".into(),
            principal: None,
            roles: ["lifecycle".into()].into(),
        },
        kind: "lifecycle".into(),
        canonical_targets: vec![host.snapshot().unwrap().intent.runtime_id.to_string()],
        expected: BTreeMap::new(),
        payload: json!({}),
    }
}
fn lifecycle(f: &Fixture, value: Lifecycle) {
    let generation = f.host.snapshot().unwrap().intent.generation;
    f.host
        .lifecycle(command(&f.host), value, generation)
        .unwrap();
    f.wake.notify_one();
}
fn fixture(step: Duration) -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap().join("control");
    let host = Arc::new(
        ControlHost::setup(
            PrivateDir::create(&root).unwrap(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        )
        .unwrap(),
    );
    let (cycles, receiver) = mpsc::unbounded_channel();
    let probe = Arc::new(Probe {
        spawns: AtomicUsize::new(0),
        refreshes: AtomicUsize::new(0),
        retired: AtomicUsize::new(0),
        shutdown: AtomicUsize::new(0),
        alive: AtomicBool::new(false),
        sequence: Mutex::new(Some(receiver)),
    });
    let driver = Arc::new(Driver {
        host: host.clone(),
        polls: AtomicUsize::new(0),
        starts: AtomicUsize::new(0),
        recovers: AtomicUsize::new(0),
        cancels: AtomicUsize::new(0),
        block: AtomicBool::new(false),
        fail_recovery: AtomicBool::new(false),
        fail_prerequisite: AtomicBool::new(false),
        defer_cancel: AtomicBool::new(false),
        release: Notify::new(),
    });
    let wake = Arc::new(Notify::new());
    let supervisor = Supervisor::with_process_factory(
        host.clone(),
        Arc::new(Factory {
            host: host.clone(),
            probe: probe.clone(),
        }),
        driver.clone(),
        wake.clone(),
        SupervisorLimits {
            heartbeat: Duration::from_millis(10),
            driver_step: step,
            shutdown: Duration::from_secs(1),
        },
    )
    .unwrap();
    let status = supervisor.observe();
    let shutdown = supervisor.shutdown_handle();
    let task = tokio::spawn(supervisor.run());
    Fixture {
        _temp: temp,
        host,
        probe,
        driver,
        wake,
        cycles,
        status,
        task,
        shutdown,
    }
}
async fn state(f: &mut Fixture, wanted: fn(&SupervisorState) -> bool) {
    tokio::time::timeout(
        Duration::from_secs(2),
        f.status.wait_for(|s| wanted(&s.state)),
    )
    .await
    .unwrap()
    .unwrap();
}
async fn count(value: &AtomicUsize, at_least: usize) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while value.load(Ordering::SeqCst) < at_least {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn stopped_never_spawns_pause_blocks_cycles_and_drain_exits_before_service_stop() {
    let mut f = fixture(Duration::from_secs(1));
    tokio::time::sleep(Duration::from_millis(25)).await;
    assert_eq!(f.probe.spawns.load(Ordering::SeqCst), 0);
    lifecycle(&f, Lifecycle::Start);
    state(&mut f, |s| matches!(s, SupervisorState::Running)).await;
    assert_eq!(f.driver.recovers.load(Ordering::SeqCst), 1);
    f.cycles.send(1).unwrap();
    count(&f.driver.polls, 1).await;
    lifecycle(&f, Lifecycle::Pause);
    state(&mut f, |s| matches!(s, SupervisorState::Paused)).await;
    f.cycles.send(2).unwrap();
    tokio::time::sleep(Duration::from_millis(25)).await;
    assert_eq!(f.driver.polls.load(Ordering::SeqCst), 1);
    lifecycle(&f, Lifecycle::Resume);
    state(&mut f, |s| matches!(s, SupervisorState::Running)).await;
    f.cycles.send(3).unwrap();
    count(&f.driver.polls, 2).await;
    lifecycle(&f, Lifecycle::Stop);
    state(&mut f, |s| matches!(s, SupervisorState::Stopped)).await;
    assert_eq!(f.probe.retired.load(Ordering::SeqCst), 1);
    assert_eq!(f.probe.shutdown.load(Ordering::SeqCst), 1);
    f.host.stop_service(command(&f.host), || Ok(())).unwrap();
    f.wake.notify_one();
    tokio::time::timeout(Duration::from_secs(1), f.task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}
#[tokio::test]
async fn forced_stop_is_durable_and_cancels_during_a_waiting_driver_step() {
    let mut f = fixture(Duration::from_secs(1));
    f.driver.block.store(true, Ordering::SeqCst);
    lifecycle(&f, Lifecycle::Start);
    state(&mut f, |s| matches!(s, SupervisorState::Running)).await;
    let lease = f.status.borrow().lease.clone().unwrap();
    f.host
        .admit(
            &lease,
            Uuid::new_v4(),
            "fixture-child".into(),
            "fixture-scope".into(),
            || Ok(()),
        )
        .unwrap();
    f.cycles.send(1).unwrap();
    count(&f.driver.polls, 1).await;
    count(&f.probe.refreshes, 2).await;
    lifecycle(&f, Lifecycle::ForceStop);
    state(&mut f, |s| matches!(s, SupervisorState::Stopped)).await;
    assert_eq!(f.driver.cancels.load(Ordering::SeqCst), 1);
    assert!(!f.host.snapshot().unwrap().orchestration_alive);
    f.task.abort();
}
#[tokio::test]
async fn a_timed_out_driver_suspends_owner_preserves_intent_and_never_retries_automatically() {
    let mut f = fixture(Duration::from_millis(45));
    f.driver.block.store(true, Ordering::SeqCst);
    lifecycle(&f, Lifecycle::Start);
    state(&mut f, |s| matches!(s, SupervisorState::Running)).await;
    f.cycles.send(1).unwrap();
    state(&mut f, |s| matches!(s, SupervisorState::Blocked)).await;
    let snapshot = f.host.snapshot().unwrap();
    assert_eq!(snapshot.intent.desired, Intent::Running);
    assert!(!snapshot.admission_open);
    assert!(!snapshot.orchestration_alive);
    assert_eq!(snapshot.unresolved_operations.len(), 1);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(f.probe.spawns.load(Ordering::SeqCst), 1);
    assert_eq!(f.driver.polls.load(Ordering::SeqCst), 1);
    f.task.abort();
}
#[tokio::test]
async fn recovery_failure_never_opens_admission_or_dispatches_a_poll() {
    let mut f = fixture(Duration::from_secs(1));
    f.driver.fail_recovery.store(true, Ordering::SeqCst);
    lifecycle(&f, Lifecycle::Start);
    state(&mut f, |s| matches!(s, SupervisorState::Blocked)).await;
    assert!(!f.host.snapshot().unwrap().admission_open);
    assert_eq!(f.driver.polls.load(Ordering::SeqCst), 0);
    assert_eq!(f.probe.spawns.load(Ordering::SeqCst), 1);
    f.task.abort();
}
#[tokio::test]
async fn unexpected_exit_closes_owner_and_retains_a_recovery_hold() {
    let mut f = fixture(Duration::from_secs(1));
    lifecycle(&f, Lifecycle::Start);
    state(&mut f, |s| matches!(s, SupervisorState::Running)).await;
    f.probe.alive.store(false, Ordering::SeqCst);
    f.wake.notify_one();
    state(&mut f, |s| matches!(s, SupervisorState::Blocked)).await;
    let snapshot = f.host.snapshot().unwrap();
    assert!(!snapshot.orchestration_alive);
    assert!(!snapshot.admission_open);
    assert_eq!(snapshot.intent.desired, Intent::Running);
    assert_eq!(snapshot.unresolved_operations.len(), 1);
    f.task.abort();
}
#[test]
fn supervisor_bounds_refuse_heartbeat_that_could_lose_ownership() {
    assert!(
        SupervisorLimits {
            heartbeat: Duration::from_secs(20),
            driver_step: Duration::from_secs(1),
            shutdown: Duration::from_secs(1)
        }
        .validate()
        .is_err()
    );
}

#[tokio::test]
async fn failed_owner_still_processes_later_force_stop_and_observes_owned_work_exit() {
    let mut f = fixture(Duration::from_millis(45));
    f.driver.block.store(true, Ordering::SeqCst);
    f.driver.defer_cancel.store(true, Ordering::SeqCst);
    lifecycle(&f, Lifecycle::Start);
    state(&mut f, |s| matches!(s, SupervisorState::Running)).await;
    let lease = f.status.borrow().lease.clone().unwrap();
    f.host
        .admit(
            &lease,
            Uuid::new_v4(),
            "failed-active-child".into(),
            "scope".into(),
            || Ok(()),
        )
        .unwrap();
    f.cycles.send(1).unwrap();
    state(&mut f, |s| matches!(s, SupervisorState::Blocked)).await;
    count(&f.driver.cancels, 1).await;
    assert_eq!(f.host.snapshot().unwrap().active_operations.len(), 1);
    f.driver.defer_cancel.store(false, Ordering::SeqCst);
    lifecycle(&f, Lifecycle::ForceStop);
    state(&mut f, |s| matches!(s, SupervisorState::Stopped)).await;
    assert!(f.host.snapshot().unwrap().active_operations.is_empty());
    assert_eq!(f.driver.cancels.load(Ordering::SeqCst), 2);
    assert_eq!(f.driver.polls.load(Ordering::SeqCst), 1);
    assert_eq!(f.probe.spawns.load(Ordering::SeqCst), 1);
    f.task.abort();
}

#[tokio::test]
async fn host_interrupt_preserves_intent_and_fences_before_owned_cancellation() {
    let mut f = fixture(Duration::from_secs(1));
    lifecycle(&f, Lifecycle::Start);
    state(&mut f, |s| matches!(s, SupervisorState::Running)).await;
    let lease = f.status.borrow().lease.clone().unwrap();
    f.host
        .admit(
            &lease,
            Uuid::new_v4(),
            "interrupt-child".into(),
            "scope".into(),
            || Ok(()),
        )
        .unwrap();
    let before = f.host.snapshot().unwrap().intent;
    f.shutdown.request();
    tokio::time::timeout(Duration::from_secs(2), &mut f.task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let after = f.host.snapshot().unwrap();
    assert_eq!(after.intent.generation, before.generation);
    assert_eq!(after.intent.desired, Intent::Running);
    assert!(!after.service_stopping);
    assert!(!after.admission_open);
    assert!(!after.orchestration_alive);
    assert!(after.active_operations.is_empty());
    assert!(!after.unresolved_operations.is_empty());
    assert_eq!(f.driver.cancels.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn idle_host_interrupt_starts_no_orchestration_and_changes_no_intent() {
    let mut f = fixture(Duration::from_secs(1));
    let before = f.host.snapshot().unwrap().intent;
    f.shutdown.request();
    tokio::time::timeout(Duration::from_secs(1), &mut f.task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let after = f.host.snapshot().unwrap();
    assert_eq!(after.intent.generation, before.generation);
    assert_eq!(after.intent.desired, Intent::Stopped);
    assert!(!after.service_stopping);
    assert_eq!(f.probe.spawns.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn restored_prerequisite_retries_only_before_any_owner_was_started() {
    let mut f = fixture(Duration::from_secs(1));
    f.driver.fail_prerequisite.store(true, Ordering::SeqCst);
    lifecycle(&f, Lifecycle::Start);
    state(&mut f, |s| matches!(s, SupervisorState::Blocked)).await;
    let generation = f.host.snapshot().unwrap().intent.generation;
    assert_eq!(f.probe.spawns.load(Ordering::SeqCst), 0);
    f.driver.fail_prerequisite.store(false, Ordering::SeqCst);
    f.shutdown.prerequisites_ready();
    state(&mut f, |s| matches!(s, SupervisorState::Running)).await;
    assert_eq!(f.host.snapshot().unwrap().intent.generation, generation);
    assert_eq!(f.probe.spawns.load(Ordering::SeqCst), 1);
    f.shutdown.request();
    tokio::time::timeout(Duration::from_secs(2), &mut f.task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn stopped_control_owner_cancels_fenced_local_handles_without_a_scheduler_lease() {
    let f = fixture(Duration::from_secs(1));
    f.driver.defer_cancel.store(true, Ordering::SeqCst);
    let operation = Uuid::new_v4();
    f.host.fence(operation).unwrap();
    lifecycle(&f, Lifecycle::ForceStop);
    count(&f.driver.cancels, 1).await;
    assert_eq!(f.probe.spawns.load(Ordering::SeqCst), 0);
    assert_eq!(f.host.snapshot().unwrap().intent.desired, Intent::Stopped);
    assert!(
        f.host
            .snapshot()
            .unwrap()
            .fenced_operations
            .contains(&operation)
    );
    f.shutdown.request();
    f.task.await.unwrap().unwrap();
}
