//! File-only compatibility fixtures; no native identity queries or execution.
#![cfg(unix)]

use kegari_platform::{BinaryIdentity, ProcessIdentity};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
};
use uuid::Uuid;

const ABC_DIGEST: &str = "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

fn executable() -> (tempfile::TempDir, PathBuf) {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let path = root.join("tool");
    fs::write(&path, b"abc").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    (temporary, path)
}

#[test]
fn identity_serialization_and_unknown_field_refusal_are_unchanged() {
    let binary = BinaryIdentity {
        path: PathBuf::from("/fixture/tool"),
        digest: ABC_DIGEST.into(),
        bytes: 3,
    };
    let binary_json = r#"{"path":"/fixture/tool","digest":"sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad","bytes":3}"#;
    assert_eq!(serde_json::to_string(&binary).unwrap(), binary_json);
    assert_eq!(
        serde_json::from_str::<BinaryIdentity>(binary_json).unwrap(),
        binary
    );
    let mut unknown = serde_json::to_value(&binary).unwrap();
    unknown["extra"] = serde_json::json!(true);
    assert!(serde_json::from_value::<BinaryIdentity>(unknown).is_err());

    let process = ProcessIdentity {
        pid: 123,
        parent_pid: 45,
        process_group: 123,
        uid: 501,
        boot_id: Uuid::from_u128(1),
        started_seconds: 100,
        started_microseconds: 200,
        executable: PathBuf::from("/fixture/tool"),
    };
    let process_json = r#"{"pid":123,"parent_pid":45,"process_group":123,"uid":501,"boot_id":"00000000-0000-0000-0000-000000000001","started_seconds":100,"started_microseconds":200,"executable":"/fixture/tool"}"#;
    assert_eq!(serde_json::to_string(&process).unwrap(), process_json);
    assert_eq!(
        serde_json::from_str::<ProcessIdentity>(process_json).unwrap(),
        process
    );
    let mut unknown = serde_json::to_value(&process).unwrap();
    unknown["extra"] = serde_json::json!(true);
    assert!(serde_json::from_value::<ProcessIdentity>(unknown).is_err());
}

#[test]
fn executable_identity_preserves_digest_and_refuses_same_size_content_change() {
    let (_temporary, path) = executable();
    let original = BinaryIdentity::inspect(&path, false).unwrap();
    assert_eq!(original.path, path);
    assert_eq!(original.bytes, 3);
    assert_eq!(original.digest, ABC_DIGEST);
    original.revalidate(false).unwrap();

    fs::write(&path, b"abd").unwrap();
    assert_eq!(fs::metadata(&path).unwrap().len(), original.bytes);
    assert_eq!(
        original.revalidate(false).unwrap_err().code,
        "identity_mismatch"
    );
    let changed = BinaryIdentity::inspect(&path, false).unwrap();
    assert_eq!(changed.bytes, original.bytes);
    assert_ne!(changed.digest, original.digest);
}

#[test]
fn executable_modes_preserve_mutable_and_immutable_inspection_rules() {
    let (_temporary, path) = executable();
    let mutable = BinaryIdentity::inspect(&path, false).unwrap();
    assert_eq!(
        BinaryIdentity::inspect(&path, true).unwrap_err().code,
        "unsafe_storage"
    );

    fs::set_permissions(&path, fs::Permissions::from_mode(0o500)).unwrap();
    assert_eq!(BinaryIdentity::inspect(&path, true).unwrap(), mutable);
    mutable.revalidate(true).unwrap();

    for mode in [0o600, 0o722] {
        fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
        assert_eq!(
            BinaryIdentity::inspect(&path, false).unwrap_err().code,
            "unsafe_storage"
        );
    }
}

#[test]
fn executable_paths_refuse_links_and_non_absolute_or_parent_components() {
    let (_temporary, path) = executable();
    let root = path.parent().unwrap();
    let linked_file = root.join("linked-tool");
    symlink(&path, &linked_file).unwrap();
    assert_eq!(
        BinaryIdentity::inspect(&linked_file, false)
            .unwrap_err()
            .code,
        "unsafe_storage"
    );
    let linked_directory = root.join("linked-directory");
    symlink(root, &linked_directory).unwrap();
    assert_eq!(
        BinaryIdentity::inspect(&linked_directory.join("tool"), false)
            .unwrap_err()
            .code,
        "unsafe_storage"
    );
    for invalid in [
        Path::new("tool").to_path_buf(),
        PathBuf::new(),
        root.join("..").join("tool"),
    ] {
        assert_eq!(
            BinaryIdentity::inspect(&invalid, false).unwrap_err().code,
            "invalid_config"
        );
    }
}
