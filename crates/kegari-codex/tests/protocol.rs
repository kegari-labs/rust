use kegari_codex::{
    CodexProfile, JsonRpcDecoder, MAX_FRAME_BYTES, RpcMessage, SUPPORTED_CODEX_VERSION,
    SessionBinding, UsageLedger,
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn binding() -> SessionBinding {
    SessionBinding {
        schema_version: 1,
        account_id: "account-one".into(),
        work_identity: "linear:workspace:child".into(),
        operation_id: "operation-one".into(),
        generation: "generation-one".into(),
        instruction_digest: "sha256:v1:instructions".into(),
        isolation_digest: "sha256:v1:scope".into(),
        profile: CodexProfile {
            model: "fixture-model".into(),
            reasoning: "high".into(),
            speed: "standard".into(),
        },
        cwd: PathBuf::from("/private/tmp/fixture/repository"),
    }
}

#[test]
fn fragmented_coalesced_json_rpc_is_atomic_and_bounded() {
    let mut decoder = JsonRpcDecoder::default();
    assert!(decoder.push(b"{\"id\":1,\"res").unwrap().is_empty());
    let messages = decoder
        .push(b"ult\":{}}\n{\"method\":\"tick\",\"params\":{}}\n")
        .unwrap();
    assert_eq!(messages.len(), 2);
    assert!(matches!(&messages[0],RpcMessage::Response{id,..} if id==1));
    decoder.finish().unwrap();
    let mut decoder = JsonRpcDecoder::default();
    assert!(
        decoder
            .push(&vec![b'x'; MAX_FRAME_BYTES])
            .unwrap()
            .is_empty()
    );
    assert_eq!(decoder.push(b"x").unwrap_err().code, "protocol_error");
    assert!(decoder.push(b"{}\n").is_err());
    let mut decoder = JsonRpcDecoder::default();
    assert!(decoder.push(b"{\"id\":null,\"result\":{}}\n").is_err());
}

#[test]
fn malformed_required_protocol_is_not_success() {
    for bytes in [
        b"[]\n".as_slice(),
        b"{\"id\":1,\"result\":{},\"error\":{}}\n",
        b"{\"method\":\"\"}\n",
        b"not-json\n",
    ] {
        assert!(JsonRpcDecoder::default().push(bytes).is_err());
    }
    let mut d = JsonRpcDecoder::default();
    d.push(b"{\"id\":1").unwrap();
    assert!(d.finish().is_err());
}

#[test]
fn usage_preserves_account_thread_baselines_on_recovery() {
    let mut usage = UsageLedger::default();
    assert_eq!(usage.observe("a", "t", 100, true), 100);
    assert_eq!(usage.observe("a", "t", 100, false), 0);
    let mut restored: UsageLedger =
        serde_json::from_slice(&serde_json::to_vec(&usage).unwrap()).unwrap();
    assert_eq!(restored.observe("a", "t", 110, false), 10);
    assert_eq!(restored.observe("a", "t", 80, false), 0);
    assert_eq!(restored.observe("a", "t", 115, false), 5);
    assert_eq!(restored.observe("b", "t", 60, false), 0);
    assert!(restored.earlier_usage_unknown.contains("b\0t"));
    restored.finish_elapsed("operation", 300);
    restored.finish_elapsed("operation", 500);
    assert_eq!(restored.ended_operations["operation"], 300);
    let prior = serde_json::to_value(&restored).unwrap();
    let mut next = restored.resumed_session("a", "t");
    assert!(next.observed_tokens.is_empty());
    assert!(next.ended_operations.is_empty());
    assert_eq!(next.observe("a", "t", 140, false), 25);
    assert_eq!(next.observe("a", "t", 140, false), 0);
    next.finish_elapsed("next-operation", 200);
    assert_eq!(next.ended_operations.len(), 1);
    assert_eq!(serde_json::to_value(&restored).unwrap(), prior);
    let mut unknown = restored.resumed_session("a", "unobserved-thread");
    assert!(
        unknown
            .earlier_usage_unknown
            .contains("a\0unobserved-thread")
    );
    assert_eq!(unknown.observe("a", "unobserved-thread", 75, false), 0);
    assert_eq!(unknown.observe("a", "unobserved-thread", 80, false), 5);
}

#[test]
fn profile_and_resume_cannot_fallback_or_cross_scope() {
    let p = binding().profile;
    let catalog = vec![
        json!({"model":"fixture-model","supportedReasoningEfforts":[{"reasoningEffort":"high"}],"serviceTiers":[]}),
    ];
    p.validate_catalog(&catalog).unwrap();
    let mut fast = p.clone();
    fast.speed = "fast".into();
    assert!(fast.validate_catalog(&catalog).is_err());
    let mut other = binding();
    other.account_id = "other".into();
    assert!(other.compatible_resume(&binding()).is_err());
    let mut resumed = binding();
    resumed.operation_id = "new-operation".into();
    resumed.compatible_resume(&binding()).unwrap();
    resumed.instruction_digest = "changed".into();
    assert!(resumed.compatible_resume(&binding()).is_err());
}

#[test]
fn checked_in_protocol_manifest_matches_bytes_and_schema_capabilities() {
    use sha2::{Digest, Sha256};
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("protocol/codex-0.164.0-alpha.11.1");
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["cli_version"], SUPPORTED_CODEX_VERSION);
    for file in manifest["files"].as_array().unwrap() {
        let bytes = std::fs::read(root.join(file["file"].as_str().unwrap())).unwrap();
        assert_eq!(
            hex::encode(Sha256::digest(&bytes)),
            file["sha256"].as_str().unwrap()
        );
        let _: Value = serde_json::from_slice(&bytes).unwrap();
    }
    let steer: Value =
        serde_json::from_slice(&std::fs::read(root.join("v2_TurnSteerParams.json")).unwrap())
            .unwrap();
    assert!(
        steer["required"]
            .as_array()
            .unwrap()
            .contains(&json!("expectedTurnId"))
    );
    let errors: Value =
        serde_json::from_slice(&std::fs::read(root.join("v2_ErrorNotification.json")).unwrap())
            .unwrap();
    assert!(
        errors["definitions"]["CodexErrorInfo"]["anyOf"][0]["enum"]
            .as_array()
            .unwrap()
            .contains(&json!("misalignmentPolicyViolation"))
    );
}
