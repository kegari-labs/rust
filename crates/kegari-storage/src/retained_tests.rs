use super::*;
use std::os::unix::fs::PermissionsExt;

fn directory() -> (tempfile::TempDir, PrivateDir) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let directory = PrivateDir::create(&root.join("private")).unwrap();
    (temp, directory)
}
fn create(directory: &PrivateDir, leaf: &str, bytes: &[u8]) -> Arc<RetainedPrivateFile> {
    let mut anchor = None;
    directory
        .create_bytes_exclusive_retained(leaf, bytes, |actual| {
            anchor = Some(actual);
            Ok(())
        })
        .unwrap();
    anchor.unwrap()
}
fn replace_identically(directory: &PrivateDir, leaf: &str, bytes: &[u8]) {
    let replacement = directory.path().join("replacement.tmp");
    std::fs::write(&replacement, bytes).unwrap();
    std::fs::set_permissions(&replacement, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::rename(&replacement, directory.path().join(leaf)).unwrap();
}

#[test]
fn retained_exclusive_creation_refuses_identical_replacement_on_first_read() {
    let (_temp, directory) = directory();
    let anchor = create(&directory, "original.bin", b"original bytes");
    replace_identically(&directory, "original.bin", b"original bytes");
    assert_eq!(
        directory.read_bytes("original.bin", 128).unwrap(),
        b"original bytes"
    );
    assert_eq!(
        directory
            .read_retained_bytes(&anchor, 128)
            .unwrap_err()
            .code,
        "ownership_conflict"
    );
}

#[test]
fn retained_creation_preserves_actual_partial_write_identity_without_read_adoption() {
    let (_temp, directory) = directory();
    let mut retained = None;
    let error = directory
        .create_bytes_exclusive_retained("partial.bin", b"complete bytes", |anchor| {
            retained = Some(anchor);
            // Real bytes written through the same protected named file after the
            // O_EXCL identity callback, before the producer's full write/sync.
            directory
                .open_file("partial.bin", libc::O_WRONLY)?
                .write_all(b"part")
                .map_err(|e| Error::io("component partial write", e))?;
            Err(Error::new(
                "component_interruption",
                "stop before full immutable write",
            ))
        })
        .unwrap_err();
    assert_eq!(error.code, "component_interruption");
    let retained = retained.unwrap();
    assert_eq!(directory.read_bytes("partial.bin", 128).unwrap(), b"part");
    assert_eq!(
        directory
            .read_retained_bytes(&retained, 128)
            .unwrap_err()
            .code,
        "record_incomplete"
    );
    replace_identically(&directory, "partial.bin", b"part");
    assert_eq!(
        directory
            .read_retained_bytes(&retained, 128)
            .unwrap_err()
            .code,
        "record_incomplete"
    );
    assert!(
        directory
            .create_bytes_exclusive_retained("partial.bin", b"complete bytes", |_| Ok(()))
            .is_err()
    );
    assert_eq!(directory.read_bytes("partial.bin", 128).unwrap(), b"part");
}

#[test]
fn retained_read_checks_unchanged_file_missing_leaf_and_exact_directory() {
    let (temp, directory) = directory();
    let anchor = create(&directory, "same.bin", b"same bytes");
    assert_eq!(
        directory.read_retained_bytes(&anchor, 128).unwrap(),
        b"same bytes"
    );
    assert_eq!(
        directory.read_retained_bytes(&anchor, 128).unwrap(),
        b"same bytes"
    );
    let other = PrivateDir::create(&temp.path().canonicalize().unwrap().join("other")).unwrap();
    other
        .create_bytes_exclusive("same.bin", b"same bytes")
        .unwrap();
    assert_eq!(
        other.read_retained_bytes(&anchor, 128).unwrap_err().code,
        "ownership_conflict"
    );
    std::fs::remove_file(directory.path().join("same.bin")).unwrap();
    assert_eq!(
        directory
            .read_retained_bytes(&anchor, 128)
            .unwrap_err()
            .code,
        "record_missing"
    );
}

#[test]
fn retained_read_keeps_original_storage_and_size_checks() {
    let (temp, directory) = directory();
    let anchor = create(&directory, "file.bin", b"same bytes");
    assert_eq!(
        directory.read_retained_bytes(&anchor, 0).unwrap_err().code,
        "invalid_config"
    );
    assert_eq!(
        directory
            .read_retained_bytes(&anchor, 8 * 1024 * 1024 + 1)
            .unwrap_err()
            .code,
        "invalid_config"
    );
    assert_eq!(
        directory.read_retained_bytes(&anchor, 1).unwrap_err().code,
        "record_corrupt"
    );
    let alias = temp.path().join("alias");
    std::fs::hard_link(directory.path().join("file.bin"), &alias).unwrap();
    assert_eq!(
        directory
            .read_retained_bytes(&anchor, 128)
            .unwrap_err()
            .code,
        "unsafe_storage"
    );
}
