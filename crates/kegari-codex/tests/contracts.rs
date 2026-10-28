use kegari_codex::{
    CodexConnectionMetadata, EventKind, PermissionConfiguration, PriorUnattributedUsage,
    ProviderIntervention, SessionBinding, SessionEvent, UsageLedger, app_server_argv,
};
use kegari_types::error::Error;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

fn exact_contract<T: DeserializeOwned + Serialize>(golden: &str) -> T {
    let value: T = serde_json::from_str(golden).unwrap();
    assert_eq!(serde_json::to_string(&value).unwrap(), golden);
    let mut unknown: Value = serde_json::from_str(golden).unwrap();
    unknown
        .as_object_mut()
        .unwrap()
        .insert("unexpected".into(), json!(true));
    assert!(serde_json::from_value::<T>(unknown).is_err());
    value
}

#[test]
fn fixed_protocol_data_serialization_and_unknown_fields_remain_unchanged() {
    // Field order, optional nulls and omitted accounting extensions are pinned
    // to the original backend serde definitions, independently of this crate.
    let binding = exact_contract::<SessionBinding>(
        r#"{"schema_version":1,"account_id":"account","work_identity":"work","operation_id":"operation","generation":"generation","instruction_digest":"instructions","isolation_digest":"isolation","profile":{"model":"model","reasoning":"high","speed":"standard"},"cwd":"/fixture/repository"}"#,
    );
    binding.validate().unwrap();
    exact_contract::<PermissionConfiguration>(
        r#"{"profile_id":"profile","settings":{"approval_policy":"never"},"isolation_digest":"isolation"}"#,
    );
    exact_contract::<UsageLedger>(
        r#"{"cumulative_baselines":{},"observed_tokens":{},"earlier_usage_unknown":[],"ended_operations":{}}"#,
    );
    exact_contract::<PriorUnattributedUsage>(
        r#"{"known_increase":3,"retained_baseline_missing":true,"timing_unknown":false}"#,
    );
    exact_contract::<ProviderIntervention>(
        r#"{"schema_version":1,"account_id":"account","thread_id":"thread","turn_id":"turn","source_method":"method","reason_code":"reason","required_resolution":"resolve","observed_at":"2026-10-28T00:00:00+00:00","source_digest":"digest"}"#,
    );
    exact_contract::<SessionEvent>(
        r#"{"kind":"notification","method":"notice","thread_id":null,"turn_id":null,"observed_at":"2026-10-28T00:00:00+00:00","data":null}"#,
    );
    exact_contract::<CodexConnectionMetadata>(
        r#"{"supported_version":"codex-cli 0.164.0-alpha.11.1","protocol_profile":"kegari-codex-0.164.0-alpha.11.1-v1","account_id":"account","auth_home":"/fixture/home","permission_digest":"digest","argument_digest":null,"model_catalog":[],"rate_limits":null,"observed_at":"2026-10-28T00:00:00+00:00"}"#,
    );
}

#[test]
fn existing_optional_defaults_nulls_and_required_fields_remain_unchanged() {
    let legacy = json!({"known_increase":3,"retained_baseline_missing":true});
    let prior: PriorUnattributedUsage = serde_json::from_value(legacy.clone()).unwrap();
    assert!(!prior.timing_unknown);
    assert_eq!(prior.first_new_turn_boundary, None);
    let mut null_boundary = legacy;
    null_boundary["first_new_turn_boundary"] = Value::Null;
    assert_eq!(
        serde_json::from_value::<PriorUnattributedUsage>(null_boundary).unwrap(),
        prior
    );
    assert_eq!(
        serde_json::to_value(prior).unwrap(),
        json!({"known_increase":3,"retained_baseline_missing":true,"timing_unknown":false})
    );
    let base = json!({"cumulative_baselines":{},"observed_tokens":{},"earlier_usage_unknown":[],"ended_operations":{}});
    let usage: UsageLedger = serde_json::from_value(base.clone()).unwrap();
    assert_eq!(serde_json::to_value(usage).unwrap(), base);
    let mut invalid = base.clone();
    invalid["prior_unattributed_usage"] = Value::Null;
    assert!(serde_json::from_value::<UsageLedger>(invalid).is_err());
    let mut missing = base;
    missing.as_object_mut().unwrap().remove("observed_tokens");
    assert!(serde_json::from_value::<UsageLedger>(missing).is_err());
    let event: SessionEvent = serde_json::from_value(
        json!({"kind":"notification","method":"notice","observed_at":"fixed","data":null}),
    )
    .unwrap();
    assert!(event.thread_id.is_none() && event.turn_id.is_none());
    assert_eq!(
        serde_json::to_value(event).unwrap(),
        json!({"kind":"notification","method":"notice","thread_id":null,"turn_id":null,"observed_at":"fixed","data":null})
    );
    assert!(
        serde_json::from_value::<PermissionConfiguration>(
            json!({"profile_id":"profile","settings":{}})
        )
        .is_err()
    );
}

#[test]
fn event_kind_spellings_and_event_construction_remain_data_only() {
    for (kind, spelling) in [
        (EventKind::SessionStarted, "session_started"),
        (EventKind::StartupFailure, "startup_failure"),
        (EventKind::TurnStarted, "turn_started"),
        (EventKind::TurnCompleted, "turn_completed"),
        (EventKind::TurnFailed, "turn_failed"),
        (EventKind::TurnCancelled, "turn_cancelled"),
        (EventKind::InputRequired, "input_required"),
        (EventKind::ProviderIntervention, "provider_intervention"),
        (EventKind::ToolResult, "tool_result"),
        (EventKind::UnsupportedTool, "unsupported_tool"),
        (EventKind::Notification, "notification"),
        (EventKind::ProtocolError, "protocol_error"),
        (EventKind::ProcessExit, "process_exit"),
        (EventKind::ModelChanged, "model_changed"),
    ] {
        assert_eq!(serde_json::to_value(&kind).unwrap(), json!(spelling));
        assert_eq!(
            serde_json::from_value::<EventKind>(json!(spelling)).unwrap(),
            kind
        );
    }
    assert!(serde_json::from_value::<EventKind>(json!("future_kind")).is_err());
    let event = SessionEvent::new(
        EventKind::Notification,
        "notice",
        Some("thread".into()),
        None,
        json!({"value":3}),
    );
    assert_eq!(event.method, "notice");
    assert_eq!(event.thread_id.as_deref(), Some("thread"));
    assert_eq!(event.turn_id, None);
    assert_eq!(event.data, json!({"value":3}));
    assert!(chrono::DateTime::parse_from_rfc3339(&event.observed_at).is_ok());
}

#[test]
fn argument_encoding_preserves_nested_values_escaping_and_error_messages() {
    let mut permissions = PermissionConfiguration {
        profile_id: "profile".into(),
        isolation_digest: "digest".into(),
        settings: json!({"z":false,"a":{"quoted\"key":["line\n\"value",true,2]}}),
    };
    assert_eq!(
        app_server_argv(&permissions).unwrap(),
        vec![
            "app-server",
            "--listen",
            "stdio://",
            "--strict-config",
            "-c",
            r#"a={"quoted\"key"=["line\n\"value",true,2]}"#,
            "-c",
            "z=false",
        ]
    );
    permissions.settings = Value::Null;
    assert_eq!(
        app_server_argv(&permissions).unwrap_err(),
        Error::invalid("permission settings must be an object")
    );
    permissions.settings = json!({"a":[false,null]});
    assert_eq!(
        app_server_argv(&permissions).unwrap_err(),
        Error::invalid("null cannot be a Codex config override")
    );
}
