//! Planner tests use diagnostic receipt data. Only the delivery producer can
//! issue the opaque capability required by production continuation admission.
use super::*;
use crate::{control::RunContinuationReference, digest::canonical_digest};
use chrono::Duration;
use std::collections::BTreeMap;

fn digest(value: &str) -> String {
    canonical_digest(1, &value).unwrap()
}
fn limits() -> RuntimeConfig {
    serde_json::from_value(
        json!({"id":Uuid::from_u128(1),"host_id":"fixture","max_agents":3,
        "max_attempts_per_phase":9,"max_turns":99,"max_review_rounds":9}),
    )
    .unwrap()
}
fn phase() -> PhaseKey {
    PhaseKey {
        work: "exact-child".into(),
        generation: digest("generation"),
        phase: "implementation".into(),
        retry_grant: None,
    }
}
fn receipt(attempt: u32, now: DateTime<Utc>) -> Receipt {
    let run = RunReference {
        workspace: Uuid::from_u128(2),
        account: "fixture-account".into(),
        normalized_state: Some("running".into()),
        phase: phase(),
        profile_digest: digest("profile"),
        instruction_digest: digest("instruction"),
        max_attempts: 3,
        max_turns: 20,
        max_review_rounds: 3,
        attempt,
    };
    let id = Uuid::from_u128(100 + u128::from(attempt));
    Receipt {
        operation_id: id,
        request_id: id,
        runtime_id: limits().id,
        owner_generation: 1,
        actor: "fixture-runtime".into(),
        kind: "admission".into(),
        canonical_targets: vec![phase().work],
        payload_digest: digest("payload"),
        expected: BTreeMap::from([("run".into(), serde_json::to_string(&run).unwrap())]),
        phase: ReceiptPhase::Reconciled,
        remote_ids: BTreeMap::new(),
        observed_result: Some(json!({"local_execution":"exited","unresolved_effects":[],
            "observation":{"outcome":"completed","local_exit_confirmed":true,"hook_error":null,
                "session":{"terminal":"completed","intervention":null,"input_required":false}}})),
        cancellation: "none".into(),
        updated_at: now - Duration::seconds(10),
    }
}
fn failure(now: DateTime<Utc>) -> Receipt {
    let mut value = receipt(1, now);
    value.operation_id = Uuid::from_u128(200);
    value.request_id = value.operation_id;
    let mut run = RunReference::from_receipt(&value).unwrap().unwrap();
    run.phase.phase = "child_validation".into();
    value
        .expected
        .insert("run".into(), serde_json::to_string(&run).unwrap());
    value
}
fn reference(predecessor: &Receipt, failure: &Receipt) -> RunContinuationReference {
    RunContinuationReference {
        predecessor: predecessor.operation_id,
        predecessor_receipt_digest: canonical_digest(1, predecessor).unwrap(),
        trigger_digest: digest("sealed-trigger"),
        progress_digest: digest("source-and-unmet-obligation"),
        failure_operation: failure.operation_id,
        review_rounds_consumed: 0,
    }
}
fn next(
    receipts: &[Receipt],
    reference: &RunContinuationReference,
    now: DateTime<Utc>,
) -> Result<RunSchedule> {
    plan_unfinished(
        &limits(),
        &phase(),
        Uuid::from_u128(2),
        receipts,
        reference,
        now,
    )
}

#[test]
fn verified_continuation_preserves_original_bounds_and_waits_for_latest_exit() {
    let now = Utc::now();
    let predecessor = receipt(1, now);
    let mut failure = failure(now);
    failure.updated_at = now;
    let reference = reference(&predecessor, &failure);
    let records = [predecessor, failure];
    assert_eq!(
        next(&records, &reference, now + Duration::milliseconds(999))
            .unwrap_err()
            .code,
        "retry_wait"
    );
    let result = next(&records, &reference, now + Duration::seconds(1)).unwrap();
    assert_eq!(
        (
            result.attempt,
            result.max_attempts,
            result.max_turns,
            result.max_review_rounds
        ),
        (2, 3, 20, 3)
    );
    assert_eq!(result.trigger, Trigger::VerifiedUnfinished);
    assert_eq!(result.not_before, now + Duration::seconds(1));
    assert_eq!(
        plan(
            &limits(),
            &phase(),
            Uuid::from_u128(2),
            &records,
            now + Duration::seconds(1)
        )
        .unwrap_err()
        .code,
        "no_work_trigger"
    );
    let mut reduced = limits();
    reduced.max_turns = 5;
    reduced.max_review_rounds = 2;
    let result = plan_unfinished(
        &reduced,
        &phase(),
        Uuid::from_u128(2),
        &records,
        &reference,
        now + Duration::seconds(1),
    )
    .unwrap();
    assert_eq!((result.max_turns, result.max_review_rounds), (5, 2));
}

#[test]
fn stale_or_different_phase_predecessors_cannot_continue() {
    let now = Utc::now();
    let predecessor = receipt(1, now);
    let failure = failure(now);
    let reference = reference(&predecessor, &failure);
    let records = [predecessor.clone(), failure.clone()];
    let mut wrong = reference.clone();
    wrong.predecessor = Uuid::from_u128(999);
    assert_eq!(
        next(&records, &wrong, now).unwrap_err().code,
        "stale_result"
    );
    wrong = reference.clone();
    wrong.predecessor_receipt_digest = digest("changed-receipt");
    assert_eq!(
        next(&records, &wrong, now).unwrap_err().code,
        "stale_result"
    );
    let later = receipt(2, now);
    assert_eq!(
        next(&[predecessor, later, failure.clone()], &reference, now)
            .unwrap_err()
            .code,
        "stale_result"
    );
    assert_eq!(
        next(&[failure], &reference, now).unwrap_err().code,
        "no_work_trigger"
    );
}

#[test]
fn missing_or_unknown_failure_and_earlier_unknown_execution_remain_held() {
    let now = Utc::now();
    let predecessor = receipt(1, now);
    let failure = failure(now);
    let reference = reference(&predecessor, &failure);
    assert_eq!(
        next(std::slice::from_ref(&predecessor), &reference, now)
            .unwrap_err()
            .code,
        "reconciliation_required"
    );
    for mutation in 0..3 {
        let mut unknown = failure.clone();
        match mutation {
            0 => unknown.phase = ReceiptPhase::Sent,
            1 => unknown.observed_result.as_mut().unwrap()["local_execution"] = json!("unknown"),
            _ => {
                unknown.observed_result.as_mut().unwrap()["unresolved_effects"] =
                    json!([Uuid::from_u128(600)])
            }
        }
        assert_eq!(
            next(&[predecessor.clone(), unknown], &reference, now)
                .unwrap_err()
                .code,
            "reconciliation_required"
        );
    }
    let mut earlier = predecessor;
    earlier.observed_result.as_mut().unwrap()["local_execution"] = json!("unknown");
    let later = receipt(2, now);
    let reference = self::reference(&later, &failure);
    assert_eq!(
        next(&[earlier, later, failure], &reference, now)
            .unwrap_err()
            .code,
        "reconciliation_required"
    );
}

#[test]
fn failure_must_belong_to_the_same_runtime_workspace_work_and_generation() {
    let now = Utc::now();
    let predecessor = receipt(1, now);
    let original = failure(now);
    let reference = reference(&predecessor, &original);
    for mutation in 0..4 {
        let mut failure = original.clone();
        let mut run = RunReference::from_receipt(&failure).unwrap().unwrap();
        match mutation {
            0 => failure.runtime_id = Uuid::from_u128(888),
            1 => run.workspace = Uuid::from_u128(888),
            2 => run.phase.work = "other-child".into(),
            _ => run.phase.generation = digest("new-generation"),
        }
        failure
            .expected
            .insert("run".into(), serde_json::to_string(&run).unwrap());
        assert_eq!(
            next(&[predecessor.clone(), failure], &reference, now)
                .unwrap_err()
                .code,
            "scope_violation"
        );
    }
}

#[test]
fn already_consumed_trigger_or_unchanged_progress_cannot_create_another_attempt() {
    let now = Utc::now();
    let original = receipt(1, now);
    let failure = failure(now);
    let mut consumed = reference(&original, &failure);
    consumed.review_rounds_consumed = 1;
    let mut predecessor = receipt(2, now);
    predecessor.expected.insert(
        "continuation".into(),
        serde_json::to_string(&consumed).unwrap(),
    );
    let base = reference(&predecessor, &failure);
    let records = [original, predecessor, failure];
    let mut repeated = base.clone();
    repeated.trigger_digest = digest("new-trigger");
    assert_eq!(
        next(&records, &repeated, now).unwrap_err().code,
        "no_progress"
    );
    repeated = base.clone();
    repeated.progress_digest = digest("new-source");
    assert_eq!(
        next(&records, &repeated, now).unwrap_err().code,
        "no_progress"
    );
    repeated.trigger_digest = digest("new-trigger");
    assert_eq!(
        next(&records, &repeated, now).unwrap_err().code,
        "record_corrupt"
    );
    repeated.review_rounds_consumed = 1;
    assert_eq!(next(&records, &repeated, now).unwrap().attempt, 3);
}

#[test]
fn finite_attempt_and_review_limits_cannot_be_reset() {
    let now = Utc::now();
    let predecessor = receipt(1, now);
    let failure = failure(now);
    let mut reference = reference(&predecessor, &failure);
    reference.review_rounds_consumed = 3;
    assert_eq!(
        next(&[predecessor.clone(), failure.clone()], &reference, now)
            .unwrap_err()
            .code,
        "review_rounds_exhausted"
    );
    let third = receipt(3, now);
    let reference = self::reference(&third, &failure);
    assert_eq!(
        next(
            &[predecessor, receipt(2, now), third, failure],
            &reference,
            now
        )
        .unwrap_err()
        .code,
        "attempts_exhausted"
    );
}

#[test]
fn generic_completion_hook_errors_and_provider_interventions_are_not_repair_authority() {
    let now = Utc::now();
    let failure = failure(now);
    for mutation in 0..7 {
        let mut predecessor = receipt(1, now);
        let observed = &mut predecessor.observed_result.as_mut().unwrap()["observation"];
        match mutation {
            0 => observed["outcome"] = json!("blocked"),
            1 => observed["session"]["terminal"] = json!("interrupted"),
            2 => observed["local_exit_confirmed"] = json!(false),
            3 => observed["hook_error"] = json!({"code":"timeout"}),
            4 => observed["session"]["intervention"] = json!({"kind":"login_required"}),
            5 => observed["session"]["input_required"] = json!(true),
            _ => *observed = json!({"outcome":"completed","summary":"all done"}),
        }
        let reference = reference(&predecessor, &failure);
        assert_eq!(
            next(&[predecessor, failure.clone()], &reference, now)
                .unwrap_err()
                .code,
            if [4, 5].contains(&mutation) {
                "provider_intervention"
            } else {
                "stale_result"
            }
        );
    }
}

fn validation(attempt: u32, now: DateTime<Utc>) -> Receipt {
    let mut receipt = receipt(attempt, now);
    receipt.operation_id = Uuid::from_u128(400 + u128::from(attempt));
    receipt.request_id = receipt.operation_id;
    let mut run = RunReference::from_receipt(&receipt).unwrap().unwrap();
    run.phase.phase = "child_local_validation".into();
    receipt
        .expected
        .insert("run".into(), serde_json::to_string(&run).unwrap());
    receipt
}
fn advanced(previous: &Receipt, source: &Receipt) -> crate::control::SourceAdvancedReference {
    crate::control::SourceAdvancedReference {
        predecessor: previous.operation_id,
        predecessor_receipt_digest: canonical_digest(1, previous).unwrap(),
        source_operation: source.operation_id,
        source_receipt_digest: canonical_digest(1, source).unwrap(),
        trigger_digest: digest("sealed-source-advancement"),
        progress_digest: digest("changed-tree-and-required-phase"),
        review_rounds_consumed: 1,
    }
}
fn next_advanced(
    receipts: &[Receipt],
    reference: &crate::control::SourceAdvancedReference,
    now: DateTime<Utc>,
) -> Result<RunSchedule> {
    let mut phase = phase();
    phase.phase = "child_local_validation".into();
    plan_source_advanced(
        &limits(),
        &phase,
        Uuid::from_u128(2),
        receipts,
        reference,
        now,
    )
}

#[test]
fn changed_source_repeats_required_phase_with_its_original_bounds_and_spacing() {
    let now = Utc::now();
    let previous = validation(1, now);
    let mut source = receipt(2, now);
    source.updated_at = now;
    let reference = advanced(&previous, &source);
    let history = [previous, source];
    assert_eq!(
        next_advanced(&history, &reference, now + Duration::milliseconds(999))
            .unwrap_err()
            .code,
        "retry_wait"
    );
    let next = next_advanced(&history, &reference, now + Duration::seconds(1)).unwrap();
    assert_eq!(
        (
            next.attempt,
            next.max_attempts,
            next.max_turns,
            next.max_review_rounds
        ),
        (2, 3, 20, 3)
    );
    assert_eq!(next.trigger, Trigger::VerifiedSourceAdvanced);
    assert_eq!(next.not_before, now + Duration::seconds(1));
    let mut phase = phase();
    phase.phase = "child_local_validation".into();
    assert_eq!(
        plan(&limits(), &phase, Uuid::from_u128(2), &history, now)
            .unwrap_err()
            .code,
        "no_work_trigger"
    );
}

#[test]
fn source_advancement_requires_exact_completed_implementation_and_expected_phase() {
    let now = Utc::now();
    let previous = validation(1, now);
    let source = receipt(2, now);
    for mutation in 0..4 {
        let mut source = source.clone();
        let mut reference = advanced(&previous, &source);
        match mutation {
            0 => reference.source_receipt_digest = digest("stale"),
            1 => {
                let mut run = RunReference::from_receipt(&source).unwrap().unwrap();
                run.phase.phase = "child_codex_review".into();
                source
                    .expected
                    .insert("run".into(), serde_json::to_string(&run).unwrap());
                reference.source_receipt_digest = canonical_digest(1, &source).unwrap();
            }
            2 => {
                source.observed_result.as_mut().unwrap()["observation"]["hook_error"] =
                    json!({"code":"timeout"});
                reference.source_receipt_digest = canonical_digest(1, &source).unwrap();
            }
            _ => {
                source.observed_result.as_mut().unwrap()["observation"]["session"]["terminal"] =
                    json!("failed");
                reference.source_receipt_digest = canonical_digest(1, &source).unwrap();
            }
        }
        assert_eq!(
            next_advanced(&[previous.clone(), source], &reference, now)
                .unwrap_err()
                .code,
            "stale_result"
        );
    }
    let reference = advanced(&previous, &source);
    assert_eq!(
        plan_source_advanced(
            &limits(),
            &phase(),
            Uuid::from_u128(2),
            &[previous, source],
            &reference,
            now
        )
        .unwrap_err()
        .code,
        "worker_binding"
    );
}

#[test]
fn validation_cannot_reuse_a_consumed_source_or_exceed_review_rounds() {
    let now = Utc::now();
    let previous = validation(1, now);
    let original_source = receipt(2, now);
    let consumed = advanced(&previous, &original_source);
    let mut second = validation(2, now);
    second.expected.insert(
        "source_advanced".into(),
        serde_json::to_string(&consumed).unwrap(),
    );
    let newer_source = receipt(3, now);
    let mut next = advanced(&second, &newer_source);
    next.trigger_digest = digest("new-trigger-same-material-source");
    let records = [previous, original_source, second, newer_source];
    assert_eq!(
        next_advanced(&records, &next, now).unwrap_err().code,
        "no_progress"
    );
    next.progress_digest = digest("new-source-tree");
    next.review_rounds_consumed = 3;
    assert_eq!(
        next_advanced(&records, &next, now).unwrap_err().code,
        "review_rounds_exhausted"
    );
    next.review_rounds_consumed = 2;
    assert_eq!(next_advanced(&records, &next, now).unwrap().attempt, 3);
}

#[test]
fn unjoined_or_stale_validation_predecessors_do_not_refresh_after_source_changes() {
    let now = Utc::now();
    let previous = validation(1, now);
    let source = receipt(2, now);
    let reference = advanced(&previous, &source);
    let mut unknown = source.clone();
    unknown.phase = ReceiptPhase::Sent;
    let unknown_reference = advanced(&previous, &unknown);
    assert_eq!(
        next_advanced(&[previous.clone(), unknown], &unknown_reference, now)
            .unwrap_err()
            .code,
        "reconciliation_required"
    );
    let later = validation(2, now);
    assert_eq!(
        next_advanced(
            &[previous.clone(), source.clone(), receipt(3, now)],
            &reference,
            now
        )
        .unwrap_err()
        .code,
        "stale_result"
    );
    assert_eq!(
        next_advanced(&[previous, later, source], &reference, now)
            .unwrap_err()
            .code,
        "stale_result"
    );
}
