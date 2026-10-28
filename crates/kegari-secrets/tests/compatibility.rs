//! Synthetic in-memory fixtures; no native Keychain or process-environment access.
use kegari_secrets::{AttestationKey, Secret, SecretStore, Signed, resolve_reference};
use kegari_types::error::{Error, Result};
use serde_json::{Value, json};
use std::sync::Mutex;

const PURPOSE: &str = "extraction_fixture";
const KEY_ID: &str = "synthetic-extraction-key";

fn key() -> AttestationKey {
    AttestationKey::new(KEY_ID.into(), (0_u8..32).collect()).unwrap()
}

fn vector() -> Value {
    // Captured from the pre-extraction backend, not calculated by this test.
    serde_json::from_str(include_str!("fixtures/attestation-v1.json")).unwrap()
}

fn baseline_record() -> Signed<Value> {
    serde_json::from_str(vector()["serialized"].as_str().unwrap()).unwrap()
}

#[test]
fn signed_record_matches_pre_extraction_bytes_signature_and_digest() {
    let golden = vector();
    let key = key();
    let payload = json!({"z": null, "a": {"second": [true, 7, "fixture"], "first": "nested"}});
    let signed = key.sign(PURPOSE, &payload).unwrap();
    assert_eq!(key.id(), KEY_ID);
    assert_eq!(signed, baseline_record());
    assert_eq!(
        serde_json::to_string(&signed).unwrap(),
        golden["serialized"].as_str().unwrap()
    );
    assert_eq!(signed.signature, golden["signature"].as_str().unwrap());
    assert_eq!(signed.digest().unwrap(), golden["digest"].as_str().unwrap());
    key.verify(PURPOSE, &signed).unwrap();
    key.verify(PURPOSE, &baseline_record()).unwrap();
}

#[test]
fn authentication_rejects_tampering_and_preserves_error_order() {
    let key = key();
    let original = baseline_record();
    for field in ["schema_version", "key_id", "purpose", "payload"] {
        let mut changed = original.clone();
        match field {
            "schema_version" => changed.schema_version = 2,
            "key_id" => changed.key_id = "another-key".into(),
            "purpose" => changed.purpose = "another_purpose".into(),
            "payload" => changed.payload["a"]["second"][1] = json!(8),
            _ => unreachable!(),
        }
        assert_eq!(
            key.verify(PURPOSE, &changed).unwrap_err().code,
            "invalid_attestation",
            "{field}"
        );
    }
    assert_eq!(
        key.verify("another_purpose", &original).unwrap_err().code,
        "invalid_attestation"
    );
    let other_key = AttestationKey::new(KEY_ID.into(), vec![42; 32]).unwrap();
    assert_eq!(
        other_key.verify(PURPOSE, &original).unwrap_err().code,
        "invalid_attestation"
    );
    for signature in ["not-hex".into(), "00".into(), "00".repeat(32)] {
        let mut changed = original.clone();
        changed.signature = signature;
        assert_eq!(
            key.verify(PURPOSE, &changed).unwrap_err().code,
            "invalid_attestation"
        );
    }
    let mut wrong_schema = original.clone();
    wrong_schema.schema_version = 2;
    wrong_schema.signature = "not-hex".into();
    assert_eq!(
        key.verify(PURPOSE, &wrong_schema).unwrap_err(),
        Error::new(
            "invalid_attestation",
            "attestation schema/key/purpose does not match"
        )
    );
    let mut uppercase = original;
    uppercase.signature.make_ascii_uppercase();
    key.verify(PURPOSE, &uppercase).unwrap();
}

#[test]
fn signed_deserialization_preserves_required_fields_and_separates_authentication() {
    let signed = baseline_record();
    let object = serde_json::to_value(&signed).unwrap();
    let mut unknown = object.clone();
    unknown["unexpected"] = json!(true);
    assert!(serde_json::from_value::<Signed<Value>>(unknown).is_err());
    for field in [
        "schema_version",
        "key_id",
        "purpose",
        "payload",
        "signature",
    ] {
        let mut missing = object.clone();
        missing.as_object_mut().unwrap().remove(field);
        assert!(
            serde_json::from_value::<Signed<Value>>(missing).is_err(),
            "{field}"
        );
    }
    let mut unverified = object;
    unverified["schema_version"] = json!(99);
    let unverified: Signed<Value> = serde_json::from_value(unverified).unwrap();
    assert_eq!(
        key().verify(PURPOSE, &unverified).unwrap_err().code,
        "invalid_attestation"
    );
    let null = key().sign(PURPOSE, &Option::<Value>::None).unwrap();
    let serialized = serde_json::to_string(&null).unwrap();
    assert!(serialized.contains("\"payload\":null"));
    let restored: Signed<Option<Value>> = serde_json::from_str(&serialized).unwrap();
    assert_eq!(restored, null);
    key().verify(PURPOSE, &restored).unwrap();
}

#[test]
fn secret_and_attestation_constructor_bounds_are_unchanged() {
    for size in [1, 65_536] {
        let secret = Secret::new(vec![0xff; size]).unwrap();
        assert_eq!(secret.bytes(), vec![0xff; size]);
        assert_eq!(format!("{secret:?}"), "Secret([REDACTED])");
    }
    for size in [0, 65_537] {
        assert_eq!(
            Secret::new(vec![1; size]).unwrap_err(),
            Error::new("missing_secret", "secret must contain 1–65536 bytes")
        );
    }
    for (id, bytes) in [
        ("", vec![7; 32]),
        (" \t", vec![7; 32]),
        (KEY_ID, vec![7; 31]),
    ] {
        assert_eq!(
            AttestationKey::new(id.into(), bytes).err().unwrap(),
            Error::invalid("attestation key needs a stable ID and at least 256 bits")
        );
    }
    for size in [32, 33] {
        let key = AttestationKey::new(" key with spaces ".into(), vec![7; size]).unwrap();
        assert_eq!(key.id(), " key with spaces ");
        let signed = key.sign(" ", &json!(null)).unwrap();
        key.verify(" ", &signed).unwrap();
        assert_eq!(
            key.sign("", &json!(null)).unwrap_err(),
            Error::invalid("signed evidence requires a domain purpose")
        );
    }
}

#[derive(Default)]
struct Memory {
    reads: Mutex<Vec<String>>,
}

impl SecretStore for Memory {
    fn read(&self, name: &str) -> Result<Secret> {
        self.reads.lock().unwrap().push(name.into());
        Err(Error::new(
            "authentication_required",
            "synthetic store locked",
        ))
    }

    fn save(&self, _: &str, _: &Secret) -> Result<()> {
        panic!("reference resolution never saves secrets")
    }
}

#[test]
fn reference_resolution_uses_only_the_declared_source() {
    let store = Memory::default();
    let environment = resolve_reference("$_TOKEN_1", &store, |name| {
        assert_eq!(name, "_TOKEN_1");
        Some("synthetic-environment-value".into())
    })
    .unwrap();
    assert_eq!(environment.bytes(), b"synthetic-environment-value");
    assert!(store.reads.lock().unwrap().is_empty());
    assert_eq!(
        resolve_reference("keychain:provider", &store, |_| panic!("no fallback")).unwrap_err(),
        Error::new("authentication_required", "synthetic store locked")
    );
    assert_eq!(*store.reads.lock().unwrap(), ["provider"]);
    for invalid in ["keychain:", "keychain:bad\nname", "$9TOKEN", "${TOKEN}"] {
        assert_eq!(
            resolve_reference(invalid, &store, |_| panic!(
                "invalid input must not resolve"
            ))
            .unwrap_err()
            .code,
            "invalid_config"
        );
    }
    assert_eq!(*store.reads.lock().unwrap(), ["provider"]);
    assert_eq!(
        resolve_reference("$EMPTY", &store, |_| Some(String::new())).unwrap_err(),
        Error::new(
            "missing_secret",
            "declared secret environment input is unavailable"
        )
    );
}
