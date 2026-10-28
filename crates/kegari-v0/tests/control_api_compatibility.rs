use kegari_control_api::api::validate_command_request;
use kegari_types::api::{CommandName, CommandRequest};
use kegari_v0::evidence::validate_digest;
use serde_json::json;
use uuid::Uuid;

#[test]
fn evaluation_digest_syntax_matches_backend_evidence_validation() {
    let mut request = CommandRequest {
        api_version: 1,
        runtime_id: Uuid::from_u128(1),
        request_id: Uuid::from_u128(2),
        command: CommandName::EvaluationRun,
        args: json!({"workspace": Uuid::from_u128(3), "suite_digest":"replaced", "case":"V0-E05", "variant":"fixture", "profile":"fixture", "repetition":1}),
        expected_config_revision: Some("fixture-revision".into()),
        expected_control_generation: Some(1),
    };
    for digest in [
        format!("sha256:{}", "a".repeat(64)),
        format!("sha256:v1:{}", "0".repeat(64)),
        format!("sha256:{}", "A".repeat(64)),
        format!("sha256:{}", "g".repeat(64)),
        format!("sha256:{}", "a".repeat(63)),
        format!("sha256:{}", "a".repeat(65)),
        format!("sha256:v2:{}", "a".repeat(64)),
        "a".repeat(64),
        format!(" sha256:{}", "a".repeat(64)),
        format!("sha256:{} ", "a".repeat(64)),
        format!("sha256:{}", "é".repeat(32)),
    ] {
        request.args["suite_digest"] = json!(digest);
        assert_eq!(
            validate_command_request(&request, request.runtime_id),
            validate_digest(&digest),
            "digest format: {digest}"
        );
    }
}
