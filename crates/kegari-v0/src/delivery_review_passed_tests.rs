//! Synthetic receipt-reader tests only. No Control admission, source/check
//! capability, model completion or actual joined runtime cache is created.
use super::*;
use crate::control::PreparationReference;
use kegari_types::run::{PhaseKey, RunReference};
fn hash(text: &str) -> String {
    kegari_digest::bytes_digest(text.as_bytes())
}
fn reference(id: u128) -> ReviewReceiptReference {
    ReviewReceiptReference {
        operation: Uuid::from_u128(id),
        receipt_digest: hash(&id.to_string()),
    }
}
fn fixture() -> (Vec<Receipt>, Uuid, PassedValidationReference) {
    let worker = Uuid::from_u128(40);
    let runtime = Uuid::from_u128(1);
    let workspace = Uuid::from_u128(2);
    let mut phase = PhaseKey {
        work: hash("canonical child identity"),
        generation: hash("generation"),
        phase: "implementation".into(),
        retry_grant: None,
    };
    let source = ReviewedSourceReference {
        paired_resume: None,
        implementation_phase: phase.clone(),
        implementation: reference(10),
        claim_digest: hash("claim"),
        seal: reference(11),
        source_digest: hash("source"),
        repository_hostname: "github.invalid".into(),
        repository_id: "42".into(),
        commit: "a".repeat(40),
        tree: "b".repeat(40),
    };
    phase.phase = "child_codex_review".into();
    let validation = reference(30);
    let passed = PassedValidationReference {
        schema_version: 1,
        source: source.clone(),
        validation: validation.clone(),
        completed_validation_digest: hash("actual cap reference only"),
        local_pass_digest: hash("actual local pass reference only"),
        retained_receipts: [
            (
                source.implementation.operation,
                source.implementation.receipt_digest.clone(),
            ),
            (source.seal.operation, source.seal.receipt_digest.clone()),
            (validation.operation, validation.receipt_digest.clone()),
        ]
        .into(),
    };
    let mut round = ReviewRoundReference {
        schema_version: 1,
        runtime,
        workspace,
        account: "account".into(),
        review_phase: phase.clone(),
        round_id: String::new(),
        ordinal: 1,
        round_limit: 3,
        source,
        passed_validation: Some(passed.clone()),
        cause: ReviewRoundCause::Initial {},
    };
    round.round_id = round.expected_round_id().unwrap();
    let prep = PreparationReference {
        worker_operation: worker,
        workspace,
        account: "account".into(),
        phase: phase.clone(),
    };
    let mut prepared = Receipt {
        operation_id: prep.operation_id().unwrap(),
        request_id: prep.operation_id().unwrap(),
        runtime_id: runtime,
        owner_generation: 1,
        actor: "owner:fixture".into(),
        kind: "admission".into(),
        canonical_targets: vec![phase.work.clone()],
        payload_digest: hash("preparation request"),
        expected: [
            ("scope".into(), hash("preparation scope")),
            (
                "worker_preparation".into(),
                serde_json::to_string(&prep).unwrap(),
            ),
            (
                "review_round".into(),
                serde_json::to_string(&round).unwrap(),
            ),
        ]
        .into(),
        phase: ReceiptPhase::Reconciled,
        remote_ids: BTreeMap::new(),
        observed_result: None,
        cancellation: "none".into(),
        updated_at: chrono::Utc::now(),
    };
    prepared.observed_result = Some(
        json!({"local_execution":"exited","unresolved_effects":[],"observation":{"phase":"worker_preparation","result":"prepared"}}),
    );
    let link = ReviewReceiptReference {
        operation: prepared.operation_id,
        receipt_digest: canonical_digest(1, &prepared).unwrap(),
    };
    let run = RunReference {
        workspace,
        account: "account".into(),
        phase: phase.clone(),
        instruction_digest: hash("instructions"),
        profile_digest: hash("profile"),
        normalized_state: None,
        max_attempts: 3,
        max_turns: 4,
        max_review_rounds: 3,
        attempt: 1,
    };
    let model = Receipt {
        operation_id: worker,
        request_id: worker,
        runtime_id: runtime,
        owner_generation: 1,
        actor: prepared.actor.clone(),
        kind: "admission".into(),
        canonical_targets: vec![phase.work],
        payload_digest: hash("model request"),
        expected: [
            ("scope".into(), hash("actual model scope")),
            ("run".into(), serde_json::to_string(&run).unwrap()),
            (
                "review_round".into(),
                serde_json::to_string(&round).unwrap(),
            ),
            (
                "review_round_preparation".into(),
                serde_json::to_string(&link).unwrap(),
            ),
        ]
        .into(),
        phase: ReceiptPhase::Prepared,
        remote_ids: BTreeMap::new(),
        observed_result: None,
        cancellation: "none".into(),
        updated_at: chrono::Utc::now(),
    };
    (vec![prepared, model], worker, passed)
}
#[test]
fn successor_identity_keeps_live_observations_separate_from_immutable_request() {
    let (mut receipts, worker, passed) = fixture();
    let original = verify_validation_successor_admission(&receipts, worker, &passed).unwrap();
    let model = receipts
        .iter_mut()
        .find(|r| r.operation_id == worker)
        .unwrap();
    model.phase = ReceiptPhase::Reconciled;
    model.observed_result =
        Some(json!({"fixture":"later observation is not completion authority"}));
    model.updated_at += chrono::Duration::seconds(1);
    assert_eq!(
        verify_validation_successor_admission(&receipts, worker, &passed).unwrap(),
        original
    );
    receipts
        .iter_mut()
        .find(|r| r.operation_id == worker)
        .unwrap()
        .payload_digest = hash("changed actual request");
    assert_ne!(
        verify_validation_successor_admission(&receipts, worker, &passed).unwrap(),
        original
    );
}
#[test]
fn successor_requires_exact_passed_reference_and_same_closed_preparation_owner() {
    let (receipts, worker, passed) = fixture();
    assert!(verify_validation_successor_admission(&receipts[1..], worker, &passed).is_err());
    let mut changed = passed.clone();
    changed.local_pass_digest = hash("replacement local pass");
    assert!(verify_validation_successor_admission(&receipts, worker, &changed).is_err());
    let mut changed = receipts;
    changed[0].owner_generation += 1;
    // Adjust only the data link to reach owner equality, not the earlier hash gate.
    let link = ReviewReceiptReference {
        operation: changed[0].operation_id,
        receipt_digest: canonical_digest(1, &changed[0]).unwrap(),
    };
    changed[1].expected.insert(
        "review_round_preparation".into(),
        serde_json::to_string(&link).unwrap(),
    );
    assert!(verify_validation_successor_admission(&changed, worker, &passed).is_err());
}

#[test]
fn oversized_host_round_is_refused_before_persisting_an_unreadable_marker() {
    let (receipts, worker, passed) = fixture();
    let mut value = verify_validation_successor_admission(&receipts, worker, &passed)
        .unwrap()
        .round()
        .clone();
    let actual = value.passed_validation.as_mut().unwrap();
    for id in 100..500 {
        actual
            .retained_receipts
            .insert(Uuid::from_u128(id), hash("retained observed receipt"));
    }
    assert_eq!(
        value.validate().unwrap_err().code,
        "review_round_history_limit"
    );
}
