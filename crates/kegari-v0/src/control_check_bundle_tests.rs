//! Real Control/private-storage ordering; callbacks are explicit component
//! observations, never processes, worker capabilities or cold launch authority.
use super::*;
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};

fn digest(value: &str) -> String {
    canonical_digest(1, &value).unwrap()
}
struct Fixture {
    _root: tempfile::TempDir,
    host: ControlHost,
    permit: Permit,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let runtime = Uuid::new_v4();
        let host = ControlHost::setup(
            PrivateDir::create(&root.path().canonicalize().unwrap().join("control")).unwrap(),
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
            owner_generation: 1,
            owner_id: Uuid::new_v4(),
        };
        host.attach_reconciled(lease.clone()).unwrap();
        let work = format!("component-parent:{}", Uuid::new_v4());
        let permit = host
            .admit_run(
                &lease,
                RunAdmission {
                    request_id: Uuid::new_v4(),
                    work_key: work.clone(),
                    scope_digest: digest("scope"),
                    reference: RunReference {
                        workspace: Uuid::new_v4(),
                        account: "component-account".into(),
                        normalized_state: None,
                        phase: crate::scheduler::PhaseKey {
                            work,
                            generation: digest("generation"),
                            phase: "parent_evidence_review".into(),
                            retry_grant: None,
                        },
                        profile_digest: digest("profile"),
                        instruction_digest: digest("instruction"),
                        max_attempts: 1,
                        max_turns: 1,
                        max_review_rounds: 1,
                        attempt: 1,
                    },
                },
                || Ok(()),
            )
            .unwrap();
        Self {
            _root: root,
            host,
            permit,
        }
    }
    fn request(&self, id: Uuid) -> MutationRequest {
        MutationRequest {
            request_id: id,
            runtime_id: self.permit.lease.runtime_id,
            actor: Actor {
                identity: format!("owner:{}", self.permit.lease.owner_id),
                roles: BTreeSet::new(),
                principal: None,
            },
            kind: "worker_validation".into(),
            canonical_targets: vec!["component-repository".into()],
            expected: BTreeMap::new(),
            payload: json!({"name":"test","kind":"integrated"}),
        }
    }
    fn plan(&self, id: Uuid) -> CheckBundlePlan {
        CheckBundlePlan::new(
            id,
            self.permit.operation_id,
            self.permit.lease.runtime_id,
            digest("binding"),
            digest("isolation"),
            vec![
                CheckBundleStepSpec {
                    process_id: Uuid::new_v4(),
                    kind: CheckMemberKind::Service,
                    service: Some("server".into()),
                    launch_digest: digest("server argv"),
                },
                CheckBundleStepSpec {
                    process_id: Uuid::new_v4(),
                    kind: CheckMemberKind::Readiness,
                    service: Some("server".into()),
                    launch_digest: digest("readiness argv"),
                },
                CheckBundleStepSpec {
                    process_id: Uuid::new_v4(),
                    kind: CheckMemberKind::Check,
                    service: None,
                    launch_digest: digest("check argv"),
                },
            ],
        )
        .unwrap()
    }
    fn begin(&self) -> InFlightCheckBundle {
        let id = Uuid::new_v4();
        self.host
            .begin_check_bundle(
                self.request(id),
                &self.permit,
                self.plan(id),
                || Ok(()),
                |_| Ok(()),
            )
            .unwrap()
            .0
    }
}

#[test]
fn ordered_bundle_steps_are_durable_before_callback_without_releasing_the_work_hold() {
    let f = Fixture::new();
    let mut bundle = f.begin();
    let id = bundle.plan.effect_id();
    let calls = AtomicUsize::new(0);
    for ordinal in 0..3 {
        f.host
            .advance_check_bundle(
                &mut bundle,
                &f.permit,
                ordinal,
                || Ok(()),
                |launch| {
                    let directory = PrivateDir::open(&f.host.root.path().join(DIRECTORY))?;
                    let saved: Attempt = serde_json::from_slice(
                        &directory.read_bytes(&format!("{id}-{ordinal}.json"), MAX_BYTES)?,
                    )
                    .map_err(|_| invalid("decode actual create-exclusive attempt"))?;
                    assert_eq!(saved.step.process_id, launch.process_id());
                    assert_eq!(saved.plan_digest, launch.plan_digest());
                    assert_eq!(saved.owner, launch.permit().operation_id);
                    assert_eq!(launch.member().bundle_effect, id);
                    assert_eq!(launch.ordinal(), ordinal);
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(f.host.operation(id).unwrap().phase, ReceiptPhase::Sent);
        assert!(f.host.snapshot().unwrap().unresolved_operations[&id].contains(&f.permit.work_key));
    }
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    let error = f
        .host
        .advance_check_bundle(
            &mut bundle,
            &f.permit,
            2,
            || Ok(()),
            |_| {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        )
        .unwrap_err();
    assert_eq!(error.code, "check_bundle_identity");
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[test]
fn initial_bundle_requires_the_complete_registered_permit_before_any_effect_write() {
    let f = Fixture::new();
    let calls = AtomicUsize::new(0);
    for change_time in [true, false] {
        let id = Uuid::new_v4();
        let mut changed = f.permit.clone();
        if change_time {
            changed.registered_at += chrono::Duration::seconds(1);
        } else {
            changed.control_generation += 1;
        }
        let error = f
            .host
            .begin_check_bundle(
                f.request(id),
                &changed,
                f.plan(id),
                || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                },
                |_| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                },
            )
            .err()
            .unwrap();
        assert_eq!(error.code, "check_bundle_identity");
        assert_eq!(f.host.operation(id).unwrap_err().code, "record_missing");
        assert!(
            f.host
                .root
                .child(DIRECTORY)
                .unwrap()
                .records()
                .unwrap()
                .is_empty()
        );
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn only_original_presend_refusal_records_zero_attempt_closure() {
    let f = Fixture::new();
    let id = Uuid::new_v4();
    let calls = AtomicUsize::new(0);
    let error = f
        .host
        .begin_check_bundle(
            f.request(id),
            &f.permit,
            f.plan(id),
            || {
                Err(Error::new(
                    "stale_result",
                    "component gate refuses before any callback",
                ))
            },
            |_| {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        )
        .err()
        .unwrap();
    assert_eq!(error.code, "stale_result");
    let receipt = f.host.operation(id).unwrap();
    assert_eq!(receipt.phase, ReceiptPhase::Reconciled);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        f.host.check_bundle_attempted_members(&receipt).unwrap(),
        Some(Vec::new())
    );
    f.host
        .verify_check_bundle_inventory(&[receipt], &[])
        .unwrap();

    let id = Uuid::new_v4();
    let error = f
        .host
        .begin_check_bundle(
            f.request(id),
            &f.permit,
            f.plan(id),
            || Ok(()),
            |_| {
                calls.fetch_add(1, Ordering::SeqCst);
                Err::<(), _>(Error::new("component_owner", "opaque registration failure"))
            },
        )
        .err()
        .unwrap();
    assert_eq!(error.code, "operation_uncertain");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let receipt = f.host.operation(id).unwrap();
    assert_eq!(receipt.phase, ReceiptPhase::Sent);
    assert!(!receipt.remote_ids.contains_key(CLOSURE_KEY));
    assert_eq!(
        f.host
            .check_bundle_attempted_members(&receipt)
            .unwrap_err()
            .code,
        "operation_uncertain"
    );
    assert!(
        f.host
            .snapshot()
            .unwrap()
            .unresolved_operations
            .contains_key(&id)
    );
}

#[test]
fn bundle_continuation_rejects_skips_wrong_original_permit_and_other_holds() {
    let f = Fixture::new();
    let mut bundle = f.begin();
    let calls = AtomicUsize::new(0);
    assert!(
        f.host
            .advance_check_bundle(
                &mut bundle,
                &f.permit,
                1,
                || Ok(()),
                |_| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            )
            .is_err()
    );
    let mut wrong = f.permit.clone();
    wrong.registered_at += chrono::Duration::seconds(1);
    assert!(
        f.host
            .advance_check_bundle(
                &mut bundle,
                &wrong,
                0,
                || Ok(()),
                |_| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            )
            .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(bundle.poisoned);

    let f = Fixture::new();
    let mut bundle = f.begin();
    // A separate actual unresolved receipt is introduced through the existing
    // operator-reconciliation path's retained hold representation. This is not
    // a process or a completed effect.
    let id = Uuid::new_v4();
    let request = f.request(id);
    let (mut receipt, _) = f
        .host
        .receipts
        .prepare(&request, f.permit.lease.owner_generation)
        .unwrap();
    f.host.receipts.mark_sent(&mut receipt).unwrap();
    f.host
        .state
        .lock()
        .unwrap()
        .holds
        .insert(id, receipt.canonical_targets.clone());
    let error = f
        .host
        .advance_check_bundle(
            &mut bundle,
            &f.permit,
            0,
            || Ok(()),
            |_| {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        )
        .unwrap_err();
    assert_eq!(error.code, "reconciliation_required");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn removed_or_substituted_attempt_and_changed_plan_refuse_further_dispatch() {
    for mutation in 0..3 {
        let f = Fixture::new();
        let mut bundle = f.begin();
        f.host
            .advance_check_bundle(&mut bundle, &f.permit, 0, || Ok(()), |_| Ok(()))
            .unwrap();
        let id = bundle.plan.effect_id();
        if mutation == 0 {
            std::fs::remove_file(bundle.directory.path().join(bundle.attempts[0].leaf())).unwrap();
        } else if mutation == 1 {
            let mut altered = bundle.attempts[0].clone();
            altered.step.process_id = Uuid::new_v4();
            bundle.directory.write(&altered.leaf(), &altered).unwrap();
        } else {
            let mut receipt = f.host.operation(id).unwrap();
            receipt
                .expected
                .insert(PLAN_KEY.into(), f.plan(id).encoded().unwrap());
            f.host.receipts.save(&receipt).unwrap();
        }
        let calls = AtomicUsize::new(0);
        assert!(
            f.host
                .advance_check_bundle(
                    &mut bundle,
                    &f.permit,
                    1,
                    || Ok(()),
                    |_| {
                        calls.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    }
                )
                .is_err()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(
            f.host
                .snapshot()
                .unwrap()
                .unresolved_operations
                .contains_key(&id)
        );
    }
}

#[test]
fn attempted_write_or_launch_failure_never_replays_or_clears_the_original_hold() {
    for fail_write in [true, false] {
        let f = Fixture::new();
        let mut bundle = f.begin();
        let id = bundle.plan.effect_id();
        if fail_write {
            bundle
                .directory
                .create_bytes_exclusive(&format!("{id}-0.json"), b"existing unrelated bytes")
                .unwrap();
        }
        let calls = AtomicUsize::new(0);
        assert!(
            f.host
                .advance_check_bundle(
                    &mut bundle,
                    &f.permit,
                    0,
                    || Ok(()),
                    |_| {
                        calls.fetch_add(1, Ordering::SeqCst);
                        Err::<(), _>(Error::new(
                            "component_start",
                            "explicit component launch failure",
                        ))
                    }
                )
                .is_err()
        );
        assert_eq!(calls.load(Ordering::SeqCst), usize::from(!fail_write));
        assert!(
            f.host
                .advance_check_bundle(
                    &mut bundle,
                    &f.permit,
                    0,
                    || Ok(()),
                    |_| {
                        calls.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    }
                )
                .is_err()
        );
        assert_eq!(calls.load(Ordering::SeqCst), usize::from(!fail_write));
        assert!(
            f.host
                .snapshot()
                .unwrap()
                .unresolved_operations
                .contains_key(&id)
        );
    }
}

#[test]
fn generic_effect_paths_cannot_admit_or_complete_a_marked_bundle() {
    let f = Fixture::new();
    let id = Uuid::new_v4();
    let mut request = f.request(id);
    request
        .expected
        .insert(PLAN_KEY.into(), f.plan(id).encoded().unwrap());
    let calls = AtomicUsize::new(0);
    assert!(
        f.host
            .begin_effect(
                request,
                &f.permit,
                || Ok(()),
                |_| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            )
            .is_err()
    );
    assert_eq!(f.host.operation(id).unwrap_err().code, "record_missing");
    let bundle = f.begin();
    let id = bundle.plan.effect_id();
    assert!(
        f.host
            .complete_effect(
                bundle.effect,
                &f.permit,
                || Ok(()),
                json!({"successful":true})
            )
            .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(
        f.host
            .snapshot()
            .unwrap()
            .unresolved_operations
            .contains_key(&id)
    );

    let f = Fixture::new();
    let id = Uuid::new_v4();
    let (effect, ()) = f
        .host
        .begin_effect(f.request(id), &f.permit, || Ok(()), |_| Ok(()))
        .unwrap();
    let receipt = f
        .host
        .complete_effect(
            effect,
            &f.permit,
            || Ok(()),
            json!({"component":"ordinary"}),
        )
        .unwrap();
    assert!(!receipt.expected.contains_key(PLAN_KEY));
    assert!(receipt.remote_ids.is_empty());
    assert_eq!(
        receipt.observed_result,
        Some(json!({"component":"ordinary"}))
    );
}

#[test]
fn malformed_plan_cannot_change_the_exact_service_readiness_check_membership() {
    let f = Fixture::new();
    let plan = f.plan(Uuid::new_v4());
    for mutation in 0..5 {
        let mut record = plan.record.clone();
        match mutation {
            0 => record.steps[1].service = Some("another-service".into()),
            1 => record.steps[2].process_id = record.steps[0].process_id,
            2 => record.steps[0].ordinal = 1,
            3 => record.steps[2].kind = CheckMemberKind::Service,
            _ => record.steps.pop().map(|_| ()).unwrap(),
        }
        assert!(CheckBundlePlan::from_record(record).is_err());
    }
}

#[test]
fn closed_membership_reader_requires_exact_tip_and_all_attempted_records() {
    let f = Fixture::new();
    let mut bundle = f.begin();
    let id = bundle.plan.effect_id();
    let pending = f.host.operation(id).unwrap();
    assert_eq!(
        f.host
            .check_bundle_attempted_members(&pending)
            .unwrap_err()
            .code,
        "operation_uncertain"
    );
    f.host
        .advance_check_bundle(&mut bundle, &f.permit, 0, || Ok(()), |_| Ok(()))
        .unwrap();
    // This is explicitly synthetic terminal metadata for the read-only data
    // parser. No settled-owner constructor, process or execution proof exists.
    let mut terminal = f.host.operation(id).unwrap();
    terminal.phase = ReceiptPhase::Acknowledged;
    terminal.remote_ids.insert(
        CLOSURE_KEY.into(),
        serde_json::to_string(&Closure {
            schema_version: 1,
            plan_digest: bundle.plan.digest().into(),
            attempted: 1,
            tip: Some(canonical_digest(1, &bundle.attempts[0]).unwrap()),
        })
        .unwrap(),
    );
    let expected = f
        .host
        .check_bundle_attempted_members(&terminal)
        .unwrap()
        .unwrap();
    assert_eq!(expected.len(), 1);
    assert_eq!(expected[0].0, bundle.plan.steps()[0].process_id());
    let mut missing_plan = terminal.clone();
    missing_plan.expected.remove(PLAN_KEY);
    assert!(
        f.host
            .check_bundle_attempted_members(&missing_plan)
            .is_err()
    );
    let mut wrong = terminal.clone();
    let mut closure: Closure = serde_json::from_str(&wrong.remote_ids[CLOSURE_KEY]).unwrap();
    closure.tip = Some(digest("wrong tip"));
    wrong
        .remote_ids
        .insert(CLOSURE_KEY.into(), serde_json::to_string(&closure).unwrap());
    assert!(f.host.check_bundle_attempted_members(&wrong).is_err());
    let first = bundle.attempts[0].clone();
    std::fs::remove_file(bundle.directory.path().join(first.leaf())).unwrap();
    assert_eq!(
        f.host
            .check_bundle_attempted_members(&terminal)
            .unwrap_err()
            .code,
        "record_missing"
    );
    bundle
        .directory
        .create_bytes_exclusive(&first.leaf(), &serde_json::to_vec(&first).unwrap())
        .unwrap();
    assert_eq!(
        f.host
            .check_bundle_attempted_members(&terminal)
            .unwrap()
            .unwrap(),
        expected
    );
    bundle
        .directory
        .create_bytes_exclusive(&format!("{id}-1.json"), b"unexpected attempted suffix")
        .unwrap();
    assert!(f.host.check_bundle_attempted_members(&terminal).is_err());
}

#[test]
fn fresh_gate_failure_and_fencing_prevent_the_next_actual_callback() {
    for fence in [false, true] {
        let f = Fixture::new();
        let mut bundle = f.begin();
        f.host
            .advance_check_bundle(&mut bundle, &f.permit, 0, || Ok(()), |_| Ok(()))
            .unwrap();
        if fence {
            f.host.fence(f.permit.operation_id).unwrap();
        }
        let calls = AtomicUsize::new(0);
        assert!(
            f.host
                .advance_check_bundle(
                    &mut bundle,
                    &f.permit,
                    1,
                    || Err(Error::new("stale_result", "component authority changed")),
                    |_| {
                        calls.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    }
                )
                .is_err()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(bundle.attempts.len(), 1);
        assert!(
            f.host
                .snapshot()
                .unwrap()
                .unresolved_operations
                .contains_key(&bundle.plan.effect_id())
        );
    }
}

#[test]
fn inventory_refuses_absent_directory_rows_removed_membership_duplicates_and_wrong_owner() {
    let f = Fixture::new();
    let mut bundle = f.begin();
    f.host
        .advance_check_bundle(&mut bundle, &f.permit, 0, || Ok(()), |_| Ok(()))
        .unwrap();
    // Data-only historical reader fixture after actual Control attempt. This
    // terminal DTO and process DTO do not establish an OS join or settlement.
    let mut terminal = f.host.operation(bundle.plan.effect_id()).unwrap();
    terminal.phase = ReceiptPhase::Acknowledged;
    terminal.remote_ids.insert(
        CLOSURE_KEY.into(),
        serde_json::to_string(&Closure {
            schema_version: 1,
            plan_digest: bundle.plan.digest().into(),
            attempted: 1,
            tip: Some(canonical_digest(1, &bundle.attempts[0]).unwrap()),
        })
        .unwrap(),
    );
    let executable =
        kegari_platform::BinaryIdentity::inspect(&std::env::current_exe().unwrap(), false).unwrap();
    let original = crate::process_records::SpawnBinding {
        operation_id: f.permit.operation_id,
        effect_id: bundle.plan.steps()[0].process_id(),
        lease: Some(f.permit.lease.clone()),
        control_owner: None,
        work_key: f.permit.work_key.clone(),
        scope_digest: f.permit.scope_digest.clone(),
        cwd: f._root.path().canonicalize().unwrap(),
        executable: executable.clone(),
        observed_executables: vec![executable],
        argv_digest: digest("component process argv"),
        kind: "repository_command".into(),
        check_bundle: Some(bundle.plan.member(&bundle.plan.steps()[0])),
    };
    let receipts = [terminal];
    f.host
        .verify_check_bundle_inventory(&receipts, std::slice::from_ref(&original))
        .unwrap();
    assert!(
        f.host
            .verify_check_bundle_inventory(&receipts, &[])
            .is_err()
    );
    assert!(
        f.host
            .verify_check_bundle_inventory(&[], std::slice::from_ref(&original))
            .is_err()
    );
    assert!(
        f.host
            .verify_check_bundle_inventory(&receipts, &[original.clone(), original.clone()])
            .is_err()
    );
    for mutation in 0..4 {
        let mut changed = original.clone();
        match mutation {
            0 => changed.check_bundle = None,
            1 => changed.operation_id = Uuid::new_v4(),
            2 => changed.lease.as_mut().unwrap().owner_id = Uuid::new_v4(),
            _ => changed.effect_id = Uuid::new_v4(),
        }
        assert!(
            f.host
                .verify_check_bundle_inventory(&receipts, &[changed])
                .is_err()
        );
    }
}
