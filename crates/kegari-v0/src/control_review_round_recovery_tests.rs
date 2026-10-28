//! Data-only unit observations. No permit, joined completion, provider recovery,
//! actual source or opaque candidate is constructed by these tests.
use super::*;
fn digest(label: &str) -> String {
    kegari_digest::bytes_digest(label.as_bytes())
}
fn first() -> ReviewRoundRecoveryReference {
    let origin = Uuid::from_u128(10);
    ReviewRoundRecoveryReference {
        schema_version: 1,
        round_reference_digest: digest("round"),
        round_plan_digest: digest("opaque-plan-reference-only"),
        origin: ReviewReceiptReference {
            operation: origin,
            receipt_digest: digest("whole-origin"),
        },
        origin_preparation: ReviewReceiptReference {
            operation: preparation_operation(origin).unwrap(),
            receipt_digest: digest("whole-preparation"),
        },
        predecessor: ReviewReceiptReference {
            operation: origin,
            receipt_digest: digest("whole-origin"),
        },
        protected_candidate_digest: digest("protected-journal-reference-only"),
        terminal_state_digest: digest("full-terminal-state"),
        classification: TransientFailureCode::ServerOverloaded,
        previous_resume_digest: None,
    }
}
#[test]
fn first_pair_metadata_preserves_exact_origin_and_preparation_identity() {
    let actual = first();
    actual.validate().unwrap();
    let mut changed = actual.clone();
    changed.predecessor.receipt_digest = digest("other-whole-origin");
    assert!(changed.validate().is_err());
    let mut changed = actual;
    changed.origin_preparation.operation = Uuid::from_u128(11);
    assert!(changed.validate().is_err());
}
#[test]
fn later_failed_session_requires_explicit_prior_resume_link_without_relabelling_origin() {
    let mut value = first();
    value.predecessor = ReviewReceiptReference {
        operation: Uuid::from_u128(30),
        receipt_digest: digest("later-failed"),
    };
    assert!(value.validate().is_err());
    value.previous_resume_digest = Some(digest("actual-prior-resume-marker"));
    value.validate().unwrap();
    value.predecessor = value.origin.clone();
    assert!(value.validate().is_err());
}
#[test]
fn unknown_schema_fields_and_failure_classes_never_supply_provenance() {
    let actual = first();
    let mut encoded = serde_json::to_value(&actual).unwrap();
    encoded["retryable"] = serde_json::json!(true);
    assert!(serde_json::from_value::<ReviewRoundRecoveryReference>(encoded).is_err());
    let mut encoded = serde_json::to_value(&actual).unwrap();
    encoded["classification"] = serde_json::json!("unknown");
    assert!(serde_json::from_value::<ReviewRoundRecoveryReference>(encoded).is_err());
    let mut wrong_schema = actual;
    wrong_schema.schema_version = 0;
    assert!(wrong_schema.validate().is_err());
}
#[test]
fn receipt_parser_bounds_do_not_become_a_new_operational_round_cap() {
    let mut receipt = Receipt {
        operation_id: Uuid::from_u128(100),
        request_id: Uuid::from_u128(100),
        runtime_id: Uuid::from_u128(1),
        owner_generation: 1,
        actor: "fixture-observation".into(),
        kind: "admission".into(),
        canonical_targets: vec![],
        payload_digest: digest("data"),
        expected: BTreeMap::new(),
        phase: ReceiptPhase::Reconciled,
        remote_ids: BTreeMap::new(),
        observed_result: None,
        cancellation: "none".into(),
        updated_at: Utc::now(),
    };
    assert!(
        decode::<ReviewRoundRecoveryReference>(&receipt, RECOVERY_KEY)
            .unwrap()
            .is_none()
    );
    receipt
        .expected
        .insert(RECOVERY_KEY.into(), " ".repeat(MAX_ENCODED + 1));
    assert!(decode::<ReviewRoundRecoveryReference>(&receipt, RECOVERY_KEY).is_err());
}

#[test]
fn absent_resumed_source_field_preserves_direct_canonical_bytes() {
    let source = crate::control::ReviewedSourceReference {
        implementation_phase: kegari_types::run::PhaseKey {
            work: "data-only-work".into(),
            generation: digest("generation"),
            phase: "implementation".into(),
            retry_grant: None,
        },
        implementation: ReviewReceiptReference {
            operation: Uuid::from_u128(20),
            receipt_digest: digest("actual-source-reference-shape-only"),
        },
        claim_digest: digest("claim"),
        seal: ReviewReceiptReference {
            operation: Uuid::from_u128(21),
            receipt_digest: digest("seal"),
        },
        source_digest: digest("source"),
        repository_hostname: "github.invalid".into(),
        repository_id: "42".into(),
        commit: "a".repeat(40),
        tree: "b".repeat(40),
        paired_resume: None,
    };
    let old_bytes = serde_json::to_vec(&source).unwrap();
    let old_value: Value = serde_json::from_slice(&old_bytes).unwrap();
    assert!(old_value.get("paired_resume").is_none());
    let restored: crate::control::ReviewedSourceReference =
        serde_json::from_slice(&old_bytes).unwrap();
    assert!(restored.paired_resume.is_none());
    assert_eq!(serde_json::to_vec(&restored).unwrap(), old_bytes);
    assert_eq!(
        canonical_digest(1, &restored).unwrap(),
        canonical_digest(1, &source).unwrap()
    );
}
