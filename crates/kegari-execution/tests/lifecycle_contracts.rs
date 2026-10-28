//! Exact bytes and digests captured from the backend before lifecycle relocation.
//! Every identity and path is synthetic; these tests perform no native inspection.
use kegari_digest::canonical_digest;
use kegari_execution::journal::{
    CheckBundleMember, HostProcessIdentity, ProcessRecord, ProcessState, SpawnBinding,
};
use kegari_execution::{CommandOutcome, ServiceObservation};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

fn cases() -> Value {
    serde_json::from_str(include_str!("fixtures/lifecycle-contracts.json")).unwrap()
}

fn compatible<T: Serialize + DeserializeOwned>(name: &str) -> T {
    let fixtures = cases();
    let row = &fixtures[name];
    let bytes = row["bytes"].as_str().unwrap();
    let value: T = serde_json::from_str(bytes).unwrap();
    assert_eq!(serde_json::to_string(&value).unwrap(), bytes, "{name}");
    assert_eq!(
        canonical_digest(1, &value).unwrap(),
        row["digest"],
        "{name}"
    );
    value
}

fn value(name: &str) -> Value {
    serde_json::from_str(cases()[name]["bytes"].as_str().unwrap()).unwrap()
}

fn rejects_unknown<T: DeserializeOwned>(name: &str) {
    let mut row = value(name);
    row["unexpected"] = json!(true);
    assert!(serde_json::from_value::<T>(row).is_err(), "{name}");
}

#[test]
fn relocated_lifecycle_records_preserve_exact_bytes_and_canonical_digests() {
    for name in ["host_identity", "host_cleanup_identity"] {
        compatible::<HostProcessIdentity>(name);
    }
    for name in ["worker_binding", "host_binding"] {
        compatible::<SpawnBinding>(name);
    }
    for name in ["service_member", "readiness_member", "check_member"] {
        compatible::<CheckBundleMember>(name);
    }
    for name in ["starting", "running", "legacy_exit", "owned_exit"] {
        compatible::<ProcessState>(name);
    }
    for name in ["schema_1", "schema_2", "schema_3"] {
        compatible::<ProcessRecord>(name);
    }
    for name in ["outcome", "signalled_outcome"] {
        compatible::<CommandOutcome>(name);
    }
    compatible::<ServiceObservation>("service_observation");
}

#[test]
fn lifecycle_records_keep_current_unknown_field_behavior() {
    rejects_unknown::<HostProcessIdentity>("host_identity");
    rejects_unknown::<SpawnBinding>("worker_binding");
    rejects_unknown::<CheckBundleMember>("service_member");
    rejects_unknown::<ProcessState>("running");
    rejects_unknown::<ProcessState>("owned_exit");
    rejects_unknown::<ProcessRecord>("schema_1");
    rejects_unknown::<CommandOutcome>("outcome");
    rejects_unknown::<ServiceObservation>("service_observation");

    // Serde's internally tagged unit variant ignores additional payload fields;
    // keep that existing behavior despite the enum's deny_unknown_fields.
    let mut starting = value("starting");
    starting["unexpected"] = json!(true);
    assert_eq!(
        serde_json::from_value::<ProcessState>(starting).unwrap(),
        ProcessState::Starting
    );
}

#[test]
fn optional_fields_preserve_omission_null_and_default_behavior() {
    let mut owner = value("host_identity");
    owner["cleanup_receipt"] = Value::Null;
    let owner: HostProcessIdentity = serde_json::from_value(owner).unwrap();
    assert!(owner.cleanup_receipt.is_none());
    assert_eq!(serde_json::to_value(owner).unwrap(), value("host_identity"));

    let mut binding = value("worker_binding");
    binding["control_owner"] = Value::Null;
    binding["check_bundle"] = Value::Null;
    let binding: SpawnBinding = serde_json::from_value(binding).unwrap();
    assert_eq!(
        serde_json::to_value(binding).unwrap(),
        value("worker_binding")
    );

    // Option fields without skip_serializing_if remain explicit null values.
    let mut host_binding = value("host_binding");
    host_binding.as_object_mut().unwrap().remove("lease");
    let host_binding: SpawnBinding = serde_json::from_value(host_binding).unwrap();
    assert!(host_binding.lease.is_none());
    assert_eq!(
        serde_json::to_value(host_binding).unwrap(),
        value("host_binding")
    );
    let mut member = value("check_member");
    member.as_object_mut().unwrap().remove("service");
    let member: CheckBundleMember = serde_json::from_value(member).unwrap();
    assert_eq!(serde_json::to_value(member).unwrap(), value("check_member"));

    let mut legacy = value("legacy_exit");
    legacy["owned_group"] = Value::Null;
    legacy["guard_release_verified"] = json!(false);
    let legacy: ProcessState = serde_json::from_value(legacy).unwrap();
    assert_eq!(serde_json::to_value(legacy).unwrap(), value("legacy_exit"));
    let mut invalid_boolean = value("legacy_exit");
    invalid_boolean["guard_release_verified"] = Value::Null;
    assert!(serde_json::from_value::<ProcessState>(invalid_boolean).is_err());

    let mut outcome = value("outcome");
    outcome["termination_signal"] = Value::Null;
    let outcome: CommandOutcome = serde_json::from_value(outcome).unwrap();
    assert_eq!(serde_json::to_value(outcome).unwrap(), value("outcome"));
    let mut signalled = value("signalled_outcome");
    signalled.as_object_mut().unwrap().remove("exit_code");
    let signalled: CommandOutcome = serde_json::from_value(signalled).unwrap();
    assert_eq!(
        serde_json::to_value(signalled).unwrap(),
        value("signalled_outcome")
    );
}

#[test]
fn deserialization_does_not_add_validation_or_accept_missing_required_data() {
    let mut binding = value("worker_binding");
    binding["operation_id"] = json!("00000000-0000-0000-0000-000000000000");
    binding["scope_digest"] = json!("invalid-but-descriptive");
    let decoded: SpawnBinding = serde_json::from_value(binding).unwrap();
    assert!(decoded.operation_id.is_nil());
    assert_eq!(decoded.scope_digest, "invalid-but-descriptive");

    let mut record = value("schema_2");
    record["schema_version"] = json!(999);
    assert_eq!(
        serde_json::from_value::<ProcessRecord>(record)
            .unwrap()
            .schema_version,
        999
    );
    let mut owner = value("host_identity");
    owner.as_object_mut().unwrap().remove("config_revision");
    assert!(serde_json::from_value::<HostProcessIdentity>(owner).is_err());
    let mut observation = value("service_observation");
    observation
        .as_object_mut()
        .unwrap()
        .remove("stop_requested");
    assert!(serde_json::from_value::<ServiceObservation>(observation).is_err());
}
