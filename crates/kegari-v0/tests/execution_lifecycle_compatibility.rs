use kegari_digest::canonical_digest;
use kegari_execution::journal::{HostProcessIdentity, SpawnBinding};
use kegari_types::control::Lease;
use kegari_v0::control::{Permit, Receipt};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

fn fixtures() -> Value {
    serde_json::from_str(include_str!(
        "../../kegari-execution/tests/fixtures/lifecycle-contracts.json"
    ))
    .unwrap()
}

fn compatible<T: Serialize + DeserializeOwned>(fixtures: &Value, name: &str) -> T {
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

#[test]
fn admission_records_preserve_embedded_execution_identities_and_digests() {
    let fixtures = fixtures();
    let lease: Lease = compatible(&fixtures, "lease");
    let permit: Permit = compatible(&fixtures, "permit");
    let owner: HostProcessIdentity = compatible(&fixtures, "host_identity");
    let binding: SpawnBinding = compatible(&fixtures, "host_binding");
    let receipt: Receipt = compatible(&fixtures, "receipt");
    assert_eq!(permit.lease, lease);
    assert_eq!(
        receipt.expected["permit"],
        canonical_digest(1, &permit).unwrap()
    );
    assert_eq!(
        receipt.expected["host_identity"],
        canonical_digest(1, &owner).unwrap()
    );
    assert_eq!(
        receipt.remote_ids["process_binding"],
        serde_json::to_string(&binding).unwrap()
    );
}
