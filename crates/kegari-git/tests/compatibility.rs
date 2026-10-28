use kegari_git as model;
use model::*;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
mod support;

fn fixture(name: &str) -> Value {
    let baseline: Value =
        serde_json::from_str(include_str!("fixtures/serialization.json")).unwrap();
    serde_json::from_str(baseline["records"][name]["json"].as_str().unwrap()).unwrap()
}

#[test]
fn exact_original_serialization_and_digests_are_preserved() {
    let baseline: Value =
        serde_json::from_str(include_str!("fixtures/serialization.json")).unwrap();
    assert_eq!(support::corpus(), baseline);
    assert_eq!(SUPPORTED_GIT_VERSION, "git version 2.56.0");
    assert_eq!(DELTA_SCHEMA, 1);
}

fn strict_record<T: Serialize + DeserializeOwned>(name: &str, required: &str) {
    let value = fixture(name);
    let decoded: T = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), value);
    let mut unknown = value.clone();
    unknown["unexpected"] = json!(true);
    assert!(
        serde_json::from_value::<T>(unknown).is_err(),
        "{name}: unknown field"
    );
    let mut missing = value.clone();
    missing.as_object_mut().unwrap().remove(required);
    assert!(
        serde_json::from_value::<T>(missing).is_err(),
        "{name}: missing field"
    );
    let mut null = value;
    null[required] = Value::Null;
    assert!(
        serde_json::from_value::<T>(null).is_err(),
        "{name}: null required field"
    );
}

#[test]
fn strict_records_keep_unknown_missing_and_null_field_refusal() {
    strict_record::<RepositoryId>("repository", "hostname");
    strict_record::<Revision>("revision", "commit");
    strict_record::<BranchName>("branch", "name");
    strict_record::<WriteScope>("scope", "paths");
    strict_record::<DeltaManifest>("delta", "git_version");
    strict_record::<ReplacementMapping>("replacement", "originals");
    strict_record::<ReviewDelta>("review", "base");
    strict_record::<ProductionIntegration>("integration", "pull_request");
    strict_record::<CandidateManifest>("candidate", "operation_id");
    strict_record::<SourceCommitMetadata>("source_metadata", "message");
    strict_record::<SourceTreeEntry>("source_entry", "mode");
    strict_record::<GitCommitIdentity>("git_commit_identity", "date");
}

#[test]
fn commit_identity_retains_permissive_unknown_fields_in_strict_metadata() {
    let mut identity = fixture("commit_identity");
    identity["unexpected"] = json!({"retained": true});
    let decoded: CommitIdentity = serde_json::from_value(identity.clone()).unwrap();
    assert_eq!(decoded, support::identity());
    identity.as_object_mut().unwrap().remove("name");
    assert!(serde_json::from_value::<CommitIdentity>(identity).is_err());
    let mut metadata = fixture("source_metadata");
    metadata["identity"]["unexpected"] = json!(true);
    let decoded: SourceCommitMetadata = serde_json::from_value(metadata).unwrap();
    assert_eq!(decoded, support::metadata());
}

fn absent_is_null<T: Serialize + DeserializeOwned>(name: &str, fields: &[&str]) {
    let mut null = fixture(name);
    for field in fields {
        null[*field] = Value::Null;
    }
    let decoded: T = serde_json::from_value(null.clone()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), null);
    let mut omitted = null.clone();
    for field in fields {
        omitted.as_object_mut().unwrap().remove(*field);
    }
    let decoded: T = serde_json::from_value(omitted).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), null);
}

#[test]
fn optional_fields_accept_missing_or_null_and_serialize_explicit_null() {
    absent_is_null::<DeltaManifest>("delta", &["replacement"]);
    absent_is_null::<ReviewDelta>("review", &["replacement_digest"]);
    absent_is_null::<CandidateManifest>("candidate", &["result", "combined_patch_digest"]);
    absent_is_null::<SourceTreeEntry>("source_entry", &["object"]);
}

#[test]
fn decoding_and_digesting_descriptive_records_do_not_validate_them() {
    let invalid: RepositoryId =
        serde_json::from_value(json!({"hostname":"UPPER", "id":""})).unwrap();
    assert!(invalid.validate().is_err());
    let mut metadata = fixture("source_metadata");
    metadata["message"] = json!("no signoff");
    let metadata: SourceCommitMetadata = serde_json::from_value(metadata).unwrap();
    assert!(metadata.validate().is_err());
    let mut delta = fixture("delta");
    delta["schema_version"] = json!(99);
    delta["validated_base"]["commit"] = json!("not an oid");
    let delta: DeltaManifest = serde_json::from_value(delta).unwrap();
    assert!(delta.digest().is_ok());
    assert!(delta.review_digest().is_ok());
    let mut review = fixture("review");
    review["base"]["tree"] = json!("not a tree");
    assert!(
        serde_json::from_value::<ReviewDelta>(review)
            .unwrap()
            .digest()
            .is_ok()
    );
    let mut integration = fixture("integration");
    integration["pull_request"] = json!(0);
    assert!(
        serde_json::from_value::<ProductionIntegration>(integration)
            .unwrap()
            .digest()
            .is_ok()
    );
}
