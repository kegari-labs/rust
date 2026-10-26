//! Aggregate task/slot components and descriptive record predicates. The
//! cancellation handles below are explicit injected components, not native
//! process, worker, confinement or repair capabilities.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::{Notify, oneshot};

fn plan() -> CheckBundlePlan {
    CheckBundlePlan::new(
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        crate::digest::bytes_digest(b"component binding"),
        crate::digest::bytes_digest(b"component isolation"),
        vec![
            CheckBundleStepSpec {
                process_id: Uuid::new_v4(),
                kind: CheckMemberKind::Service,
                service: Some("fixture".into()),
                launch_digest: crate::digest::bytes_digest(b"service"),
            },
            CheckBundleStepSpec {
                process_id: Uuid::new_v4(),
                kind: CheckMemberKind::Readiness,
                service: Some("fixture".into()),
                launch_digest: crate::digest::bytes_digest(b"readiness"),
            },
            CheckBundleStepSpec {
                process_id: Uuid::new_v4(),
                kind: CheckMemberKind::Check,
                service: None,
                launch_digest: crate::digest::bytes_digest(b"check"),
            },
        ],
    )
    .unwrap()
}
fn outcome(code: i32) -> CommandOutcome {
    CommandOutcome {
        schema_version: 1,
        argv: vec!["component".into()],
        cwd: "/component".into(),
        isolation_digest: crate::digest::bytes_digest(b"component isolation"),
        exit_code: Some(code),
        termination_signal: None,
        timed_out: false,
        output_truncated: false,
        stdout: String::new(),
        stderr: String::new(),
        elapsed_ms: 0,
    }
}
struct ComponentOwner {
    stop: AtomicBool,
    exited: AtomicBool,
    calls: AtomicUsize,
    notification: Notify,
    cancel_error: bool,
}
impl CommandCancellation for ComponentOwner {
    fn cancel(&self) -> Result<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.stop.store(true, Ordering::SeqCst);
        self.notification.notify_one();
        if self.cancel_error {
            Err(Error::new("component_stop", "injected stop error"))
        } else {
            Ok(())
        }
    }
    fn exited(&self) -> Result<bool> {
        Ok(self.exited.load(Ordering::SeqCst))
    }
}
fn attach_component(owner: &Arc<CheckBundleOwner>, cancel_error: bool) -> Arc<ComponentOwner> {
    let component = Arc::new(ComponentOwner {
        stop: AtomicBool::new(false),
        exited: AtomicBool::new(false),
        calls: AtomicUsize::new(0),
        notification: Notify::new(),
        cancel_error,
    });
    let retained = component.clone();
    let task = tokio::spawn(async move {
        loop {
            let notified = retained.notification.notified();
            if retained.stop.load(Ordering::SeqCst) {
                break;
            }
            notified.await;
        }
        retained.exited.store(true, Ordering::SeqCst);
        Ok(outcome(0))
    });
    let mut state = owner.lock().unwrap();
    let ordinal = state.attempted;
    state.attempted += 1;
    state.slots[ordinal].cancellation = Some(component.clone());
    state.slots[ordinal].waiter = Some(task);
    component
}

#[tokio::test]
async fn known_partial_prefix_requires_every_join_and_exact_plan() {
    let owner = CheckBundleOwner::new(plan(), Duration::from_secs(10), 32);
    let first = attach_component(&owner, false);
    let second = attach_component(&owner, false);
    assert_eq!(owner.attempted_prefix(), 2);
    assert_eq!(owner.verify_live_prefix(&owner.plan).unwrap(), 2);
    assert!(owner.verify_live_prefix(&plan()).is_err());
    assert!(!owner.exited().unwrap());
    owner.cleanup().await.unwrap();
    owner.verify_settled(&owner.plan).unwrap();
    assert!(first.exited().unwrap() && second.exited().unwrap());
    assert!(owner.verify_settled(&plan()).is_err());
    let records = owner.records().unwrap();
    assert_eq!(records.len(), 2);
    assert!(
        records
            .iter()
            .all(|record| record.outcome.is_some() && record.error.is_none())
    );
    assert!(owner.lock().unwrap().slots[2].cancellation.is_none());
}

#[tokio::test]
async fn one_stop_error_still_cancels_and_joins_every_retained_member() {
    let owner = CheckBundleOwner::new(plan(), Duration::from_secs(10), 32);
    let first = attach_component(&owner, false);
    let last = attach_component(&owner, true);
    assert_eq!(owner.cleanup().await.unwrap_err().code, "component_stop");
    assert!(first.calls.load(Ordering::SeqCst) > 0);
    assert!(last.calls.load(Ordering::SeqCst) > 0);
    assert!(owner.exited().unwrap());
    // Known joined closure does not erase the original cancellation error.
    assert_eq!(owner.records().unwrap().len(), 2);
}

#[tokio::test]
async fn timed_out_borrow_returns_original_waiter_for_owned_cleanup() {
    let owner = CheckBundleOwner::new(plan(), Duration::from_secs(10), 32);
    let component = attach_component(&owner, false);
    assert_eq!(
        owner.wait(0, Instant::now()).await.unwrap_err().code,
        "timeout"
    );
    assert!(owner.lock().unwrap().slots[0].waiter.is_some());
    assert!(!component.exited().unwrap());
    owner.cleanup().await.unwrap();
    assert!(owner.exited().unwrap());
}

#[tokio::test]
async fn dropping_aggregate_future_retains_cleanup_and_original_deadline() {
    let owner = CheckBundleOwner::new(plan(), Duration::from_secs(10), 32);
    let component = attach_component(&owner, false);
    let retained = owner.clone();
    let (started, ready) = oneshot::channel();
    let task = tokio::spawn(async move {
        let _cleanup = CleanupOnDrop {
            owner: retained,
            armed: true,
        };
        started.send(()).unwrap();
        std::future::pending::<()>().await;
    });
    ready.await.unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    tokio::time::timeout(Duration::from_secs(2), async {
        while !owner.exited().unwrap() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(component.exited().unwrap());
    let original = *owner.cleanup_deadline.get().unwrap();
    owner.cleanup().await.unwrap();
    assert_eq!(*owner.cleanup_deadline.get().unwrap(), original);
}

#[tokio::test]
async fn attempted_start_without_actual_handle_never_becomes_settled() {
    let owner = CheckBundleOwner::new(plan(), Duration::from_secs(10), 32);
    owner.lock().unwrap().attempted = 1;
    owner.uncertain.store(true, Ordering::SeqCst);
    assert_eq!(
        owner.cleanup().await.unwrap_err().code,
        "operation_uncertain"
    );
    assert!(!owner.exited().unwrap());
    assert!(owner.verify_live_prefix(&owner.plan).is_err());
    assert!(owner.verify_settled(&owner.plan).is_err());
    let records = owner.records().unwrap();
    assert_eq!(records.len(), 1);
    assert!(records[0].outcome.is_none());
}

#[test]
fn main_check_failure_eligibility_requires_clean_service_and_readiness_records() {
    let record = CheckBundleExecutionRecord {
        schema_version: 1,
        effect_id: Uuid::new_v4(),
        plan_digest: crate::digest::bytes_digest(b"plan"),
        services: vec![ServiceExecutionRecord {
            name: "fixture".into(),
            service_process: Uuid::new_v4(),
            readiness_process: Uuid::new_v4(),
            readiness: outcome(0),
            terminal: outcome(0),
            stopped: isolation::ServiceObservation {
                leader_running: false,
                stop_requested: true,
                kill_escalated: false,
            },
        }],
    };
    assert!(record.services_completed_cleanly());
    for change in 0..6 {
        let mut failed = record.clone();
        let service = &mut failed.services[0];
        match change {
            0 => service.readiness.exit_code = Some(1),
            1 => service.terminal.termination_signal = Some(libc::SIGKILL),
            2 => service.terminal.timed_out = true,
            3 => service.readiness.output_truncated = true,
            4 => service.stopped.kill_escalated = true,
            5 => service.stopped.leader_running = true,
            _ => unreachable!(),
        }
        assert!(!failed.services_completed_cleanly());
    }
    // This predicate is descriptive and cannot construct a FailedCheckExecution.
    // Actual main-check failure, confinement and unchanged source remain separate.
}

#[tokio::test]
async fn warm_empty_bundle_uses_original_control_and_registry_owner_cold_records_do_not() {
    for attempt_missing in [false, true] {
        use crate::control::{
            Actor, ControlHost, Lease, Lifecycle, MutationRequest, ReceiptPhase, ReceiptStore,
            RunAdmission, RunReference,
        };
        use crate::storage::PrivateDir;
        use crate::worker::ExecutionRegistry;
        use serde_json::json;
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().canonicalize().unwrap().join("control");
        let runtime = Uuid::new_v4();
        let host = ControlHost::setup(
            PrivateDir::create(&directory).unwrap(),
            runtime,
            Uuid::new_v4(),
        )
        .unwrap();
        host.lifecycle(
            MutationRequest {
                request_id: Uuid::new_v4(),
                runtime_id: runtime,
                actor: Actor {
                    identity: "component operator".into(),
                    roles: BTreeSet::from(["lifecycle".into()]),
                    principal: None,
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
            owner_id: Uuid::new_v4(),
            owner_generation: 1,
        };
        host.attach_reconciled(lease.clone()).unwrap();
        let work = format!("component-parent:{}", Uuid::new_v4());
        let permit = host
            .admit_run(
                &lease,
                RunAdmission {
                    request_id: Uuid::new_v4(),
                    work_key: work.clone(),
                    scope_digest: crate::digest::bytes_digest(b"scope"),
                    reference: RunReference {
                        workspace: Uuid::new_v4(),
                        account: "component-account".into(),
                        normalized_state: None,
                        phase: crate::scheduler::PhaseKey {
                            work,
                            generation: crate::digest::bytes_digest(b"generation"),
                            phase: "parent_evidence_review".into(),
                            retry_grant: None,
                        },
                        profile_digest: crate::digest::bytes_digest(b"profile"),
                        instruction_digest: crate::digest::bytes_digest(b"instruction"),
                        max_attempts: 1,
                        max_turns: 1,
                        max_review_rounds: 1,
                        attempt: 1,
                    },
                },
                || Ok(()),
            )
            .unwrap();
        let effect = Uuid::new_v4();
        let template = plan();
        let actual_plan = CheckBundlePlan::new(
            effect,
            permit.operation_id,
            runtime,
            crate::digest::bytes_digest(b"component binding"),
            template.isolation_digest().into(),
            template
                .steps()
                .iter()
                .map(|step| CheckBundleStepSpec {
                    process_id: step.process_id(),
                    kind: step.kind().clone(),
                    service: step.service().map(str::to_owned),
                    launch_digest: step.launch_digest().into(),
                })
                .collect(),
        )
        .unwrap();
        // This is the actual production empty aggregate initializer. The component
        // has no prepared source, child, native admission or confinement evidence.
        let owner = CheckBundleOwner::new(actual_plan.clone(), Duration::from_secs(30), 32);
        let registry = ExecutionRegistry::default();
        let (mut bundle, ()) = host
            .begin_check_bundle(
                MutationRequest {
                    request_id: effect,
                    runtime_id: runtime,
                    actor: Actor {
                        identity: format!("owner:{}", lease.owner_id),
                        roles: BTreeSet::new(),
                        principal: None,
                    },
                    kind: "worker_validation".into(),
                    canonical_targets: vec!["component-repository".into()],
                    expected: BTreeMap::new(),
                    payload: json!({"name":"fixture","kind":"integrated"}),
                },
                &permit,
                actual_plan,
                || Ok(()),
                |receipt| {
                    registry
                        .reserve(permit.operation_id, receipt.operation_id)?
                        .attach(owner.clone())
                },
            )
            .unwrap();
        let receipts = ReceiptStore::new(PrivateDir::open(&directory.join("receipts")).unwrap());
        host.serialize_observation(|_| {
            let receipt = receipts.get(effect)?;
            assert_eq!(receipt.phase, ReceiptPhase::Sent);
            assert!(
                host.verify_check_bundle_inventory(std::slice::from_ref(&receipt), &[])
                    .is_err()
            );
            host.verify_live_check_bundle_inventory(
                std::slice::from_ref(&receipt),
                &[],
                |operation, effect| registry.retained_check_bundle(operation, effect),
            )?;
            let unrelated: Arc<dyn CommandCancellation> =
                CheckBundleOwner::new(plan(), Duration::from_secs(30), 32);
            assert!(
                host.verify_live_check_bundle_inventory(&[receipt], &[], |_, _| Ok(Some(
                    unrelated.clone()
                )))
                .is_err()
            );
            assert!(
                registry
                    .retained_check_bundle(Uuid::new_v4(), effect)?
                    .is_none()
            );
            assert!(
                registry
                    .retained_check_bundle(permit.operation_id, Uuid::new_v4())?
                    .is_none()
            );
            Ok(())
        })
        .unwrap();
        let wrong_effect = Uuid::new_v4();
        registry
            .reserve(permit.operation_id, wrong_effect)
            .unwrap()
            .attach(owner.clone())
            .unwrap();
        assert_eq!(
            registry
                .retained_check_bundle(permit.operation_id, wrong_effect)
                .err()
                .unwrap()
                .code,
            "ownership_conflict"
        );
        registry.remove(permit.operation_id, wrong_effect).unwrap();
        if !attempt_missing {
            let original = registry
                .retained_check_bundle(permit.operation_id, effect)
                .unwrap()
                .unwrap();
            owner.cleanup().await.unwrap();
            assert!(owner.exited().unwrap());
            assert_eq!(registry.observe_exited(permit.operation_id).unwrap(), 0);
            let still_retained = registry
                .retained_check_bundle(permit.operation_id, effect)
                .unwrap()
                .unwrap();
            assert!(Arc::ptr_eq(&original, &still_retained));
            let pending = host.operation(effect).unwrap();
            assert_eq!(pending.phase, ReceiptPhase::Sent);
            host.serialize_observation(|_| {
                host.verify_live_check_bundle_inventory(
                    std::slice::from_ref(&pending),
                    &[],
                    |operation, effect| registry.retained_check_bundle(operation, effect),
                )?;
                assert_eq!(
                    registry.observe_settled_check_bundles(&host, &host.operations()?)?,
                    0
                );
                Ok(())
            })
            .unwrap();
            host.complete_check_bundle_failure(
                bundle,
                &permit,
                &owner,
                || Ok(()),
                &Error::new(
                    "component_no_launch",
                    "known empty aggregate closed before any child",
                ),
            )
            .unwrap();
            host.serialize_observation(|_| {
                // A previously captured pending receipt cannot trigger pruning.
                assert_eq!(
                    registry.observe_settled_check_bundles(&host, &[pending])?,
                    0
                );
                let receipt = host.operation(effect)?;
                assert_eq!(receipt.phase, ReceiptPhase::Acknowledged);
                host.verify_check_bundle_inventory(std::slice::from_ref(&receipt), &[])?;
                let unrelated = CheckBundleOwner::new(plan(), Duration::from_secs(30), 32);
                assert!(
                    host.check_bundle_owner_closed(&receipt, &unrelated)
                        .is_err()
                );
                assert_eq!(
                    registry.observe_settled_check_bundles(&host, &host.operations()?)?,
                    1
                );
                assert!(
                    registry
                        .retained_check_bundle(permit.operation_id, effect)?
                        .is_none()
                );
                assert_eq!(
                    registry.observe_settled_check_bundles(&host, &host.operations()?)?,
                    0
                );
                Ok(())
            })
            .unwrap();
            continue;
        }
        // A real durable dispatch attempt which returns before any native launch
        // cannot be hidden by the still-empty retained owner or inferred process row.
        assert_eq!(
            host.advance_check_bundle(
                &mut bundle,
                &permit,
                0,
                || Ok(()),
                |_| {
                    Err::<(), _>(Error::new(
                        "component_prelaunch",
                        "fixture stops before creating any child",
                    ))
                }
            )
            .unwrap_err()
            .code,
            "component_prelaunch"
        );
        host.serialize_observation(|_| {
            let receipt = receipts.get(effect)?;
            assert!(
                host.verify_live_check_bundle_inventory(&[receipt], &[], |operation, effect| {
                    registry.retained_check_bundle(operation, effect)
                })
                .is_err()
            );
            Ok(())
        })
        .unwrap();
        assert_eq!(owner.attempted_prefix(), 0);
    }
}
