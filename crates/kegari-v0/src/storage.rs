//! Owner-only, descriptor-relative control storage. No symlink-following writes.
use crate::{
    digest::canonical_digest,
    error::{Error, Result},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    ffi::CString,
    fs::File,
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::{Component, Path, PathBuf},
    sync::Arc,
};
use uuid::Uuid;

const MAX_RECORD: u64 = 256 * 1024;

#[derive(Debug)]
pub struct PrivateDir {
    path: PathBuf,
    dir: File,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteBoundary {
    Created,
    Written,
    FileSynced,
    Renamed,
    DirectorySynced,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record<T> {
    schema_version: u32,
    digest: String,
    payload: T,
}

fn name(value: &str) -> Result<CString> {
    if value.is_empty() || value == "." || value == ".." || value.contains('/') {
        return Err(Error::invalid(
            "storage names must be single nonempty path components",
        ));
    }
    CString::new(value).map_err(|_| Error::invalid("NUL in storage name"))
}

fn check_owner(file: &File, directory: bool) -> Result<()> {
    let m = file
        .metadata()
        .map_err(|e| Error::io("read control file metadata", e))?;
    // SAFETY: geteuid has no arguments or memory preconditions.
    if m.uid() != unsafe { libc::geteuid() }
        || m.mode() & 0o077 != 0
        || m.is_dir() != directory
        || (!directory && (!m.is_file() || m.nlink() != 1))
    {
        return Err(Error::new(
            "unsafe_storage",
            "control data must be owner-only, owned by this OS user, and not linked",
        ));
    }
    Ok(())
}

fn open_directory(path: &Path) -> Result<File> {
    if !path.is_absolute() {
        return Err(Error::invalid("control path must be absolute"));
    }
    let mut current = File::open("/").map_err(|e| Error::io("open filesystem root", e))?;
    for component in path.components() {
        match component {
            Component::RootDir => (),
            Component::Normal(part) => {
                let part = CString::new(part.as_bytes())
                    .map_err(|_| Error::invalid("NUL in control path"))?;
                // SAFETY: live parent descriptor and NUL-terminated component; returned fd owns a new reference.
                let fd = unsafe {
                    libc::openat(
                        current.as_raw_fd(),
                        part.as_ptr(),
                        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                    )
                };
                if fd < 0 {
                    return Err(Error::io(
                        "open control directory without symlinks",
                        std::io::Error::last_os_error(),
                    ));
                }
                // SAFETY: successful openat returned a newly owned descriptor.
                current = unsafe { File::from_raw_fd(fd) };
            }
            _ => return Err(Error::invalid("control path contains traversal")),
        }
    }
    Ok(current)
}

impl PrivateDir {
    pub fn create(path: &Path) -> Result<Self> {
        let parent = path
            .parent()
            .ok_or_else(|| Error::invalid("control root needs a parent"))?;
        let leaf = path
            .file_name()
            .and_then(|p| p.to_str())
            .ok_or_else(|| Error::invalid("invalid control root"))?;
        let parent_fd = open_directory(parent)?;
        let leaf = name(leaf)?;
        // SAFETY: parent_fd and leaf remain live for this syscall.
        let r = unsafe { libc::mkdirat(parent_fd.as_raw_fd(), leaf.as_ptr(), 0o700) };
        if r < 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists {
            return Err(Error::io(
                "create private directory",
                std::io::Error::last_os_error(),
            ));
        }
        parent_fd
            .sync_all()
            .map_err(|e| Error::io("flush new directory entry", e))?;
        Self::open(path)
    }

    pub fn open(path: &Path) -> Result<Self> {
        let dir = open_directory(path)?;
        check_owner(&dir, true)?;
        Ok(Self {
            path: path.to_path_buf(),
            dir,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn revalidate(&self) -> Result<()> {
        let current = open_directory(&self.path)?;
        check_owner(&current, true)?;
        let a = self
            .dir
            .metadata()
            .map_err(|e| Error::io("read owned directory", e))?;
        let b = current
            .metadata()
            .map_err(|e| Error::io("read current directory", e))?;
        if (a.dev(), a.ino()) != (b.dev(), b.ino()) {
            return Err(Error::new(
                "ownership_conflict",
                "control directory was replaced",
            ));
        }
        Ok(())
    }

    pub fn child(&self, leaf: &str) -> Result<Self> {
        self.revalidate()?;
        name(leaf)?;
        Self::create(&self.path.join(leaf))
    }

    fn open_file(&self, leaf: &str, flags: i32) -> Result<File> {
        self.revalidate()?;
        let leaf = name(leaf)?;
        // SAFETY: a checked relative filename and retained directory fd; never follow symlinks.
        let fd = unsafe {
            libc::openat(
                self.dir.as_raw_fd(),
                leaf.as_ptr(),
                flags | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
                0o600,
            )
        };
        if fd < 0 {
            let error = std::io::Error::last_os_error();
            return Err(if error.kind() == std::io::ErrorKind::NotFound {
                Error::new("record_missing", "required control record is missing")
            } else {
                Error::io("open private record", error)
            });
        }
        // SAFETY: fd is newly owned after successful openat.
        let file = unsafe { File::from_raw_fd(fd) };
        check_owner(&file, false)?;
        Ok(file)
    }

    pub fn read<T: DeserializeOwned + Serialize>(&self, leaf: &str) -> Result<T> {
        let bytes = self.read_bytes(leaf, MAX_RECORD)?;
        let record: Record<T> = serde_json::from_slice(&bytes).map_err(|_| {
            Error::new(
                "record_corrupt",
                "control record is invalid JSON or has unknown fields",
            )
        })?;
        if record.schema_version != 1 {
            return Err(Error::new(
                "incompatible_version",
                "unsupported control record version; preserved unchanged",
            ));
        }
        if canonical_digest(1, &record.payload)? != record.digest {
            return Err(Error::new(
                "record_corrupt",
                "control record digest does not match",
            ));
        }
        Ok(record.payload)
    }

    /// Non-secret configuration/client preferences use their native TOML format.
    /// Control records continue to use the versioned, digested JSON methods.
    pub fn read_bytes(&self, leaf: &str, maximum: u64) -> Result<Vec<u8>> {
        if maximum == 0 || maximum > 8 * 1024 * 1024 {
            return Err(Error::invalid("invalid private file size bound"));
        }
        let file = self.open_file(leaf, libc::O_RDONLY)?;
        if file
            .metadata()
            .map_err(|e| Error::io("read record size", e))?
            .len()
            > maximum
        {
            return Err(Error::new(
                "record_corrupt",
                "private file exceeds its size bound",
            ));
        }
        let mut bytes = Vec::new();
        file.take(maximum + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| Error::io("read control record", e))?;
        if bytes.len() as u64 > maximum {
            return Err(Error::new(
                "record_corrupt",
                "private file grew beyond its size bound",
            ));
        }
        Ok(bytes)
    }

    pub fn write<T: Serialize>(&self, leaf: &str, value: &T) -> Result<()> {
        self.write_observed(leaf, value, |_| Ok(()))
    }

    /// Fault hooks support deterministic crash-boundary tests. A failure never implies
    /// the old value survived: after rename, callers must close admission and reconcile.
    pub fn write_observed<T: Serialize>(
        &self,
        leaf: &str,
        value: &T,
        boundary: impl FnMut(WriteBoundary) -> Result<()>,
    ) -> Result<()> {
        let record = Record {
            schema_version: 1,
            digest: canonical_digest(1, value)?,
            payload: value,
        };
        let bytes = serde_json::to_vec(&record)
            .map_err(|_| Error::invalid("control record encoding failed"))?;
        if bytes.len() as u64 > MAX_RECORD {
            return Err(Error::invalid(
                "receipt exceeds 256 KiB; store artifact references instead",
            ));
        }
        self.write_bytes_observed(leaf, &bytes, boundary)
    }

    pub fn write_bytes(&self, leaf: &str, bytes: &[u8]) -> Result<()> {
        self.write_bytes_observed(leaf, bytes, |_| Ok(()))
    }

    pub fn write_bytes_observed(
        &self,
        leaf: &str,
        bytes: &[u8],
        mut boundary: impl FnMut(WriteBoundary) -> Result<()>,
    ) -> Result<()> {
        if bytes.len() > 8 * 1024 * 1024 {
            return Err(Error::invalid("private file exceeds 8 MiB"));
        }
        self.revalidate()?;
        let destination = name(leaf)?;
        // Refuse replacing even an owner-created symlink or hard-linked control record.
        match self.open_file(leaf, libc::O_RDONLY) {
            Ok(_) => (),
            Err(e) if e.code == "record_missing" => (),
            Err(e) => return Err(e),
        }
        let temporary = format!(".{}.{}.tmp", leaf, Uuid::new_v4());
        let mut file = self.open_file(&temporary, libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL)?;
        boundary(WriteBoundary::Created)?;
        file.write_all(bytes)
            .map_err(|e| Error::io("write control record", e))?;
        boundary(WriteBoundary::Written)?;
        file.sync_all()
            .map_err(|e| Error::io("flush control record", e))?;
        boundary(WriteBoundary::FileSynced)?;
        self.revalidate()?;
        let temporary = name(&temporary)?;
        // SAFETY: both single-component names are anchored to the same retained fd.
        if unsafe {
            libc::renameat(
                self.dir.as_raw_fd(),
                temporary.as_ptr(),
                self.dir.as_raw_fd(),
                destination.as_ptr(),
            )
        } < 0
        {
            return Err(Error::io(
                "atomically replace control record",
                std::io::Error::last_os_error(),
            ));
        }
        boundary(WriteBoundary::Renamed)?;
        self.dir
            .sync_all()
            .map_err(|e| Error::io("flush control record directory", e))?;
        boundary(WriteBoundary::DirectorySynced)
    }

    pub fn records(&self) -> Result<Vec<String>> {
        self.revalidate()?;
        let mut records = Vec::new();
        for entry in std::fs::read_dir(&self.path)
            .map_err(|e| Error::io("enumerate recovery receipts", e))?
        {
            let entry = entry.map_err(|e| Error::io("read recovery entry", e))?;
            let leaf = entry
                .file_name()
                .into_string()
                .map_err(|_| Error::new("record_corrupt", "non-UTF8 recovery record name"))?;
            if leaf.ends_with(".json") {
                self.open_file(&leaf, libc::O_RDONLY)?;
                records.push(leaf);
            }
        }
        records.sort();
        Ok(records)
    }

    pub fn lock(&self, leaf: &str) -> Result<ExclusiveLock> {
        let file = self.open_file(leaf, libc::O_RDWR | libc::O_CREAT)?;
        fs2::FileExt::try_lock_exclusive(&file).map_err(|_| {
            Error::new(
                "ownership_conflict",
                "another process holds this runtime lock",
            )
        })?;
        self.dir
            .sync_all()
            .map_err(|e| Error::io("flush lock directory", e))?;
        Ok(ExclusiveLock {
            _file: Arc::new(file),
        })
    }
}

/// The live descriptor is the lock, not its filename/PID metadata.
#[derive(Clone)]
pub struct ExclusiveLock {
    _file: Arc<File>,
}
impl ExclusiveLock {
    /// Duplicate the actual locked open-file description for an owned child.
    /// The child spawn hook, never an ambient setting, clears close-on-exec.
    pub(crate) fn inherited_file(&self) -> Result<File> {
        self._file
            .try_clone()
            .map_err(|error| Error::io("duplicate owned process guard", error))
    }
}
