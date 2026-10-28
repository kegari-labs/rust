use kegari_digest::canonical_digest;
use kegari_storage::PrivateDir;
use kegari_types::error::Error;
use kegari_v0::control::*;
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    os::unix::fs::PermissionsExt,
    sync::Arc,
};
use tempfile::TempDir;
use uuid::Uuid;

fn directory() -> (TempDir, PrivateDir) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap().join("state");
    let directory = PrivateDir::create(&root).unwrap();
    (temp, directory)
}
fn actor() -> Actor {
    Actor {
        identity: "os:501".into(),
        principal: None,
        roles: BTreeSet::from(["lifecycle".into()]),
    }
}
fn command(runtime: Uuid, id: Uuid) -> MutationRequest {
    MutationRequest {
        request_id: id,
        runtime_id: runtime,
        actor: actor(),
        kind: "lifecycle".into(),
        canonical_targets: vec![runtime.to_string()],
        expected: BTreeMap::new(),
        payload: json!(null),
    }
}

#[test]
fn pending_protocol_allows_interleaved_tools_but_exit_retains_unknown_work_hold() {
    let (_temp, host, lease) = started();
    let permit = host
        .admit(
            &lease,
            Uuid::new_v4(),
            "work".into(),
            canonical_digest(1, &"scope").unwrap(),
            || Ok(()),
        )
        .unwrap();
    let mut request = command(lease.runtime_id, Uuid::new_v4());
    request.kind = "codex/turn/start".into();
    request.canonical_targets = vec![format!("codex-session:{}", permit.operation_id)];
    let pending = request.request_id;
    let (_ticket, ()) = host
        .begin_protocol_effect(request.clone(), &permit, || Ok(()), |_| Ok(()))
        .unwrap();
    let mut next = request.clone();
    next.request_id = Uuid::new_v4();
    assert_eq!(
        host.begin_protocol_effect(next, &permit, || Ok(()), |_| Ok(()))
            .err()
            .unwrap()
            .code,
        "reconciliation_required"
    );
    let mut tool = command(lease.runtime_id, Uuid::new_v4());
    tool.kind = "local_check".into();
    tool.canonical_targets = vec!["local-check".into()];
    let (ticket, ()) = host
        .begin_effect(tool, &permit, || Ok(()), |_| Ok(()))
        .unwrap();
    host.complete_effect(
        ticket,
        &permit,
        || Ok(()),
        json!({"observed":"actual_fixture_exit"}),
    )
    .unwrap();
    let retired = host
        .retire_local(
            permit.operation_id,
            json!({"owned_task":"joined","outcome":"interrupted"}),
        )
        .unwrap();
    assert!(host.snapshot().unwrap().active_operations.is_empty());
    assert_eq!(
        retired.observed_result.as_ref().unwrap()["unresolved_effects"],
        json!([pending])
    );
    assert!(host.snapshot().unwrap().unresolved_operations[&pending].contains(&"work".to_string()));
    assert_eq!(
        host.admit(
            &lease,
            Uuid::new_v4(),
            "work".into(),
            canonical_digest(1, &"scope").unwrap(),
            || Ok(())
        )
        .unwrap_err()
        .code,
        "reconciliation_required"
    );
    let mut invalid = request;
    invalid.request_id = Uuid::new_v4();
    invalid.kind = "linear_comment".into();
    assert_eq!(
        host.begin_protocol_effect(invalid, &permit, || Ok(()), |_| Ok(()))
            .err()
            .unwrap()
            .code,
        "scope_violation"
    );
}

#[test]
fn local_exit_does_not_clear_unknown_remote_effect_or_block_graceful_drain() {
    let (temp, host, lease) = started();
    let permit = host
        .admit(
            &lease,
            Uuid::new_v4(),
            "remote-work".into(),
            canonical_digest(1, &"scope").unwrap(),
            || Ok(()),
        )
        .unwrap();
    let mut effect = command(lease.runtime_id, Uuid::new_v4());
    effect.kind = "linear_comment".into();
    effect.canonical_targets = vec!["remote-target".into()];
    let unknown = effect.request_id;
    assert_eq!(
        host.mutate(
            effect,
            Some(&permit),
            None,
            || Ok(()),
            |_| Err(Error::new("timeout", "fixture lost acknowledgement"))
        )
        .unwrap_err()
        .code,
        "operation_uncertain"
    );
    let generation = host.snapshot().unwrap().intent.generation;
    host.lifecycle(
        command(lease.runtime_id, Uuid::new_v4()),
        Lifecycle::Stop,
        generation,
    )
    .unwrap();
    assert!(
        host.finish(permit.operation_id, json!({"succeeded":true}))
            .is_err()
    );
    host.retire_local(permit.operation_id, json!({"broker_task":"joined"}))
        .unwrap();
    assert!(host.snapshot().unwrap().active_operations.is_empty());
    assert!(
        host.snapshot()
            .unwrap()
            .unresolved_operations
            .contains_key(&unknown)
    );
    host.orchestration_exited(&lease).unwrap();
    assert!(!host.snapshot().unwrap().draining);
    assert_eq!(host.snapshot().unwrap().intent.desired, Intent::Stopped);
    // An unresolved remote result must not be mistaken for an unresolved local
    // process after its local admission has retired. Recovery retains the
    // remote receipt while permitting the already-drained service to stop.
    let unresolved = canonical_digest(1, &host.operation(unknown).unwrap()).unwrap();
    drop(host);
    let root = PrivateDir::open(&temp.path().canonicalize().unwrap().join("state")).unwrap();
    let recovered =
        ControlHost::recover(PrivateDir::open(root.path()).unwrap(), lease.runtime_id).unwrap();
    recovered
        .stop_service(command(lease.runtime_id, Uuid::new_v4()), || {
            root.write_bytes("fixture-service-shutdown", b"reserved")
        })
        .unwrap();
    assert_eq!(
        root.read_bytes("fixture-service-shutdown", 64).unwrap(),
        b"reserved"
    );
    assert!(recovered.snapshot().unwrap().service_stopping);
    assert_eq!(
        canonical_digest(1, &recovered.operation(unknown).unwrap()).unwrap(),
        unresolved
    );
}

#[test]
fn recovered_local_ownership_blocks_service_shutdown_without_live_permits() {
    for kind in ["admission", "owner_suspension"] {
        let (temp, host, lease) = started();
        let operation = if kind == "admission" {
            host.admit(
                &lease,
                Uuid::new_v4(),
                "unfinished-local-work".into(),
                canonical_digest(1, &"scope").unwrap(),
                || Ok(()),
            )
            .unwrap()
            .operation_id
        } else {
            host.suspend_owner(&lease, "fixture_observation_lost")
                .unwrap()
                .operation_id
        };
        host.lifecycle(
            command(lease.runtime_id, Uuid::new_v4()),
            Lifecycle::Stop,
            host.snapshot().unwrap().intent.generation,
        )
        .unwrap();
        let unresolved = canonical_digest(1, &host.operation(operation).unwrap()).unwrap();
        drop(host);
        let root = PrivateDir::open(&temp.path().canonicalize().unwrap().join("state")).unwrap();
        let recovered =
            ControlHost::recover(PrivateDir::open(root.path()).unwrap(), lease.runtime_id).unwrap();
        let before = recovered.snapshot().unwrap();
        assert_eq!(before.intent.desired, Intent::Stopped);
        assert!(before.active_operations.is_empty());
        assert!(before.pending_local_processes.is_empty());
        assert!(!before.orchestration_alive);
        assert!(before.unresolved_operations.contains_key(&operation));
        let request = command(lease.runtime_id, Uuid::new_v4());
        let error = recovered
            .stop_service(request.clone(), || {
                root.write_bytes("fixture-service-shutdown", b"reserved")
            })
            .unwrap_err();
        assert_eq!(error.code, "stop_in_progress", "{kind}");
        assert!(!root.path().join("fixture-service-shutdown").exists());
        assert!(!recovered.snapshot().unwrap().service_stopping);
        assert_eq!(
            canonical_digest(1, &recovered.operation(operation).unwrap()).unwrap(),
            unresolved
        );
        assert_eq!(
            recovered
                .stop_service(request, || panic!("rejected shutdown must not replay"))
                .unwrap_err()
                .code,
            "stop_in_progress"
        );
    }
}

#[test]
fn unfinished_service_shutdown_replay_retains_uncertainty_without_reserving() {
    for sent in [false, true] {
        let (_temp, root) = directory();
        let runtime = Uuid::new_v4();
        let host = ControlHost::setup(
            PrivateDir::open(root.path()).unwrap(),
            runtime,
            Uuid::new_v4(),
        )
        .unwrap();
        let mut request = command(runtime, Uuid::new_v4());
        request.kind = "service_stop".into();
        request.payload = json!({"service":"stop"});
        let store = ReceiptStore::new(root.child("receipts").unwrap());
        let (mut receipt, _) = store.prepare(&request, 0).unwrap();
        if sent {
            store.mark_sent(&mut receipt).unwrap();
        }
        let before = canonical_digest(1, &receipt).unwrap();
        drop(host);
        let recovered =
            ControlHost::recover(PrivateDir::open(root.path()).unwrap(), runtime).unwrap();
        assert_eq!(
            recovered
                .stop_service(request, || panic!("unfinished shutdown must not replay"))
                .unwrap_err()
                .code,
            "operation_uncertain"
        );
        assert_eq!(
            canonical_digest(1, &store.get(receipt.request_id).unwrap()).unwrap(),
            before
        );
        assert!(!recovered.snapshot().unwrap().service_stopping);
    }
}

#[test]
fn failed_intent_persistence_blocks_service_reservation_until_recovery() {
    let (_temp, root) = directory();
    let runtime = Uuid::new_v4();
    let host = ControlHost::setup(
        PrivateDir::open(root.path()).unwrap(),
        runtime,
        Uuid::new_v4(),
    )
    .unwrap();
    let retained_intent = root.read_bytes("intent.json", 65536).unwrap();
    let intent_path = root.path().join("intent.json");
    fs::remove_file(&intent_path).unwrap();
    fs::create_dir(&intent_path).unwrap();
    assert!(
        host.lifecycle(command(runtime, Uuid::new_v4()), Lifecycle::Start, 0)
            .is_err()
    );
    assert_eq!(host.snapshot().unwrap().intent.desired, Intent::Stopped);
    assert!(host.snapshot().unwrap().persistence_error.is_some());
    // Restore the fixture's original durable state. The live host must still
    // honor its persistence-error latch until the normal recovery path reads it.
    fs::remove_dir(&intent_path).unwrap();
    root.write_bytes("intent.json", &retained_intent).unwrap();
    assert_eq!(
        host.stop_service(command(runtime, Uuid::new_v4()), || {
            root.write_bytes("fixture-service-shutdown", b"reserved")
        })
        .unwrap_err()
        .code,
        "reconciliation_required"
    );
    assert!(!root.path().join("fixture-service-shutdown").exists());
    assert!(!host.snapshot().unwrap().service_stopping);
    drop(host);
    let recovered = ControlHost::recover(PrivateDir::open(root.path()).unwrap(), runtime).unwrap();
    recovered
        .stop_service(command(runtime, Uuid::new_v4()), || {
            root.write_bytes("fixture-service-shutdown", b"reserved")
        })
        .unwrap();
    assert_eq!(
        root.read_bytes("fixture-service-shutdown", 64).unwrap(),
        b"reserved"
    );
}

#[test]
fn service_shutdown_fence_serializes_lifecycle_and_preserves_failed_checks() {
    let (_temp, directory) = directory();
    let runtime = Uuid::new_v4();
    let host = ControlHost::setup(directory, runtime, Uuid::new_v4()).unwrap();
    let rejected = Uuid::new_v4();
    assert!(
        host.stop_service(command(runtime, rejected), || Err(Error::new(
            "ownership_conflict",
            "fixture prior process remains"
        )))
        .is_err()
    );
    assert!(!host.snapshot().unwrap().service_stopping);
    assert!(
        host.stop_service(command(runtime, rejected), || panic!(
            "rejected request must not replay"
        ))
        .is_err()
    );
    let accepted = Uuid::new_v4();
    host.stop_service(command(runtime, accepted), || Ok(()))
        .unwrap();
    assert!(host.snapshot().unwrap().service_stopping);
    assert_eq!(
        host.lifecycle(command(runtime, Uuid::new_v4()), Lifecycle::Start, 0)
            .unwrap_err()
            .code,
        "service_stopping"
    );
    host.stop_service(command(runtime, accepted), || {
        panic!("accepted reservation must not replay")
    })
    .unwrap();
    assert_eq!(host.snapshot().unwrap().intent.desired, Intent::Stopped);
}

#[test]
fn configuration_gate_observes_serialized_intent_without_reentering_host() {
    let (_temp, host, lease) = started();
    let mut request = command(lease.runtime_id, Uuid::new_v4());
    request.actor.roles.insert("configure".into());
    let error = host
        .mutate_observed(
            request,
            None,
            Some("configure"),
            |snapshot| {
                if snapshot.intent.desired == Intent::Running {
                    Err(Error::new(
                        "requires_pause",
                        "fixture account replacement requires pause",
                    ))
                } else {
                    Ok(())
                }
            },
            |_| panic!("running replacement must not write"),
        )
        .unwrap_err();
    assert_eq!(error.code, "requires_pause");
}

#[test]
fn in_flight_local_effect_releases_gate_and_force_rejects_its_success() {
    let (_temp, host, lease) = started();
    let permit = host
        .admit(
            &lease,
            Uuid::new_v4(),
            "fixture-child".into(),
            "fixture-scope".into(),
            || Ok(()),
        )
        .unwrap();
    let mut request = command(lease.runtime_id, Uuid::new_v4());
    request.kind = "local_check".into();
    request.canonical_targets = vec!["fixture-child".into()];
    let effect_id = request.request_id;
    let (effect, launched) = host
        .begin_effect(
            request.clone(),
            &permit,
            || Ok(()),
            |_| Ok("owned fixture process"),
        )
        .unwrap();
    assert_eq!(launched, "owned fixture process");
    assert_eq!(host.operation(effect_id).unwrap().phase, ReceiptPhase::Sent);
    let generation = host.snapshot().unwrap().intent.generation;
    host.lifecycle(
        command(lease.runtime_id, Uuid::new_v4()),
        Lifecycle::ForceStop,
        generation,
    )
    .unwrap();
    assert!(
        host.snapshot()
            .unwrap()
            .fenced_operations
            .contains(&permit.operation_id)
    );
    assert!(
        host.begin_effect::<()>(request, &permit, || Ok(()), |_| panic!("never relaunch"))
            .is_err()
    );
    assert_eq!(
        host.complete_effect(effect, &permit, || Ok(()), json!({"exit_code":0}))
            .unwrap_err()
            .code,
        "access_denied"
    );
    let receipt = host.operation(effect_id).unwrap();
    assert_eq!(receipt.phase, ReceiptPhase::Acknowledged);
    assert_eq!(
        receipt.observed_result.unwrap()["effect"],
        "exited_evidence_rejected"
    );
    assert!(
        !host
            .snapshot()
            .unwrap()
            .unresolved_operations
            .contains_key(&effect_id)
    );
}
fn started() -> (TempDir, Arc<ControlHost>, Lease) {
    let (temp, dir) = directory();
    let runtime = Uuid::new_v4();
    let host = Arc::new(ControlHost::setup(dir, runtime, Uuid::new_v4()).unwrap());
    assert_eq!(host.snapshot().unwrap().intent.desired, Intent::Stopped);
    host.lifecycle(command(runtime, Uuid::new_v4()), Lifecycle::Start, 0)
        .unwrap();
    assert!(!host.snapshot().unwrap().admission_open);
    let lease = Lease {
        runtime_id: runtime,
        owner_generation: 1,
        owner_id: Uuid::new_v4(),
    };
    host.attach_reconciled(lease.clone()).unwrap();
    (temp, host, lease)
}

#[test]
fn metadata_free_admission_binds_the_current_serialized_generation() {
    let (_temp, host, lease) = started();
    let old = host.snapshot().unwrap().intent.generation;
    host.lifecycle(
        command(lease.runtime_id, Uuid::new_v4()),
        Lifecycle::Pause,
        old,
    )
    .unwrap();
    host.lifecycle(
        command(lease.runtime_id, Uuid::new_v4()),
        Lifecycle::Resume,
        old + 1,
    )
    .unwrap();
    let rejected = Uuid::new_v4();
    let result = host.admit_observed(
        &lease,
        rejected,
        "release-parent".into(),
        canonical_digest(1, &"plan").unwrap(),
        |snapshot| {
            if snapshot.intent.generation != old {
                return Err(Error::new(
                    "stale_control",
                    "preflight generation is obsolete",
                ));
            }
            Ok(())
        },
    );
    assert_eq!(result.unwrap_err().code, "stale_control");
    assert_eq!(host.operation(rejected).unwrap_err().code, "record_missing");
    assert!(host.snapshot().unwrap().active_operations.is_empty());
    let mut observed = None;
    let permit = host
        .admit_observed(
            &lease,
            Uuid::new_v4(),
            "release-parent".into(),
            canonical_digest(1, &"plan").unwrap(),
            |snapshot| {
                observed = Some(snapshot.intent.generation);
                assert_eq!(snapshot.intent.desired, Intent::Running);
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(observed, Some(old + 2));
    assert_eq!(permit.control_generation, old + 2);
    assert_eq!(
        host.operation(permit.operation_id).unwrap().expected["control"],
        (old + 2).to_string()
    );
}

#[test]
fn metadata_budget_is_atomic_and_survives_restart_without_model_claims() {
    let (temp, host, lease) = started();
    let admission = |attempt, max_attempts| MetadataAdmission {
        request_id: Uuid::new_v4(),
        work_key: "canonical-parent".into(),
        scope_digest: canonical_digest(1, &"scope").unwrap(),
        reference: MetadataReference {
            purpose: "parent_authorization".into(),
            generation: canonical_digest(1, &"authorization-snapshot").unwrap(),
            attempt,
            max_attempts,
        },
    };
    let first = host
        .admit_metadata(&lease, admission(1, 2), || Ok(()))
        .unwrap();
    let receipt = host.operation(first.operation_id).unwrap();
    assert!(RunReference::from_receipt(&receipt).unwrap().is_none());
    assert_eq!(
        MetadataReference::from_receipt(&receipt)
            .unwrap()
            .unwrap()
            .max_attempts,
        2
    );
    host.retire_local(first.operation_id, json!({"known_no_effect":true}))
        .unwrap();
    assert_eq!(
        host.admit_metadata(&lease, admission(1, 2), || Ok(()))
            .unwrap_err()
            .code,
        "attempts_exhausted"
    );
    drop(host);
    let recovered = ControlHost::recover(
        PrivateDir::open(&temp.path().canonicalize().unwrap().join("state")).unwrap(),
        lease.runtime_id,
    )
    .unwrap();
    let renewed = Lease {
        owner_generation: 2,
        owner_id: Uuid::new_v4(),
        ..lease
    };
    recovered.attach_reconciled(renewed.clone()).unwrap();
    assert_eq!(
        recovered
            .admit_metadata(&renewed, admission(2, 99), || Ok(()))
            .unwrap_err()
            .code,
        "attempts_exhausted"
    );
    let second = recovered
        .admit_metadata(&renewed, admission(2, 2), || Ok(()))
        .unwrap();
    recovered
        .retire_local(second.operation_id, json!({"known_no_effect":true}))
        .unwrap();
    assert_eq!(
        recovered
            .admit_metadata(&renewed, admission(3, 3), || Ok(()))
            .unwrap_err()
            .code,
        "attempts_exhausted"
    );
    assert_eq!(
        recovered
            .operations()
            .unwrap()
            .iter()
            .filter(|receipt| MetadataReference::from_receipt(receipt).unwrap().is_some())
            .count(),
        2
    );
}

#[test]
fn v0_t08_corrupt_version_and_missing_intent_never_start() {
    let (_temp, dir) = directory();
    let runtime = Uuid::new_v4();
    let path = dir.path().to_path_buf();
    assert!(ControlHost::recover(dir, runtime).is_err());
    let dir = PrivateDir::open(&path).unwrap();
    dir.write("intent.json", &json!({"anything":1})).unwrap();
    assert!(ControlHost::recover(dir, runtime).is_err());
    let bytes = br#"{"schema_version":99,"digest":"x","payload":{"a":1}}"#;
    fs::write(path.join("test.json"), bytes).unwrap();
    fs::set_permissions(path.join("test.json"), fs::Permissions::from_mode(0o600)).unwrap();
    let dir = PrivateDir::open(&path).unwrap();
    assert_eq!(
        dir.read::<serde_json::Value>("test.json").unwrap_err().code,
        "incompatible_version"
    );
    assert_eq!(fs::read(path.join("test.json")).unwrap(), bytes);
}

#[test]
fn v0_t07_exclusive_control_owner_and_canonical_runtime() {
    let (_temp, dir) = directory();
    let path = dir.path().to_path_buf();
    let runtime = Uuid::new_v4();
    let first = ControlHost::setup(dir, runtime, Uuid::new_v4()).unwrap();
    assert!(ControlHost::recover(PrivateDir::open(&path).unwrap(), runtime).is_err());
    drop(first);
    assert!(ControlHost::recover(PrivateDir::open(&path).unwrap(), Uuid::new_v4()).is_err());
    assert_eq!(
        ControlHost::recover(PrivateDir::open(&path).unwrap(), runtime)
            .unwrap()
            .snapshot()
            .unwrap()
            .intent
            .desired,
        Intent::Stopped
    );
}

#[test]
fn v0_t31_pause_admission_race_and_stop_drain() {
    let (_temp, host, lease) = started();
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let h = host.clone();
    let l = lease.clone();
    let b = barrier.clone();
    let admission = std::thread::spawn(move || {
        b.wait();
        h.admit(
            &l,
            Uuid::new_v4(),
            "linear:workspace:child".into(),
            "scope".into(),
            || Ok(()),
        )
    });
    barrier.wait();
    host.lifecycle(
        command(lease.runtime_id, Uuid::new_v4()),
        Lifecycle::Pause,
        1,
    )
    .unwrap();
    let result = admission.join().unwrap();
    assert!(!host.snapshot().unwrap().admission_open);
    assert!(
        host.admit(
            &lease,
            Uuid::new_v4(),
            "another".into(),
            "scope".into(),
            || Ok(())
        )
        .is_err()
    );
    if let Ok(permit) = result {
        host.validate_permit(&permit, || Ok(())).unwrap();
        host.finish(permit.operation_id, json!({"outcome":"interrupted"}))
            .unwrap();
    }
    host.lifecycle(
        command(lease.runtime_id, Uuid::new_v4()),
        Lifecycle::Stop,
        2,
    )
    .unwrap();
    assert!(host.snapshot().unwrap().draining);
    assert_eq!(
        host.lifecycle(
            command(lease.runtime_id, Uuid::new_v4()),
            Lifecycle::Start,
            3
        )
        .unwrap_err()
        .code,
        "stop_in_progress"
    );
    host.orchestration_exited(&lease).unwrap();
    assert!(!host.snapshot().unwrap().draining);
}

#[test]
fn v0_t09_receipt_unknown_never_replays_and_identity_is_bound() {
    let (_temp, dir) = directory();
    let store = ReceiptStore::new(dir);
    let runtime = Uuid::new_v4();
    let request = command(runtime, Uuid::new_v4());
    let (mut receipt, fresh) = store.prepare(&request, 1).unwrap();
    assert!(fresh);
    store.mark_sent(&mut receipt).unwrap();
    let (replayed, fresh) = store.prepare(&request, 1).unwrap();
    assert!(!fresh);
    assert_eq!(replayed.phase, ReceiptPhase::Sent);
    assert!(
        store
            .reconcile(receipt.request_id, Reconciliation::NotVisible)
            .is_err()
    );
    let mut changed = request.clone();
    changed.actor.identity = "os:502".into();
    assert_eq!(
        store.prepare(&changed, 1).unwrap_err().code,
        "request_conflict"
    );
    assert!(
        store
            .reconcile(receipt.request_id, Reconciliation::Ambiguous)
            .is_err()
    );
    let reconciled = store
        .reconcile(
            receipt.request_id,
            Reconciliation::UniqueVerified {
                remote_ids: BTreeMap::from([("comment".into(), "provider-comment-1".into())]),
                result: json!({"observed":"published"}),
                payload_digest: receipt.payload_digest,
                actor: receipt.actor,
            },
        )
        .unwrap();
    assert_eq!(reconciled.phase, ReceiptPhase::Reconciled);
    assert_eq!(store.all().unwrap().len(), 1);
}

#[test]
fn recovery_rechecks_latest_serialized_state_and_preserves_rejected_unknown() {
    let (_temp, host, lease) = started();
    let permit = host
        .admit(
            &lease,
            Uuid::new_v4(),
            "parent-recovery".into(),
            canonical_digest(1, &"scope").unwrap(),
            || Ok(()),
        )
        .unwrap();
    let mut effect = command(lease.runtime_id, Uuid::new_v4());
    effect.kind = "linear_comment".into();
    effect.canonical_targets = vec!["parent-recovery".into()];
    let id = effect.request_id;
    assert_eq!(
        host.mutate(
            effect,
            Some(&permit),
            None,
            || Ok(()),
            |_| { Err(Error::new("timeout", "fixture lost acknowledgement")) }
        )
        .unwrap_err()
        .code,
        "operation_uncertain"
    );
    host.retire_local(permit.operation_id, json!({"owned_task":"joined"}))
        .unwrap();
    let before = host.snapshot().unwrap();
    let retained = host.operation(id).unwrap();
    host.lifecycle(
        command(lease.runtime_id, Uuid::new_v4()),
        Lifecycle::Stop,
        before.intent.generation,
    )
    .unwrap();
    let stopped = host.snapshot().unwrap();
    let mut recover = actor();
    recover.roles.insert("recover".into());
    let error = host
        .reconcile_operation_observed(&recover, id, |receipt, current| {
            assert_eq!(receipt.request_id, id);
            assert_eq!(current.intent.desired, stopped.intent.desired);
            assert_eq!(current.intent.generation, stopped.intent.generation);
            if current.intent.generation != before.intent.generation {
                return Err(Error::new(
                    "stale_generation",
                    "fixture observation is stale",
                ));
            }
            panic!("reconciliation must see the intervening lifecycle command")
        })
        .unwrap_err();
    assert_eq!(error.code, "stale_generation");
    assert_eq!(
        serde_json::to_value(host.operation(id).unwrap()).unwrap(),
        serde_json::to_value(&retained).unwrap()
    );
    assert!(
        host.snapshot()
            .unwrap()
            .unresolved_operations
            .contains_key(&id)
    );

    // A fresh read-only proof can resolve the same unknown while STOPPED.
    let reconciled = host
        .reconcile_operation_observed(&recover, id, |receipt, current| {
            assert_eq!(current.intent.desired, Intent::Stopped);
            assert_eq!(current.intent.generation, stopped.intent.generation);
            Ok(Reconciliation::UniqueVerified {
                remote_ids: BTreeMap::from([("comment".into(), "exact-native-record".into())]),
                result: json!({"observed":"published"}),
                payload_digest: receipt.payload_digest.clone(),
                actor: receipt.actor.clone(),
            })
        })
        .unwrap();
    assert_eq!(reconciled.phase, ReceiptPhase::Reconciled);
    assert!(
        !host
            .snapshot()
            .unwrap()
            .unresolved_operations
            .contains_key(&id)
    );
    assert_eq!(host.snapshot().unwrap().intent.desired, Intent::Stopped);
}

#[test]
fn v0_t23_force_fences_permits_and_restart_preserves_stop() {
    let (temp, host, lease) = started();
    let permit = host
        .admit(
            &lease,
            Uuid::new_v4(),
            "child".into(),
            "scope".into(),
            || Ok(()),
        )
        .unwrap();
    host.lifecycle(
        command(lease.runtime_id, Uuid::new_v4()),
        Lifecycle::ForceStop,
        1,
    )
    .unwrap();
    assert!(host.validate_permit(&permit, || Ok(())).is_err());
    drop(host);
    let recovered = ControlHost::recover(
        PrivateDir::open(&temp.path().canonicalize().unwrap().join("state")).unwrap(),
        lease.runtime_id,
    )
    .unwrap();
    assert_eq!(
        recovered.snapshot().unwrap().intent.desired,
        Intent::Stopped
    );
    assert!(!recovered.snapshot().unwrap().admission_open);
    assert!(recovered.attach_reconciled(lease).is_err());
    assert!(recovered.operation(permit.operation_id).is_ok());
}

#[test]
fn lifecycle_replay_preserves_rejection_and_never_reverses_newer_intent() {
    let (_temp, host, lease) = started();
    host.lifecycle(
        command(lease.runtime_id, Uuid::new_v4()),
        Lifecycle::Pause,
        1,
    )
    .unwrap();
    let request = command(lease.runtime_id, Uuid::new_v4());
    assert_eq!(
        host.lifecycle(request.clone(), Lifecycle::Resume, 1)
            .unwrap_err()
            .code,
        "stale_control"
    );
    assert_eq!(
        host.lifecycle(request, Lifecycle::Resume, 1)
            .unwrap_err()
            .code,
        "stale_control"
    );
    assert_eq!(host.snapshot().unwrap().intent.desired, Intent::Paused);
}

#[test]
fn rejected_mutation_replay_retains_the_error_across_restart() {
    let (temp, host, lease) = started();
    let mut request = command(lease.runtime_id, Uuid::new_v4());
    request.actor.roles.insert("accept".into());
    request.kind = "accept".into();
    assert_eq!(
        host.mutate(
            request.clone(),
            None,
            Some("accept"),
            || Err(Error::new("stale_result", "old token")),
            |_| panic!("rejected action must not reach provider")
        )
        .unwrap_err()
        .code,
        "stale_result"
    );
    drop(host);
    let recovered = ControlHost::recover(
        PrivateDir::open(&temp.path().canonicalize().unwrap().join("state")).unwrap(),
        lease.runtime_id,
    )
    .unwrap();
    assert_eq!(
        recovered
            .mutate(
                request,
                None,
                Some("accept"),
                || panic!("replay"),
                |_| panic!("replay")
            )
            .unwrap_err()
            .code,
        "stale_result"
    );
}

#[test]
fn mutation_is_serialized_with_pause_and_unknown_replay_sends_nothing() {
    let (_temp, host, lease) = started();
    let permit = host
        .admit(
            &lease,
            Uuid::new_v4(),
            "child".into(),
            "scope".into(),
            || Ok(()),
        )
        .unwrap();
    let mut request = command(lease.runtime_id, Uuid::new_v4());
    request.kind = "comment".into();
    let h = host.clone();
    let p = permit.clone();
    let r = request.clone();
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let mutation = std::thread::spawn(move || {
        h.mutate(
            r,
            Some(&p),
            None,
            || Ok(()),
            |_| {
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Err(Error::new("provider_transport", "response lost"))
            },
        )
    });
    entered_rx.recv().unwrap();
    let (ack_tx, ack_rx) = std::sync::mpsc::channel();
    let h = host.clone();
    let runtime = lease.runtime_id;
    let pause = std::thread::spawn(move || {
        let result = h.lifecycle(command(runtime, Uuid::new_v4()), Lifecycle::Pause, 1);
        ack_tx.send(()).unwrap();
        result
    });
    assert!(
        ack_rx
            .recv_timeout(std::time::Duration::from_millis(20))
            .is_err()
    );
    release_tx.send(()).unwrap();
    assert_eq!(
        mutation.join().unwrap().unwrap_err().code,
        "operation_uncertain"
    );
    pause.join().unwrap().unwrap();
    let replay = host
        .mutate(
            request,
            Some(&permit),
            None,
            || panic!("replay must not revalidate for new effects"),
            |_| panic!("unknown effect must not replay"),
        )
        .unwrap();
    assert_eq!(replay.phase, ReceiptPhase::Sent);
    assert!(!host.snapshot().unwrap().admission_open);
    let mut new_request = command(lease.runtime_id, Uuid::new_v4());
    new_request.kind = "comment".into();
    assert_eq!(
        host.mutate(
            new_request,
            Some(&permit),
            None,
            || Ok(()),
            |_| panic!("overlapping unknown effect must hold new IDs too")
        )
        .unwrap_err()
        .code,
        "reconciliation_required"
    );
}

#[test]
fn recovered_unfinished_work_cannot_be_admitted_under_new_owner() {
    let (temp, host, lease) = started();
    host.admit(
        &lease,
        Uuid::new_v4(),
        "unfinished-child".into(),
        "scope".into(),
        || Ok(()),
    )
    .unwrap();
    drop(host);
    let recovered = ControlHost::recover(
        PrivateDir::open(&temp.path().canonicalize().unwrap().join("state")).unwrap(),
        lease.runtime_id,
    )
    .unwrap();
    let new_owner = Lease {
        owner_generation: 2,
        owner_id: Uuid::new_v4(),
        ..lease
    };
    recovered.attach_reconciled(new_owner.clone()).unwrap();
    assert_eq!(
        recovered
            .admit(
                &new_owner,
                Uuid::new_v4(),
                "unfinished-child".into(),
                "scope".into(),
                || Ok(())
            )
            .unwrap_err()
            .code,
        "reconciliation_required"
    );
    assert!(
        recovered
            .admit(
                &new_owner,
                Uuid::new_v4(),
                "independent-child".into(),
                "scope".into(),
                || Ok(())
            )
            .is_ok()
    );
}

#[test]
fn canonical_identity_ignores_object_order_not_arrays() {
    assert_eq!(
        canonical_digest(1, &json!({"b":2,"a":1})).unwrap(),
        canonical_digest(1, &json!({"a":1,"b":2})).unwrap()
    );
    assert_ne!(
        canonical_digest(1, &json!([1, 2])).unwrap(),
        canonical_digest(1, &json!([2, 1])).unwrap()
    );
    assert!(canonical_digest(1, &json!(1.5)).is_err());
}

#[test]
fn active_background_command_cannot_be_reconciled_under_its_running_task() {
    let (_temp, directory) = directory();
    let runtime = Uuid::new_v4();
    let host = ControlHost::setup(directory, runtime, Uuid::new_v4()).unwrap();
    let mut request = command(runtime, Uuid::new_v4());
    request.kind = "command/Setup".into();
    request.actor.roles.insert("configure".into());
    let id = request.request_id;
    host.queue_command(request, "configure", || Ok(())).unwrap();
    let mut recover = actor();
    recover.roles.insert("recover".into());
    assert_eq!(
        host.reconcile_operation(&recover, id, |_| panic!(
            "active command cannot be taken over"
        ))
        .unwrap_err()
        .code,
        "operation_active"
    );
    host.complete_command(id, Ok(json!({"effect":"verified"})))
        .unwrap();
    assert!(host.snapshot().unwrap().background_commands.is_empty());
    assert_eq!(host.operation(id).unwrap().phase, ReceiptPhase::Reconciled);
}

#[test]
fn failed_owner_is_fenced_without_claiming_exit_or_changing_operator_intent() {
    let (_temp, host, lease) = started();
    let permit = host
        .admit(
            &lease,
            Uuid::new_v4(),
            "fixture-child".into(),
            "fixture-scope".into(),
            || Ok(()),
        )
        .unwrap();
    let first = host.suspend_owner(&lease, "channel_failed").unwrap();
    let duplicate = host.suspend_owner(&lease, "channel_failed").unwrap();
    assert_eq!(first.operation_id, duplicate.operation_id);
    let state = host.snapshot().unwrap();
    assert_eq!(state.intent.desired, Intent::Running);
    assert!(state.orchestration_alive);
    assert!(!state.admission_open);
    assert!(!state.reconciled);
    assert!(state.fenced_operations.contains(&permit.operation_id));
    assert!(
        state
            .unresolved_operations
            .contains_key(&first.operation_id)
    );
    assert_eq!(first.phase, ReceiptPhase::Sent);
    assert_eq!(
        first.observed_result.unwrap()["process_exit"],
        "unconfirmed"
    );
    host.orchestration_exited(&lease).unwrap();
    assert!(!host.snapshot().unwrap().orchestration_alive);
    assert!(
        host.snapshot()
            .unwrap()
            .unresolved_operations
            .contains_key(&duplicate.operation_id)
    );
}

#[test]
fn atomic_run_budget_survives_restart_and_profile_edits_without_reset() {
    let (temp, host, lease) = started();
    let reference = RunReference {
        normalized_state: Some("todo".into()),
        workspace: Uuid::new_v4(),
        account: "actual-fixture-account".into(),
        phase: kegari_v0::scheduler::PhaseKey {
            work: "canonical-child".into(),
            generation: canonical_digest(1, &"definition-correction").unwrap(),
            phase: "implementation".into(),
            retry_grant: None,
        },
        profile_digest: canonical_digest(1, &"profile-a").unwrap(),
        instruction_digest: canonical_digest(1, &"instructions").unwrap(),
        max_attempts: 2,
        max_turns: 20,
        max_review_rounds: 3,
        attempt: 1,
    };
    let admission = |reference: RunReference| RunAdmission {
        request_id: Uuid::new_v4(),
        work_key: "exact-work-generation".into(),
        scope_digest: canonical_digest(1, &"assembled-contract").unwrap(),
        reference,
    };
    let first = host
        .admit_run(&lease, admission(reference.clone()), || Ok(()))
        .unwrap();
    let recorded = RunReference::from_receipt(&host.operation(first.operation_id).unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(recorded.attempt, 1);
    host.retire_local(
        first.operation_id,
        json!({"known_failure":"fixture-process-exited"}),
    )
    .unwrap();
    assert_eq!(
        host.admit_run(&lease, admission(reference.clone()), || Ok(()))
            .unwrap_err()
            .code,
        "attempts_exhausted"
    );
    let runtime = lease.runtime_id;
    drop(host);
    let host = ControlHost::recover(
        PrivateDir::open(&temp.path().canonicalize().unwrap().join("state")).unwrap(),
        runtime,
    )
    .unwrap();
    let renewed = Lease {
        runtime_id: runtime,
        owner_generation: 2,
        owner_id: Uuid::new_v4(),
    };
    host.attach_reconciled(renewed.clone()).unwrap();
    let mut second = reference.clone();
    second.attempt = 2;
    second.profile_digest = canonical_digest(1, &"profile-b").unwrap();
    second.max_attempts = 9;
    let next = host
        .admit_run(&renewed, admission(second.clone()), || Ok(()))
        .unwrap();
    host.retire_local(
        next.operation_id,
        json!({"known_failure":"fixture-process-exited"}),
    )
    .unwrap();
    second.attempt = 3;
    assert_eq!(
        host.admit_run(&renewed, admission(second), || Ok(()))
            .unwrap_err()
            .code,
        "attempts_exhausted"
    );
    let mut correction = reference;
    correction.phase.generation = canonical_digest(1, &"new-material-correction").unwrap();
    assert!(
        host.admit_run(&renewed, admission(correction), || Ok(()))
            .is_ok()
    );
}

#[test]
fn concurrent_session_capacity_is_counted_and_reserved_in_one_host_transaction() {
    let (_temp, host, lease) = started();
    let barrier = Arc::new(std::sync::Barrier::new(3));
    let attempts = (0..2)
        .map(|index| {
            let host = host.clone();
            let lease = lease.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let work = format!("canonical-child-{index}");
                let reference = RunReference {
                    workspace: Uuid::from_u128(2),
                    account: "same-actual-account".into(),
                    normalized_state: Some("todo".into()),
                    phase: kegari_v0::scheduler::PhaseKey {
                        work: work.clone(),
                        generation: canonical_digest(1, &"current-generation").unwrap(),
                        phase: "implementation".into(),
                        retry_grant: None,
                    },
                    profile_digest: canonical_digest(1, &"profile").unwrap(),
                    instruction_digest: canonical_digest(1, &"instructions").unwrap(),
                    max_attempts: 3,
                    max_turns: 20,
                    max_review_rounds: 3,
                    attempt: 1,
                };
                barrier.wait();
                host.admit_run_observed(
                    &lease,
                    RunAdmission {
                        request_id: Uuid::new_v4(),
                        work_key: work,
                        scope_digest: canonical_digest(1, &"contract").unwrap(),
                        reference,
                    },
                    |snapshot| {
                        let used = snapshot
                            .active_operations
                            .iter()
                            .map(|permit| {
                                RunReference::from_receipt(&host.operation(permit.operation_id)?)
                            })
                            .collect::<kegari_types::error::Result<Vec<_>>>()?
                            .into_iter()
                            .flatten()
                            .filter(|run| run.account == "same-actual-account")
                            .count();
                        if used >= 1 {
                            Err(Error::new("capacity_wait", "actual account cap is full"))
                        } else {
                            Ok(())
                        }
                    },
                )
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    let results = attempts
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .find_map(|result| result.as_ref().err())
            .unwrap()
            .code,
        "capacity_wait"
    );
    assert_eq!(host.snapshot().unwrap().active_operations.len(), 1);
    assert_eq!(
        host.operations()
            .unwrap()
            .iter()
            .filter(|receipt| RunReference::from_receipt(receipt).unwrap().is_some())
            .count(),
        1
    );
}
