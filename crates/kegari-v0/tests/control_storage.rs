use kegari_v0::{
    control::*,
    digest::canonical_digest,
    error::Error,
    storage::{PrivateDir, WriteBoundary},
};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    os::unix::fs::{PermissionsExt, symlink},
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
fn v0_t08_each_write_boundary_recovers_old_or_new_valid_record() {
    for stop in [
        WriteBoundary::Created,
        WriteBoundary::Written,
        WriteBoundary::FileSynced,
        WriteBoundary::Renamed,
        WriteBoundary::DirectorySynced,
    ] {
        let (_temp, dir) = directory();
        dir.write("intent.json", &json!({"desired":"RUNNING"}))
            .unwrap();
        let result = dir.write_observed("intent.json", &json!({"desired":"STOPPED"}), |point| {
            if point == stop {
                Err(Error::new("injected_crash", "durability boundary"))
            } else {
                Ok(())
            }
        });
        assert!(result.is_err());
        let recovered: serde_json::Value = PrivateDir::open(dir.path())
            .unwrap()
            .read("intent.json")
            .unwrap();
        assert_eq!(
            recovered["desired"],
            if matches!(
                stop,
                WriteBoundary::Renamed | WriteBoundary::DirectorySynced
            ) {
                "STOPPED"
            } else {
                "RUNNING"
            }
        );
        assert_eq!(
            fs::metadata(dir.path().join("intent.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
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
fn v0_t12_control_storage_rejects_symlinks_hardlinks_and_permissions() {
    let (temp, dir) = directory();
    let outside = temp.path().join("outside");
    fs::write(&outside, b"preserve").unwrap();
    symlink(&outside, dir.path().join("intent.json")).unwrap();
    assert!(dir.write("intent.json", &json!(1)).is_err());
    assert_eq!(fs::read(&outside).unwrap(), b"preserve");
    fs::hard_link(&outside, dir.path().join("hard.json")).unwrap();
    assert!(dir.read::<serde_json::Value>("hard.json").is_err());
    assert!(dir.write("../escape.json", &json!(1)).is_err());
    assert!(dir.write("/absolute.json", &json!(1)).is_err());
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
    assert!(dir.write("safe.json", &json!(1)).is_err());
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
