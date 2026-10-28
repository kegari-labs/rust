use chrono::{DateTime, Utc};
use kegari_scheduler::*;
use kegari_types::run::PhaseKey;
use serde::Serialize;
use serde_json::{Value, json};
use uuid::Uuid;

fn fixed_time() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-01-02T03:04:05Z")
        .unwrap()
        .with_timezone(&Utc)
}
fn fixed_work() -> ReadyWork {
    ReadyWork {
        canonical_key: "work:fixture".into(),
        workspace: Uuid::from_u128(1),
        account: "fixture-account".into(),
        generation: "generation-1".into(),
        state: "Ready".into(),
        priority: Some(3),
        created_at: Some(fixed_time()),
        kind: WorkKind::Child,
        trigger: Some(Trigger::InitialAuthorization),
        not_before: fixed_time(),
        gate_blockers: vec!["fixture-blocker".into()],
    }
}
fn fixed_attempt() -> AttemptRecord {
    AttemptRecord {
        operation_id: Uuid::from_u128(2),
        key: PhaseKey {
            work: "work:fixture".into(),
            generation: "generation-1".into(),
            phase: "implementation".into(),
            retry_grant: None,
        },
        ordinal: 1,
        turns: 2,
        review_round: 0,
        progress_digest: None,
        outcome: "running".into(),
    }
}
fn encoded(value: &impl Serialize) -> String {
    serde_json::to_string(value).unwrap()
}
fn samples() -> Value {
    let mut optional_work = fixed_work();
    optional_work.priority = None;
    optional_work.created_at = None;
    optional_work.trigger = None;
    let mut optional_attempt = fixed_attempt();
    optional_attempt.key.retry_grant = Some(Uuid::from_u128(3));
    optional_attempt.progress_digest = Some("fixture-progress-digest".into());
    json!({
        "triggers": ([
            Trigger::InitialAuthorization, Trigger::Correction,
            Trigger::DependencySatisfied, Trigger::ScheduledRetry,
            Trigger::VerifiedUnfinished, Trigger::VerifiedSourceAdvanced,
            Trigger::VerifiedParentReassembly, Trigger::OperatorRetry,
        ].iter().map(encoded).collect::<Vec<_>>()),
        "work_kinds": ([WorkKind::Child, WorkKind::Coordination, WorkKind::IndependentReview]
            .iter().map(encoded).collect::<Vec<_>>()),
        "ready_full": encoded(&fixed_work()),
        "ready_optional_nulls": encoded(&optional_work),
        "allocation": encoded(&Allocation {
            operation: Uuid::from_u128(2), work: fixed_work(), started_at: fixed_time(),
        }),
        "quotas": ([
            Quota::Unknown,
            Quota::Available { observed_at: fixed_time() },
            Quota::Exhausted { observed_at: fixed_time(), reset_at: None },
            Quota::Exhausted { observed_at: fixed_time(), reset_at: Some(fixed_time()) },
        ].iter().map(encoded).collect::<Vec<_>>()),
        "attempt_nulls": encoded(&fixed_attempt()),
        "attempt_supplied": encoded(&optional_attempt),
    })
}

#[test]
fn scheduler_serialization_matches_original_bytes() {
    let baseline: Value =
        serde_json::from_str(include_str!("fixtures/serialization.json")).unwrap();
    assert_eq!(samples(), baseline);
}

fn roundtrip<T: serde::de::DeserializeOwned + Serialize>(bytes: &str) {
    let decoded: T = serde_json::from_str(bytes).unwrap();
    assert_eq!(encoded(&decoded), bytes);
}
fn unknown_is_ignored<T: serde::de::DeserializeOwned + Serialize>(bytes: &str) {
    let mut value: Value = serde_json::from_str(bytes).unwrap();
    value["future_field"] = json!({"descriptive":true});
    let decoded: T = serde_json::from_value(value).unwrap();
    assert_eq!(encoded(&decoded), bytes);
}

#[test]
fn scheduler_roundtrips_and_unknown_field_rules_are_unchanged() {
    let baseline: Value =
        serde_json::from_str(include_str!("fixtures/serialization.json")).unwrap();
    for value in baseline["triggers"].as_array().unwrap() {
        roundtrip::<Trigger>(value.as_str().unwrap());
    }
    for value in baseline["work_kinds"].as_array().unwrap() {
        roundtrip::<WorkKind>(value.as_str().unwrap());
    }
    for key in ["ready_full", "ready_optional_nulls"] {
        let bytes = baseline[key].as_str().unwrap();
        roundtrip::<ReadyWork>(bytes);
        unknown_is_ignored::<ReadyWork>(bytes);
    }
    let allocation = baseline["allocation"].as_str().unwrap();
    roundtrip::<Allocation>(allocation);
    unknown_is_ignored::<Allocation>(allocation);
    let mut nested: Value = serde_json::from_str(allocation).unwrap();
    nested["work"]["future_field"] = json!(true);
    assert_eq!(
        encoded(&serde_json::from_value::<Allocation>(nested).unwrap()),
        allocation
    );
    for value in baseline["quotas"].as_array().unwrap() {
        let bytes = value.as_str().unwrap();
        roundtrip::<Quota>(bytes);
        unknown_is_ignored::<Quota>(bytes);
    }
    for key in ["attempt_nulls", "attempt_supplied"] {
        let bytes = baseline[key].as_str().unwrap();
        roundtrip::<AttemptRecord>(bytes);
        let mut value: Value = serde_json::from_str(bytes).unwrap();
        value["future_field"] = json!(true);
        assert!(serde_json::from_value::<AttemptRecord>(value).is_err());
        let mut nested: Value = serde_json::from_str(bytes).unwrap();
        nested["key"]["future_field"] = json!(true);
        assert_eq!(
            encoded(&serde_json::from_value::<AttemptRecord>(nested).unwrap()),
            bytes
        );
    }
}

fn omitted_matches_null<T: serde::de::DeserializeOwned + Serialize>(bytes: &str, field: &str) {
    let mut value: Value = serde_json::from_str(bytes).unwrap();
    value.as_object_mut().unwrap().remove(field);
    let omitted: T = serde_json::from_value(value.clone()).unwrap();
    value[field] = Value::Null;
    let explicit_null: T = serde_json::from_value(value).unwrap();
    assert_eq!(encoded(&omitted), encoded(&explicit_null));
    assert_eq!(serde_json::to_value(omitted).unwrap()[field], Value::Null);
}

#[test]
fn scheduler_optional_fields_preserve_missing_and_explicit_null() {
    let baseline: Value =
        serde_json::from_str(include_str!("fixtures/serialization.json")).unwrap();
    for field in ["priority", "created_at", "trigger"] {
        omitted_matches_null::<ReadyWork>(baseline["ready_full"].as_str().unwrap(), field);
    }
    omitted_matches_null::<Quota>(baseline["quotas"][3].as_str().unwrap(), "reset_at");
    omitted_matches_null::<AttemptRecord>(
        baseline["attempt_supplied"].as_str().unwrap(),
        "progress_digest",
    );
    let mut missing: Value =
        serde_json::from_str(baseline["attempt_nulls"].as_str().unwrap()).unwrap();
    missing["key"]
        .as_object_mut()
        .unwrap()
        .remove("retry_grant");
    let decoded: AttemptRecord = serde_json::from_value(missing).unwrap();
    assert_eq!(
        encoded(&decoded),
        baseline["attempt_nulls"].as_str().unwrap()
    );
}
