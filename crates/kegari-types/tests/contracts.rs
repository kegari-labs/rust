//! Wire and formatting contracts only; these DTOs confer no runtime authority.
use chrono::{DateTime, Utc};
use kegari_types::{
    api::{CommandName, CommandRequest, Envelope},
    error::Error,
    snapshot::{Node, Page, Segment},
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use uuid::Uuid;

const RUNTIME: &str = "00000000-0000-0000-0000-000000000001";
const REQUEST: &str = "00000000-0000-0000-0000-000000000002";
const OBSERVED: &str = "2026-10-28T12:00:00Z";
const EXPIRES: &str = "2026-10-28T12:05:00Z";

fn id(value: &str) -> Uuid {
    Uuid::parse_str(value).unwrap()
}
fn time(value: &str) -> DateTime<Utc> {
    value.parse().unwrap()
}
fn roundtrip<T: Serialize + DeserializeOwned>(expected: Value) {
    let decoded: T = serde_json::from_value(expected.clone()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), expected);
}
fn rejects_unknown<T: DeserializeOwned>(mut value: Value) {
    value["unexpected"] = json!(true);
    assert!(serde_json::from_value::<T>(value).is_err());
}

#[test]
fn all_command_names_keep_their_existing_wire_spellings() {
    use CommandName::*;
    for (command, name) in [
        (Overview, "overview"),
        (Status, "status"),
        (Show, "show"),
        (Open, "open"),
        (Approve, "approve"),
        (Correct, "correct"),
        (Release, "release"),
        (Setup, "setup"),
        (ConnectionList, "connection_list"),
        (ConnectionAdd, "connection_add"),
        (ConnectionReconnect, "connection_reconnect"),
        (WorkspaceList, "workspace_list"),
        (WorkspaceAdd, "workspace_add"),
        (WorkspaceConfigure, "workspace_configure"),
        (ConfigShow, "config_show"),
        (ConfigCheck, "config_check"),
        (Pause, "pause"),
        (Resume, "resume"),
        (Stop, "stop"),
        (Start, "start"),
        (ServiceStop, "service_stop"),
        (EvaluationRun, "evaluation_run"),
        (OperationShow, "operation_show"),
        (OperationReconcile, "operation_reconcile"),
        (Retry, "retry"),
        (ReleaseRecover, "release_recover"),
        (Cleanup, "cleanup"),
    ] {
        assert_eq!(serde_json::to_value(&command).unwrap(), json!(name));
        assert_eq!(
            serde_json::from_value::<CommandName>(json!(name)).unwrap(),
            command
        );
    }
    for invalid in [json!("Status"), json!("unknown"), Value::Null, json!({})] {
        assert!(serde_json::from_value::<CommandName>(invalid).is_err());
    }
}

#[test]
fn request_shape_optional_fields_and_debug_remain_compatible() {
    let request = CommandRequest {
        api_version: 1,
        runtime_id: id(RUNTIME),
        request_id: id(REQUEST),
        command: CommandName::Correct,
        args: json!({"message":"private-input-marker"}),
        expected_config_revision: None,
        expected_control_generation: None,
    };
    assert_eq!(
        serde_json::to_string(&request).unwrap(),
        r#"{"api_version":1,"runtime_id":"00000000-0000-0000-0000-000000000001","request_id":"00000000-0000-0000-0000-000000000002","command":"correct","args":{"message":"private-input-marker"},"expected_config_revision":null,"expected_control_generation":null}"#
    );
    let explicit_null = serde_json::to_value(&request).unwrap();
    roundtrip::<CommandRequest>(explicit_null.clone());
    let mut omitted = explicit_null.clone();
    for field in ["expected_config_revision", "expected_control_generation"] {
        omitted.as_object_mut().unwrap().remove(field);
    }
    let decoded: CommandRequest = serde_json::from_value(omitted).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), explicit_null);
    let mut populated = explicit_null.clone();
    populated["expected_config_revision"] = json!("revision-fixture");
    populated["expected_control_generation"] = json!(7);
    roundtrip::<CommandRequest>(populated);
    rejects_unknown::<CommandRequest>(explicit_null);
    assert_eq!(
        format!("{request:?}"),
        format!(
            "CommandRequest {{ runtime_id: {RUNTIME}, request_id: {REQUEST}, command: Correct, args: \"[OMITTED]\" }}"
        )
    );
    assert!(!format!("{request:?}").contains("private-input-marker"));
}

#[test]
fn envelope_preserves_explicit_nulls_and_nested_errors() {
    let envelope = Envelope {
        api_version: 1,
        request_id: id(REQUEST),
        runtime_id: id(RUNTIME),
        config_revision: None,
        observed_at: time(OBSERVED),
        ok: true,
        operation_id: None,
        state: None,
        data: json!({"observed":true}),
        error: None,
    };
    let expected = json!({
        "api_version":1,"request_id":REQUEST,"runtime_id":RUNTIME,
        "config_revision":null,"observed_at":OBSERVED,"ok":true,
        "operation_id":null,"state":null,"data":{"observed":true},"error":null
    });
    assert_eq!(serde_json::to_value(envelope).unwrap(), expected);
    roundtrip::<Envelope>(expected.clone());
    let mut omitted = expected.clone();
    for field in ["config_revision", "operation_id", "state", "error"] {
        omitted.as_object_mut().unwrap().remove(field);
    }
    let decoded: Envelope = serde_json::from_value(omitted).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), expected);
    rejects_unknown::<Envelope>(expected.clone());
    let mut failed = expected;
    failed["ok"] = json!(false);
    failed["config_revision"] = json!("revision-fixture");
    failed["operation_id"] = json!(REQUEST);
    failed["state"] = json!("blocked");
    failed["data"] = Value::Null;
    failed["error"] = json!({"code":"fixture_error","message":"safe message",
        "retryable":false,"details":null,"next_action":null});
    roundtrip::<Envelope>(failed.clone());
    failed["error"]["unexpected"] = json!(true);
    assert!(serde_json::from_value::<Envelope>(failed).is_err());
}

#[test]
fn errors_keep_safe_constructors_formatting_and_wire_shape() {
    let base = Error::new("fixture_error", "safe message");
    let expected = json!({"code":"fixture_error","message":"safe message",
        "retryable":false,"details":null,"next_action":null});
    assert_eq!(serde_json::to_value(&base).unwrap(), expected);
    roundtrip::<Error>(expected.clone());
    let mut omitted = expected.clone();
    omitted.as_object_mut().unwrap().remove("next_action");
    assert_eq!(serde_json::from_value::<Error>(omitted).unwrap(), base);
    rejects_unknown::<Error>(expected);
    let detailed = base
        .clone()
        .detail(json!({"operation":REQUEST}))
        .next("Inspect status");
    roundtrip::<Error>(json!({"code":"fixture_error","message":"safe message",
        "retryable":false,"details":{"operation":REQUEST},"next_action":"Inspect status"}));
    assert_eq!(detailed.to_string(), "fixture_error: safe message");
    assert_eq!(detailed.details, json!({"operation":REQUEST}));
    assert_eq!(detailed.next_action.as_deref(), Some("Inspect status"));
    assert_eq!(Error::invalid("invalid fixture").code, "invalid_config");
    let safe = Error::io(
        "read fixture",
        std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "private-os-error-marker",
        ),
    );
    assert_eq!(
        safe.to_string(),
        "io_error: read fixture: permission denied"
    );
    assert!(
        !serde_json::to_string(&safe)
            .unwrap()
            .contains("private-os-error-marker")
    );
    assert!(std::error::Error::source(&safe).is_none());
}

#[test]
fn snapshot_variants_and_page_shape_roundtrip_without_implicit_policy() {
    let entries = vec![
        Node::Object {
            path: vec![],
            keys: vec!["items".into(), "text".into()],
        },
        Node::Array {
            path: vec![Segment::Field("items".into())],
            length: 1,
        },
        Node::Scalar {
            path: vec![Segment::Field("items".into()), Segment::Index(0)],
            value: json!(42),
        },
        Node::Text {
            path: vec![Segment::Field("text".into())],
            offset: 0,
            total_bytes: 2,
            complete_digest: "digest-fixture".into(),
            text: "é".into(),
        },
    ];
    let expected_entries = json!([
        {"type":"object","path":[],"keys":["items","text"]},
        {"type":"array","path":[{"type":"field","value":"items"}],"length":1},
        {"type":"scalar","path":[{"type":"field","value":"items"},{"type":"index","value":0}],"value":42},
        {"type":"text","path":[{"type":"field","value":"text"}],"offset":0,"total_bytes":2,"complete_digest":"digest-fixture","text":"é"}
    ]);
    assert_eq!(serde_json::to_value(&entries).unwrap(), expected_entries);
    for entry in expected_entries.as_array().unwrap() {
        roundtrip::<Node>(entry.clone());
        rejects_unknown::<Node>(entry.clone());
    }
    assert!(serde_json::from_value::<Node>(json!({"type":"unknown","path":[]})).is_err());
    roundtrip::<Segment>(json!({"type":"field","value":"items"}));
    roundtrip::<Segment>(json!({"type":"index","value":0}));
    assert_eq!(
        serde_json::from_value::<Segment>(
            json!({"type":"field","value":"items","unexpected":true})
        )
        .unwrap(),
        Segment::Field("items".into())
    );
    assert!(serde_json::from_value::<Segment>(json!({"type":"index","value":-1})).is_err());
    let page = Page {
        snapshot_id: id(RUNTIME),
        observed_at: time(OBSERVED),
        expires_at: time(EXPIRES),
        projection_digest: "projection-fixture".into(),
        cursor: None,
        next_cursor: Some(id(REQUEST)),
        total_entries: 4,
        entries,
    };
    let expected = json!({"snapshot_id":RUNTIME,"observed_at":OBSERVED,"expires_at":EXPIRES,
        "projection_digest":"projection-fixture","cursor":null,"next_cursor":REQUEST,
        "total_entries":4,"entries":expected_entries});
    assert_eq!(serde_json::to_value(page).unwrap(), expected);
    roundtrip::<Page>(expected.clone());
    rejects_unknown::<Page>(expected.clone());
    let mut final_page = expected;
    final_page["cursor"] = json!(REQUEST);
    final_page["next_cursor"] = Value::Null;
    roundtrip::<Page>(final_page.clone());
    final_page.as_object_mut().unwrap().remove("next_cursor");
    let decoded: Page = serde_json::from_value(final_page).unwrap();
    assert!(decoded.next_cursor.is_none());
}
