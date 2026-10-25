//! Data-only decoder tests. Receipts are deliberately synthetic observations:
//! no ControlHost, permit, actual joined completion or source capability exists.
use super::*;
use crate::{control::ReceiptPhase, digest::bytes_digest};
use std::collections::BTreeMap;
use uuid::Uuid;

fn digest(label: &str) -> String {
    bytes_digest(label.as_bytes())
}
fn reference(id: u128) -> ReviewReceiptReference {
    ReviewReceiptReference {
        operation: Uuid::from_u128(id),
        receipt_digest: digest(&id.to_string()),
    }
}
fn base(granted: bool) -> ReviewRoundReference {
    let review_phase = PhaseKey {
        work: "linear:https://linear.invalid/graphql:workspace:child".into(),
        generation: digest("generation"),
        phase: "child_codex_review".into(),
        retry_grant: granted.then(|| Uuid::from_u128(99)),
    };
    let mut implementation_phase = review_phase.clone();
    implementation_phase.phase = "implementation".into();
    implementation_phase.retry_grant = None;
    let mut value = ReviewRoundReference {
        schema_version: 1,
        runtime: Uuid::from_u128(1),
        workspace: Uuid::from_u128(2),
        account: "fixture-account".into(),
        review_phase,
        round_id: String::new(),
        ordinal: 1,
        round_limit: 3,
        source: ReviewedSourceReference {
            implementation_phase,
            implementation: reference(10),
            claim_digest: digest("claim"),
            seal: reference(11),
            source_digest: digest("source"),
            repository_hostname: "github.invalid".into(),
            repository_id: "42".into(),
            commit: "a".repeat(40),
            tree: "b".repeat(40),
        },
        cause: ReviewRoundCause::Initial {},
        passed_validation: None,
    };
    value.round_id = value.expected_round_id().unwrap();
    value
}
fn findings(value: &ReviewRoundReference) -> FindingsReference {
    FindingsReference {
        review: reference(20),
        round_id: value.round_id.clone(),
        round_ordinal: value.ordinal,
        round_reference_digest: digest("review-marker"),
        outcome_proposal: Uuid::from_u128(21),
        outcome_claim_digest: digest("exact findings claim"),
        reviewed_source_digest: value.source.source_digest.clone(),
    }
}
fn paired(granted: bool, ordinal: u32) -> ReviewRoundReference {
    let mut value = base(granted);
    value.ordinal = ordinal;
    value.round_id = value.expected_round_id().unwrap();
    value.cause = ReviewRoundCause::PairedRepair {
        findings: findings(&value),
    };
    value
}
fn after_repair(granted: bool) -> ReviewRoundReference {
    let prior = paired(granted, 1);
    let mut value = prior.clone();
    value.ordinal = 2;
    value.source.implementation = reference(30);
    value.source.claim_digest = digest("repaired claim");
    value.source.seal = reference(31);
    value.source.source_digest = digest("repaired source");
    value.source.commit = "c".repeat(40);
    value.source.tree = "d".repeat(40);
    value.round_id = value.expected_round_id().unwrap();
    value.cause = ReviewRoundCause::AfterRepair {
        findings: findings(&prior),
        repair: value.source.implementation.clone(),
        paired_round_reference_digest: canonical_digest(1, &prior).unwrap(),
    };
    value
}
fn run(value: &ReviewRoundReference) -> Receipt {
    let attempt = if matches!(&value.cause, ReviewRoundCause::Initial {}) {
        1
    } else {
        2
    };
    let reference = RunReference {
        workspace: value.workspace,
        account: value.account.clone(),
        normalized_state: Some("running".into()),
        phase: value.subject_phase().clone(),
        profile_digest: digest("profile"),
        instruction_digest: digest("instructions"),
        max_attempts: 3,
        max_turns: 20,
        max_review_rounds: 3,
        attempt,
    };
    let mut expected = BTreeMap::from([
        (KEY.into(), serde_json::to_string(value).unwrap()),
        ("run".into(), serde_json::to_string(&reference).unwrap()),
        ("scope".into(), digest("scope")),
        (
            PREPARATION_LINK.into(),
            serde_json::to_string(&ReviewReceiptReference {
                operation: crate::control::preparation_operation(Uuid::from_u128(100)).unwrap(),
                receipt_digest: digest("synthetic closed preparation"),
            })
            .unwrap(),
        ),
    ]);
    match &value.cause {
        ReviewRoundCause::PairedRepair { findings } => {
            let link = RunContinuationReference {
                predecessor: value.source.implementation.operation,
                predecessor_receipt_digest: value.source.implementation.receipt_digest.clone(),
                trigger_digest: digest("trigger"),
                progress_digest: digest("progress"),
                failure_operation: findings.review.operation,
                review_rounds_consumed: value.ordinal,
            };
            expected.insert("continuation".into(), serde_json::to_string(&link).unwrap());
        }
        ReviewRoundCause::RepairContinuation {
            current_source,
            validation,
            ..
        } => {
            let link = RunContinuationReference {
                predecessor: current_source.implementation.operation,
                predecessor_receipt_digest: current_source.implementation.receipt_digest.clone(),
                trigger_digest: digest("continued failed local obligation"),
                progress_digest: digest("current source plus failed configured checks"),
                failure_operation: validation.validation.operation,
                review_rounds_consumed: value.ordinal,
            };
            expected.insert("continuation".into(), serde_json::to_string(&link).unwrap());
        }
        ReviewRoundCause::AfterRepair {
            findings, repair, ..
        } => {
            let link = SourceAdvancedReference {
                predecessor: findings.review.operation,
                predecessor_receipt_digest: findings.review.receipt_digest.clone(),
                source_operation: repair.operation,
                source_receipt_digest: repair.receipt_digest.clone(),
                trigger_digest: digest("trigger"),
                progress_digest: digest("progress"),
                review_rounds_consumed: value.ordinal - 1,
            };
            expected.insert(
                "source_advanced".into(),
                serde_json::to_string(&link).unwrap(),
            );
        }
        _ => (),
    }
    Receipt {
        operation_id: Uuid::from_u128(100),
        request_id: Uuid::from_u128(100),
        runtime_id: value.runtime,
        owner_generation: 1,
        actor: "explicit data fixture".into(),
        kind: "admission".into(),
        canonical_targets: vec![value.review_phase.work.clone()],
        payload_digest: digest("payload"),
        expected,
        phase: ReceiptPhase::Prepared,
        remote_ids: BTreeMap::new(),
        observed_result: None,
        cancellation: "none".into(),
        updated_at: chrono::Utc::now(),
    }
}
fn preparation(value: &ReviewRoundReference) -> Receipt {
    let mut receipt = run(value);
    receipt.expected.remove("run");
    receipt.expected.remove("continuation");
    receipt.expected.remove("source_advanced");
    receipt.expected.remove(PREPARATION_LINK);
    let reference = PreparationReference {
        worker_operation: receipt.operation_id,
        workspace: value.workspace,
        account: value.account.clone(),
        phase: value.subject_phase().clone(),
    };
    receipt.operation_id = reference.operation_id().unwrap();
    receipt.request_id = receipt.operation_id;
    receipt.expected.insert(
        "worker_preparation".into(),
        serde_json::to_string(&reference).unwrap(),
    );
    receipt
}
fn decode(value: &ReviewRoundReference, receipt: &Receipt) {
    assert_eq!(
        ReviewRoundReference::from_receipt(receipt)
            .unwrap()
            .as_ref(),
        Some(value)
    );
}

#[test]
fn ordinary_and_granted_rounds_have_exact_preparation_and_run_identity() {
    for granted in [false, true] {
        for value in [base(granted), paired(granted, 1), after_repair(granted)] {
            decode(&value, &preparation(&value));
            decode(&value, &run(&value));
            assert_eq!(value.source.implementation_phase.retry_grant, None);
            assert_eq!(value.review_phase.retry_grant.is_some(), granted);
        }
    }
}

#[test]
fn transient_retry_preserves_round_but_never_grants_retry_authority() {
    let first = base(true);
    let mut retry = first.clone();
    retry.round_limit = 2;
    retry.cause = ReviewRoundCause::TransientRetry {
        predecessor: reference(22),
        prior_round_reference_digest: canonical_digest(1, &first).unwrap(),
        terminal_state_digest: digest("terminal turn"),
        classified_failure_digest: digest("adapter failure"),
        classification: TransientFailureCode::ServerOverloaded,
    };
    assert_eq!(retry.expected_round_id().unwrap(), first.round_id);
    decode(&retry, &run(&retry));
    let mut invalid = serde_json::to_value(&retry).unwrap();
    invalid["cause"]["classification"] = serde_json::json!("unauthorized");
    assert!(serde_json::from_value::<ReviewRoundReference>(invalid).is_err());
    // The decoder accepts provenance only. Actual supported classification,
    // latest predecessor, known exit and backoff remain producer/Control checks.
}

#[test]
fn final_round_findings_can_reference_its_pair_but_cannot_allocate_another_round() {
    let last = paired(true, 3);
    decode(&last, &run(&last));
    let mut next = after_repair(true);
    next.ordinal = 4;
    next.round_id = next.expected_round_id().unwrap();
    if let ReviewRoundCause::AfterRepair { findings, .. } = &mut next.cause {
        findings.round_ordinal = 3;
    }
    assert!(next.validate().is_err());
    // No admission is performed; implementation attempts/turns remain separate.
}

#[test]
fn round_identity_includes_workspace_grant_and_exact_source_provenance() {
    let initial = base(false);
    assert_ne!(initial.round_id, base(true).round_id);
    let mut changed = initial.clone();
    changed.workspace = Uuid::from_u128(8);
    assert_ne!(initial.round_id, changed.expected_round_id().unwrap());
    changed = initial.clone();
    changed.source.implementation.receipt_digest = digest("changed exact source receipt");
    assert_ne!(initial.round_id, changed.expected_round_id().unwrap());
    changed = initial.clone();
    changed.source.claim_digest = digest("changed claim");
    assert_ne!(initial.round_id, changed.expected_round_id().unwrap());
    changed = initial.clone();
    changed.source.seal.receipt_digest = digest("changed seal");
    assert_ne!(initial.round_id, changed.expected_round_id().unwrap());
}

#[test]
fn decoder_rejects_mixed_or_rebound_admissions_and_foreign_grant() {
    let value = base(true);
    let original = run(&value);
    for which in [
        "runtime",
        "request",
        "target",
        "metadata",
        "owner-kind",
        "scope",
        "run-and-prep",
    ] {
        let mut receipt = original.clone();
        match which {
            "runtime" => receipt.runtime_id = Uuid::from_u128(8),
            "request" => receipt.request_id = Uuid::from_u128(8),
            "target" => receipt.canonical_targets.push("another work".into()),
            "metadata" => {
                receipt.expected.insert("metadata".into(), "{}".into());
            }
            "owner-kind" => receipt.kind = "linear_comment".into(),
            "scope" => {
                receipt.expected.remove("scope");
            }
            "run-and-prep" => {
                receipt
                    .expected
                    .insert("worker_preparation".into(), "{}".into());
            }
            _ => unreachable!(),
        }
        assert!(
            ReviewRoundReference::from_receipt(&receipt).is_err(),
            "{which}"
        );
    }
    for which in ["workspace", "account", "grant", "phase"] {
        let mut receipt = original.clone();
        let mut changed: RunReference = serde_json::from_str(&receipt.expected["run"]).unwrap();
        match which {
            "workspace" => changed.workspace = Uuid::from_u128(8),
            "account" => changed.account = "different account".into(),
            "grant" => changed.phase.retry_grant = Some(Uuid::from_u128(8)),
            "phase" => changed.phase.phase = "implementation".into(),
            _ => unreachable!(),
        }
        receipt
            .expected
            .insert("run".into(), serde_json::to_string(&changed).unwrap());
        assert!(
            ReviewRoundReference::from_receipt(&receipt).is_err(),
            "{which}"
        );
    }
    let mut wrong_preparation = preparation(&value);
    wrong_preparation.operation_id = Uuid::from_u128(8);
    wrong_preparation.request_id = wrong_preparation.operation_id;
    assert!(ReviewRoundReference::from_receipt(&wrong_preparation).is_err());
}

#[test]
fn paired_and_after_repair_links_must_match_the_atomic_cause() {
    for value in [paired(false, 2), after_repair(false)] {
        let mut receipt = run(&value);
        let key = if matches!(&value.cause, ReviewRoundCause::PairedRepair { .. }) {
            "continuation"
        } else {
            "source_advanced"
        };
        let mut link: serde_json::Value = serde_json::from_str(&receipt.expected[key]).unwrap();
        link["predecessor_receipt_digest"] = serde_json::json!(digest("substituted predecessor"));
        receipt
            .expected
            .insert(key.into(), serde_json::to_string(&link).unwrap());
        assert!(ReviewRoundReference::from_receipt(&receipt).is_err());
        receipt = run(&value);
        receipt.expected.remove(key);
        assert!(ReviewRoundReference::from_receipt(&receipt).is_err());
    }
    let mut value = paired(false, 2);
    if let ReviewRoundCause::PairedRepair { findings } = &mut value.cause {
        findings.reviewed_source_digest = digest("another source");
    }
    assert!(value.validate().is_err());
    let mut value = after_repair(false);
    if let ReviewRoundCause::AfterRepair { findings, .. } = &mut value.cause {
        findings.round_ordinal = 2; // skips a paired predecessor
    }
    assert!(value.validate().is_err());
}

#[test]
fn schema_bounds_unknown_fields_and_legacy_absence_are_conservative() {
    let value = base(false);
    assert_eq!(
        serde_json::to_string(&value.cause).unwrap(),
        r#"{"kind":"initial"}"#
    );
    for which in [
        "schema",
        "zero",
        "limit",
        "identity",
        "nil",
        "digest",
        "oid",
        "repository",
    ] {
        let mut changed = value.clone();
        match which {
            "schema" => changed.schema_version = 2,
            "zero" => changed.ordinal = 0,
            "limit" => changed.round_limit = 0,
            "identity" => changed.account = "account\nextra".into(),
            "nil" => changed.source.seal.operation = Uuid::nil(),
            "digest" => changed.source.claim_digest = "not-a-digest".into(),
            "oid" => changed.source.commit = "A".repeat(40),
            "repository" => changed.source.repository_id = "042".into(),
            _ => unreachable!(),
        }
        changed.round_id = changed.expected_round_id().unwrap();
        assert!(changed.validate().is_err(), "{which}");
    }
    for path in ["top", "phase", "source-phase", "cause"] {
        let mut encoded = serde_json::to_value(&value).unwrap();
        let object = match path {
            "top" => &mut encoded,
            "phase" => &mut encoded["review_phase"],
            "source-phase" => &mut encoded["source"]["implementation_phase"],
            "cause" => &mut encoded["cause"],
            _ => unreachable!(),
        };
        object["unknown_authority"] = serde_json::json!(true);
        assert!(
            serde_json::from_value::<ReviewRoundReference>(encoded).is_err(),
            "{path}"
        );
    }
    let mut receipt = run(&value);
    receipt
        .expected
        .insert(KEY.into(), " ".repeat(MAX_ENCODED_BYTES + 1));
    assert!(ReviewRoundReference::from_receipt(&receipt).is_err());
    receipt.expected.remove(KEY);
    assert!(ReviewRoundReference::from_receipt(&receipt).is_err());
    receipt.expected.remove(PREPARATION_LINK);
    assert!(
        ReviewRoundReference::from_receipt(&receipt)
            .unwrap()
            .is_none()
    );
    // None is absence, never ordinal zero or permission to reset a legacy floor.
}

#[test]
fn exact_receipt_observation_checks_whole_hash_without_minting_authority() {
    let receipt = run(&base(false));
    let expected = ReviewReceiptReference {
        operation: receipt.operation_id,
        receipt_digest: canonical_digest(1, &receipt).unwrap(),
    };
    expected.matches_observation(&receipt).unwrap();
    let mut changed = receipt;
    changed.updated_at += chrono::Duration::seconds(1);
    assert!(expected.matches_observation(&changed).is_err());
}

#[test]
fn shape_decoder_does_not_recurse_into_round_aware_link_readers() {
    let value = paired(false, 2);
    let mut receipt = run(&value);
    receipt
        .expected
        .insert("continuation".into(), "not-json".into());
    // The first layer checks bounded marker/admission identity only. The full
    // structural reader separately rejects the malformed link. Neither is a
    // Control admission or a producer of an executable capability.
    assert_eq!(
        ReviewRoundReference::decode_admission(&receipt).unwrap(),
        Some(value)
    );
    assert!(ReviewRoundReference::from_receipt(&receipt).is_err());
}

#[test]
fn paired_round_namespace_does_not_relabel_legacy_implementation_counter() {
    let value = paired(true, 3);
    let mut receipt = run(&value);
    let mut actual: RunReference = serde_json::from_str(&receipt.expected["run"]).unwrap();
    actual.max_review_rounds = 1;
    actual.max_attempts = 2;
    actual.max_turns = 4;
    receipt
        .expected
        .insert("run".into(), serde_json::to_string(&actual).unwrap());
    let mut link: RunContinuationReference =
        serde_json::from_str(&receipt.expected["continuation"]).unwrap();
    link.review_rounds_consumed = 9; // retained legacy scalar, never rewritten to 3
    receipt
        .expected
        .insert("continuation".into(), serde_json::to_string(&link).unwrap());
    let original_run = receipt.expected["run"].clone();
    let original_link = receipt.expected["continuation"].clone();
    decode(&value, &receipt);
    assert_eq!(receipt.expected["run"], original_run);
    assert_eq!(receipt.expected["continuation"], original_link);
    // This only parses separate namespaces. The future Control verifier must
    // prove complete typed ancestry and the actual independent attempt/turn
    // bounds; absence of that proof remains a blocker, including legacy history.
}

#[test]
fn model_preparation_link_is_required_bounded_and_canonical() {
    let value = base(false);
    for which in ["missing", "foreign", "oversized", "malformed"] {
        let mut receipt = run(&value);
        match which {
            "missing" => {
                receipt.expected.remove(PREPARATION_LINK);
            }
            "foreign" => {
                receipt.expected.insert(
                    PREPARATION_LINK.into(),
                    serde_json::to_string(&reference(777)).unwrap(),
                );
            }
            "oversized" => {
                receipt
                    .expected
                    .insert(PREPARATION_LINK.into(), " ".repeat(4097));
            }
            "malformed" => {
                receipt
                    .expected
                    .insert(PREPARATION_LINK.into(), "{}".into());
            }
            _ => unreachable!(),
        }
        assert!(
            ReviewRoundReference::from_receipt(&receipt).is_err(),
            "{which}"
        );
    }
    let mut prep = preparation(&value);
    prep.expected.insert(
        PREPARATION_LINK.into(),
        run(&value).expected[PREPARATION_LINK].clone(),
    );
    assert!(ReviewRoundReference::from_receipt(&prep).is_err());
}

#[test]
fn positive_configured_rounds_above_old_draft_limits_have_no_hidden_budget_cap() {
    let mut value = paired(false, 65_536);
    value.round_limit = 65_536;
    value.validate().unwrap();
    let mut receipt = run(&value);
    let mut actual: RunReference = serde_json::from_str(&receipt.expected["run"]).unwrap();
    actual.max_review_rounds = 65_536;
    receipt
        .expected
        .insert("run".into(), serde_json::to_string(&actual).unwrap());
    decode(&value, &receipt);
    // This proves only reference shape, not a 65,536-round execution history.
    // Actual history is iterative, resource bounded and checked under Control.
}

#[test]
fn validation_source_scalar_is_provenance_not_a_review_allocation() {
    let mut receipt = run(&after_repair(false));
    receipt.expected.remove(KEY);
    receipt.expected.remove(PREPARATION_LINK);
    let mut actual: RunReference = serde_json::from_str(&receipt.expected["run"]).unwrap();
    actual.phase.phase = "child_local_validation".into();
    actual.max_review_rounds = 1;
    receipt
        .expected
        .insert("run".into(), serde_json::to_string(&actual).unwrap());
    let mut link: SourceAdvancedReference =
        serde_json::from_str(&receipt.expected["source_advanced"]).unwrap();
    link.review_rounds_consumed = 9; // existing audit floor, never reset to zero
    let encoded = serde_json::to_string(&link).unwrap();
    receipt
        .expected
        .insert("source_advanced".into(), encoded.clone());
    assert_eq!(
        SourceAdvancedReference::from_receipt(&receipt).unwrap(),
        Some(link)
    );
    assert_eq!(receipt.expected["source_advanced"], encoded);
    assert!(
        ReviewRoundReference::from_receipt(&receipt)
            .unwrap()
            .is_none()
    );
    actual.phase.phase = "child_codex_review".into();
    receipt
        .expected
        .insert("run".into(), serde_json::to_string(&actual).unwrap());
    assert!(SourceAdvancedReference::from_receipt(&receipt).is_err());
    // Data-only: real validation admission must independently check the actual
    // source receipt, immutable paired ancestry and original phase/session caps.
}

// These are data-only extension tests, not proof/Control/joined capability tests.
fn repair_continuation(granted: bool) -> ReviewRoundReference {
    let prior = paired(granted, 3);
    let mut value = prior.clone();
    let mut source = prior.source.clone();
    source.implementation = reference(30);
    source.seal = reference(31);
    source.claim_digest = digest("actual repair claim");
    source.source_digest = digest("actual repaired source");
    source.commit = "c".repeat(40);
    source.tree = "d".repeat(40);
    let mut validation_phase = prior.review_phase.clone();
    validation_phase.phase = "child_local_validation".into();
    validation_phase.retry_grant = None;
    let validation = RepairValidationReference {
        lineage: crate::control::RepairValidationLineage::Direct {},
        validation: reference(40),
        phase: validation_phase,
        source_advanced: SourceAdvancedReference {
            predecessor: uuid::Uuid::from_u128(12),
            predecessor_receipt_digest: digest("prior local validation"),
            source_operation: source.implementation.operation,
            source_receipt_digest: source.implementation.receipt_digest.clone(),
            trigger_digest: digest("actual validation advancement"),
            progress_digest: digest("actual validation advancement progress"),
            review_rounds_consumed: 3,
        },
        outcome_proposal: uuid::Uuid::from_u128(41),
        outcome_claim_digest: digest("actual validation claim"),
        failure_inventory_digest: digest("all local validation receipts"),
    };
    value.cause = ReviewRoundCause::RepairContinuation {
        findings: findings(&prior),
        previous_round_reference_digest: canonical_digest(1, &prior).unwrap(),
        current_source: Box::new(source),
        validation: Box::new(validation),
    };
    value
}
#[test]
fn continued_repair_keeps_last_allowed_round_and_uses_latest_source() {
    for granted in [false, true] {
        let value = repair_continuation(granted);
        assert_eq!(value.round_id, paired(granted, 3).round_id);
        assert_eq!(value.ordinal, value.round_limit);
        assert_eq!(value.subject_phase(), &value.source.implementation_phase);
        assert_ne!(value.dispatch_source(), &value.source);
        decode(&value, &preparation(&value));
        decode(&value, &run(&value));
    }
}
#[test]
fn continued_repair_cannot_reuse_first_source_or_findings_as_new_failure() {
    let value = repair_continuation(true);
    let mut receipt = run(&value);
    let mut link: RunContinuationReference =
        serde_json::from_str(&receipt.expected["continuation"]).unwrap();
    link.predecessor = value.source.implementation.operation;
    link.predecessor_receipt_digest = value.source.implementation.receipt_digest.clone();
    receipt
        .expected
        .insert("continuation".into(), serde_json::to_string(&link).unwrap());
    assert!(ReviewRoundReference::from_receipt(&receipt).is_err());
    let mut receipt = run(&value);
    let mut link: RunContinuationReference =
        serde_json::from_str(&receipt.expected["continuation"]).unwrap();
    let ReviewRoundCause::RepairContinuation { findings, .. } = &value.cause else {
        unreachable!()
    };
    link.failure_operation = findings.review.operation;
    receipt
        .expected
        .insert("continuation".into(), serde_json::to_string(&link).unwrap());
    assert!(ReviewRoundReference::from_receipt(&receipt).is_err());
}
#[test]
fn continued_repair_rejects_changed_round_grant_namespace_and_validation_source() {
    for mutation in 0..5 {
        let mut value = repair_continuation(true);
        match mutation {
            0 => value.ordinal -= 1,
            1 => value.review_phase.retry_grant = Some(uuid::Uuid::from_u128(98)),
            2 => {
                let ReviewRoundCause::RepairContinuation { current_source, .. } = &mut value.cause
                else {
                    unreachable!()
                };
                current_source.implementation_phase.retry_grant = Some(uuid::Uuid::from_u128(98));
            }
            3 => {
                let ReviewRoundCause::RepairContinuation { validation, .. } = &mut value.cause
                else {
                    unreachable!()
                };
                validation.source_advanced.source_receipt_digest = digest("substitution");
            }
            _ => {
                let ReviewRoundCause::RepairContinuation { current_source, .. } = &mut value.cause
                else {
                    unreachable!()
                };
                current_source.repository_id = "43".into();
            }
        }
        assert!(value.validate().is_err());
    }
}

#[test]
fn direct_validation_legacy_encoding_stays_canonical_and_retried_origin_is_distinct() {
    let direct = repair_continuation(false);
    let bytes = serde_json::to_vec(&direct).unwrap();
    let encoded: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(encoded.pointer("/cause/validation/lineage").is_none());
    let restored: ReviewRoundReference = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(serde_json::to_vec(&restored).unwrap(), bytes);
    let mut retried = direct;
    let ReviewRoundCause::RepairContinuation { validation, .. } = &mut retried.cause else {
        unreachable!()
    };
    let origin = validation.validation.clone();
    let origin_phase = validation.phase.clone();
    validation.validation = reference(80);
    validation.phase.retry_grant = Some(Uuid::from_u128(81));
    validation.lineage = RepairValidationLineage::Retried {
        origin: Box::new(origin.clone()),
        origin_phase,
        completed_retry_digest: digest("opaque completed validation reference only"),
        native_target: "linear:https://linear.invalid/graphql:native-workspace:native-issue".into(),
        inventory: [
            (origin.operation, origin.receipt_digest.clone()),
            (
                validation.validation.operation,
                validation.validation.receipt_digest.clone(),
            ),
        ]
        .into(),
        grants: [(
            Uuid::from_u128(81),
            digest("actual grant publication reference"),
        )]
        .into(),
    };
    assert_eq!(validation.source_admission(), &origin);
    assert_ne!(validation.source_admission(), &validation.validation);
    retried.validate().unwrap();
    decode(&retried, &preparation(&retried));
    decode(&retried, &run(&retried));
    for mutation in 0..4 {
        let mut changed = retried.clone();
        let ReviewRoundCause::RepairContinuation { validation, .. } = &mut changed.cause else {
            unreachable!()
        };
        let RepairValidationLineage::Retried {
            origin,
            inventory,
            grants,
            origin_phase,
            ..
        } = &mut validation.lineage
        else {
            unreachable!()
        };
        match mutation {
            0 => **origin = validation.validation.clone(),
            1 => {
                inventory.remove(&origin.operation);
            }
            2 => grants.clear(),
            _ => origin_phase.generation = digest("other generation"),
        }
        assert!(changed.validate().is_err());
    }
}

#[test]
fn direct_validation_marker_rejects_unknown_authority_fields() {
    let accepted: RepairValidationLineage =
        serde_json::from_value(serde_json::json!({"kind":"direct"})).unwrap();
    assert!(accepted.is_direct());
    assert_eq!(
        serde_json::to_value(accepted).unwrap(),
        serde_json::json!({"kind":"direct"})
    );
    assert!(
        serde_json::from_value::<RepairValidationLineage>(
            serde_json::json!({"kind":"direct","authority":"invented"})
        )
        .is_err()
    );
}
