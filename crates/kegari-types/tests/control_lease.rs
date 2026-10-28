use kegari_types::control::Lease;
use serde_json::{Value, json};

const LEASE: &str = "{\"runtime_id\":\"00000000-0000-0000-0000-000000000001\",\"owner_generation\":7,\"owner_id\":\"00000000-0000-0000-0000-000000000002\"}";

#[test]
fn lease_preserves_exact_serialization_and_field_requirements() {
    let lease: Lease = serde_json::from_str(LEASE).unwrap();
    assert_eq!(serde_json::to_string(&lease).unwrap(), LEASE);
    assert_eq!(lease.owner_generation, 7);
    for field in ["runtime_id", "owner_generation", "owner_id"] {
        let mut missing: Value = serde_json::from_str(LEASE).unwrap();
        missing.as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<Lease>(missing).is_err());
        let mut null: Value = serde_json::from_str(LEASE).unwrap();
        null[field] = Value::Null;
        assert!(serde_json::from_value::<Lease>(null).is_err());
    }
    let mut unknown: Value = serde_json::from_str(LEASE).unwrap();
    unknown["unexpected"] = json!(true);
    assert!(serde_json::from_value::<Lease>(unknown).is_err());
}
