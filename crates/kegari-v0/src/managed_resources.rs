//! Automatically observed immutable read resources. No expected digest or tool
//! mapping is supplied by an operator. Regular bytes and link text are captured
//! through no-follow descriptors; link targets are not traversed or claimed as
//! observed. The execution domain, not this evidence, supplies access authority.
use super::PathGrant;
use kegari_types::error::{Error, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{CStr, CString},
    fs::File,
    io::{Read, Write},
    os::fd::{AsRawFd, FromRawFd, RawFd},
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};

const MAX_ENTRIES: usize = 65_536;
const MAX_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_TEXT_BYTES: usize = 32 * 1024 * 1024;
const MAX_PATH_BYTES: usize = 4096;
const MAX_DEPTH: usize = 256;
const MAX_ELAPSED: Duration = Duration::from_secs(30);

fn limit() -> Error {
    Error::new(
        "managed_resource_limit",
        "managed resource inventory exceeds its finite bound",
    )
}
fn changed() -> Error {
    Error::new(
        "identity_mismatch",
        "managed resource contents or original filesystem identity changed",
    )
}
fn invalid() -> Error {
    Error::new(
        "managed_resource_invalid",
        "resource observation requires a canonical regular file/directory tree",
    )
}

/// One budget is shared by every content-bound resource in a policy. Clock
/// checks surround OS operations and each hash chunk; a synchronous syscall is
/// not preemptible and must itself return before its elapsed time is observed.
pub(super) struct ResourceBudget {
    deadline: Instant,
    entries: usize,
    bytes: u64,
    text: usize,
    file_bytes: u64,
}
impl Default for ResourceBudget {
    fn default() -> Self {
        Self::new()
    }
}
impl ResourceBudget {
    pub(super) fn new() -> Self {
        Self::until(Instant::now() + MAX_ELAPSED)
    }
    pub(super) fn until(deadline: Instant) -> Self {
        Self {
            deadline: deadline.min(Instant::now() + MAX_ELAPSED),
            entries: MAX_ENTRIES,
            bytes: MAX_BYTES,
            text: MAX_TEXT_BYTES,
            file_bytes: MAX_FILE_BYTES,
        }
    }
    pub(super) fn check(&self) -> Result<()> {
        if Instant::now() >= self.deadline {
            Err(limit())
        } else {
            Ok(())
        }
    }
    fn entry(&mut self, name_bytes: usize) -> Result<()> {
        self.check()?;
        self.entries = self.entries.checked_sub(1).ok_or_else(limit)?;
        self.text = self.text.checked_sub(name_bytes).ok_or_else(limit)?;
        Ok(())
    }
    fn text(&mut self, bytes: usize) -> Result<()> {
        self.check()?;
        self.text = self.text.checked_sub(bytes).ok_or_else(limit)?;
        Ok(())
    }
    fn bytes(&mut self, bytes: u64) -> Result<()> {
        self.check()?;
        self.bytes = self.bytes.checked_sub(bytes).ok_or_else(limit)?;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    path: String,
    kind: EntryKind,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum EntryKind {
    Directory,
    File { bytes: u64, digest: String },
    Link { target: String },
}

/// Host metadata deliberately does not enter the portable content digest. It is
/// retained separately so a fresh, byte-identical replacement is not silently
/// substituted for the originally captured input on revalidation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Version {
    device: u64,
    inode: u64,
    mode: u64,
    links: u64,
    uid: u64,
    gid: u64,
    bytes: i64,
    modified: i64,
    modified_ns: i64,
    changed: i64,
    changed_ns: i64,
}
fn unsigned<T: TryInto<u64>>(value: T) -> Result<u64> {
    value.try_into().map_err(|_| invalid())
}
fn signed<T: TryInto<i64>>(value: T) -> Result<i64> {
    value.try_into().map_err(|_| invalid())
}
impl Version {
    fn from_stat(value: &libc::stat) -> Result<Self> {
        Ok(Self {
            device: unsigned(value.st_dev)?,
            inode: unsigned(value.st_ino)?,
            mode: unsigned(value.st_mode)?,
            links: unsigned(value.st_nlink)?,
            uid: unsigned(value.st_uid)?,
            gid: unsigned(value.st_gid)?,
            bytes: signed(value.st_size)?,
            modified: signed(value.st_mtime)?,
            modified_ns: signed(value.st_mtime_nsec)?,
            changed: signed(value.st_ctime)?,
            changed_ns: signed(value.st_ctime_nsec)?,
        })
    }
    fn is(&self, kind: libc::mode_t) -> bool {
        self.mode & u64::from(libc::S_IFMT) == u64::from(kind)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BoundManagedResource {
    schema_version: u32,
    input: PathGrant,
    root: PathBuf,
    entries: Vec<Entry>,
    versions: Vec<Version>,
    digest: String,
}
impl BoundManagedResource {
    pub(super) fn root(&self) -> &Path {
        &self.root
    }

    /// Membership in the original observation only. This does not observe
    /// current contents, grant access or replace full resource revalidation.
    #[cfg(test)]
    fn verify_file(&self, path: &Path, digest: &str, bytes: u64) -> Result<()> {
        self.verify_file_with_budget(path, digest, bytes, &ResourceBudget::new())
    }
    pub(super) fn verify_file_with_budget(
        &self,
        path: &Path,
        digest: &str,
        bytes: u64,
        budget: &ResourceBudget,
    ) -> Result<()> {
        budget.check()?;
        let suffix = path.strip_prefix(&self.root).map_err(|_| changed())?;
        let name = if suffix.as_os_str().is_empty() {
            "."
        } else {
            if suffix
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
            {
                return Err(changed());
            }
            suffix.to_str().ok_or_else(changed)?
        };
        for entry in &self.entries {
            budget.check()?;
            if entry.path == name {
                return match &entry.kind {
                    EntryKind::File {
                        bytes: original_bytes,
                        digest: original_digest,
                    } if *original_bytes == bytes && original_digest == digest => Ok(()),
                    _ => Err(changed()),
                };
            }
        }
        Err(changed())
    }

    pub(super) fn capture(input: &PathGrant, budget: &mut ResourceBudget) -> Result<Self> {
        charge_grant(input, budget)?;
        let (root, entries, versions) = capture_tree(&input.path, input.directory, budget)?;
        validate_entries(&root, input.directory, &entries, budget.deadline)?;
        let digest = manifest_digest(&entries, budget.deadline)?;
        budget.check()?;
        Ok(Self {
            schema_version: 2,
            input: input.clone(),
            root,
            entries,
            versions,
            digest,
        })
    }
    /// Bounded validation of the retained observation; no filesystem contents
    /// are read here. Full recapture remains separate and outside Control.
    pub(super) fn validate_binding_with_budget(
        &self,
        input: &PathGrant,
        budget: &mut ResourceBudget,
    ) -> Result<()> {
        charge_grant(input, budget)?;
        for entry in &self.entries {
            budget.entry(entry.path.len())?;
            match &entry.kind {
                EntryKind::File { bytes, .. } => budget.bytes(*bytes)?,
                EntryKind::Link { target } => budget.text(target.len())?,
                EntryKind::Directory => (),
            }
        }
        if self.schema_version != 2
            || self.input != *input
            || self.root != input.path
            || self.entries.len() != self.versions.len()
            || self.entries.len() > MAX_ENTRIES
        {
            return Err(changed());
        }
        validate_entries(&self.root, input.directory, &self.entries, budget.deadline)?;
        for (entry, version) in self.entries.iter().zip(&self.versions) {
            budget.check()?;
            let matches = match &entry.kind {
                EntryKind::Directory => version.is(libc::S_IFDIR),
                EntryKind::File { bytes, .. } => {
                    version.is(libc::S_IFREG) && unsigned(version.bytes)? == *bytes
                }
                EntryKind::Link { target } => {
                    version.is(libc::S_IFLNK) && unsigned(version.bytes)? == target.len() as u64
                }
            };
            if !matches {
                return Err(changed());
            }
        }
        if self.digest != manifest_digest(&self.entries, budget.deadline)? {
            return Err(changed());
        }
        budget.check()
    }
    pub(super) fn revalidate(&self, budget: &mut ResourceBudget) -> Result<()> {
        if Self::capture(&self.input, budget)? != *self {
            return Err(changed());
        }
        Ok(())
    }
    /// Recheck the original filesystem versions without reading file contents
    /// or enumerating a new inventory. Directory versions detect added/removed
    /// names; every intermediate directory is opened without following links.
    /// This is a dispatch-time check, not a replacement for full recapture.
    pub(super) fn revalidate_versions(&self, budget: &mut ResourceBudget) -> Result<()> {
        budget.check()?;
        let originals = self
            .entries
            .iter()
            .zip(&self.versions)
            .map(|(entry, version)| (entry.path.as_str(), version))
            .collect::<BTreeMap<_, _>>();
        let original_root = originals.get(".").ok_or_else(invalid)?;
        let name = self.root.to_str().ok_or_else(invalid)?;
        if self
            .root
            .canonicalize()
            .map_err(|e| Error::io("resolve original resource root", e))?
            != self.root
            || stat_at(libc::AT_FDCWD, name)? != **original_root
        {
            return Err(changed());
        }
        let root = open_at(libc::AT_FDCWD, name, self.input.directory)?;
        if stat_file(&root)? != **original_root {
            return Err(changed());
        }
        for (relative, original) in &originals {
            budget.check()?;
            let actual = if *relative == "." {
                stat_file(&root)?
            } else {
                let parts = relative.split('/').collect::<Vec<_>>();
                let mut parent = open_at(root.as_raw_fd(), ".", true)?;
                let mut prefix = String::new();
                for part in &parts[..parts.len() - 1] {
                    budget.check()?;
                    if !prefix.is_empty() {
                        prefix.push('/');
                    }
                    prefix.push_str(part);
                    parent = open_at(parent.as_raw_fd(), part, true)?;
                    if stat_file(&parent)?
                        != **originals.get(prefix.as_str()).ok_or_else(invalid)?
                    {
                        return Err(changed());
                    }
                }
                stat_at(parent.as_raw_fd(), parts[parts.len() - 1])?
            };
            if actual != **original {
                return Err(changed());
            }
        }
        if stat_file(&root)? != **original_root
            || stat_at(libc::AT_FDCWD, name)? != **original_root
            || self
                .root
                .canonicalize()
                .map_err(|e| Error::io("recheck original resource root", e))?
                != self.root
        {
            return Err(changed());
        }
        budget.check()
    }
    #[cfg(test)]
    fn read_grants(&self) -> Vec<PathGrant> {
        vec![self.input.clone()]
    }
}

#[cfg(test)]
fn content_digest(path: &Path, directory: bool, budget: &mut ResourceBudget) -> Result<String> {
    Ok(BoundManagedResource::capture(
        &PathGrant {
            path: path.into(),
            directory,
        },
        budget,
    )?
    .digest)
}

fn normalized_root(path: &Path) -> Result<()> {
    let text = path.to_str().ok_or_else(invalid)?;
    if !path.is_absolute()
        || path.parent().is_none()
        || text.len() > MAX_PATH_BYTES
        || text.contains('\0')
        || path
            .components()
            .any(|part| !matches!(part, Component::RootDir | Component::Normal(_)))
        || path.components().collect::<PathBuf>().as_os_str() != path.as_os_str()
    {
        return Err(invalid());
    }
    Ok(())
}
fn charge_grant(input: &PathGrant, budget: &mut ResourceBudget) -> Result<()> {
    normalized_root(&input.path)?;
    budget.text(input.path.to_str().ok_or_else(invalid)?.len())
}
fn relative(path: &str) -> Result<Vec<&str>> {
    if path == "." {
        return Ok(vec![]);
    }
    if path.is_empty() || path.len() > MAX_PATH_BYTES || path.contains('\0') {
        return Err(invalid());
    }
    let parts: Vec<_> = path.split('/').collect();
    if parts.len() > MAX_DEPTH
        || parts
            .iter()
            .any(|p| p.is_empty() || *p == "." || *p == "..")
    {
        return Err(invalid());
    }
    Ok(parts)
}
fn validate_entries(
    root: &Path,
    directory: bool,
    entries: &[Entry],
    deadline: Instant,
) -> Result<()> {
    normalized_root(root)?;
    if entries.is_empty() || entries.len() > MAX_ENTRIES {
        return Err(limit());
    }
    let mut index: BTreeMap<&str, usize> = BTreeMap::new();
    let mut total_bytes = 0_u64;
    let mut text = 0_usize;
    let mut previous: Option<&str> = None;
    for (ordinal, entry) in entries.iter().enumerate() {
        if Instant::now() >= deadline {
            return Err(limit());
        }
        let parts = relative(&entry.path)?;
        if ordinal > 0 {
            if previous.is_some_and(|prior| prior >= entry.path.as_str()) {
                return Err(invalid());
            }
            previous = Some(&entry.path);
        }
        text = text.checked_add(entry.path.len()).ok_or_else(limit)?;
        if ordinal == 0 {
            if entry.path != "."
                || (matches!(entry.kind, EntryKind::Directory) != directory)
                || matches!(entry.kind, EntryKind::Link { .. })
            {
                return Err(invalid());
            }
        } else {
            if parts.is_empty() {
                return Err(invalid());
            }
            let parent = if parts.len() == 1 {
                ".".to_owned()
            } else {
                parts[..parts.len() - 1].join("/")
            };
            let parent_index = *index.get(parent.as_str()).ok_or_else(invalid)?;
            if !matches!(entries[parent_index].kind, EntryKind::Directory) {
                return Err(invalid());
            }
        }
        match &entry.kind {
            EntryKind::File { bytes, digest } => {
                if *bytes > MAX_FILE_BYTES {
                    return Err(limit());
                }
                total_bytes = total_bytes.checked_add(*bytes).ok_or_else(limit)?;
                if digest.len() != 71
                    || !digest.starts_with("sha256:")
                    || !digest[7..]
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                {
                    return Err(invalid());
                }
            }
            EntryKind::Link { target } => {
                if target.is_empty() || target.len() > MAX_PATH_BYTES || target.contains('\0') {
                    return Err(invalid());
                }
                text = text.checked_add(target.len()).ok_or_else(limit)?;
            }
            EntryKind::Directory => (),
        }
        if text > MAX_TEXT_BYTES || total_bytes > MAX_BYTES {
            return Err(limit());
        }
        index.insert(entry.path.as_str(), ordinal);
    }
    // Link text is evidence about this entry only. Do not resolve links here:
    // doing so would turn an observation into extra filesystem authority and
    // would incorrectly require every ordinary resource to be a closed graph.
    Ok(())
}

struct HashWriter {
    hash: Sha256,
    deadline: Instant,
}
impl Write for HashWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if Instant::now() >= self.deadline {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "resource deadline",
            ));
        }
        self.hash.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn manifest_digest(entries: &[Entry], deadline: Instant) -> Result<String> {
    #[derive(Serialize)]
    struct Manifest<'a> {
        domain: &'static str,
        schema_version: u32,
        entries: &'a [Entry],
    }
    let mut writer = HashWriter {
        hash: Sha256::new(),
        deadline,
    };
    serde_json::to_writer(
        &mut writer,
        &Manifest {
            domain: "kegari.environment-resource-observation",
            schema_version: 2,
            entries,
        },
    )
    .map_err(|_| limit())?;
    Ok(format!("sha256:v2:{}", hex::encode(writer.hash.finalize())))
}

fn cstring(value: &str) -> Result<CString> {
    CString::new(value).map_err(|_| invalid())
}
fn stat_at(parent: RawFd, name: &str) -> Result<Version> {
    let name = cstring(name)?;
    let mut value = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: valid C string and writable stat storage; no link is followed.
    if unsafe {
        libc::fstatat(
            parent,
            name.as_ptr(),
            value.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } != 0
    {
        return Err(Error::io(
            "inspect managed resource entry",
            std::io::Error::last_os_error(),
        ));
    }
    Version::from_stat(&unsafe { value.assume_init() })
}
fn stat_file(file: &File) -> Result<Version> {
    let mut value = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: the live descriptor is borrowed and the stat destination is valid.
    if unsafe { libc::fstat(file.as_raw_fd(), value.as_mut_ptr()) } != 0 {
        return Err(Error::io(
            "inspect managed resource descriptor",
            std::io::Error::last_os_error(),
        ));
    }
    Version::from_stat(&unsafe { value.assume_init() })
}
fn open_at(parent: RawFd, name: &str, directory: bool) -> Result<File> {
    let name = cstring(name)?;
    let flags = libc::O_RDONLY
        | libc::O_NOFOLLOW
        | libc::O_CLOEXEC
        | libc::O_NONBLOCK
        | if directory { libc::O_DIRECTORY } else { 0 };
    // SAFETY: openat receives a live parent (or AT_FDCWD) and a valid C string.
    let fd = unsafe { libc::openat(parent, name.as_ptr(), flags) };
    if fd < 0 {
        return Err(Error::io(
            "open managed resource without links",
            std::io::Error::last_os_error(),
        ));
    }
    // SAFETY: this newly opened descriptor is owned exactly once.
    Ok(unsafe { File::from_raw_fd(fd) })
}
fn link_at(parent: RawFd, name: &str) -> Result<String> {
    let name = cstring(name)?;
    let mut bytes = [0_u8; MAX_PATH_BYTES + 1];
    // SAFETY: bounded writable buffer and valid name; readlinkat follows no link.
    let count = unsafe {
        libc::readlinkat(
            parent,
            name.as_ptr(),
            bytes.as_mut_ptr().cast(),
            bytes.len(),
        )
    };
    if count < 0 {
        return Err(Error::io(
            "read managed resource link",
            std::io::Error::last_os_error(),
        ));
    }
    let count = usize::try_from(count).map_err(|_| invalid())?;
    if count == 0 || count > MAX_PATH_BYTES {
        return Err(limit());
    }
    String::from_utf8(bytes[..count].to_vec()).map_err(|_| invalid())
}
struct Directory(*mut libc::DIR);
impl Drop for Directory {
    fn drop(&mut self) {
        unsafe {
            libc::closedir(self.0);
        }
    }
}
fn names(file: &File, prefix: &str, budget: &mut ResourceBudget) -> Result<Vec<String>> {
    // Open '.' to obtain an independent directory offset, not dup's shared one.
    let directory = open_at(file.as_raw_fd(), ".", true)?;
    use std::os::fd::IntoRawFd;
    let fd = directory.into_raw_fd();
    let raw = unsafe { libc::fdopendir(fd) };
    if raw.is_null() {
        let error = std::io::Error::last_os_error();
        unsafe {
            libc::close(fd);
        }
        return Err(Error::io("enumerate managed resource directory", error));
    }
    let directory = Directory(raw);
    let mut result = BTreeSet::new();
    loop {
        budget.check()?;
        #[cfg(target_os = "macos")]
        unsafe {
            *libc::__error() = 0;
        }
        #[cfg(target_os = "linux")]
        unsafe {
            *libc::__errno_location() = 0;
        }
        let entry = unsafe { libc::readdir(directory.0) };
        if entry.is_null() {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(0) {
                return Err(Error::io("read managed resource directory", error));
            }
            break;
        }
        // SAFETY: readdir's entry remains valid until the next call; d_name is
        // a terminated directory-entry name, copied before that next call.
        let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }
            .to_str()
            .map_err(|_| invalid())?;
        if name == "." || name == ".." {
            continue;
        }
        if name.is_empty() || name.contains('/') {
            return Err(invalid());
        }
        let relative_bytes = if prefix == "." {
            name.len()
        } else {
            prefix.len() + 1 + name.len()
        };
        if relative_bytes > MAX_PATH_BYTES {
            return Err(limit());
        }
        budget.entry(relative_bytes)?;
        if !result.insert(name.to_owned()) {
            return Err(changed());
        }
    }
    Ok(result.into_iter().collect())
}

struct Frame {
    file: File,
    relative: String,
    version: Version,
    names: Vec<String>,
    next: usize,
}
fn file_kind(file: &mut File, version: &Version, budget: &mut ResourceBudget) -> Result<EntryKind> {
    let expected = unsigned(version.bytes)?;
    if expected > budget.file_bytes || expected > budget.bytes {
        return Err(limit());
    }
    let mut hash = Sha256::new();
    let mut count = 0_u64;
    let mut chunk = [0_u8; 64 * 1024];
    loop {
        budget.check()?;
        let size = file
            .read(&mut chunk)
            .map_err(|e| Error::io("hash managed resource", e))?;
        if size == 0 {
            break;
        }
        count = count.checked_add(size as u64).ok_or_else(limit)?;
        if count > expected || count > budget.file_bytes {
            return Err(changed());
        }
        budget.bytes(size as u64)?;
        hash.update(&chunk[..size]);
    }
    if count != expected || stat_file(file)? != *version {
        return Err(changed());
    }
    Ok(EntryKind::File {
        bytes: count,
        digest: format!("sha256:{}", hex::encode(hash.finalize())),
    })
}
fn capture_tree(
    path: &Path,
    directory: bool,
    budget: &mut ResourceBudget,
) -> Result<(PathBuf, Vec<Entry>, Vec<Version>)> {
    normalized_root(path)?;
    budget.check()?;
    let root = path
        .canonicalize()
        .map_err(|e| Error::io("resolve managed content root", e))?;
    if root != path {
        return Err(invalid());
    }
    let root_name = root.to_str().ok_or_else(invalid)?;
    let version = stat_at(libc::AT_FDCWD, root_name)?;
    if if directory {
        !version.is(libc::S_IFDIR)
    } else {
        !version.is(libc::S_IFREG)
    } {
        return Err(invalid());
    }
    let mut root_file = open_at(libc::AT_FDCWD, root_name, directory)?;
    if stat_file(&root_file)? != version {
        return Err(changed());
    }
    budget.entry(1)?;
    let mut captured = BTreeMap::new();
    if !directory {
        let kind = file_kind(&mut root_file, &version, budget)?;
        captured.insert(".".to_owned(), (kind, version.clone()));
    } else {
        captured.insert(".".to_owned(), (EntryKind::Directory, version.clone()));
        let mut visited = BTreeSet::from([(version.device, version.inode)]);
        let first_names = names(&root_file, ".", budget)?;
        let mut stack = vec![Frame {
            file: open_at(root_file.as_raw_fd(), ".", true)?,
            relative: ".".into(),
            version: version.clone(),
            names: first_names,
            next: 0,
        }];
        while !stack.is_empty() {
            budget.check()?;
            let frame = stack.last_mut().ok_or_else(invalid)?;
            if frame.next == frame.names.len() {
                if stat_file(&frame.file)? != frame.version {
                    return Err(changed());
                }
                stack.pop();
                continue;
            }
            let name = frame.names[frame.next].clone();
            frame.next += 1;
            let relative = if frame.relative == "." {
                name.clone()
            } else {
                format!("{}/{name}", frame.relative)
            };
            if relative.split('/').count() > MAX_DEPTH {
                return Err(limit());
            }
            let actual = stat_at(frame.file.as_raw_fd(), &name)?;
            if actual.is(libc::S_IFLNK) {
                let target = link_at(frame.file.as_raw_fd(), &name)?;
                budget.text(target.len())?;
                if stat_at(frame.file.as_raw_fd(), &name)? != actual {
                    return Err(changed());
                }
                captured.insert(relative, (EntryKind::Link { target }, actual));
            } else if actual.is(libc::S_IFREG) || actual.is(libc::S_IFDIR) {
                let mut file = open_at(frame.file.as_raw_fd(), &name, actual.is(libc::S_IFDIR))?;
                if stat_file(&file)? != actual {
                    return Err(changed());
                }
                if actual.is(libc::S_IFDIR) {
                    if !visited.insert((actual.device, actual.inode)) {
                        return Err(invalid());
                    }
                    let names = names(&file, &relative, budget)?;
                    captured.insert(relative.clone(), (EntryKind::Directory, actual.clone()));
                    stack.push(Frame {
                        file,
                        relative,
                        version: actual,
                        names,
                        next: 0,
                    });
                } else {
                    let kind = file_kind(&mut file, &actual, budget)?;
                    captured.insert(relative, (kind, actual));
                }
            } else {
                return Err(invalid());
            }
        }
    }
    // Recheck every original entry after hashing. Directory enumeration alone
    // cannot detect a file modified after it was read without changing its name.
    for (relative, (_, original)) in &captured {
        budget.check()?;
        let current = if relative == "." {
            stat_file(&root_file)?
        } else {
            let parts = relative.split('/').collect::<Vec<_>>();
            let mut parent = open_at(root_file.as_raw_fd(), ".", true)?;
            let mut prefix = String::new();
            for part in &parts[..parts.len() - 1] {
                if !prefix.is_empty() {
                    prefix.push('/');
                }
                prefix.push_str(part);
                parent = open_at(parent.as_raw_fd(), part, true)?;
                if stat_file(&parent)? != captured.get(&prefix).ok_or_else(invalid)?.1 {
                    return Err(changed());
                }
            }
            stat_at(parent.as_raw_fd(), parts[parts.len() - 1])?
        };
        if current != *original {
            return Err(changed());
        }
    }
    if stat_at(libc::AT_FDCWD, root_name)? != version
        || root
            .canonicalize()
            .map_err(|e| Error::io("recheck managed root", e))?
            != root
    {
        return Err(changed());
    }
    budget.check()?;
    // The synthetic root is first regardless of ordinary filename punctuation.
    // All real entries then have stable bytewise lexical order.
    let root_entry = captured.remove(".").ok_or_else(invalid)?;
    let (entries, versions) = std::iter::once((".".to_owned(), root_entry))
        .chain(captured)
        .map(|(path, (kind, version))| (Entry { path, kind }, version))
        .unzip();
    Ok((root, entries, versions))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::symlink};

    struct Tree {
        _temp: tempfile::TempDir,
        root: PathBuf,
    }
    impl Tree {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().canonicalize().unwrap().join("resource");
            fs::create_dir(&root).unwrap();
            Self { _temp: temp, root }
        }
        fn digest(&self) -> Result<String> {
            content_digest(&self.root, true, &mut ResourceBudget::new())
        }
        fn input(&self) -> PathGrant {
            PathGrant {
                path: self.root.clone(),
                directory: true,
            }
        }
        fn capture(&self) -> BoundManagedResource {
            BoundManagedResource::capture(&self.input(), &mut ResourceBudget::new()).unwrap()
        }
    }
    fn binding(resource: &BoundManagedResource, input: &PathGrant) -> Result<()> {
        resource.validate_binding_with_budget(input, &mut ResourceBudget::new())
    }

    #[test]
    fn stable_manifest_orders_real_names_after_synthetic_root() {
        let left = Tree::new();
        let right = Tree::new();
        for name in ["z", "!file", "-file", ".hidden"] {
            fs::write(left.root.join(name), name.as_bytes()).unwrap();
        }
        for name in [".hidden", "-file", "!file", "z"] {
            fs::write(right.root.join(name), name.as_bytes()).unwrap();
        }
        assert_eq!(left.digest().unwrap(), right.digest().unwrap());
        let captured = left.capture();
        assert_eq!(
            captured
                .entries
                .iter()
                .map(|e| e.path.as_str())
                .collect::<Vec<_>>(),
            [".", "!file", "-file", ".hidden", "z"]
        );
        assert_eq!(
            captured.read_grants(),
            vec![PathGrant {
                path: left.root.clone(),
                directory: true
            }]
        );
    }

    #[test]
    fn original_resource_versions_refuse_changed_inventory_and_replaced_inputs() {
        for case in 0..6 {
            let tree = Tree::new();
            fs::create_dir(tree.root.join("nested")).unwrap();
            let source = tree.root.join("nested/source");
            fs::write(&source, b"original").unwrap();
            let captured = tree.capture();
            captured
                .revalidate_versions(&mut ResourceBudget::new())
                .unwrap();
            match case {
                0 => fs::write(&source, b"changed contents and length").unwrap(),
                1 => fs::write(tree.root.join("nested/new"), b"new input").unwrap(),
                2 => fs::remove_file(&source).unwrap(),
                3 => {
                    fs::rename(&source, tree.root.join("original-inode")).unwrap();
                    fs::write(&source, b"original").unwrap();
                }
                4 => {
                    fs::rename(
                        tree.root.join("nested"),
                        tree.root.join("original-directory"),
                    )
                    .unwrap();
                    symlink("original-directory", tree.root.join("nested")).unwrap();
                }
                _ => {
                    let moved = tree.root.with_file_name("original-root");
                    fs::rename(&tree.root, &moved).unwrap();
                    symlink(moved, &tree.root).unwrap();
                }
            }
            // The retained descriptive inventory remains valid; current
            // filesystem versions must nevertheless refuse before dispatch.
            binding(&captured, &captured.input).unwrap();
            assert!(
                captured
                    .revalidate_versions(&mut ResourceBudget::new())
                    .is_err(),
                "case {case}"
            );
        }
    }

    #[test]
    fn resource_version_preflight_keeps_original_file_identity_and_deadline() {
        let tree = Tree::new();
        let path = tree.root.join("input");
        fs::write(&path, b"original").unwrap();
        let grant = PathGrant {
            path: path.clone(),
            directory: false,
        };
        let captured = BoundManagedResource::capture(&grant, &mut ResourceBudget::new()).unwrap();
        captured
            .revalidate_versions(&mut ResourceBudget::new())
            .unwrap();
        assert_eq!(
            captured
                .revalidate_versions(&mut ResourceBudget::until(Instant::now()))
                .unwrap_err()
                .code,
            "managed_resource_limit"
        );
        fs::rename(&path, tree.root.join("old-input")).unwrap();
        fs::write(path, b"original").unwrap();
        assert_eq!(
            captured
                .revalidate_versions(&mut ResourceBudget::new())
                .unwrap_err()
                .code,
            "identity_mismatch"
        );
    }

    #[test]
    fn original_file_membership_refuses_other_bytes_paths_and_links() {
        let tree = Tree::new();
        fs::write(tree.root.join("source"), b"observed").unwrap();
        symlink("source", tree.root.join("alias")).unwrap();
        let resource = tree.capture();
        let digest = crate::digest::bytes_digest(b"observed");
        resource
            .verify_file(&tree.root.join("source"), &digest, 8)
            .unwrap();
        for path in [
            tree.root.clone(),
            tree.root.join("alias"),
            tree.root.join("missing"),
            tree.root.join("../source"),
        ] {
            assert!(resource.verify_file(&path, &digest, 8).is_err());
        }
        assert!(
            resource
                .verify_file(&tree.root.join("source"), &digest, 7)
                .is_err()
        );
        assert!(
            resource
                .verify_file(
                    &tree.root.join("source"),
                    &crate::digest::bytes_digest(b"changed"),
                    8
                )
                .is_err()
        );
        fs::write(tree.root.join("source"), b"changed!").unwrap();
        // Original membership is deliberately not a fresh filesystem proof.
        resource
            .verify_file(&tree.root.join("source"), &digest, 8)
            .unwrap();
        assert!(resource.revalidate(&mut ResourceBudget::new()).is_err());
    }

    #[test]
    fn full_content_revalidation_detects_changed_new_and_deleted_files() {
        for mutation in 0..3 {
            let tree = Tree::new();
            fs::write(tree.root.join("a"), b"original").unwrap();
            let resource = tree.capture();
            resource.revalidate(&mut ResourceBudget::new()).unwrap();
            match mutation {
                0 => fs::write(tree.root.join("a"), b"replaced").unwrap(),
                1 => fs::write(tree.root.join("new"), b"new member").unwrap(),
                _ => fs::remove_file(tree.root.join("a")).unwrap(),
            }
            assert_ne!(tree.digest().unwrap(), resource.digest);
            assert_eq!(
                resource
                    .revalidate(&mut ResourceBudget::new())
                    .unwrap_err()
                    .code,
                "identity_mismatch"
            );
            // Binding validation is deliberately filesystem-free. It remains
            // valid as a retained declaration, but cannot replace full recapture.
            binding(&resource, &resource.input).unwrap();
        }
    }

    #[test]
    fn identical_byte_substitution_preserves_content_but_not_original_identity() {
        let tree = Tree::new();
        fs::write(tree.root.join("a"), b"original").unwrap();
        let resource = tree.capture();
        let replacement = tree.root.parent().unwrap().join("replacement");
        fs::write(&replacement, b"original").unwrap();
        fs::rename(&replacement, tree.root.join("a")).unwrap();
        assert_eq!(tree.digest().unwrap(), resource.digest);
        assert_eq!(
            resource
                .revalidate(&mut ResourceBudget::new())
                .unwrap_err()
                .code,
            "identity_mismatch"
        );
    }

    #[test]
    fn link_observations_are_bounded_and_do_not_duplicate_target_bytes() {
        let tree = Tree::new();
        fs::create_dir(tree.root.join("dir")).unwrap();
        fs::write(tree.root.join("dir/file"), b"payload").unwrap();
        symlink("dir", tree.root.join("alias")).unwrap();
        symlink("alias/../alias/file", tree.root.join("repeated")).unwrap();
        symlink(tree.root.join("dir/file"), tree.root.join("absolute")).unwrap();
        let input = tree.input();
        let mut budget = ResourceBudget::new();
        let resource = BoundManagedResource::capture(&input, &mut budget).unwrap();
        assert_eq!(MAX_BYTES - budget.bytes, 7);
        assert_eq!(
            resource
                .entries
                .iter()
                .filter(|e| matches!(e.kind, EntryKind::File { .. }))
                .count(),
            1
        );
        resource.revalidate(&mut ResourceBudget::new()).unwrap();
    }

    #[test]
    fn external_dangling_and_cyclic_links_record_text_without_following_targets() {
        // Closed-link and per-file-map catalog requirements are superseded by
        // SPEC 1.4. Preserve no-follow behavior; sandbox/domain tests separately
        // enforce where an actual command may resolve and access a target.
        for case in 0..5 {
            let tree = Tree::new();
            fs::create_dir(tree.root.join("dir")).unwrap();
            fs::write(tree.root.join("dir/file"), b"a").unwrap();
            match case {
                0 => {
                    let outside = tree.root.parent().unwrap().join("outside");
                    fs::write(&outside, b"unobserved outside bytes").unwrap();
                    symlink(&outside, tree.root.join("link")).unwrap();
                }
                1 => symlink("missing", tree.root.join("link")).unwrap(),
                2 => symlink("link", tree.root.join("link")).unwrap(),
                3 => {
                    symlink("other", tree.root.join("link")).unwrap();
                    symlink("link", tree.root.join("other")).unwrap();
                }
                _ => symlink("..", tree.root.join("dir/ancestor")).unwrap(),
            }
            let mut budget = ResourceBudget::new();
            let observed = BoundManagedResource::capture(&tree.input(), &mut budget).unwrap();
            assert_eq!(MAX_BYTES - budget.bytes, 1);
            binding(&observed, &tree.input()).unwrap();
            observed.revalidate(&mut ResourceBudget::new()).unwrap();
            if case == 0 {
                fs::write(
                    tree.root.parent().unwrap().join("outside"),
                    b"changed but not observed",
                )
                .unwrap();
                observed.revalidate(&mut ResourceBudget::new()).unwrap();
                fs::remove_file(tree.root.join("link")).unwrap();
                symlink("different-target", tree.root.join("link")).unwrap();
                assert_eq!(
                    observed
                        .revalidate(&mut ResourceBudget::new())
                        .unwrap_err()
                        .code,
                    "identity_mismatch"
                );
            }
        }
    }

    #[test]
    fn observation_derives_identity_without_expected_digests_or_executable_mappings() {
        let tree = Tree::new();
        let file = tree.root.join("unfamiliar-program");
        fs::write(&file, b"arbitrary bytes, never executed").unwrap();
        let grant = PathGrant {
            path: file.clone(),
            directory: false,
        };
        let observed = BoundManagedResource::capture(&grant, &mut ResourceBudget::new()).unwrap();
        assert_eq!(observed.read_grants(), vec![grant.clone()]);
        assert!(observed.digest.starts_with("sha256:v2:"));
        observed.revalidate(&mut ResourceBudget::new()).unwrap();
        fs::write(&file, b"new environment identity").unwrap();
        let fresh = BoundManagedResource::capture(&grant, &mut ResourceBudget::new()).unwrap();
        assert_ne!(observed.digest, fresh.digest);
        assert_eq!(
            observed
                .revalidate(&mut ResourceBudget::new())
                .unwrap_err()
                .code,
            "identity_mismatch"
        );
        let mut legacy = serde_json::to_value(&observed).unwrap();
        legacy["mappings"] = serde_json::json!([file]);
        assert!(serde_json::from_value::<BoundManagedResource>(legacy).is_err());
    }

    #[test]
    fn canonical_root_and_regular_file_requirements_refuse_alias_and_fifo() {
        let tree = Tree::new();
        fs::write(tree.root.join("file"), b"x").unwrap();
        let alias = tree.root.parent().unwrap().join("alias");
        symlink(&tree.root, &alias).unwrap();
        assert_eq!(
            content_digest(&alias, true, &mut ResourceBudget::new())
                .unwrap_err()
                .code,
            "managed_resource_invalid"
        );
        let noncanonical = PathBuf::from(format!("{}/./", tree.root.display()));
        assert!(content_digest(&noncanonical, true, &mut ResourceBudget::new()).is_err());
        let fifo = CString::new(tree.root.join("fifo").to_str().unwrap()).unwrap();
        // Ordinary owned-temp filesystem fixture: special files are rejected
        // from metadata without opening them or waiting for a writer.
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        assert_eq!(tree.digest().unwrap_err().code, "managed_resource_invalid");
    }

    #[test]
    fn capture_limits_entries_bytes_file_size_text_and_elapsed_work() {
        let tree = Tree::new();
        fs::write(tree.root.join("file"), b"four").unwrap();
        for case in 0..5 {
            let mut budget = ResourceBudget::new();
            match case {
                0 => budget.entries = 1,
                1 => budget.bytes = 3,
                2 => budget.file_bytes = 3,
                3 => budget.text = 2,
                _ => budget.deadline = Instant::now(),
            }
            assert_eq!(
                content_digest(&tree.root, true, &mut budget)
                    .unwrap_err()
                    .code,
                "managed_resource_limit"
            );
        }
    }

    #[test]
    fn shared_capture_and_retained_binding_cannot_reset_policy_budgets() {
        let left = Tree::new();
        let right = Tree::new();
        fs::write(left.root.join("file"), b"four").unwrap();
        fs::write(right.root.join("file"), b"four").unwrap();
        let first = left.capture();
        let second = right.capture();
        for structural in [false, true] {
            for resource in ["entries", "bytes", "text", "deadline"] {
                let mut budget = ResourceBudget::new();
                match resource {
                    "entries" => budget.entries = 3,
                    "bytes" => budget.bytes = 7,
                    "text" => {
                        let mut one = ResourceBudget::new();
                        binding_charge(&first, &mut one).unwrap();
                        budget.text = MAX_TEXT_BYTES - one.text;
                    }
                    _ => (),
                }
                if structural {
                    binding_charge(&first, &mut budget).unwrap();
                } else {
                    BoundManagedResource::capture(&first.input, &mut budget).unwrap();
                }
                if resource == "deadline" {
                    budget.deadline = Instant::now();
                }
                let result = if structural {
                    binding_charge(&second, &mut budget)
                } else {
                    BoundManagedResource::capture(&second.input, &mut budget).map(|_| ())
                };
                assert_eq!(result.unwrap_err().code, "managed_resource_limit");
            }
        }
    }
    fn binding_charge(resource: &BoundManagedResource, budget: &mut ResourceBudget) -> Result<()> {
        resource.validate_binding_with_budget(&resource.input, budget)
    }

    #[test]
    fn serialized_snapshot_roundtrip_and_forged_grant_refusal() {
        let tree = Tree::new();
        fs::create_dir(tree.root.join("dir")).unwrap();
        fs::write(tree.root.join("dir/file"), b"map").unwrap();
        symlink("dir/file", tree.root.join("alias")).unwrap();
        let original = tree.capture();
        let encoded = serde_json::to_value(&original).unwrap();
        let decoded: BoundManagedResource = serde_json::from_value(encoded.clone()).unwrap();
        assert_eq!(decoded, original);
        binding(&decoded, &original.input).unwrap();
        decoded.revalidate(&mut ResourceBudget::new()).unwrap();
        for mutation in 0..4 {
            let mut forged = encoded.clone();
            match mutation {
                0 => forged["schema_version"] = serde_json::json!(1),
                1 => forged["root"] = serde_json::json!("/unlisted"),
                2 => {
                    forged["entries"][3]["kind"]["digest"] =
                        serde_json::json!(format!("sha256:{}", "0".repeat(64)))
                }
                _ => forged["input"]["directory"] = serde_json::json!(false),
            }
            let forged: BoundManagedResource = serde_json::from_value(forged).unwrap();
            assert!(binding(&forged, &original.input).is_err());
        }
        let mut unknown = encoded;
        unknown["entries"][0]["unexpected"] = serde_json::json!(true);
        assert!(serde_json::from_value::<BoundManagedResource>(unknown).is_err());

        let mut duplicate = serde_json::to_value(&original).unwrap();
        let root_entry = duplicate["entries"][0].clone();
        let root_version = duplicate["versions"][0].clone();
        duplicate["entries"]
            .as_array_mut()
            .unwrap()
            .insert(1, root_entry);
        duplicate["versions"]
            .as_array_mut()
            .unwrap()
            .insert(1, root_version);
        let duplicate: BoundManagedResource = serde_json::from_value(duplicate).unwrap();
        assert_eq!(
            binding(&duplicate, &original.input).unwrap_err().code,
            "managed_resource_invalid"
        );
    }
}
