use kegari_types::run::{PhaseKey, RunReference};
use serde_json::{Value, json};
use uuid::Uuid;

fn reference() -> RunReference {
    RunReference {
        workspace: Uuid::parse_str("01234567-89ab-cdef-0123-456789abcdef").unwrap(),
        account: "synthetic-account".into(),
        normalized_state: None,
        phase: PhaseKey {
            work: "issue:42".into(),
            generation: "a".repeat(64),
            phase: "implementation".into(),
            retry_grant: None,
        },
        profile_digest: "b".repeat(64),
        instruction_digest: "c".repeat(64),
        max_attempts: 3,
        max_turns: 20,
        max_review_rounds: 2,
        attempt: 1,
    }
}

#[test]
fn run_reference_retains_exact_serialized_fields_and_order() {
    let expected = concat!(
        "{\"workspace\":\"01234567-89ab-cdef-0123-456789abcdef\",",
        "\"account\":\"synthetic-account\",\"normalized_state\":null,",
        "\"phase\":{\"work\":\"issue:42\",",
        "\"generation\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",",
        "\"phase\":\"implementation\",\"retry_grant\":null},",
        "\"profile_digest\":\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\",",
        "\"instruction_digest\":\"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc\",",
        "\"max_attempts\":3,\"max_turns\":20,\"max_review_rounds\":2,\"attempt\":1}"
    );
    assert_eq!(serde_json::to_string(&reference()).unwrap(), expected);
    let decoded: RunReference = serde_json::from_str(expected).unwrap();
    assert_eq!(serde_json::to_string(&decoded).unwrap(), expected);
}

#[test]
fn run_reference_preserves_omitted_null_and_supplied_state() {
    let mut value = serde_json::to_value(reference()).unwrap();
    value.as_object_mut().unwrap().remove("normalized_state");
    let omitted: RunReference = serde_json::from_value(value.clone()).unwrap();
    assert!(omitted.normalized_state.is_none());
    assert_eq!(
        serde_json::to_value(omitted).unwrap()["normalized_state"],
        Value::Null
    );
    value["normalized_state"] = Value::Null;
    assert!(
        serde_json::from_value::<RunReference>(value.clone())
            .unwrap()
            .normalized_state
            .is_none()
    );
    value["normalized_state"] = json!("in progress");
    assert_eq!(
        serde_json::from_value::<RunReference>(value)
            .unwrap()
            .normalized_state
            .as_deref(),
        Some("in progress")
    );
}

#[test]
fn run_reference_refuses_unknown_missing_and_malformed_fields() {
    let value = serde_json::to_value(reference()).unwrap();
    let mut unknown = value.clone();
    unknown["unexpected"] = json!(true);
    assert!(serde_json::from_value::<RunReference>(unknown).is_err());
    for field in [
        "workspace",
        "account",
        "phase",
        "profile_digest",
        "instruction_digest",
        "max_attempts",
        "max_turns",
        "max_review_rounds",
        "attempt",
    ] {
        let mut missing = value.clone();
        missing.as_object_mut().unwrap().remove(field);
        assert!(
            serde_json::from_value::<RunReference>(missing).is_err(),
            "missing {field}"
        );
        let mut null = value.clone();
        null[field] = Value::Null;
        assert!(
            serde_json::from_value::<RunReference>(null).is_err(),
            "null {field}"
        );
    }
    let mut wrong_state = value.clone();
    wrong_state["normalized_state"] = json!(42);
    assert!(serde_json::from_value::<RunReference>(wrong_state).is_err());
    let mut wrong_count = value;
    wrong_count["max_turns"] = json!(-1);
    assert!(serde_json::from_value::<RunReference>(wrong_count).is_err());
}

#[test]
fn phase_key_preserves_unknown_field_acceptance_and_optional_retry_grant() {
    let phase = reference().phase;
    let encoded = serde_json::to_string(&phase).unwrap();
    assert_eq!(
        encoded,
        "{\"work\":\"issue:42\",\"generation\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"phase\":\"implementation\",\"retry_grant\":null}"
    );
    let mut value = serde_json::to_value(&phase).unwrap();
    value["future_field"] = json!({"accepted": true});
    assert_eq!(
        serde_json::from_value::<PhaseKey>(value.clone()).unwrap(),
        phase
    );
    value.as_object_mut().unwrap().remove("retry_grant");
    assert_eq!(
        serde_json::from_value::<PhaseKey>(value.clone()).unwrap(),
        phase
    );
    let retry = Uuid::parse_str("fedcba98-7654-3210-fedc-ba9876543210").unwrap();
    value["retry_grant"] = json!(retry);
    assert_eq!(
        serde_json::from_value::<PhaseKey>(value)
            .unwrap()
            .retry_grant,
        Some(retry)
    );
    let mut run = serde_json::to_value(reference()).unwrap();
    run["phase"]["future_field"] = json!(true);
    assert!(serde_json::from_value::<RunReference>(run).is_ok());
}
