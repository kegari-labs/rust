use super::*;

// Receipt DATA predicate tests. These helpers never register a worker, invent
// a completion, mint an origin, or claim that any process actually exited.
fn owner() -> Receipt {
    let operation = Uuid::new_v4();
    Receipt {
        operation_id: operation,
        request_id: operation,
        runtime_id: Uuid::new_v4(),
        owner_generation: 4,
        actor: "fixture-owner".into(),
        kind: "admission".into(),
        canonical_targets: vec!["exact-parent-work".into()],
        payload_digest: bytes_digest(b"fixture admission data"),
        expected: BTreeMap::from([("scope".into(), bytes_digest(b"scope"))]),
        phase: ReceiptPhase::Reconciled,
        remote_ids: BTreeMap::new(),
        observed_result: Some(
            json!({"local_execution":"exited","unresolved_effects":[],"observation":{"outcome":"failed"}}),
        ),
        cancellation: "not_requested".into(),
        updated_at: Utc::now(),
    }
}
fn effect(owner: &Receipt) -> Receipt {
    let mut receipt = owner.clone();
    receipt.operation_id = Uuid::new_v4();
    receipt.request_id = receipt.operation_id;
    receipt.kind = "worker_validation".into();
    receipt.phase = ReceiptPhase::Acknowledged;
    receipt.expected = BTreeMap::from([
        ("admission_operation".into(), owner.operation_id.to_string()),
        ("admitted_work".into(), owner.canonical_targets[0].clone()),
        ("admitted_scope".into(), owner.expected["scope"].clone()),
    ]);
    receipt.observed_result = Some(json!({"fixture":"known result data"}));
    receipt
}
#[test]
fn terminal_receipt_data_does_not_hide_unresolved_or_nonretired_ownership() {
    let mut receipt = owner();
    closed(&receipt).unwrap();
    receipt.observed_result.as_mut().unwrap()["unresolved_effects"] = json!([Uuid::new_v4()]);
    assert_eq!(closed(&receipt).unwrap_err().code, "parent_retirement");
    receipt = owner();
    receipt.phase = ReceiptPhase::Acknowledged;
    assert_eq!(closed(&receipt).unwrap_err().code, "parent_retirement");
    receipt = owner();
    receipt.observed_result = Some(json!({"local_execution":"exited"}));
    assert_eq!(closed(&receipt).unwrap_err().code, "parent_retirement");
}
#[test]
fn complete_effect_inventory_retains_failed_results_and_refuses_scope_owner_or_phase_drift() {
    let owner = owner();
    let first = effect(&owner);
    let mut second = effect(&owner);
    second.observed_result = Some(json!({"error":{"code":"fixture-failure"}}));
    let original = effects(&[first.clone(), second.clone()], &[&owner]).unwrap();
    assert_eq!(original.len(), 2);
    assert_ne!(
        effects(std::slice::from_ref(&first), &[&owner]).unwrap(),
        original
    );
    let mut changed = second.clone();
    changed.owner_generation += 1;
    assert_eq!(
        effects(&[first.clone(), changed], &[&owner])
            .unwrap_err()
            .code,
        "parent_retirement"
    );
    let mut changed = second.clone();
    changed
        .expected
        .insert("admitted_scope".into(), bytes_digest(b"substituted"));
    assert_eq!(
        effects(&[first.clone(), changed], &[&owner])
            .unwrap_err()
            .code,
        "parent_retirement"
    );
    let mut changed = second;
    changed.phase = ReceiptPhase::Sent;
    assert_eq!(
        effects(&[first.clone(), changed], &[&owner])
            .unwrap_err()
            .code,
        "parent_retirement"
    );
    assert_eq!(
        effects(&[first.clone(), first], &[&owner])
            .unwrap_err()
            .code,
        "parent_retirement"
    );
}
#[test]
fn protected_worker_record_inventory_detects_deletion_and_changed_hook_bytes() {
    // Actual protected byte I/O only; fixture values are not signed provenance.
    let temp = tempfile::tempdir().unwrap();
    let records = PrivateDir::create(&temp.path().canonicalize().unwrap().join("records")).unwrap();
    for name in ["parent-origin.json", "preparation.json", "contract.json"] {
        records
            .write(name, &json!({"fixture":"data-only"}))
            .unwrap();
    }
    let hooks = records.child("hooks").unwrap();
    hooks
        .write("hooks.json", &json!({"fixture":"initial ledger data"}))
        .unwrap();
    let before = worker_files(&records).unwrap();
    hooks
        .write("hooks.json", &json!({"fixture":"changed ledger data"}))
        .unwrap();
    assert_ne!(worker_files(&records).unwrap(), before);
    std::fs::remove_file(records.path().join("preparation.json")).unwrap();
    assert_eq!(
        worker_files(&records).unwrap_err().code,
        "parent_retirement"
    );
}

#[test]
fn retained_receipt_scan_never_decodes_as_complete_emitted_effect_history() {
    let encoded =
        serde_json::to_value(ParentEffectHistory::RetainedReceiptScanWithoutEmittedIndex).unwrap();
    assert_eq!(
        encoded,
        json!("retained_receipt_scan_without_emitted_index")
    );
    assert!(
        serde_json::from_value::<ParentEffectHistory>(json!("complete_emitted_effect_history"))
            .is_err()
    );
    assert!(serde_json::from_value::<ParentEffectHistory>(json!({"complete":true})).is_err());
}
