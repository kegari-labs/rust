//! Actual Control/storage components only. No provider, process, model or
//! joined-worker capability is manufactured by these receipt-read regressions.
use super::*;
use crate::control::Lease;
use crate::control::Lifecycle;
use crate::control::MutationRequest;
use kegari_control_api::identity::Actor;
use std::sync::mpsc;
use std::time::Duration as Wait;

fn request(runtime: Uuid, kind: &str, target: &str) -> MutationRequest {
    MutationRequest {
        request_id: Uuid::new_v4(),
        runtime_id: runtime,
        actor: Actor {
            identity: "receipt-snapshot-component".into(),
            principal: None,
            roles: BTreeSet::from(["lifecycle".into()]),
        },
        kind: kind.into(),
        canonical_targets: vec![target.into()],
        expected: BTreeMap::new(),
        payload: json!({"component":"receipt snapshot"}),
    }
}
fn started() -> (tempfile::TempDir, Arc<ControlHost>, Lease) {
    let temp = tempfile::tempdir().unwrap();
    let runtime = Uuid::new_v4();
    let root = temp.path().canonicalize().unwrap().join("state");
    let host = Arc::new(
        ControlHost::setup(PrivateDir::create(&root).unwrap(), runtime, Uuid::new_v4()).unwrap(),
    );
    host.lifecycle(
        request(runtime, "lifecycle", &runtime.to_string()),
        Lifecycle::Start,
        0,
    )
    .unwrap();
    // Same explicit Control-only lease boundary as control_storage fixtures;
    // this is not an orchestration process or native worker admission.
    let lease = Lease {
        runtime_id: runtime,
        owner_generation: 1,
        owner_id: Uuid::new_v4(),
    };
    host.attach_reconciled(lease.clone()).unwrap();
    (temp, host, lease)
}

#[test]
fn reconstruction_snapshot_waits_for_actual_receipt_replacement_gate() {
    let (_temp, host, lease) = started();
    let permit = host
        .admit(
            &lease,
            Uuid::new_v4(),
            "snapshot-work".into(),
            canonical_digest(1, &"snapshot scope").unwrap(),
            || Ok(()),
        )
        .unwrap();
    let (effect, ()) = host
        .begin_effect(
            request(lease.runtime_id, "fixture_callback", "snapshot-resource"),
            &permit,
            || Ok(()),
            |_| Ok(()),
        )
        .unwrap();
    let effect_id = effect.operation_id();
    let before = reconstruction_receipts(&host).unwrap();
    let before_digest = canonical_digest(1, &before).unwrap();
    assert_eq!(
        before
            .iter()
            .find(|r| r.operation_id == effect_id)
            .unwrap()
            .phase,
        ReceiptPhase::Sent
    );
    let (entered_tx, entered_rx) = mpsc::sync_channel(1);
    let (release_tx, release_rx) = mpsc::sync_channel(1);
    let writer_host = host.clone();
    let writer_permit = permit.clone();
    let writer = std::thread::spawn(move || {
        writer_host.complete_effect(
            effect,
            &writer_permit,
            || {
                // The actual completion gate is already held, before the
                // protected Sent -> Acknowledged replacement.
                entered_tx.send(()).map_err(|_| {
                    Error::new("fixture_barrier", "completion entry receiver disappeared")
                })?;
                release_rx.recv_timeout(Wait::from_secs(10)).map_err(|_| {
                    Error::new("fixture_barrier", "completion release was not received")
                })?;
                Ok(())
            },
            json!({"component_callback":"returned; no process or provider launched"}),
        )
    });
    entered_rx.recv_timeout(Wait::from_secs(10)).unwrap();
    let (starting_tx, starting_rx) = mpsc::sync_channel(1);
    let (snapshot_tx, snapshot_rx) = mpsc::sync_channel(1);
    let reader_host = host.clone();
    let reader = std::thread::spawn(move || {
        starting_tx.send(()).unwrap();
        snapshot_tx
            .send(reconstruction_receipts(&reader_host))
            .unwrap();
    });
    starting_rx.recv_timeout(Wait::from_secs(10)).unwrap();
    // A bounded negative wait asserts the reader cannot return the old Sent
    // epoch while the real writer owns Control. It is not a retry or a sleep
    // used to establish freshness, and all actual work is joined below.
    let early = snapshot_rx.recv_timeout(Wait::from_millis(100));
    let blocked = matches!(&early, Err(mpsc::RecvTimeoutError::Timeout));
    release_tx.send(()).unwrap();
    let acknowledged = writer.join().unwrap().unwrap();
    let observed = early
        .or_else(|_| snapshot_rx.recv_timeout(Wait::from_secs(10)))
        .unwrap()
        .unwrap();
    reader.join().unwrap();
    host.retire_local(permit.operation_id, json!({"component_complete":true}))
        .unwrap();

    assert!(
        blocked,
        "receipt reconstruction escaped the active replacement gate"
    );
    let captured = observed
        .iter()
        .find(|r| r.operation_id == effect_id)
        .unwrap();
    assert_eq!(captured.phase, ReceiptPhase::Acknowledged);
    assert_eq!(
        canonical_digest(1, captured).unwrap(),
        canonical_digest(1, &acknowledged).unwrap()
    );
    assert_eq!(canonical_digest(1, &before).unwrap(), before_digest);
    assert_eq!(
        before
            .iter()
            .find(|r| r.operation_id == effect_id)
            .unwrap()
            .phase,
        ReceiptPhase::Sent,
        "the owned earlier snapshot is data; later writes cannot change its epoch"
    );
}

#[test]
fn reconstruction_snapshot_keeps_actual_link_refusal() {
    let (temp, host, lease) = started();
    let permit = host
        .admit(
            &lease,
            Uuid::new_v4(),
            "snapshot-work".into(),
            canonical_digest(1, &"snapshot scope").unwrap(),
            || Ok(()),
        )
        .unwrap();
    let before = canonical_digest(1, &reconstruction_receipts(&host).unwrap()).unwrap();
    let receipt = temp
        .path()
        .join("state/receipts")
        .join(format!("{}.json", permit.operation_id));
    let alias = temp.path().join("linked-receipt");
    std::fs::hard_link(&receipt, &alias).unwrap();
    let refused = reconstruction_receipts(&host).unwrap_err();
    std::fs::remove_file(alias).unwrap();
    let restored = reconstruction_receipts(&host).unwrap();
    host.retire_local(permit.operation_id, json!({"no_effect_was_launched":true}))
        .unwrap();
    assert_eq!(refused.code, "unsafe_storage");
    assert_eq!(canonical_digest(1, &restored).unwrap(), before);
}
