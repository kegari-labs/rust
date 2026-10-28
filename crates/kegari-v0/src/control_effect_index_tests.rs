//! Actual Control + protected-file ordering tests. No worker process, joined
//! completion, human acceptance, or cold execution capability is manufactured.
use super::*;
use kegari_storage::WriteBoundary;
use kegari_types::run::PhaseKey;
use kegari_types::run::RunReference;
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn digest(label: &str) -> String {
    canonical_digest(1, &label).unwrap()
}
struct Fixture {
    _dir: tempfile::TempDir,
    host: ControlHost,
    lease: Lease,
    run: RunReference,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Uuid::new_v4();
        let host = ControlHost::setup(
            PrivateDir::create(&dir.path().canonicalize().unwrap().join("control")).unwrap(),
            runtime,
            Uuid::new_v4(),
        )
        .unwrap();
        host.lifecycle(
            MutationRequest {
                request_id: Uuid::new_v4(),
                runtime_id: runtime,
                actor: Actor {
                    identity: "component human".into(),
                    roles: ["lifecycle".into()].into(),
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
            owner_generation: 1,
            owner_id: Uuid::new_v4(),
        };
        host.attach_reconciled(lease.clone()).unwrap();
        let run = RunReference {
            workspace: Uuid::new_v4(),
            account: "component-account".into(),
            normalized_state: None,
            phase: PhaseKey {
                work: format!("parent:{}", Uuid::new_v4()),
                generation: digest("generation"),
                phase: "parent_evidence_review".into(),
                retry_grant: None,
            },
            profile_digest: digest("profile"),
            instruction_digest: digest("instruction"),
            max_attempts: 2,
            max_turns: 3,
            max_review_rounds: 2,
            attempt: 1,
        };
        Self {
            _dir: dir,
            host,
            lease,
            run,
        }
    }
    fn admit(&self) -> Permit {
        self.host
            .admit_run(
                &self.lease,
                RunAdmission {
                    request_id: Uuid::new_v4(),
                    work_key: self.run.phase.work.clone(),
                    scope_digest: digest("scope"),
                    reference: self.run.clone(),
                },
                || Ok(()),
            )
            .unwrap()
    }
    fn request(&self, permit: &Permit, label: &str) -> MutationRequest {
        MutationRequest {
            request_id: Uuid::new_v4(),
            runtime_id: self.lease.runtime_id,
            actor: Actor {
                identity: format!("owner:{}", self.lease.owner_id),
                roles: BTreeSet::new(),
                principal: None,
            },
            kind: label.into(),
            canonical_targets: vec![format!("component:{}", permit.operation_id)],
            expected: BTreeMap::new(),
            payload: json!({"component":label}),
        }
    }
    fn inventory(&self, permit: &Permit) -> Inventory {
        inventory(&self.host, permit.operation_id)
    }
}
fn inventory(host: &ControlHost, owner: Uuid) -> Inventory {
    read(
        &host.root,
        &host.receipts.get(owner).unwrap(),
        &receipts(&host.receipts).unwrap(),
        &index_names(&host.root).unwrap(),
    )
    .unwrap()
}
fn callback_observes_member(host: &ControlHost, permit: &Permit, receipt: &Receipt) {
    assert_eq!(receipt.phase, ReceiptPhase::Sent);
    let stored = host.receipts.get(receipt.operation_id).unwrap();
    assert_eq!(stored.phase, ReceiptPhase::Sent);
    assert_eq!(
        inventory(host, permit.operation_id)
            .members
            .get(&receipt.operation_id),
        Some(&receipt_request_digest(&stored).unwrap())
    );
}
fn ok_observation(receipt: &Receipt) -> Reconciliation {
    Reconciliation::UniqueVerified {
        remote_ids: BTreeMap::new(),
        result: json!({"component_readback":"known"}),
        payload_digest: receipt.payload_digest.clone(),
        actor: receipt.actor.clone(),
    }
}

#[test]
fn independent_integrated_review_has_its_own_retained_effect_inventory() {
    let mut f = Fixture::new();
    f.run.phase.phase = "parent_integrated_review".into();
    let p = f.admit();
    assert_eq!(
        f.host
            .receipts
            .get(p.operation_id)
            .unwrap()
            .expected
            .get(KEY),
        Some(&"1".into())
    );
    let receipt = f
        .host
        .mutate(
            f.request(&p, "review_feedback"),
            Some(&p),
            None,
            || Ok(()),
            |r| {
                callback_observes_member(&f.host, &p, r);
                Ok((
                    BTreeMap::new(),
                    json!({"component":"review feedback recorded"}),
                ))
            },
        )
        .unwrap();
    f.host
        .retire_local(p.operation_id, json!({"component_only":true}))
        .unwrap();
    assert!(f.inventory(&p).frozen.is_some());
    recover(&f.host.root, &f.host.receipts).unwrap();
    std::fs::remove_file(
        f.host
            .receipts
            .directory
            .path()
            .join(format!("{}.json", receipt.operation_id)),
    )
    .unwrap();
    let error = recover(&f.host.root, &f.host.receipts).unwrap_err();
    assert_eq!(error.code, "record_missing");
    assert_eq!(error.message, "indexed effect receipt is missing");
}

#[test]
fn all_dispatch_funnels_register_before_callback_and_keep_acknowledged_members() {
    let f = Fixture::new();
    let p = f.admit();
    assert_eq!(
        f.host
            .receipts
            .get(p.operation_id)
            .unwrap()
            .expected
            .get(KEY),
        Some(&"1".into())
    );
    let calls = AtomicUsize::new(0);
    let request = f.request(&p, "local");
    let (effect, ()) = f
        .host
        .begin_effect(
            request.clone(),
            &p,
            || Ok(()),
            |r| {
                callback_observes_member(&f.host, &p, r);
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        )
        .unwrap();
    f.host
        .complete_effect(effect, &p, || Ok(()), json!({"component_exit":true}))
        .unwrap();
    assert_eq!(
        f.host
            .begin_effect(
                request,
                &p,
                || Ok(()),
                |_| -> Result<()> { panic!("must not replay") }
            )
            .err()
            .unwrap()
            .code,
        "operation_uncertain"
    );
    f.host
        .mutate(
            f.request(&p, "sync"),
            Some(&p),
            None,
            || Ok(()),
            |r| {
                callback_observes_member(&f.host, &p, r);
                calls.fetch_add(1, Ordering::SeqCst);
                Ok((BTreeMap::new(), json!({"known":true})))
            },
        )
        .unwrap();
    let RemoteReservation::Prepared(reserved) = f
        .host
        .reserve_remote(f.request(&p, "remote"), Some(p.clone()), None, |_| Ok(()))
        .unwrap()
    else {
        panic!("fresh reservation")
    };
    let (effect, ()) = f
        .host
        .launch_remote(
            reserved,
            Ok(()),
            |_| Ok(()),
            |r| {
                callback_observes_member(&f.host, &p, r);
                calls.fetch_add(1, Ordering::SeqCst);
            },
        )
        .unwrap();
    f.host
        .complete_remote(effect, Ok((BTreeMap::new(), json!({"provider_ack":true}))))
        .unwrap();
    let mut protocol = f.request(&p, "codex/turn/start");
    protocol.canonical_targets = vec![format!("codex-session:{}", p.operation_id)];
    let (effect, ()) = f
        .host
        .begin_protocol_effect(
            protocol,
            &p,
            || Ok(()),
            |r| {
                callback_observes_member(&f.host, &p, r);
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        )
        .unwrap();
    f.host
        .complete_effect(effect, &p, || Ok(()), json!({"wire_ack":true}))
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 4);
    assert_eq!(f.inventory(&p).cursor.count, 4);
    let closed = f
        .host
        .retire_local(
            p.operation_id,
            json!({"component_only":"no real worker join"}),
        )
        .unwrap();
    assert_eq!(
        closed.remote_ids.get(KEY),
        Some(&canonical_digest(1, &f.inventory(&p).cursor).unwrap())
    );
    assert!(f.inventory(&p).frozen.is_some());
    recover(&f.host.root, &f.host.receipts).unwrap();
}

#[test]
fn deleted_acknowledged_receipt_is_required_before_first_historical_scan() {
    let f = Fixture::new();
    let p = f.admit();
    let r = f
        .host
        .mutate(
            f.request(&p, "sync"),
            Some(&p),
            None,
            || Ok(()),
            |_| Ok((BTreeMap::new(), Value::Null)),
        )
        .unwrap();
    f.host
        .retire_local(p.operation_id, json!({"component_only":true}))
        .unwrap();
    let leaf = format!("{}.json", r.operation_id);
    let bytes = f
        .host
        .receipts
        .directory
        .read_bytes(&leaf, 256 * 1024)
        .unwrap();
    std::fs::remove_file(f.host.receipts.directory.path().join(&leaf)).unwrap();
    // The first index reader detects a file that a post-hoc receipt scan omits.
    assert!(
        !f.host
            .receipts
            .all()
            .unwrap()
            .iter()
            .any(|v| v.operation_id == r.operation_id)
    );
    let error = recover(&f.host.root, &f.host.receipts).unwrap_err();
    assert_eq!(error.code, "record_missing");
    assert_eq!(error.message, "indexed effect receipt is missing");
    f.host
        .receipts
        .directory
        .create_bytes_exclusive(&leaf, &bytes)
        .unwrap();
    recover(&f.host.root, &f.host.receipts).unwrap();
}

#[test]
fn zero_effect_preparation_requires_its_actual_frozen_root_and_preserves_identity() {
    let f = Fixture::new();
    let reference = PreparationReference {
        worker_operation: Uuid::new_v4(),
        workspace: f.run.workspace,
        account: f.run.account.clone(),
        phase: f.run.phase.clone(),
    };
    let p = f
        .host
        .admit_preparation(
            &f.lease,
            PreparationAdmission {
                request_id: reference.operation_id().unwrap(),
                work_key: reference.phase.work.clone(),
                scope_digest: digest("scope"),
                reference: reference.clone(),
            },
            || Ok(()),
        )
        .unwrap();
    assert_eq!(
        ordinary_preparation_request_digest(&p, &reference).unwrap(),
        receipt_request_digest(&f.host.receipts.get(p.operation_id).unwrap()).unwrap()
    );
    f.host
        .retire_local(
            p.operation_id,
            json!({"phase":"worker_preparation","result":"prepared"}),
        )
        .unwrap();
    assert_eq!(f.inventory(&p).cursor.count, 0);
    for suffix in ["frozen", "root"] {
        let leaf = name(p.operation_id, suffix);
        let bytes = f.host.root.read_bytes(&leaf, 256 * 1024).unwrap();
        std::fs::remove_file(f.host.root.path().join(&leaf)).unwrap();
        assert!(recover(&f.host.root, &f.host.receipts).is_err());
        f.host.root.create_bytes_exclusive(&leaf, &bytes).unwrap();
    }
    recover(&f.host.root, &f.host.receipts).unwrap();
}

#[test]
fn rejected_gate_keeps_reserved_membership_and_no_callback() {
    let f = Fixture::new();
    let p = f.admit();
    let calls = AtomicUsize::new(0);
    let request = f.request(&p, "sync");
    let id = request.request_id;
    let error = f
        .host
        .mutate(
            request,
            Some(&p),
            None,
            || Err(Error::new("component_denied", "no authority")),
            |_| {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok((BTreeMap::new(), Value::Null))
            },
        )
        .unwrap_err();
    assert_eq!(error.code, "component_denied");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let receipt = f.host.receipts.get(id).unwrap();
    assert_eq!(receipt.phase, ReceiptPhase::Reconciled);
    assert_eq!(receipt.observed_result.unwrap()["effect"], "not_sent");
    assert_eq!(f.inventory(&p).cursor.count, 1);
    f.host
        .retire_local(p.operation_id, json!({"component_only":true}))
        .unwrap();
    recover(&f.host.root, &f.host.receipts).unwrap();
}

#[test]
fn uncertain_head_writes_never_dispatch_or_recreate_writer() {
    for boundary in [
        WriteBoundary::Created,
        WriteBoundary::Written,
        WriteBoundary::FileSynced,
        WriteBoundary::Renamed,
        WriteBoundary::DirectorySynced,
    ] {
        let f = Fixture::new();
        let p = f.admit();
        f.host.state.lock().unwrap().effect_index_fault = Some(boundary);
        let request = f.request(&p, "local");
        let id = request.request_id;
        let calls = AtomicUsize::new(0);
        let err = f
            .host
            .begin_effect(
                request,
                &p,
                || Ok(()),
                |_| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                },
            )
            .err()
            .unwrap();
        assert_eq!(err.code, "effect_index");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            f.host.receipts.get(id).unwrap().phase,
            ReceiptPhase::Prepared
        );
        let snapshot = f.host.snapshot().unwrap();
        assert!(!snapshot.admission_open);
        assert!(snapshot.fenced_operations.contains(&p.operation_id));
        assert!(snapshot.unresolved_operations.contains_key(&id));
        assert_eq!(
            f.host
                .begin_effect(f.request(&p, "another"), &p, || Ok(()), |_| Ok(()))
                .err()
                .unwrap()
                .code,
            "access_denied"
        );
        if matches!(
            boundary,
            WriteBoundary::Renamed | WriteBoundary::DirectorySynced
        ) {
            // Exact durable membership allows cleanup-only local retirement; the
            // Prepared effect remains unresolved. No launch is retried.
            let retired = f
                .host
                .retire_local(p.operation_id, json!({"component_only":"cleanup"}))
                .unwrap();
            assert!(
                retired.observed_result.unwrap()["unresolved_effects"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(id))
            );
        } else {
            assert_eq!(
                f.host
                    .retire_local(p.operation_id, Value::Null)
                    .unwrap_err()
                    .code,
                "effect_index"
            );
        }
    }
}

#[test]
fn reservation_before_owner_freeze_cannot_transmit_after_preflight() {
    let f = Fixture::new();
    let p = f.admit();
    let RemoteReservation::Prepared(reservation) = f
        .host
        .reserve_remote(f.request(&p, "remote"), Some(p.clone()), None, |_| Ok(()))
        .unwrap()
    else {
        panic!()
    };
    let operation = reservation.operation_id();
    let retired = f
        .host
        .retire_local(p.operation_id, json!({"component_only":true}))
        .unwrap();
    assert!(
        retired.observed_result.unwrap()["unresolved_effects"]
            .as_array()
            .unwrap()
            .contains(&json!(operation))
    );
    let before = f.inventory(&p).cursor;
    let calls = AtomicUsize::new(0);
    assert_eq!(
        f.host
            .launch_remote(
                reservation,
                Ok(()),
                |_| Ok(()),
                |_| {
                    calls.fetch_add(1, Ordering::SeqCst);
                }
            )
            .err()
            .unwrap()
            .code,
        "access_denied"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(f.inventory(&p).cursor, before);
    assert_eq!(
        f.host
            .receipts
            .get(operation)
            .unwrap()
            .observed_result
            .unwrap()["effect"],
        "not_sent"
    );
    assert!(
        !f.host
            .snapshot()
            .unwrap()
            .unresolved_operations
            .contains_key(&operation)
    );
}

#[test]
fn transmitted_unknown_remains_indexed_and_reconciliation_preserves_frozen_reference() {
    let f = Fixture::new();
    let p = f.admit();
    let RemoteReservation::Prepared(reservation) = f
        .host
        .reserve_remote(f.request(&p, "remote"), Some(p.clone()), None, |_| Ok(()))
        .unwrap()
    else {
        panic!()
    };
    let (effect, ()) = f
        .host
        .launch_remote(reservation, Ok(()), |_| Ok(()), |_| ())
        .unwrap();
    let operation = effect.operation;
    let owner = f
        .host
        .retire_local(p.operation_id, json!({"component_only":true}))
        .unwrap();
    assert_eq!(
        f.host
            .complete_remote(effect, Err(Error::new("transport", "lost ack")))
            .unwrap_err()
            .code,
        "operation_uncertain"
    );
    assert!(
        f.host
            .snapshot()
            .unwrap()
            .unresolved_operations
            .contains_key(&operation)
    );
    let receipt = f.host.receipts.get(operation).unwrap();
    f.host
        .reconcile_owned(&receipt, ok_observation(&receipt))
        .unwrap();
    let rewritten = f
        .host
        .reconcile_owned(&owner, ok_observation(&owner))
        .unwrap();
    assert_eq!(rewritten.remote_ids.get(KEY), owner.remote_ids.get(KEY));
    recover(&f.host.root, &f.host.receipts).unwrap();
}

#[test]
fn live_operator_reconcile_cannot_fake_freeze_or_discard_membership() {
    let f = Fixture::new();
    let p = f.admit();
    let actor = Actor {
        identity: "component recovery".into(),
        roles: ["recover".into()].into(),
        principal: None,
    };
    assert_eq!(
        f.host
            .reconcile_operation(&actor, p.operation_id, |r| Ok(ok_observation(r)))
            .unwrap_err()
            .code,
        "operation_active"
    );
    assert!(
        f.host
            .snapshot()
            .unwrap()
            .active_operations
            .iter()
            .any(|v| v.operation_id == p.operation_id)
    );
    assert!(f.inventory(&p).frozen.is_none());
}

#[test]
fn cold_valid_open_inventory_has_no_writer_and_retains_owner_hold() {
    let f = Fixture::new();
    let p = f.admit();
    let root = f.host.root.path().to_owned();
    let runtime = f.lease.runtime_id;
    drop(f.host);
    let host = ControlHost::recover(PrivateDir::open(&root).unwrap(), runtime).unwrap();
    assert!(host.state.lock().unwrap().effect_indexes.is_empty());
    assert!(host.snapshot().unwrap().active_operations.is_empty());
    assert!(
        host.snapshot()
            .unwrap()
            .unresolved_operations
            .contains_key(&p.operation_id)
    );
    let receipt = host.receipts.get(p.operation_id).unwrap();
    let error = host
        .reconcile_owned(&receipt, ok_observation(&receipt))
        .unwrap_err();
    assert_eq!(error.code, "effect_index");
    assert_eq!(
        error.message,
        "owner reconciliation cannot invent frozen membership"
    );
}

#[test]
fn concurrent_protocol_reservations_share_contiguous_membership() {
    let f = Fixture::new();
    let p = f.admit();
    let host = Arc::new(f.host);
    let calls = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for i in 0..4 {
            let host = &host;
            let p = &p;
            let calls = &calls;
            scope.spawn(move || {
                let request = MutationRequest {
                    request_id: Uuid::new_v4(),
                    runtime_id: p.lease.runtime_id,
                    actor: Actor {
                        identity: "component worker".into(),
                        roles: BTreeSet::new(),
                        principal: None,
                    },
                    kind: "codex/turn/start".into(),
                    canonical_targets: vec![format!("codex-session:{}", p.operation_id)],
                    expected: BTreeMap::new(),
                    payload: json!({"i":i}),
                };
                // Rejection still registers membership; it avoids intentionally
                // overlapping real protocol requests while exercising mutex order.
                let err = host
                    .begin_protocol_effect(
                        request,
                        p,
                        || Err(Error::new("component_denied", "ordered gate")),
                        |_| {
                            calls.fetch_add(1, Ordering::SeqCst);
                            Ok(())
                        },
                    )
                    .err()
                    .unwrap();
                assert_eq!(err.code, "component_denied");
            });
        }
    });
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(inventory(&host, p.operation_id).cursor.count, 4);
}

#[test]
fn retirement_post_rename_failure_retains_exact_cleanup_only_retry() {
    let f = Fixture::new();
    let p = f.admit();
    f.host.state.lock().unwrap().effect_index_fault = Some(WriteBoundary::Renamed);
    let observation = json!({"component_only":"closed local owner"});
    assert_eq!(
        f.host
            .retire_local(p.operation_id, observation.clone())
            .unwrap_err()
            .code,
        "effect_index"
    );
    let persisted = f.host.receipts.get(p.operation_id).unwrap();
    assert!(persisted.remote_ids.contains_key(KEY));
    assert!(
        f.host
            .snapshot()
            .unwrap()
            .active_operations
            .iter()
            .any(|v| v.operation_id == p.operation_id)
    );
    assert_eq!(
        f.host
            .begin_effect(f.request(&p, "forbidden"), &p, || Ok(()), |_| Ok(()))
            .err()
            .unwrap()
            .code,
        "access_denied"
    );
    let changed = f
        .host
        .retire_local(p.operation_id, json!({"changed":true}))
        .unwrap_err();
    assert_eq!(changed.code, "effect_index");
    assert_eq!(
        changed.message,
        "owner retirement retry changed its original observation"
    );
    let closed = f.host.retire_local(p.operation_id, observation).unwrap();
    assert_eq!(
        canonical_digest(1, &closed).unwrap(),
        canonical_digest(1, &persisted).unwrap()
    );
    assert!(f.host.snapshot().unwrap().active_operations.is_empty());
}

#[test]
fn missing_index_never_backfills_a_nonfresh_effect_or_downgrades_declared_owner() {
    let f = Fixture::new();
    let p = f.admit();
    let request = f.request(&p, "sync");
    f.host
        .mutate(
            request.clone(),
            Some(&p),
            None,
            || Ok(()),
            |_| Ok((BTreeMap::new(), Value::Null)),
        )
        .unwrap();
    let leaf = entry_name(p.operation_id, 1);
    let bytes = f.host.root.read_bytes(&leaf, 256 * 1024).unwrap();
    std::fs::remove_file(f.host.root.path().join(&leaf)).unwrap();
    let error = f
        .host
        .mutate(
            request,
            Some(&p),
            None,
            || Ok(()),
            |_| panic!("no replay or backfill"),
        )
        .unwrap_err();
    assert_eq!(error.code, "record_missing");
    assert!(!f.host.root.path().join(&leaf).exists());
    f.host.root.create_bytes_exclusive(&leaf, &bytes).unwrap();
    let mut admission = f.host.receipts.get(p.operation_id).unwrap();
    admission.expected.remove(KEY);
    f.host.receipts.save(&admission).unwrap();
    assert_eq!(
        recover(&f.host.root, &f.host.receipts).unwrap_err().message,
        "effect index has no declared retained owner"
    );
}

#[test]
fn first_index_creation_failure_retains_prepared_admission_without_permit() {
    let f = Fixture::new();
    let operation = Uuid::new_v4();
    // Exact protected immutable collision, no forged worker/run capability.
    create(
        &f.host.root,
        &name(operation, "root"),
        &json!({"unrelated":"collision"}),
    )
    .unwrap();
    let error = f
        .host
        .admit_run(
            &f.lease,
            RunAdmission {
                request_id: operation,
                work_key: f.run.phase.work.clone(),
                scope_digest: digest("scope"),
                reference: f.run.clone(),
            },
            || Ok(()),
        )
        .err()
        .unwrap();
    assert_eq!(error.code, "io_error");
    assert_eq!(
        f.host.receipts.get(operation).unwrap().phase,
        ReceiptPhase::Prepared
    );
    assert!(f.host.snapshot().unwrap().active_operations.is_empty());
    assert!(!f.host.snapshot().unwrap().admission_open);
    assert!(recover(&f.host.root, &f.host.receipts).is_err());
}

#[test]
fn unexpected_index_tail_or_orphan_owner_is_never_ignored() {
    let f = Fixture::new();
    let p = f.admit();
    let leaf = entry_name(p.operation_id, 2);
    create(&f.host.root, &leaf, &json!({"uncommitted":"tail"})).unwrap();
    assert_eq!(
        recover(&f.host.root, &f.host.receipts).unwrap_err().message,
        "owner effect index has uncommitted or unexpected records"
    );
    std::fs::remove_file(f.host.root.path().join(leaf)).unwrap();
    let orphan = name(Uuid::new_v4(), "root");
    create(&f.host.root, &orphan, &json!({"orphan":true})).unwrap();
    assert_eq!(
        recover(&f.host.root, &f.host.receipts).unwrap_err().message,
        "effect index has no declared retained owner"
    );
}

#[test]
fn legacy_absent_marker_recovery_does_not_create_an_index_writer() {
    let f = Fixture::new();
    let p = Permit {
        operation_id: Uuid::new_v4(),
        lease: f.lease.clone(),
        control_generation: f.host.snapshot().unwrap().intent.generation,
        work_key: f.run.phase.work.clone(),
        scope_digest: digest("legacy scope"),
        registered_at: Utc::now(),
    };
    // Explicit old-format receipt fixture, not a registered permit or worker.
    let mut request =
        admission_identity::request(&p, Some(AdmissionReference::Run(f.run.clone()))).unwrap();
    request.expected.remove(KEY);
    let (mut legacy, fresh) = f
        .host
        .receipts
        .prepare(&request, f.lease.owner_generation)
        .unwrap();
    assert!(fresh);
    legacy.phase = ReceiptPhase::Reconciled;
    legacy.observed_result = Some(json!({"component_only":"legacy closed metadata"}));
    f.host.receipts.save(&legacy).unwrap();
    let digest = receipt_request_digest(&legacy).unwrap();
    recover(&f.host.root, &f.host.receipts).unwrap();
    assert_eq!(
        receipt_request_digest(&f.host.receipts.get(p.operation_id).unwrap()).unwrap(),
        digest
    );
    assert!(index_names(&f.host.root).unwrap().is_empty());
    assert!(f.host.state.lock().unwrap().effect_indexes.is_empty());
}

#[test]
fn finish_freezes_membership_but_cannot_discard_unknown_effect_holds() {
    let f = Fixture::new();
    let p = f.admit();
    let request = f.request(&p, "unknown");
    let id = request.request_id;
    assert_eq!(
        f.host
            .mutate(
                request,
                Some(&p),
                None,
                || Ok(()),
                |_| Err(Error::new("transport", "unknown"))
            )
            .unwrap_err()
            .code,
        "operation_uncertain"
    );
    assert_eq!(
        f.host.finish(p.operation_id, Value::Null).unwrap_err().code,
        "reconciliation_required"
    );
    assert!(f.inventory(&p).frozen.is_none());
    let receipt = f.host.receipts.get(id).unwrap();
    f.host
        .reconcile_owned(&receipt, ok_observation(&receipt))
        .unwrap();
    f.host
        .finish(
            p.operation_id,
            json!({"component_only":"known observation"}),
        )
        .unwrap();
    assert!(f.inventory(&p).frozen.is_some());
    assert!(
        f.host
            .receipts
            .get(p.operation_id)
            .unwrap()
            .remote_ids
            .contains_key(KEY)
    );
}

#[test]
fn lost_frozen_record_is_not_recreated_during_retained_retirement_retry() {
    let f = Fixture::new();
    let p = f.admit();
    f.host.state.lock().unwrap().effect_index_fault = Some(WriteBoundary::Created);
    let observation = json!({"component_only":"retirement persistence retry"});
    assert_eq!(
        f.host
            .retire_local(p.operation_id, observation.clone())
            .unwrap_err()
            .code,
        "effect_index"
    );
    assert_eq!(
        f.host.receipts.get(p.operation_id).unwrap().phase,
        ReceiptPhase::Prepared
    );
    let leaf = name(p.operation_id, "frozen");
    let bytes = f.host.root.read_bytes(&leaf, 256 * 1024).unwrap();
    std::fs::remove_file(f.host.root.path().join(&leaf)).unwrap();
    let error = f
        .host
        .retire_local(p.operation_id, observation.clone())
        .unwrap_err();
    assert_eq!(error.code, "record_missing");
    assert_eq!(error.message, "known frozen owner membership is missing");
    assert!(!f.host.root.path().join(&leaf).exists());
    f.host.root.create_bytes_exclusive(&leaf, &bytes).unwrap();
    f.host.retire_local(p.operation_id, observation).unwrap();
}

#[test]
fn bounded_head_and_duplicate_membership_tampering_are_rejected() {
    let f = Fixture::new();
    let p = f.admit();
    let a = f
        .host
        .mutate(
            f.request(&p, "a"),
            Some(&p),
            None,
            || Ok(()),
            |_| Ok((BTreeMap::new(), Value::Null)),
        )
        .unwrap();
    f.host
        .mutate(
            f.request(&p, "b"),
            Some(&p),
            None,
            || Ok(()),
            |_| Ok((BTreeMap::new(), Value::Null)),
        )
        .unwrap();
    let leaf = name(p.operation_id, "head");
    let original = f.host.root.read_bytes(&leaf, 256 * 1024).unwrap();
    let mut cursor: Cursor = f.host.root.read(&leaf).unwrap();
    cursor.count = MAX_RECORDS as u32 + 1;
    f.host.root.write(&leaf, &cursor).unwrap();
    assert_eq!(
        recover(&f.host.root, &f.host.receipts).unwrap_err().message,
        "owner effect index head differs from root or bound"
    );
    f.host.root.write_bytes(&leaf, &original).unwrap();
    let second = entry_name(p.operation_id, 2);
    let mut entry: Entry = f.host.root.read(&second).unwrap();
    entry.operation = a.operation_id;
    entry.receipt = receipt_request_digest(&a).unwrap();
    f.host.root.write(&second, &entry).unwrap();
    assert_eq!(
        recover(&f.host.root, &f.host.receipts).unwrap_err().message,
        "owner effect index has a gap, fork, or duplicate"
    );
}

fn rewrite_effect_owner(host: &ControlHost, operation: Uuid, changed: bool) {
    let mut receipt = host.receipts.get(operation).unwrap();
    let payload = receipt.payload_digest.clone();
    if changed {
        receipt
            .expected
            .insert("admission_operation".into(), Uuid::new_v4().to_string());
    } else {
        receipt.expected.remove("admission_operation");
    }
    host.receipts.save(&receipt).unwrap();
    assert_eq!(
        host.receipts.get(operation).unwrap().payload_digest,
        payload
    );
}

#[test]
fn reserved_remote_refuses_missing_or_changed_owner_before_any_transmit() {
    for changed in [false, true] {
        let f = Fixture::new();
        let p = f.admit();
        let RemoteReservation::Prepared(reservation) = f
            .host
            .reserve_remote(f.request(&p, "remote"), Some(p.clone()), None, |_| Ok(()))
            .unwrap()
        else {
            panic!()
        };
        let operation = reservation.operation_id();
        rewrite_effect_owner(&f.host, operation, changed);
        let calls = AtomicUsize::new(0);
        let error = f
            .host
            .launch_remote(
                reservation,
                Ok(()),
                |_| Ok(()),
                |_| {
                    calls.fetch_add(1, Ordering::SeqCst);
                },
            )
            .err()
            .unwrap();
        assert_eq!(error.code, "effect_index");
        assert_eq!(
            error.message,
            "effect receipt differs from its actual reserved identity"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            f.host.receipts.get(operation).unwrap().phase,
            ReceiptPhase::Prepared
        );
        assert!(
            f.host
                .snapshot()
                .unwrap()
                .unresolved_operations
                .contains_key(&operation)
        );
    }
}

#[test]
fn actual_local_and_remote_completion_keep_holds_when_effect_owner_is_rewritten() {
    for remote in [false, true] {
        for changed in [false, true] {
            let f = Fixture::new();
            let p = f.admit();
            let (operation, error) = if remote {
                let RemoteReservation::Prepared(reservation) = f
                    .host
                    .reserve_remote(f.request(&p, "remote"), Some(p.clone()), None, |_| Ok(()))
                    .unwrap()
                else {
                    panic!()
                };
                let (effect, ()) = f
                    .host
                    .launch_remote(reservation, Ok(()), |_| Ok(()), |_| ())
                    .unwrap();
                let operation = effect.operation;
                rewrite_effect_owner(&f.host, operation, changed);
                (
                    operation,
                    f.host
                        .complete_remote(
                            effect,
                            Ok((BTreeMap::new(), json!({"component_ack":true}))),
                        )
                        .unwrap_err(),
                )
            } else {
                let (effect, ()) = f
                    .host
                    .begin_effect(f.request(&p, "local"), &p, || Ok(()), |_| Ok(()))
                    .unwrap();
                let operation = effect.operation_id();
                rewrite_effect_owner(&f.host, operation, changed);
                (
                    operation,
                    f.host
                        .complete_effect(effect, &p, || Ok(()), json!({"component_exit":true}))
                        .unwrap_err(),
                )
            };
            assert_eq!(error.code, "effect_index");
            assert_eq!(
                error.message,
                "effect receipt differs from its actual reserved identity"
            );
            assert_eq!(
                f.host.receipts.get(operation).unwrap().phase,
                ReceiptPhase::Sent
            );
            assert!(
                f.host
                    .receipts
                    .get(operation)
                    .unwrap()
                    .observed_result
                    .is_none()
            );
            assert!(
                f.host
                    .snapshot()
                    .unwrap()
                    .unresolved_operations
                    .contains_key(&operation)
            );
        }
    }
}

#[test]
fn owned_and_operator_reconciliation_use_retained_membership_without_owner_field() {
    for operator in [false, true] {
        for changed in [false, true] {
            let f = Fixture::new();
            let p = f.admit();
            let request = f.request(&p, "unknown");
            let operation = request.request_id;
            assert_eq!(
                f.host
                    .mutate(
                        request,
                        Some(&p),
                        None,
                        || Ok(()),
                        |_| Err(Error::new("transport", "unknown"))
                    )
                    .unwrap_err()
                    .code,
                "operation_uncertain"
            );
            rewrite_effect_owner(&f.host, operation, changed);
            let receipt = f.host.receipts.get(operation).unwrap();
            let error = if operator {
                let actor = Actor {
                    identity: "component recovery".into(),
                    roles: ["recover".into()].into(),
                    principal: None,
                };
                f.host
                    .reconcile_operation(&actor, operation, |r| Ok(ok_observation(r)))
                    .unwrap_err()
            } else {
                f.host
                    .reconcile_owned(&receipt, ok_observation(&receipt))
                    .unwrap_err()
            };
            assert_eq!(error.code, "effect_index");
            assert_eq!(
                error.message,
                "effect differs from its retained indexed identity"
            );
            assert_eq!(
                f.host.receipts.get(operation).unwrap().phase,
                ReceiptPhase::Sent
            );
            assert!(
                f.host
                    .snapshot()
                    .unwrap()
                    .unresolved_operations
                    .contains_key(&operation)
            );
        }
    }
}

#[test]
fn cold_member_lookup_remains_read_only_and_refuses_lost_effect_owner() {
    let f = Fixture::new();
    let p = f.admit();
    let request = f.request(&p, "unknown");
    let operation = request.request_id;
    assert_eq!(
        f.host
            .mutate(
                request,
                Some(&p),
                None,
                || Ok(()),
                |_| Err(Error::new("transport", "unknown"))
            )
            .unwrap_err()
            .code,
        "operation_uncertain"
    );
    f.host
        .retire_local(p.operation_id, json!({"component_only":true}))
        .unwrap();
    let root = f.host.root.path().to_owned();
    let runtime = f.lease.runtime_id;
    drop(f.host);
    let host = ControlHost::recover(PrivateDir::open(&root).unwrap(), runtime).unwrap();
    assert!(host.state.lock().unwrap().effect_indexes.is_empty());
    assert!(
        host.state
            .lock()
            .unwrap()
            .effect_history
            .members
            .contains_key(&operation)
    );
    rewrite_effect_owner(&host, operation, false);
    let receipt = host.receipts.get(operation).unwrap();
    assert_eq!(
        host.reconcile_owned(&receipt, ok_observation(&receipt))
            .unwrap_err()
            .message,
        "effect differs from its retained indexed identity"
    );
    assert!(host.snapshot().unwrap().active_operations.is_empty());
    assert!(
        host.snapshot()
            .unwrap()
            .unresolved_operations
            .contains_key(&operation)
    );
}

#[test]
fn retained_frozen_reference_cannot_downgrade_to_legacy_after_policy_and_files_loss() {
    let f = Fixture::new();
    let p = f.admit();
    f.host
        .retire_local(p.operation_id, json!({"component_only":true}))
        .unwrap();
    let mut owner = f.host.receipts.get(p.operation_id).unwrap();
    assert!(owner.remote_ids.contains_key(KEY));
    owner.expected.remove(KEY);
    f.host.receipts.save(&owner).unwrap();
    for leaf in index_names(&f.host.root).unwrap() {
        std::fs::remove_file(f.host.root.path().join(leaf)).unwrap();
    }
    let error = recover(&f.host.root, &f.host.receipts).unwrap_err();
    assert_eq!(error.code, "effect_index");
    assert_eq!(
        error.message,
        "frozen owner reference lost its required index declaration"
    );
}

#[test]
fn late_actual_remote_ack_does_not_change_original_retirement_retry() {
    for readback in [false, true] {
        for boundary in [WriteBoundary::Created, WriteBoundary::Renamed] {
            let f = Fixture::new();
            let p = f.admit();
            let RemoteReservation::Prepared(reservation) = f
                .host
                .reserve_remote(f.request(&p, "remote"), Some(p.clone()), None, |_| Ok(()))
                .unwrap()
            else {
                panic!()
            };
            let calls = AtomicUsize::new(0);
            let (effect, ()) = f
                .host
                .launch_remote(
                    reservation,
                    Ok(()),
                    |_| Ok(()),
                    |_| {
                        calls.fetch_add(1, Ordering::SeqCst);
                    },
                )
                .unwrap();
            let operation = effect.operation;
            f.host.state.lock().unwrap().effect_index_fault = Some(boundary);
            let observation =
                json!({"component_only":"actual owner retirement API, no worker join claim"});
            assert_eq!(
                f.host
                    .retire_local(p.operation_id, observation.clone())
                    .unwrap_err()
                    .code,
                "effect_index"
            );
            let originally_retained = f.host.state.lock().unwrap().effect_indexes[&p.operation_id]
                .retirement
                .clone()
                .unwrap();
            assert!(
                originally_retained.observed_result.as_ref().unwrap()["unresolved_effects"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(operation))
            );
            if readback {
                assert_eq!(
                    f.host
                        .complete_remote(effect, Err(Error::new("transport", "lost ack")))
                        .unwrap_err()
                        .code,
                    "operation_uncertain"
                );
                let pending = f.host.receipts.get(operation).unwrap();
                f.host
                    .reconcile_owned(&pending, ok_observation(&pending))
                    .unwrap();
            } else {
                f.host
                    .complete_remote(
                        effect,
                        Ok((BTreeMap::new(), json!({"component_ack":"late"}))),
                    )
                    .unwrap();
            }
            assert!(
                !f.host
                    .snapshot()
                    .unwrap()
                    .unresolved_operations
                    .contains_key(&operation)
            );
            assert_eq!(
                f.host
                    .retire_local(p.operation_id, json!({"different":true}))
                    .unwrap_err()
                    .message,
                "owner retirement retry changed its original observation"
            );
            let final_receipt = f.host.retire_local(p.operation_id, observation).unwrap();
            assert_eq!(
                canonical_digest(1, &final_receipt).unwrap(),
                canonical_digest(1, &originally_retained).unwrap()
            );
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            assert!(f.host.snapshot().unwrap().active_operations.is_empty());
            recover(&f.host.root, &f.host.receipts).unwrap();
        }
    }
}

#[test]
fn missing_retired_owner_cannot_be_readmitted_or_replace_known_anchors() {
    for keep_index in [false, true] {
        for different_subject in [false, true] {
            let f = Fixture::new();
            let p = f.admit();
            f.host
                .retire_local(p.operation_id, json!({"component_only":true}))
                .unwrap();
            let (owners, members) = {
                let state = f.host.state.lock().unwrap();
                (
                    state.effect_history.owners.clone(),
                    state.effect_history.members.clone(),
                )
            };
            let leaf = format!("{}.json", p.operation_id);
            std::fs::remove_file(f.host.receipts.directory.path().join(&leaf)).unwrap();
            if !keep_index {
                for leaf in index_names(&f.host.root).unwrap() {
                    std::fs::remove_file(f.host.root.path().join(leaf)).unwrap();
                }
            }
            let before_names = index_names(&f.host.root).unwrap();
            let mut run = f.run.clone();
            if different_subject {
                run.phase.work = "different canonical work".into();
                run.phase.generation = digest("new generation under old UUID");
                run.instruction_digest = digest("changed instructions");
            }
            let calls = AtomicUsize::new(0);
            let error = f
                .host
                .admit_run(
                    &f.lease,
                    RunAdmission {
                        request_id: p.operation_id,
                        work_key: run.phase.work.clone(),
                        scope_digest: digest("different source"),
                        reference: run,
                    },
                    || {
                        calls.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    },
                )
                .unwrap_err();
            assert_eq!(error.code, "record_missing");
            assert_eq!(calls.load(Ordering::SeqCst), 0);
            assert!(!f.host.receipts.directory.path().join(&leaf).exists());
            assert_eq!(index_names(&f.host.root).unwrap(), before_names);
            let state = f.host.state.lock().unwrap();
            assert_eq!(state.effect_history.owners, owners);
            assert_eq!(state.effect_history.members, members);
            assert!(!state.permits.contains_key(&p.operation_id));
            assert!(!state.open);
        }
    }
}

#[test]
fn missing_known_effect_is_not_recreated_by_fresh_prepare() {
    for different_payload in [false, true] {
        let f = Fixture::new();
        let p = f.admit();
        let original_request = f.request(&p, "sync");
        let receipt = f
            .host
            .mutate(
                original_request.clone(),
                Some(&p),
                None,
                || Ok(()),
                |_| Ok((BTreeMap::new(), json!({"component_ack":true}))),
            )
            .unwrap();
        let (owners, members) = {
            let state = f.host.state.lock().unwrap();
            (
                state.effect_history.owners.clone(),
                state.effect_history.members.clone(),
            )
        };
        let leaf = format!("{}.json", receipt.operation_id);
        std::fs::remove_file(f.host.receipts.directory.path().join(&leaf)).unwrap();
        let mut request = original_request;
        if different_payload {
            request.kind = "another effect kind".into();
            request.payload = json!({"different":true});
        }
        let calls = AtomicUsize::new(0);
        let error = f
            .host
            .mutate(
                request,
                Some(&p),
                None,
                || Ok(()),
                |_| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok((BTreeMap::new(), Value::Null))
                },
            )
            .unwrap_err();
        assert_eq!(error.code, "record_missing");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(!f.host.receipts.directory.path().join(&leaf).exists());
        let state = f.host.state.lock().unwrap();
        assert_eq!(state.effect_history.owners, owners);
        assert_eq!(state.effect_history.members, members);
        assert!(state.holds.contains_key(&receipt.operation_id));
    }
}

#[test]
fn intact_known_owner_preserves_request_conflict_and_defensive_creation_never_replaces_it() {
    let f = Fixture::new();
    let p = f.admit();
    let original = f
        .host
        .retire_local(p.operation_id, json!({"component_only":true}))
        .unwrap();
    let (owners, members) = {
        let state = f.host.state.lock().unwrap();
        (
            state.effect_history.owners.clone(),
            state.effect_history.members.clone(),
        )
    };
    let mut run = f.run.clone();
    run.phase.work = "other work under same UUID".into();
    run.phase.generation = digest("other generation");
    let error = f
        .host
        .admit_run(
            &f.lease,
            RunAdmission {
                request_id: p.operation_id,
                work_key: run.phase.work.clone(),
                scope_digest: digest("new scope"),
                reference: run,
            },
            || Ok(()),
        )
        .unwrap_err();
    assert_eq!(error.code, "request_conflict");
    assert_eq!(
        canonical_digest(1, &f.host.receipts.get(p.operation_id).unwrap()).unwrap(),
        canonical_digest(1, &original).unwrap()
    );
    let mut state = f.host.state.lock().unwrap();
    assert_eq!(
        f.host
            .create_effect_index(&mut state, &original)
            .unwrap_err()
            .message,
        "new owner cannot replace retained indexed identity"
    );
    assert_eq!(state.effect_history.owners, owners);
    assert_eq!(state.effect_history.members, members);
    assert!(!state.permits.contains_key(&p.operation_id));
}

#[test]
fn operator_kind_cannot_recreate_a_deleted_known_owner_uuid() {
    let f = Fixture::new();
    let p = f.admit();
    f.host
        .retire_local(p.operation_id, json!({"component_only":true}))
        .unwrap();
    let before = f.host.root.read_bytes("intent.json", 256 * 1024).unwrap();
    let generation = f.host.snapshot().unwrap().intent.generation;
    let leaf = format!("{}.json", p.operation_id);
    std::fs::remove_file(f.host.receipts.directory.path().join(&leaf)).unwrap();
    let request = MutationRequest {
        request_id: p.operation_id,
        runtime_id: f.lease.runtime_id,
        actor: Actor {
            identity: "component human".into(),
            roles: ["lifecycle".into()].into(),
            principal: None,
        },
        kind: "stop".into(),
        canonical_targets: vec![f.lease.runtime_id.to_string()],
        expected: BTreeMap::new(),
        payload: Value::Null,
    };
    assert_eq!(
        f.host
            .lifecycle(request, Lifecycle::Stop, generation)
            .unwrap_err()
            .code,
        "record_missing"
    );
    assert_eq!(
        f.host.root.read_bytes("intent.json", 256 * 1024).unwrap(),
        before
    );
    assert!(!f.host.receipts.directory.path().join(&leaf).exists());
    assert!(
        f.host
            .state
            .lock()
            .unwrap()
            .effect_history
            .owners
            .contains_key(&p.operation_id)
    );
}

#[test]
fn indexed_replay_failure_preserves_sent_resources_against_an_already_active_owner() {
    // Actual Control reservations, Sent transition, callback and protected reads.
    // No provider response, worker completion or reconciliation is fabricated.
    for failure in ["missing", "changed_identity", "conflicting_request"] {
        let f = Fixture::new();
        let a = f.admit();
        let mut b_run = f.run.clone();
        b_run.phase.work = format!("other-parent:{}", Uuid::new_v4());
        let b = f
            .host
            .admit_run(
                &f.lease,
                RunAdmission {
                    request_id: Uuid::new_v4(),
                    work_key: b_run.phase.work.clone(),
                    scope_digest: digest("other owned scope"),
                    reference: b_run,
                },
                || Ok(()),
            )
            .unwrap();
        let mut request = f.request(&a, "pending remote");
        request.canonical_targets = vec![
            "component:shared-canonical-resource".into(),
            "component:second-canonical-resource".into(),
        ];
        let RemoteReservation::Prepared(reserved) = f
            .host
            .reserve_remote(request.clone(), Some(a.clone()), None, |_| Ok(()))
            .unwrap()
        else {
            panic!("fresh actual A reservation")
        };
        let a_calls = AtomicUsize::new(0);
        let (_in_flight_a, ()) = f
            .host
            .launch_remote(
                reserved,
                Ok(()),
                |_| Ok(()),
                |_| {
                    a_calls.fetch_add(1, Ordering::SeqCst);
                },
            )
            .unwrap();
        let original = f.host.receipts.get(request.request_id).unwrap();
        assert_eq!(original.phase, ReceiptPhase::Sent);
        let leaf = format!("{}.json", request.request_id);
        let bytes = f
            .host
            .receipts
            .directory
            .read_bytes(&leaf, MAX_BYTES as u64)
            .unwrap();
        let before = f.host.snapshot().unwrap().unresolved_operations[&request.request_id].clone();
        let mut expected_hold = request.canonical_targets.clone();
        expected_hold.push(a.work_key.clone());
        assert_eq!(before, expected_hold);
        let mut replay = request.clone();
        match failure {
            "missing" => {
                std::fs::remove_file(f.host.receipts.directory.path().join(&leaf)).unwrap();
            }
            "changed_identity" => {
                let mut changed = original.clone();
                changed.expected.remove("admission_operation");
                f.host.receipts.directory.write(&leaf, &changed).unwrap();
            }
            "conflicting_request" => {
                replay.canonical_targets = vec!["component:unrelated-replay-target".into()];
            }
            _ => unreachable!(),
        }
        let replay_calls = AtomicUsize::new(0);
        let error = f
            .host
            .reserve_remote(replay, Some(a.clone()), None, |_| {
                replay_calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .err()
            .unwrap();
        assert_eq!(
            error.code,
            match failure {
                "missing" => "record_missing",
                "changed_identity" => "effect_index",
                "conflicting_request" => "request_conflict",
                _ => unreachable!(),
            }
        );
        assert_eq!(replay_calls.load(Ordering::SeqCst), 0);
        let after = f.host.snapshot().unwrap();
        if failure == "conflicting_request" {
            expected_hold.push("component:unrelated-replay-target".into());
        }
        assert_eq!(
            after.unresolved_operations[&request.request_id],
            expected_hold
        );
        assert!(!after.admission_open);
        assert!(after.fenced_operations.contains(&a.operation_id));
        assert!(!after.fenced_operations.contains(&b.operation_id));

        // Restore precisely the original Sent bytes. This is neither an ACK
        // nor a readback, so the original hold and pending owner must remain.
        f.host
            .receipts
            .directory
            .write_bytes(&leaf, &bytes)
            .unwrap();
        assert_eq!(
            f.host
                .receipts
                .directory
                .read_bytes(&leaf, MAX_BYTES as u64)
                .unwrap(),
            bytes,
        );
        assert_eq!(
            f.host.receipts.get(request.request_id).unwrap().phase,
            ReceiptPhase::Sent
        );
        assert!(
            f.host
                .snapshot()
                .unwrap()
                .pending_external_effects
                .contains(&request.request_id)
        );
        f.host.validate_permit(&b, || Ok(())).unwrap();
        let mut overlap = f.request(&b, "other owner overlap");
        overlap.canonical_targets = vec![request.canonical_targets[0].clone()];
        let b_operation = overlap.request_id;
        let b_calls = AtomicUsize::new(0);
        let error = f
            .host
            .reserve_remote(overlap, Some(b.clone()), None, |_| {
                b_calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .err()
            .unwrap();
        assert_eq!(error.code, "reconciliation_required");
        assert_eq!(
            error.message,
            "an overlapping or saturated remote operation is pending"
        );
        assert_eq!(b_calls.load(Ordering::SeqCst), 0);
        assert_eq!(a_calls.load(Ordering::SeqCst), 1);
        let refused = f.host.receipts.get(b_operation).unwrap();
        assert_eq!(refused.phase, ReceiptPhase::Reconciled);
        assert_eq!(
            refused.observed_result.as_ref().unwrap()["effect"],
            "not_sent"
        );
        let final_state = f.host.snapshot().unwrap();
        assert_eq!(
            final_state.unresolved_operations[&request.request_id],
            expected_hold
        );
        assert!(
            final_state
                .pending_external_effects
                .contains(&request.request_id)
        );
        assert!(!final_state.pending_external_effects.contains(&b_operation));
    }
}
