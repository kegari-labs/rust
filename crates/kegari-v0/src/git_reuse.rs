//! A clean source proof for one retained execution, never process-exit authority.
use super::*;
use crate::execution::{ExecutionBinding, OwnedDirectory};
use crate::isolation::ValidatedIsolation;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

const MAX_ENTRIES: usize = 100_000;
const MAX_BYTES: u64 = 256 * 1024 * 1024;
const MAX_DEPTH: usize = 128;

/// Housekeeping calls this only after reading the protected closed family and
/// exact original admission. It grants fixed read-only metadata access, not
/// clean-source, process-exit or session-resumption authority.
pub(crate) fn retained_execution_host_read_root(
    configuration: &kegari_config::Config,
    binding: &ExecutionBinding,
    boundary: &ValidatedIsolation,
    ownership: &OwnedDirectory,
    lineage_digest: String,
) -> Result<VerifiedHostReadRoot> {
    crate::evidence::validate_digest(&lineage_digest)?;
    binding.validate(boundary)?;
    ownership.verify(binding, boundary)?;
    if binding.runtime_id != configuration.runtime.id {
        return Err(Error::new(
            "identity_mismatch",
            "retained execution read belongs to another runtime",
        ));
    }
    host_read_root(
        binding.repository.clone(),
        &boundary.plan().repository,
        crate::config::config_revision(configuration)?,
        lineage_digest,
        Some(ownership.marker().into()),
    )
}

/// Minted only after fresh host-owned read-only Git checks and stable, complete
/// filesystem observations. It cannot be reconstructed from a public record.
#[derive(Clone)]
pub struct VerifiedReusableSource {
    binding: ExecutionBinding,
    boundary: ValidatedIsolation,
    marker: PathBuf,
    marker_digest: String,
    clone_marker: PathBuf,
    binary: crate::platform::BinaryIdentity,
    snapshot: Snapshot,
    digest: String,
}
impl VerifiedReusableSource {
    pub fn binding(&self) -> &ExecutionBinding {
        &self.binding
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
    /// No Git subprocess, permit lookup or Control mutex reentry occurs here.
    pub fn revalidate_filesystem(&self) -> Result<()> {
        self.binding.validate(&self.boundary)?;
        OwnedDirectory::reopen(&self.marker, &self.marker_digest, &self.boundary)?
            .verify(&self.binding, &self.boundary)?;
        if crate::platform::BinaryIdentity::inspect(&self.binary.path, true)? != self.binary
            || Snapshot::capture(&self.boundary, &self.marker, &self.clone_marker)? != self.snapshot
        {
            return Err(changed(
                "reusable source, administration, identity or pinned Git changed",
            ));
        }
        Ok(())
    }
}
impl Git {
    pub fn verify_reusable_execution(
        &self,
        ownership: &OwnedDirectory,
        binding: &ExecutionBinding,
        boundary: &ValidatedIsolation,
    ) -> Result<VerifiedReusableSource> {
        let scope = self.host_scope.as_ref().ok_or_else(|| {
            Error::new(
                "process_authority_required",
                "source reuse requires fresh host metadata authority, never an old worker permit",
            )
        })?;
        let key = crate::authority::IssueKey::new(
            &binding.work.linear_endpoint,
            binding.work.linear_workspace,
            binding.work.child.unwrap_or(binding.work.parent),
        )?
        .identity()?;
        if scope.authority.identity().runtime_id != binding.runtime_id
            || scope.authority.identity().work_key != key
            || scope.authority.identity().operation_id == binding.operation_id
            || scope.authority.binary() != &self.binary
        {
            return Err(Error::new(
                "scope_violation",
                "source reuse metadata belongs to another runtime, work or binary",
            ));
        }
        scope.authority.validate_root(&boundary.plan().repository)?;
        binding.validate(boundary)?;
        ownership.verify(binding, boundary)?;
        if binding.revision.commit.len() != 40 || binding.revision.tree.len() != 40 {
            return Err(Error::new(
                "unsupported_repository",
                "source reuse requires the supported SHA-1 object format",
            ));
        }
        let clone_marker = boundary.plan().execution_root.join(format!(
            "prepared-clone-{}.json",
            ownership.identity().directory_id
        ));
        let before = Snapshot::capture(boundary, ownership.marker(), &clone_marker)?;
        let prepared = read_clone_record(&clone_marker)?;
        if !prepared.prepared
            || prepared.identity != CloneOwner::Child(ownership.identity().clone())
            || prepared.revision != binding.revision
            || prepared.git_version != self.version()
            || !matches!(prepared.schema_version, 1 | 2)
            || (prepared.schema_version == 2) != prepared.source_provenance.is_some()
        {
            return Err(changed(
                "source reuse requires its original completed exact clone preparation",
            ));
        }
        self.verify_execution_clone(&boundary.plan().repository, &binding.revision)?;
        self.assert_clean(&boundary.plan().repository)?;
        let tree = self.source_tree(&boundary.plan().repository, &binding.revision.tree)?;
        // Index flags such as assume-unchanged must not hide actual source edits.
        let files = before
            .repository
            .iter()
            .filter(|(path, entry)| {
                !path.as_os_str().is_empty() && !path.starts_with(".git") && !entry.directory
            })
            .map(|(path, entry)| {
                Ok((
                    path.to_str()
                        .ok_or_else(|| changed("source path is not UTF-8"))?
                        .to_owned(),
                    (
                        if entry.stamp.mode & 0o111 == 0 {
                            "100644"
                        } else {
                            "100755"
                        }
                        .to_owned(),
                        entry.blob.clone().unwrap(),
                    ),
                ))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        if files != tree {
            return Err(changed(
                "actual source bytes or executable modes differ from the original commit tree",
            ));
        }
        let mut expected_directories = BTreeSet::from([PathBuf::new()]);
        for path in tree.keys() {
            let mut parent = Path::new(path).parent();
            while let Some(directory) = parent {
                expected_directories.insert(directory.to_path_buf());
                parent = directory.parent();
            }
        }
        if before
            .repository
            .iter()
            .filter(|(path, entry)| entry.directory && !path.starts_with(".git"))
            .map(|(path, _)| path.clone())
            .collect::<BTreeSet<_>>()
            != expected_directories
        {
            return Err(changed(
                "source has extra untracked directories outside its original tree",
            ));
        }
        let after = Snapshot::capture(boundary, ownership.marker(), &clone_marker)?;
        if before != after {
            return Err(changed(
                "source changed while verifying its exact clean revision",
            ));
        }
        let digest = canonical_digest(
            1,
            &serde_json::json!({"purpose":"reusable_execution_source",
            "binding":binding,"isolation":boundary.digest(),"marker":ownership.digest(),
            "git":self.binary,"metadata_authority":scope.authority.identity(),"snapshot":after}),
        )?;
        let proof = VerifiedReusableSource {
            binding: binding.clone(),
            boundary: boundary.clone(),
            marker: ownership.marker().into(),
            marker_digest: ownership.digest().into(),
            clone_marker,
            binary: self.binary.clone(),
            snapshot: after,
            digest,
        };
        proof.revalidate_filesystem()?;
        Ok(proof)
    }
}

fn changed(message: &str) -> Error {
    Error::new("source_reuse_blocked", message)
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct Stamp {
    device: u64,
    inode: u64,
    uid: u32,
    mode: u32,
    links: u64,
    bytes: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}
impl Stamp {
    fn read(file: &File) -> Result<Self> {
        let m = file
            .metadata()
            .map_err(|e| Error::io("inspect reusable source descriptor", e))?;
        Ok(Self {
            device: m.dev(),
            inode: m.ino(),
            uid: m.uid(),
            mode: m.mode(),
            links: m.nlink(),
            bytes: m.len(),
            modified: (m.mtime(), m.mtime_nsec()),
            changed: (m.ctime(), m.ctime_nsec()),
        })
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct Entry {
    stamp: Stamp,
    directory: bool,
    digest: Option<String>,
    blob: Option<String>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct Snapshot {
    execution: Stamp,
    repository: BTreeMap<PathBuf, Entry>,
    markers: BTreeMap<PathBuf, Entry>,
}
impl Snapshot {
    fn capture(boundary: &ValidatedIsolation, marker: &Path, clone_marker: &Path) -> Result<Self> {
        let root = &boundary.plan().execution_root;
        let execution = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY | libc::O_CLOEXEC)
            .open(root)
            .map_err(|e| Error::io("open reusable execution root", e))?;
        let before = Stamp::read(&execution)?;
        let name = boundary
            .plan()
            .repository
            .strip_prefix(root)
            .map_err(|_| changed("repository escaped execution root"))?;
        if name.components().count() != 1 {
            return Err(changed(
                "reusable clone must be a direct owned execution child",
            ));
        }
        let source = open_at(&execution, name, true)?;
        let mut repository = BTreeMap::new();
        let mut bytes = 0;
        walk(&source, Path::new(""), &mut repository, &mut bytes, 0)?;
        let mut markers = BTreeMap::new();
        for path in [marker, clone_marker] {
            let name = path
                .strip_prefix(root)
                .map_err(|_| changed("reuse marker escaped execution root"))?;
            if name.components().count() != 1 {
                return Err(changed(
                    "reuse marker must be a direct owned execution child",
                ));
            }
            let file = open_at(&execution, name, false)?;
            let entry = capture_file(&file, &mut bytes)?;
            if Stamp::read(&open_at(&execution, name, false)?)? != entry.stamp {
                return Err(changed("reuse marker was replaced"));
            }
            markers.insert(name.to_path_buf(), entry);
        }
        let named_execution = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY | libc::O_CLOEXEC)
            .open(root)
            .map_err(|e| Error::io("reopen reusable execution root", e))?;
        if Stamp::read(&execution)? != before
            || Stamp::read(&named_execution)? != before
            || Stamp::read(&open_at(&execution, name, true)?)? != repository[Path::new("")].stamp
        {
            return Err(changed(
                "execution root or repository was replaced during capture",
            ));
        }
        Ok(Self {
            execution: before,
            repository,
            markers,
        })
    }
}
fn open_at(parent: &File, name: &Path, directory: bool) -> Result<File> {
    use std::os::unix::ffi::OsStrExt;
    let name = std::ffi::CString::new(name.as_os_str().as_bytes())
        .map_err(|_| changed("source path contains NUL"))?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY
                | libc::O_NOFOLLOW
                | libc::O_NONBLOCK
                | libc::O_CLOEXEC
                | if directory { libc::O_DIRECTORY } else { 0 },
        )
    };
    if fd < 0 {
        return Err(Error::io(
            "open reusable source entry",
            std::io::Error::last_os_error(),
        ));
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
fn capture_file(file: &File, total: &mut u64) -> Result<Entry> {
    let before = Stamp::read(file)?;
    if before.mode & libc::S_IFMT as u32 != libc::S_IFREG as u32
        || before.links != 1
        || before.uid != unsafe { libc::geteuid() }
        || before.mode & 0o022 != 0
    {
        return Err(changed(
            "reuse requires owned ordinary unlinked files without symlinks or external writes",
        ));
    }
    *total = total
        .checked_add(before.bytes)
        .filter(|n| *n <= MAX_BYTES)
        .ok_or_else(|| changed("reusable source exceeds byte bound"))?;
    let mut sha256 = Sha256::new();
    let mut blob = sha1::Sha1::new();
    blob.update(format!("blob {}\0", before.bytes).as_bytes());
    let mut read = file;
    let mut count = 0u64;
    let mut buffer = [0u8; 65536];
    loop {
        let n = read
            .read(&mut buffer)
            .map_err(|e| Error::io("hash reusable source", e))?;
        if n == 0 {
            break;
        }
        count += n as u64;
        if count > before.bytes {
            return Err(changed("source grew during capture"));
        }
        sha256.update(&buffer[..n]);
        blob.update(&buffer[..n]);
    }
    if count != before.bytes || Stamp::read(file)? != before {
        return Err(changed("source changed during capture"));
    }
    Ok(Entry {
        stamp: before,
        directory: false,
        digest: Some(format!("sha256:{}", hex::encode(sha256.finalize()))),
        blob: Some(hex::encode(blob.finalize())),
    })
}
struct DirectoryStream(*mut libc::DIR);
impl Drop for DirectoryStream {
    fn drop(&mut self) {
        unsafe {
            libc::closedir(self.0);
        }
    }
}
fn walk(
    directory: &File,
    relative: &Path,
    entries: &mut BTreeMap<PathBuf, Entry>,
    total: &mut u64,
    depth: usize,
) -> Result<()> {
    if depth > MAX_DEPTH || entries.len() >= MAX_ENTRIES {
        return Err(changed("reusable source inventory exceeds bound"));
    }
    let before = Stamp::read(directory)?;
    if before.uid != unsafe { libc::geteuid() } || before.mode & 0o022 != 0 {
        return Err(changed("reuse directory is writable outside its owner"));
    }
    entries.insert(
        relative.into(),
        Entry {
            stamp: before.clone(),
            directory: true,
            digest: None,
            blob: None,
        },
    );
    let fd = unsafe { libc::dup(directory.as_raw_fd()) };
    if fd < 0 {
        return Err(Error::io(
            "duplicate reusable source directory",
            std::io::Error::last_os_error(),
        ));
    }
    let stream = unsafe { libc::fdopendir(fd) };
    if stream.is_null() {
        unsafe { libc::close(fd) };
        return Err(Error::io(
            "enumerate reusable source",
            std::io::Error::last_os_error(),
        ));
    }
    let stream = DirectoryStream(stream);
    loop {
        set_errno(0)?;
        let item = unsafe { libc::readdir(stream.0) };
        if item.is_null() {
            if std::io::Error::last_os_error().raw_os_error() != Some(0) {
                return Err(Error::io(
                    "read reusable source inventory",
                    std::io::Error::last_os_error(),
                ));
            }
            break;
        }
        let name = unsafe { std::ffi::CStr::from_ptr((*item).d_name.as_ptr()) }
            .to_str()
            .map_err(|_| changed("reuse paths must be UTF-8"))?;
        if name == "." || name == ".." {
            continue;
        }
        if entries.len() >= MAX_ENTRIES {
            return Err(changed("reusable source inventory exceeds bound"));
        }
        let child = open_at(directory, Path::new(name), false)?;
        let metadata = child
            .metadata()
            .map_err(|e| Error::io("inspect reusable child", e))?;
        let path = relative.join(name);
        if metadata.is_dir() {
            walk(&child, &path, entries, total, depth + 1)?;
        } else {
            entries.insert(path.clone(), capture_file(&child, total)?);
        }
        if Stamp::read(&open_at(directory, Path::new(name), metadata.is_dir())?)?
            != entries[&path].stamp
        {
            return Err(changed("source entry was replaced during capture"));
        }
    }
    if Stamp::read(directory)? != before {
        return Err(changed("directory changed during capture"));
    }
    Ok(())
}
fn set_errno(value: i32) -> Result<()> {
    #[cfg(target_os = "macos")]
    unsafe {
        *libc::__error() = value;
    }
    #[cfg(target_os = "linux")]
    unsafe {
        *libc::__errno_location() = value;
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = value;
        return Err(Error::new(
            "unsupported_platform",
            "source inventory requires a supported Unix directory API",
        ));
    }
    Ok(())
}
