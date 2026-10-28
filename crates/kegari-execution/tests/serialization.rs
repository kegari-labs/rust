use kegari_execution::{CommandObservation, EnvironmentObservation, IsolationPlan};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

fn roundtrip<T: DeserializeOwned + Serialize>(bytes: &str) {
    let value: T = serde_json::from_str(bytes).unwrap();
    assert_eq!(serde_json::to_string(&value).unwrap(), bytes);
}

#[test]
fn captured_environment_records_preserve_exact_bytes_and_digests() {
    // Captured before relocation. Paths and native metadata are descriptive;
    // this portable serialization check never accesses the captured filesystem.
    let corpus: Value = serde_json::from_str(include_str!("fixtures/environment.json")).unwrap();
    for key in ["unmanaged_plan_bytes", "plan_bytes", "derived_plan_bytes"] {
        roundtrip::<IsolationPlan>(corpus[key].as_str().unwrap());
    }
    roundtrip::<CommandObservation>(corpus["command_bytes"].as_str().unwrap());
    for key in ["environment_bytes", "derived_environment_bytes"] {
        roundtrip::<EnvironmentObservation>(corpus[key].as_str().unwrap());
    }
    for (bytes, digest) in [
        ("plan_bytes", "plan_digest"),
        ("command_bytes", "command_digest"),
        ("environment_bytes", "environment_digest"),
        ("derived_plan_bytes", "derived_digest"),
    ] {
        let value: Value = serde_json::from_str(corpus[bytes].as_str().unwrap()).unwrap();
        assert_eq!(
            kegari_digest::canonical_digest(1, &value).unwrap(),
            corpus[digest]
        );
    }
    assert_eq!(
        kegari_execution::BoundManagedExecution::automatic_policy_digest(
            corpus["build"].as_str().unwrap()
        )
        .unwrap(),
        corpus["policy_digest"]
    );
}

#[test]
fn retained_plan_defaults_nulls_and_unknown_fields_are_unchanged() {
    let corpus: Value = serde_json::from_str(include_str!("fixtures/environment.json")).unwrap();
    let mut plan: Value =
        serde_json::from_str(corpus["unmanaged_plan_bytes"].as_str().unwrap()).unwrap();
    assert_eq!(plan["managed_execution"], Value::Null);
    plan.as_object_mut().unwrap().remove("managed_execution");
    plan.as_object_mut().unwrap().remove("resource_writes");
    let decoded: IsolationPlan = serde_json::from_value(plan.clone()).unwrap();
    assert!(decoded.managed_execution.is_none());
    assert!(decoded.resource_writes.is_empty());
    plan["unknown"] = json!(true);
    assert!(serde_json::from_value::<IsolationPlan>(plan.clone()).is_err());
    plan.as_object_mut().unwrap().remove("unknown");
    plan["resource_writes"] = Value::Null;
    assert!(serde_json::from_value::<IsolationPlan>(plan).is_err());
    let mut bound: Value = serde_json::from_str(corpus["plan_bytes"].as_str().unwrap()).unwrap();
    bound["managed_execution"]["unknown"] = json!(true);
    assert!(serde_json::from_value::<IsolationPlan>(bound).is_err());
}
