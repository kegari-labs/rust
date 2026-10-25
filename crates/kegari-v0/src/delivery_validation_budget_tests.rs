//! Synthetic receipt-observation tests only. These do not create source/check
//! capabilities, runtime joined entries, native grants, or actual admissions.
use super::*;
fn hash(value: &str) -> String {
    crate::digest::bytes_digest(value.as_bytes())
}
fn run() -> RunReference {
    RunReference {
        workspace: Uuid::from_u128(2),
        account: "fixture".into(),
        normalized_state: None,
        phase: PhaseKey {
            work: hash("child"),
            generation: hash("generation"),
            phase: "child_local_validation".into(),
            retry_grant: None,
        },
        profile_digest: hash("profile"),
        instruction_digest: hash("instruction"),
        max_attempts: 2048,
        max_turns: 7,
        max_review_rounds: 3,
        attempt: 1,
    }
}
fn receipt(id: u128, reference: &RunReference) -> Receipt {
    Receipt {
        operation_id: Uuid::from_u128(id),
        request_id: Uuid::from_u128(id),
        runtime_id: Uuid::from_u128(1),
        owner_generation: 1,
        actor: "owner:fixture".into(),
        kind: "admission".into(),
        canonical_targets: vec![reference.phase.work.clone()],
        payload_digest: hash("request"),
        expected: [
            ("scope".into(), hash("scope")),
            ("run".into(), serde_json::to_string(reference).unwrap()),
        ]
        .into(),
        phase: ReceiptPhase::Reconciled,
        remote_ids: BTreeMap::new(),
        observed_result: Some(
            json!({"local_execution":"exited","unresolved_effects":[],"observation":{"outcome":"completed"}}),
        ),
        cancellation: "none".into(),
        updated_at: chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
    }
}
#[test]
fn accounted_validation_prefix_keeps_every_grant_namespace_and_whole_record() {
    let original = run();
    let mut granted = original.clone();
    granted.phase.retry_grant = Some(Uuid::from_u128(90));
    let old = receipt(10, &original);
    let current = receipt(11, &granted);
    let receipts = vec![old.clone(), current.clone()];
    let retained = receipts
        .iter()
        .map(|r| (r.operation_id, canonical_digest(1, r).unwrap()))
        .collect::<BTreeMap<_, _>>();
    verify_accounted_validation_history(&receipts, old.runtime_id, &granted, &retained).unwrap();
    let mut missing = retained.clone();
    missing.remove(&old.operation_id);
    assert_eq!(
        verify_accounted_validation_history(&receipts, old.runtime_id, &granted, &missing)
            .unwrap_err()
            .code,
        "validation_source_lineage"
    );
    let mut changed = receipts.clone();
    changed[0].cancellation = "requested".into();
    assert_eq!(
        verify_accounted_validation_history(&changed, old.runtime_id, &granted, &retained)
            .unwrap_err()
            .code,
        "validation_source_lineage"
    );
    let mut later_grant = granted.clone();
    later_grant.phase.retry_grant = Some(Uuid::from_u128(91));
    let mut extra = receipts;
    extra.push(receipt(12, &later_grant));
    // No native-grant list is involved: deleting its native comment cannot
    // erase this newer actual namespace from the protected receipt inventory.
    assert_eq!(
        verify_accounted_validation_history(&extra, old.runtime_id, &granted, &retained)
            .unwrap_err()
            .code,
        "validation_source_lineage"
    );
}
#[test]
fn duplicate_validation_identity_and_unrelated_workspace_are_distinct() {
    let expected = run();
    let one = receipt(10, &expected);
    let retained = BTreeMap::from([(one.operation_id, canonical_digest(1, &one).unwrap())]);
    assert_eq!(
        verify_accounted_validation_history(
            &[one.clone(), one.clone()],
            one.runtime_id,
            &expected,
            &retained
        )
        .unwrap_err()
        .code,
        "validation_source_lineage"
    );
    let mut other = expected.clone();
    other.workspace = Uuid::from_u128(3);
    let mut receipts = vec![one.clone(), receipt(11, &other)];
    let mut foreign = receipt(12, &expected);
    foreign.runtime_id = Uuid::from_u128(99);
    receipts.push(foreign);
    verify_accounted_validation_history(&receipts, one.runtime_id, &expected, &retained).unwrap();
}
#[test]
fn newer_implementation_grant_changes_source_inventory_without_changing_configured_caps() {
    let reference = run();
    let mut implementation = reference.clone();
    implementation.phase.phase = "implementation".into();
    let first = receipt(20, &implementation);
    let initial =
        implementation_inventory(std::slice::from_ref(&first), first.runtime_id, &reference)
            .unwrap();
    implementation.phase.retry_grant = Some(Uuid::from_u128(92));
    let second = receipt(21, &implementation);
    assert_ne!(
        initial,
        implementation_inventory(&[first.clone(), second], first.runtime_id, &reference).unwrap()
    );
    // The observation helper does not clamp the configured budget to an
    // arbitrary history depth, reset an ordinal, or mutate the original input.
    assert_eq!(reference.max_attempts, 2048);
    assert_eq!(reference.attempt, 1);
    let mut unrelated = receipt(22, &implementation);
    unrelated.runtime_id = Uuid::from_u128(3);
    assert_eq!(
        initial,
        implementation_inventory(&[first.clone(), unrelated], first.runtime_id, &reference)
            .unwrap()
    );
}

#[test]
fn retained_preparation_requires_successful_known_exit_before_closing_ancestry() {
    let model = receipt(10, &run());
    let header = crate::control::PreparationReference {
        worker_operation: model.operation_id,
        workspace: run().workspace,
        account: run().account,
        phase: run().phase,
    };
    let mut prep = model.clone();
    prep.operation_id = header.operation_id().unwrap();
    prep.request_id = prep.operation_id;
    prep.expected.remove("run");
    prep.expected.insert(
        "worker_preparation".into(),
        serde_json::to_string(&header).unwrap(),
    );
    prep.observed_result = Some(json!({"local_execution":"exited","unresolved_effects":[],
        "observation":{"phase":"worker_preparation","result":"prepared"}}));
    let mut retained = BTreeMap::new();
    retain_preparation(
        &mut retained,
        &[model.clone(), prep.clone()],
        model.operation_id,
    )
    .unwrap();
    assert_eq!(
        retained.get(&prep.operation_id),
        Some(&canonical_digest(1, &prep).unwrap())
    );
    for mutation in 0..8 {
        let mut changed = prep.clone();
        match mutation {
            0 => changed.observed_result = None,
            1 => changed.observed_result.as_mut().unwrap()["local_execution"] = json!("unknown"),
            2 => {
                changed.observed_result.as_mut().unwrap()["unresolved_effects"] =
                    json!(["unknown effect"])
            }
            3 => {
                changed
                    .observed_result
                    .as_mut()
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .remove("unresolved_effects");
            }
            4 => changed.observed_result.as_mut().unwrap()["observation"]["phase"] = json!("other"),
            5 => {
                changed.observed_result.as_mut().unwrap()["observation"]["result"] = json!("failed")
            }
            6 => changed.owner_generation += 1,
            _ => changed.phase = ReceiptPhase::Prepared,
        }
        let mut retained = BTreeMap::new();
        assert!(
            retain_preparation(&mut retained, &[model.clone(), changed], model.operation_id)
                .is_err(),
            "mutation {mutation}"
        );
        assert!(retained.is_empty());
    }
}
