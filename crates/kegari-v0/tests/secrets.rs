use kegari_storage::PrivateDir;
use kegari_types::error::{Error, Result};
use kegari_v0::secrets::{Keychain, Secret, SecretStore, resolve_reference};
use std::sync::Mutex;

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
    assert!(!format!("{:?}", value.credential().unwrap()).contains("sentinel"));
    assert!(resolve_reference("keychain:missing", &store, |_| Some("fallback".into())).is_err());
    assert!(resolve_reference("$MISSING", &store, |_| None).is_err());
    assert!(resolve_reference("$ENV/suffix", &store, |_| Some("secret".into())).is_err());
    assert!(resolve_reference("literal-secret", &store, |_| None).is_err());
    assert!(Keychain::new(uuid::Uuid::nil()).is_err());
}
#[test]
fn native_config_bytes_keep_owner_only_atomic_storage_and_bounds() {
    use std::os::unix::fs::PermissionsExt;
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().canonicalize().unwrap().join("private");
    let root = PrivateDir::create(&path).unwrap();
    root.write_bytes("config.toml", b"schema_version = 1\n")
        .unwrap();
    assert_eq!(
        root.read_bytes("config.toml", 1024).unwrap(),
        b"schema_version = 1\n"
    );
    assert_eq!(
        std::fs::metadata(path.join("config.toml"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert!(root.read_bytes("config.toml", 1).is_err());
    std::os::unix::fs::symlink(path.join("config.toml"), path.join("alias.toml")).unwrap();
    assert!(root.write_bytes("alias.toml", b"replace").is_err());
    assert_eq!(
        root.read_bytes("config.toml", 1024).unwrap(),
        b"schema_version = 1\n"
    );
}
