use kegari_storage::PrivateDir;

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
