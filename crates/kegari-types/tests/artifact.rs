use kegari_types::artifact::ArtifactProof;
use serde_json::{Value, json};
use std::path::PathBuf;
use uuid::Uuid;

fn proof() -> ArtifactProof {
    ArtifactProof {
        path: PathBuf::from("/fixture/artifact.bin"),
        digest: "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        bytes: 7,
        validation_operation: Uuid::parse_str("01234567-89ab-cdef-0123-456789abcdef").unwrap(),
        candidate_vector_digest: None,
    }
}

#[test]
fn artifact_proof_preserves_exact_bytes_and_roundtrip() {
    let expected = concat!(
        "{\"path\":\"/fixture/artifact.bin\",",
        "\"digest\":\"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",",
        "\"bytes\":7,\"validation_operation\":\"01234567-89ab-cdef-0123-456789abcdef\",",
        "\"candidate_vector_digest\":null}"
    );
    assert_eq!(serde_json::to_string(&proof()).unwrap(), expected);
    let decoded: ArtifactProof = serde_json::from_str(expected).unwrap();
    assert_eq!(decoded, proof());
    assert_eq!(serde_json::to_string(&decoded).unwrap(), expected);
}

#[test]
fn artifact_proof_preserves_optional_vector_omission_null_and_value() {
    let mut value = serde_json::to_value(proof()).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .remove("candidate_vector_digest");
    let omitted: ArtifactProof = serde_json::from_value(value.clone()).unwrap();
    assert!(omitted.candidate_vector_digest.is_none());
    assert_eq!(
        serde_json::to_value(omitted).unwrap()["candidate_vector_digest"],
        Value::Null
    );
    value["candidate_vector_digest"] = Value::Null;
    assert!(
        serde_json::from_value::<ArtifactProof>(value.clone())
            .unwrap()
            .candidate_vector_digest
            .is_none()
    );
    let digest = "sha256:v1:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    value["candidate_vector_digest"] = json!(digest);
    assert_eq!(
        serde_json::from_value::<ArtifactProof>(value)
            .unwrap()
            .candidate_vector_digest
            .as_deref(),
        Some(digest)
    );
}

#[test]
fn artifact_proof_preserves_required_fields_and_unknown_field_refusal() {
    let value = serde_json::to_value(proof()).unwrap();
    for field in ["path", "digest", "bytes", "validation_operation"] {
        let mut missing = value.clone();
        missing.as_object_mut().unwrap().remove(field);
        assert!(
            serde_json::from_value::<ArtifactProof>(missing).is_err(),
            "missing {field}"
        );
        let mut null = value.clone();
        null[field] = Value::Null;
        assert!(
            serde_json::from_value::<ArtifactProof>(null).is_err(),
            "null {field}"
        );
    }
    let mut unknown = value;
    unknown["verified"] = json!(true);
    assert!(serde_json::from_value::<ArtifactProof>(unknown).is_err());
}

#[test]
fn artifact_proof_preserves_malformed_field_refusal_without_adding_policy() {
    let value = serde_json::to_value(proof()).unwrap();
    for (field, malformed) in [
        ("path", json!(42)),
        ("digest", json!([])),
        ("bytes", json!(-1)),
        ("validation_operation", json!("invalid-uuid")),
        ("candidate_vector_digest", json!(false)),
    ] {
        let mut changed = value.clone();
        changed[field] = malformed;
        assert!(
            serde_json::from_value::<ArtifactProof>(changed).is_err(),
            "malformed {field}"
        );
    }
    // This is descriptive data, not a validator or an execution capability.
    let descriptive: ArtifactProof = serde_json::from_value(json!({
        "path":"relative", "digest":"", "bytes":0,
        "validation_operation":"00000000-0000-0000-0000-000000000000"
    }))
    .unwrap();
    assert_eq!(descriptive.path, PathBuf::from("relative"));
    assert!(descriptive.digest.is_empty());
    assert!(descriptive.validation_operation.is_nil());
}
