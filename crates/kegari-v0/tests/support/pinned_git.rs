//! An owned test-tool copy, not host installation or native qualification.
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};
pub fn prepare(root: &Path) -> PathBuf {
    let source = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|directory| directory.join("git"))
        .find(|path| path.is_file())
        .expect("fixture Git is required")
        .canonicalize()
        .unwrap();
    let directory = root.canonicalize().unwrap().join("fixture-pinned-git");
    fs::create_dir_all(&directory).unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    let target = directory.join("git");
    let expected = fs::read(&source).unwrap();
    if target.exists() {
        assert_eq!(
            fs::read(&target).unwrap(),
            expected,
            "fixture tool bytes changed"
        );
    } else {
        use std::io::Write;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .unwrap();
        output.write_all(&expected).unwrap();
        output.sync_all().unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o555)).unwrap();
    }
    let identity = kegari_platform::BinaryIdentity::inspect(&target, false).unwrap();
    assert_eq!(identity.digest, kegari_digest::bytes_digest(&expected));
    target
}
