use kegari_secrets::{AttestationKey, Keychain, Secret, SecretStore, resolve_reference};
use kegari_types::error::{Error, Result};
use std::{collections::BTreeMap, sync::Mutex};

#[derive(Default)]
struct Memory(Mutex<std::collections::BTreeMap<String, Vec<u8>>>);
impl SecretStore for Memory {
    fn read(&self, name: &str) -> Result<Secret> {
        Secret::new(
            self.0
                .lock()
                .unwrap()
                .get(name)
                .cloned()
                .ok_or_else(|| Error::new("missing_secret", "fixture absent"))?,
        )
    }
    fn save(&self, name: &str, value: &Secret) -> Result<()> {
        self.0
            .lock()
            .unwrap()
            .insert(name.into(), value.bytes().to_vec());
        Ok(())
    }
}
#[test]
fn explicit_secret_inputs_never_fall_back_or_expose_debug_values() {
    let store = Memory::default();
    store
        .save(
            "provider",
            &Secret::new(b"sentinel-token".to_vec()).unwrap(),
        )
        .unwrap();
    let value = resolve_reference("keychain:provider", &store, |_| {
        panic!("no environment fallback")
    })
    .unwrap();
    assert_eq!(value.bytes(), b"sentinel-token");
    assert!(!format!("{value:?}").contains("sentinel"));
    assert!(resolve_reference("keychain:missing", &store, |_| Some("fallback".into())).is_err());
    assert!(resolve_reference("$MISSING", &store, |_| None).is_err());
    assert!(resolve_reference("$ENV/suffix", &store, |_| Some("secret".into())).is_err());
    assert!(resolve_reference("literal-secret", &store, |_| None).is_err());
    assert!(Keychain::new(uuid::Uuid::nil()).is_err());
}

#[test]
fn attestation_rejects_unserializable_payload_without_panicking() {
    let key = AttestationKey::new("fixture-invalid-json".into(), vec![7; 32]).unwrap();
    let payload = BTreeMap::from([(("structured", "key"), "unsupported JSON key")]);
    assert!(key.sign("fixture_payload", &payload).is_err());
}
