use sha2::{Digest, Sha256};
use std::{
    env, fs,
    path::{Path, PathBuf},
};

fn collect(root: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(root).expect("read build inputs") {
        let entry = entry.expect("read build input");
        let kind = entry.file_type().expect("read build input type");
        assert!(!kind.is_symlink(), "build inputs may not be symlinks");
        if kind.is_dir() {
            collect(&entry.path(), files);
        } else if kind.is_file() {
            files.push(entry.path());
        }
    }
}

fn main() {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest root"));
    let mut files = Vec::new();
    // All process roles retain one application identity. Track extracted packages
    // as build inputs without introducing a reverse Cargo dependency.
    for name in [
        "src",
        "instructions",
        "protocol",
        "examples",
        "../kegari-cli/src",
        "../kegari-config/src",
        "../kegari-digest/src",
        "../kegari-providers/src",
        "../kegari-types/src",
        "../kegari-storage/src",
        "../kegari-process/src",
    ] {
        println!("cargo:rerun-if-changed={name}");
        collect(&root.join(name), &mut files);
    }
    for name in [
        "build.rs",
        "Cargo.toml",
        "../kegari-cli/Cargo.toml",
        "../kegari-config/Cargo.toml",
        "../kegari-digest/Cargo.toml",
        "../kegari-providers/Cargo.toml",
        "../kegari-types/Cargo.toml",
        "../kegari-storage/Cargo.toml",
        "../kegari-process/Cargo.toml",
        "../../Cargo.toml",
        "../../Cargo.lock",
        "../../rust-toolchain.toml",
    ] {
        println!("cargo:rerun-if-changed={name}");
        files.push(root.join(name));
    }
    files.sort();
    let mut hash = Sha256::new();
    hash.update(b"kegari-build-inputs-v1\0");
    for key in ["TARGET", "PROFILE", "CARGO_PKG_VERSION"] {
        let value = env::var(key).expect("build environment");
        hash.update(key.as_bytes());
        hash.update([0]);
        hash.update(value.as_bytes());
        hash.update([0]);
    }
    for path in files {
        let relative = path.strip_prefix(&root).expect("relative build input");
        let contents = fs::read(&path).expect("read exact build bytes");
        hash.update(relative.to_str().expect("UTF8 input name").as_bytes());
        hash.update([0]);
        hash.update((contents.len() as u64).to_be_bytes());
        hash.update(contents);
    }
    println!(
        "cargo:rustc-env=KEGARI_BUILD_ID=sha256:{:x}",
        hash.finalize()
    );
}
