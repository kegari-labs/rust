//! Component tasks and real Control journals. These tests do not launch native
//! workers or manufacture completed model/process capabilities.
use super::*;
use crate::control::ControlHost;
use crate::control::Lifecycle;
use crate::control::MutationRequest;
use kegari_control_api::identity::Actor;
use tokio::sync::oneshot;

fn command(host: &ControlHost) -> MutationRequest {
    let runtime = host.snapshot().unwrap().intent.runtime_id;
    MutationRequest {
        request_id: Uuid::new_v4(),
        runtime_id: runtime,
        actor: Actor {
            identity: "task-component".into(),
            principal: None,
            roles: ["lifecycle".into()].into(),
        },
        kind: "lifecycle".into(),
        canonical_targets: vec![runtime.to_string()],
        expected: BTreeMap::new(),
        payload: json!({}),
    }
}

fn started() -> (tempfile::TempDir, Arc<ControlHost>, Lease) {
    let temp = tempfile::tempdir().unwrap();
    let runtime = Uuid::new_v4();
    let control = Arc::new(
        ControlHost::setup(
            PrivateDir::create(&temp.path().canonicalize().unwrap().join("control")).unwrap(),
            runtime,
            Uuid::new_v4(),
        )
        .unwrap(),
    );
    control
        .lifecycle(command(&control), Lifecycle::Start, 0)
        .unwrap();
    let lease = Lease {
        runtime_id: runtime,
        owner_generation: 1,
        owner_id: Uuid::new_v4(),
    };
    control.attach_reconciled(lease.clone()).unwrap();
    (temp, control, lease)
}

async fn complete(slot: &mut ProgressionSlot) -> (ProgressionMode, Lease, Result<()>) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Some(joined) = slot.join_finished().await {
                return joined;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("owned component task completes")
}

#[tokio::test]
async fn pending_maintenance_cannot_overlap_or_starve_requested_advance() {
    let (_temp, control, lease) = started();
    let mut slot = ProgressionSlot::default();
    assert_eq!(
        slot.select(
            &lease,
            &control.snapshot().unwrap(),
            true,
            false,
            Duration::from_secs(10)
        ),
        Some(ProgressionMode::Maintain)
    );
    let (release, wait) = oneshot::channel();
    slot.task = Some(ProgressionTask {
        mode: ProgressionMode::Maintain,
        lease: lease.clone(),
        join: tokio::spawn(async move {
            wait.await.unwrap();
            Ok(())
        }),
    });
    slot.requested_advance = Some(lease.clone());
    for _ in 0..4 {
        assert!(slot.join_finished().await.is_none());
        assert_eq!(
            slot.select(
                &lease,
                &control.snapshot().unwrap(),
                true,
                false,
                Duration::from_secs(10)
            ),
            None
        );
        assert!(slot.task.is_some());
    }
    release.send(()).unwrap();
    complete(&mut slot).await.2.unwrap();
    assert_eq!(
        slot.select(
            &lease,
            &control.snapshot().unwrap(),
            true,
            false,
            Duration::from_secs(10)
        ),
        Some(ProgressionMode::Advance),
        "the queued poll wins over another maintenance pass"
    );
    slot.task = Some(ProgressionTask {
        mode: ProgressionMode::Advance,
        lease: lease.clone(),
        join: tokio::spawn(async { Ok(()) }),
    });
    complete(&mut slot).await.2.unwrap();
    assert_eq!(slot.ready_advance, Some(lease.clone()));
    assert_eq!(
        slot.select(
            &lease,
            &control.snapshot().unwrap(),
            true,
            false,
            Duration::from_secs(10)
        ),
        None,
        "joined advance is consumed before maintenance can begin"
    );
}

#[tokio::test]
async fn force_stop_fences_pending_phase_without_dropping_its_owned_task() {
    let (_temp, control, lease) = started();
    let permit = control
        .admit(
            &lease,
            Uuid::new_v4(),
            "phase-component".into(),
            canonical_digest(1, &"scope").unwrap(),
            || Ok(()),
        )
        .unwrap();
    let (release, wait) = oneshot::channel();
    let owned_control = control.clone();
    let owned_permit = permit.clone();
    let mut slot = ProgressionSlot {
        task: Some(ProgressionTask {
            mode: ProgressionMode::Advance,
            lease: lease.clone(),
            join: tokio::spawn(async move {
                wait.await.unwrap();
                owned_control.validate_permit(&owned_permit, || Ok(()))
            }),
        }),
        ..Default::default()
    };
    let generation = control.snapshot().unwrap().intent.generation;
    control
        .lifecycle(command(&control), Lifecycle::ForceStop, generation)
        .unwrap();
    assert!(
        control
            .snapshot()
            .unwrap()
            .fenced_operations
            .contains(&permit.operation_id)
    );
    assert!(slot.join_finished().await.is_none());
    assert!(!drain_observation(&control.snapshot().unwrap(), 1, 0, 0).local_clear);
    release.send(()).unwrap();
    assert!(complete(&mut slot).await.2.is_err());
    assert!(slot.ready_advance.is_none());
    assert!(slot.error.is_some());
    assert!(
        control
            .snapshot()
            .unwrap()
            .active_operations
            .iter()
            .any(|active| active.operation_id == permit.operation_id),
        "joining a fenced task cannot retire its admission or infer process exit"
    );
}

#[tokio::test]
async fn stopped_final_maintenance_becomes_clear_and_never_restarts_advance() {
    let (_temp, control, lease) = started();
    let mut slot = ProgressionSlot {
        requested_advance: Some(lease.clone()),
        ready_advance: Some(lease.clone()),
        ..Default::default()
    };
    let generation = control.snapshot().unwrap().intent.generation;
    control
        .lifecycle(command(&control), Lifecycle::Stop, generation)
        .unwrap();
    let snapshot = control.snapshot().unwrap();
    assert_eq!(
        slot.select(&lease, &snapshot, true, false, Duration::ZERO),
        Some(ProgressionMode::Maintain)
    );
    assert!(slot.requested_advance.is_none());
    assert!(slot.ready_advance.is_none());
    slot.task = Some(ProgressionTask {
        mode: ProgressionMode::Maintain,
        lease: lease.clone(),
        join: tokio::spawn(async { Ok(()) }),
    });
    complete(&mut slot).await.2.unwrap();
    assert_eq!(
        slot.select(&lease, &snapshot, true, false, Duration::ZERO),
        None,
        "a clear stopped owner does not schedule endless maintenance"
    );
    assert!(drain_observation(&snapshot, 0, 0, 0).local_clear);
}

#[tokio::test]
async fn observation_join_cancellation_retains_handle_and_nonblocking_pending_status() {
    let (_temp, built, _delivery) =
        crate::runtime_delivery::tests::fixture_authorization(false).await;
    let owner =
        ObservationMaintenance::new(built.runtime.observations.clone(), Arc::new(Notify::new()));
    let projection = built.runtime.observations.snapshot().unwrap();
    let (release, wait) = oneshot::channel();
    owner.slot.lock().await.task = Some(tokio::spawn(async move {
        wait.await.unwrap();
        Ok(projection)
    }));
    owner.status.lock().unwrap().pending = true;
    assert!(
        tokio::time::timeout(Duration::from_millis(5), owner.join())
            .await
            .is_err()
    );
    assert!(owner.pending().unwrap());
    assert_eq!(owner.status().unwrap().joined, 0);
    let waiting = owner.join();
    tokio::pin!(waiting);
    tokio::select! {
        _ = &mut waiting => panic!("unreleased observation completed"),
        _ = tokio::task::yield_now() => {}
    }
    assert!(
        owner.pending().unwrap(),
        "pending status cannot wait behind a join mutex"
    );
    assert!(matches!(
        owner.tick(1, true).await.unwrap(),
        ObservationTick::Pending
    ));
    release.send(()).unwrap();
    assert!(waiting.await.unwrap().is_some());
    assert!(!owner.pending().unwrap());
    assert_eq!(owner.status().unwrap().joined, 1);
}

#[tokio::test]
async fn panicked_observation_invalidates_cache_and_phase_failure_joins_remaining_refresh() {
    let (_temp, built, _delivery) =
        crate::runtime_delivery::tests::fixture_authorization(false).await;
    let cache = built.runtime.observations.clone();
    let owner = ObservationMaintenance::new(cache.clone(), Arc::new(Notify::new()));
    owner.slot.lock().await.task = Some(tokio::spawn(async {
        panic!("component observation panic")
    }));
    owner.status.lock().unwrap().pending = true;
    assert_eq!(
        owner.join().await.unwrap_err().code,
        "observation_unavailable"
    );
    assert!(cache.admission_snapshot().is_err());
    assert!(owner.status().unwrap().error.is_some());

    let projection = cache.snapshot().unwrap();
    let (release, wait) = oneshot::channel();
    owner.slot.lock().await.task = Some(tokio::spawn(async move {
        wait.await.unwrap();
        Ok(projection)
    }));
    owner.status.lock().unwrap().pending = true;
    let result = owner.maintain_while(10_000, async {
        Err::<(), _>(Error::new("stale_result", "component correction changed"))
    });
    tokio::pin!(result);
    tokio::select! {
        _ = &mut result => panic!("phase failure discarded its pending refresh"),
        _ = tokio::task::yield_now() => {}
    }
    assert!(owner.pending().unwrap());
    release.send(()).unwrap();
    let (phase, refresh) = result.await;
    assert_eq!(phase.unwrap_err().code, "stale_result");
    refresh.unwrap();
    assert!(!owner.pending().unwrap());
    assert_eq!(owner.status().unwrap().joined, 2);
}

#[test]
fn observation_cadence_cannot_exceed_ten_seconds() {
    assert_eq!(observation_cadence(250), Duration::from_millis(250));
    assert_eq!(observation_cadence(60_000), Duration::from_secs(10));
}

/// This progression component owns a real finite admission but starts no
/// process/effect. Its explicit barrier models a still-running delivery phase;
/// it cannot create a native exit, model result, or delivery proof.
struct HeldProgression {
    control: Arc<ControlHost>,
    permit: crate::control::Permit,
    release: Mutex<Option<oneshot::Receiver<()>>>,
    started: Notify,
    advances: std::sync::atomic::AtomicUsize,
    maintenance: std::sync::atomic::AtomicUsize,
    cancellations: std::sync::atomic::AtomicUsize,
}

impl RuntimeProgression for HeldProgression {
    fn reconcile(&self) -> SupervisorFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }

    fn advance<'a>(&'a self, lease: &'a Lease) -> SupervisorFuture<'a, ()> {
        Box::pin(async move {
            assert_eq!(lease, &self.permit.lease);
            self.advances
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let wait = self
                .release
                .lock()
                .unwrap()
                .take()
                .expect("one owned phase");
            self.started.notify_one();
            wait.await.expect("test explicitly releases the phase");
            self.control.validate_permit(&self.permit, || Ok(()))
        })
    }

    fn maintain(&self) -> SupervisorFuture<'_, ()> {
        Box::pin(async move {
            self.maintenance
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        })
    }

    fn cancel_fenced<'a>(&'a self, operations: &'a [Uuid]) -> SupervisorFuture<'a, ()> {
        Box::pin(async move {
            assert_eq!(operations, &[self.permit.operation_id]);
            assert!(
                self.control
                    .snapshot()?
                    .fenced_operations
                    .contains(&self.permit.operation_id)
            );
            self.cancellations
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_driver_keeps_observing_and_joins_held_progression_across_pause_and_force_stop() {
    use crate::{
        corrections::{CorrectionDependencies, CorrectionPublisher, wire_fixture as wire},
        reconcile::{DependencyOutcomes, GatePurpose},
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    let (_root, built, _delivery, provider, owner) =
        crate::runtime_delivery::tests::fixture_authorization_with_wire(true).await;
    // Use a real configured cadence. Notifications below await actual reads;
    // no observation timestamps or task slots are injected by the fixture.
    built.configuration.config.lock().unwrap().runtime.poll_ms = 100;
    let initial = built.runtime.observations.refresh().await.unwrap();
    let gate = initial
        .parent(wire::id(2), wire::id(100))
        .unwrap()
        .fresh_gate(
            Some(wire::id(101)),
            GatePurpose::NewOperation,
            &DependencyOutcomes::default(),
            Utc::now(),
            chrono::Duration::seconds(30),
        )
        .unwrap();
    let _active = built
        .runtime
        .observations
        .capture_active(
            gate.workspace_id(),
            gate.config_revision(),
            gate.workflow_digest(),
        )
        .unwrap();
    let (lease, _owner_lock) = owner.start(&built.runtime);
    let permit = built
        .runtime
        .control
        .admit(
            &lease,
            Uuid::new_v4(),
            gate.work_key().unwrap(),
            canonical_digest(1, &"held driver composition phase").unwrap(),
            || {
                built
                    .runtime
                    .observations
                    .active_gate(&gate, &DependencyOutcomes::default())?;
                Ok(())
            },
        )
        .unwrap();
    let original_receipt = canonical_digest(
        1,
        &built
            .runtime
            .control
            .operation(permit.operation_id)
            .unwrap(),
    )
    .unwrap();
    let (release, wait) = oneshot::channel();
    let progression = Arc::new(HeldProgression {
        control: built.runtime.control.clone(),
        permit: permit.clone(),
        release: Mutex::new(Some(wait)),
        started: Notify::new(),
        advances: AtomicUsize::new(0),
        maintenance: AtomicUsize::new(0),
        cancellations: AtomicUsize::new(0),
    });
    let current_owner = built.runtime.registration.clone();
    let control_owner = current_owner.clone();
    let corrections = Arc::new(CorrectionPublisher::new(CorrectionDependencies {
        configuration: built.runtime.configuration.clone(),
        control: built.runtime.control.clone(),
        observations: built.runtime.observations.clone(),
        secrets: built.runtime.services.secrets.clone(),
        key: built.runtime.services.key.clone(),
        inputs: built.runtime.services.inputs.clone(),
        owner_check: Arc::new(move |lease| current_owner.verify_active_owner(lease)),
        control_owner_check: Arc::new(move || control_owner.verify_control_owner()),
    }));
    let driver = RuntimeDriver::new(
        RuntimeDriverServices {
            runtime: built.runtime.clone(),
            delivery: progression.clone(),
            corrections,
            processes: built.processes.clone(),
        },
        DriverLimits {
            worker_seconds: 60,
            authorization_tasks: 1,
        },
    )
    .unwrap();

    // The component deadline includes a complete production-bounded observation
    // read; it does not shorten that read's independent 30-second limit.
    let mut observation_diagnostic = Value::Null;
    tokio::time::timeout(Duration::from_secs(60), async {
        driver.poll(&lease, 1).await.unwrap();
        progression.started.notified().await;
        for sequence in 2..5 {
            driver.poll(&lease, sequence).await.unwrap();
            let snapshot = built.runtime.control.snapshot().unwrap();
            assert!(
                !driver
                    .maintain(&lease, &snapshot)
                    .await
                    .unwrap()
                    .local_clear
            );
        }
        assert_eq!(progression.advances.load(Ordering::SeqCst), 1);
        assert_eq!(progression.maintenance.load(Ordering::SeqCst), 0);
        assert!(driver.progression_status().unwrap().pending);
        assert_eq!(driver.progression_status().unwrap().joined, 0);

        let title = "Provider changed while the actual driver phase is held";
        let request_count = {
            let mut native = provider.lock().unwrap();
            native.set_parent_title(title);
            native.requests.len()
        };
        loop {
            let wake = built.runtime.services.wake.notified();
            let observed = built.runtime.observations.snapshot().unwrap();
            let workspace = &observed.workspaces[&wire::id(2)];
            let parent = observed.parent(wire::id(2), wire::id(100)).unwrap();
            if !workspace.complete || parent.issue.is_none() {
                // A change between complete native read passes may publish an
                // incomplete diagnostic projection. Preserve its actual error
                // without assuming which provider/read failed.
                observation_diagnostic = json!({
                    "workspace_complete": workspace.complete,
                    "workspace_error": workspace.error,
                    "parent_blockers": parent.blockers,
                    "health": built.runtime.observations.health().unwrap()
                });
                assert!(
                    parent
                        .fresh_gate(
                            Some(wire::id(101)),
                            GatePurpose::NewOperation,
                            &DependencyOutcomes::default(),
                            Utc::now(),
                            chrono::Duration::seconds(30),
                        )
                        .is_err(),
                    "incomplete observation supplied fresh authority: {observation_diagnostic}"
                );
            } else if parent
                .issue
                .as_ref()
                .is_some_and(|issue| issue.title == title)
            {
                break;
            }
            wake.await;
        }
        assert!(
            provider.lock().unwrap().requests[request_count..]
                .iter()
                .any(|(_, body)| body["query"].as_str().is_some_and(|query| {
                    query.contains("KegariParents") || query.contains("KegariRefresh")
                }))
        );
        assert!(
            built
                .runtime
                .observations
                .active_gate(&gate, &DependencyOutcomes::default())
                .is_err()
        );
        assert_eq!(
            canonical_digest(
                1,
                &built
                    .runtime
                    .control
                    .operation(permit.operation_id)
                    .unwrap()
            )
            .unwrap(),
            original_receipt
        );

        let control = &built.runtime.control;
        let generation = control.snapshot().unwrap().intent.generation;
        control
            .lifecycle(command(control), Lifecycle::Pause, generation)
            .unwrap();
        let paused = control.snapshot().unwrap();
        assert!(!driver.maintain(&lease, &paused).await.unwrap().local_clear);
        assert_eq!(progression.advances.load(Ordering::SeqCst), 1);
        assert_eq!(progression.maintenance.load(Ordering::SeqCst), 0);
        assert!(driver.progression_status().unwrap().pending);

        control
            .lifecycle(
                command(control),
                Lifecycle::ForceStop,
                paused.intent.generation,
            )
            .unwrap();
        driver.cancel_fenced(&[permit.operation_id]).await.unwrap();
        assert_eq!(progression.cancellations.load(Ordering::SeqCst), 1);
        assert!(!driver.exited(&lease).await.unwrap().local_clear);
        assert_eq!(driver.progression_status().unwrap().joined, 0);
        release.send(()).unwrap();
        // The normal entrypoint must actually join the failed phase. It must
        // neither drop it on lifecycle changes nor infer retirement from it.
        loop {
            let wake = built.runtime.services.wake.notified();
            match driver.exited(&lease).await {
                Err(error) => {
                    assert_eq!(driver.progression_status().unwrap().joined, 1);
                    assert_eq!(
                        driver.progression_status().unwrap().error.unwrap().code,
                        error.code
                    );
                    break;
                }
                Ok(drain) => assert!(!drain.local_clear),
            }
            wake.await;
        }
        assert!(!driver.progression_status().unwrap().pending);
        assert!(
            control
                .snapshot()
                .unwrap()
                .active_operations
                .iter()
                .any(|actual| actual.operation_id == permit.operation_id)
        );
        assert!(
            built
                .runtime
                .executions
                .active_operations()
                .unwrap()
                .is_empty()
        );
        control
            .retire_local(
                permit.operation_id,
                json!({"component_phase":"joined","no_process_or_external_effect_started":true}),
            )
            .unwrap();
        loop {
            let wake = built.runtime.services.wake.notified();
            if driver.exited(&lease).await.unwrap().local_clear {
                break;
            }
            wake.await;
        }
        assert_eq!(progression.maintenance.load(Ordering::SeqCst), 1);
        assert_eq!(progression.advances.load(Ordering::SeqCst), 1);
        assert_eq!(driver.progression_status().unwrap().joined, 2);
        for _ in 0..3 {
            assert!(driver.exited(&lease).await.unwrap().local_clear);
        }
        assert_eq!(progression.maintenance.load(Ordering::SeqCst), 1);
        assert!(!driver.observations.pending().unwrap());
        assert!(driver.state().unwrap().tasks.is_empty());
        assert_eq!(
            control
                .operations()
                .unwrap()
                .iter()
                .filter(|receipt| receipt.kind == "admission")
                .count(),
            1,
            "no autonomous worker or replacement phase was admitted"
        );
    })
    .await
    .unwrap_or_else(|error| {
        panic!(
            "actual driver entrypoints and owned tasks did not reach their explicit barriers: \
             {error}; last incomplete observation: {observation_diagnostic}"
        )
    });
}
