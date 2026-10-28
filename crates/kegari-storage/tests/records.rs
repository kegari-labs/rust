use kegari_storage::{PrivateDir, WriteBoundary};
use kegari_types::error::Error;
use serde_json::json;
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
};
use tempfile::TempDir;

fn directory() -> (TempDir, PrivateDir) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap().join("state");
    let directory = PrivateDir::create(&root).unwrap();
    (temp, directory)
}

#[test]
fn v0_t08_each_write_boundary_recovers_old_or_new_valid_record() {
    for stop in [
        WriteBoundary::Created,
        WriteBoundary::Written,
        WriteBoundary::FileSynced,
        WriteBoundary::Renamed,
        WriteBoundary::DirectorySynced,
    ] {
        let (_temp, dir) = directory();
        dir.write("intent.json", &json!({"desired":"RUNNING"}))
            .unwrap();
        let result = dir.write_observed("intent.json", &json!({"desired":"STOPPED"}), |point| {
            if point == stop {
                Err(Error::new("injected_crash", "durability boundary"))
            } else {
                Ok(())
            }
        });
        assert!(result.is_err());
        let recovered: serde_json::Value = PrivateDir::open(dir.path())
            .unwrap()
            .read("intent.json")
            .unwrap();
        assert_eq!(
            recovered["desired"],
            if matches!(
                stop,
                WriteBoundary::Renamed | WriteBoundary::DirectorySynced
            ) {
                "STOPPED"
            } else {
                "RUNNING"
            }
        );
        assert_eq!(
            fs::metadata(dir.path().join("intent.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[test]
fn v0_t12_control_storage_rejects_symlinks_hardlinks_and_permissions() {
    let (temp, dir) = directory();
    let outside = temp.path().join("outside");
    fs::write(&outside, b"preserve").unwrap();
    symlink(&outside, dir.path().join("intent.json")).unwrap();
    assert_eq!(
        dir.read::<serde_json::Value>("intent.json")
            .unwrap_err()
            .code,
        "unsafe_storage"
    );
    assert!(dir.write("intent.json", &json!(1)).is_err());
    assert_eq!(fs::read(&outside).unwrap(), b"preserve");
    let linked_directory = temp.path().join("linked-control");
    symlink(dir.path(), &linked_directory).unwrap();
    assert_eq!(
        PrivateDir::open(&linked_directory).unwrap_err().code,
        "unsafe_storage"
    );
    fs::hard_link(&outside, dir.path().join("hard.json")).unwrap();
    assert!(dir.read::<serde_json::Value>("hard.json").is_err());
    assert!(dir.write("../escape.json", &json!(1)).is_err());
    assert!(dir.write("/absolute.json", &json!(1)).is_err());
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
    assert!(dir.write("safe.json", &json!(1)).is_err());
}

#[test]
fn protected_record_encoding_and_decode_errors_remain_compatible() {
    let (_temp, dir) = directory();
    let value = json!({"b":[1,"two"],"a":true});
    let expected = br#"{"schema_version":1,"digest":"sha256:v1:e75f1c66fb37047ed8c904526793282ef3a3b95b8735159e40d2d9d9fda945ee","payload":{"a":true,"b":[1,"two"]}}"#;
    dir.write("record.json", &value).unwrap();
    assert_eq!(dir.read_bytes("record.json", 1024).unwrap(), expected);
    assert_eq!(
        kegari_storage::decode_record::<serde_json::Value>(expected).unwrap(),
        value
    );
    for (bytes, code) in [
        (b"not JSON".as_slice(), "record_corrupt"),
        (
            br#"{"schema_version":99,"digest":"x","payload":null}"#,
            "incompatible_version",
        ),
        (
            br#"{"schema_version":1,"digest":"x","payload":null}"#,
            "record_corrupt",
        ),
        (
            br#"{"schema_version":1,"digest":"x","payload":null,"extra":true}"#,
            "record_corrupt",
        ),
    ] {
        dir.write_bytes("invalid.json", bytes).unwrap();
        assert_eq!(
            dir.read::<serde_json::Value>("invalid.json")
                .unwrap_err()
                .code,
            code
        );
        assert_eq!(dir.read_bytes("invalid.json", 1024).unwrap(), bytes);
    }
    assert!(dir.read_bytes("record.json", 8).is_err());
}

#[test]
fn process_guard_duplicate_preserves_identity_cloexec_and_lock_lifetime() {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::MetadataExt;
    let (_temp, dir) = directory();
    let lock = dir.lock("guard.lock").unwrap();
    let duplicate = lock.inherited_file().unwrap();
    let metadata = duplicate.metadata().unwrap();
    assert_eq!(lock.identity().unwrap(), (metadata.dev(), metadata.ino()));
    for fd in [lock.raw_fd(), duplicate.as_raw_fd()] {
        // SAFETY: both descriptors remain owned and live throughout this check.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        assert!(flags >= 0);
        assert_ne!(flags & libc::FD_CLOEXEC, 0);
    }
    drop(lock);
    assert!(dir.try_lock("guard.lock").unwrap().is_none());
    drop(duplicate);
    assert!(dir.try_lock("guard.lock").unwrap().is_some());
}
