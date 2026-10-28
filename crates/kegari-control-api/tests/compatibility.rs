use kegari_control_api::{
    api::evaluation_trial_identity,
    identity::{Actor, Principal},
};
use kegari_types::{
    api::{CommandName, CommandRequest},
    error::Error,
};
use serde_json::json;
use uuid::Uuid;

#[test]
fn actor_and_principal_serialization_preserves_existing_contract() {
    let actor = Actor {
        identity: "operator".into(),
        roles: ["inspect".into(), "configure".into()].into(),
        principal: Some(Principal::Unix { uid: 501 }),
    };
    let encoded = r#"{"identity":"operator","roles":["configure","inspect"],"principal":{"transport":"unix","uid":501}}"#;
    assert_eq!(serde_json::to_string(&actor).unwrap(), encoded);
    assert_eq!(serde_json::from_str::<Actor>(encoded).unwrap(), actor);
    let principal = Principal::Tailscale {
        user_id: 42,
        node_id: "node-7".into(),
    };
    let encoded = r#"{"transport":"tailscale","user_id":42,"node_id":"node-7"}"#;
    assert_eq!(serde_json::to_string(&principal).unwrap(), encoded);
    assert_eq!(
        serde_json::from_str::<Principal>(encoded).unwrap(),
        principal
    );
    let mut absent = actor.clone();
    absent.principal = None;
    assert_eq!(
        serde_json::to_value(&absent).unwrap()["principal"],
        serde_json::Value::Null
    );
    assert_eq!(
        serde_json::from_value::<Actor>(
            json!({"identity":"operator","roles":["inspect","configure"]})
        )
        .unwrap(),
        absent
    );
    for value in [
        json!({"identity":"operator","roles":["inspect"],"principal":null,"extra":true}),
        json!({"identity":"operator","roles":["inspect"],"principal":{"transport":"unix","uid":501,"extra":true}}),
        json!({"identity":"operator","roles":["inspect"],"principal":{"transport":"tailscale","user_id":42,"node_id":"node-7","extra":true}}),
        json!({"identity":"operator","roles":["inspect"],"principal":{"transport":"unix"}}),
        json!({"roles":["inspect"],"principal":null}),
    ] {
        assert!(serde_json::from_value::<Actor>(value).is_err());
    }
}

#[test]
fn actor_role_check_preserves_errors_and_does_not_replace_principal_validation() {
    let mut actor = Actor {
        identity: "operator".into(),
        roles: ["inspect".into()].into(),
        principal: None,
    };
    actor.require("inspect").unwrap();
    assert_eq!(
        actor.require("configure").unwrap_err(),
        Error::new(
            "access_denied",
            "authenticated actor lacks configure authority"
        )
    );
    actor.identity = " \t".into();
    assert_eq!(
        actor.require("inspect").unwrap_err(),
        Error::new(
            "access_denied",
            "authenticated actor lacks inspect authority"
        )
    );
}

#[test]
fn evaluation_trial_identity_preserves_fixed_canonical_vector() {
    let mut request = CommandRequest {
        api_version: 1,
        runtime_id: Uuid::parse_str("11111111-1111-4111-8111-111111111111").unwrap(),
        request_id: Uuid::parse_str("33333333-3333-4333-8333-333333333333").unwrap(),
        command: CommandName::EvaluationRun,
        args: json!({
            "workspace":"22222222-2222-4222-8222-222222222222",
            "suite_digest":format!("sha256:{}", "a".repeat(64)),
            "case":"V0-E05", "variant":"independent_malicious",
            "profile":"operator_evaluated", "repetition":2
        }),
        expected_config_revision: Some(format!("sha256:{}", "b".repeat(64))),
        expected_control_generation: Some(7),
    };
    // Captured from the pre-extraction field/schema definition with an independent
    // SHA-256 calculation over sorted, compact UTF-8 JSON (no application helper).
    let expected = "sha256:v1:d137937a413707a64129f2e12a8599952061567393c4dee921e5b5a2ecae08b9";
    assert_eq!(evaluation_trial_identity(&request).unwrap(), expected);
    request.request_id = Uuid::parse_str("44444444-4444-4444-8444-444444444444").unwrap();
    request.expected_control_generation = Some(8);
    assert_eq!(evaluation_trial_identity(&request).unwrap(), expected);
    request.args["repetition"] = json!(3);
    assert_ne!(evaluation_trial_identity(&request).unwrap(), expected);
    request.args["repetition"] = json!(0);
    assert_eq!(
        evaluation_trial_identity(&request).unwrap_err(),
        Error::new(
            "invalid_input",
            "evaluation needs exact declared repetition and expected config/control revisions"
        )
    );
    request.command = CommandName::Status;
    request.args = json!({});
    assert_eq!(
        evaluation_trial_identity(&request).unwrap_err(),
        Error::invalid("not an evaluation command")
    );
}
