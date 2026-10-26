//! Same-creation identity for optional immutable diagnostics. No authority is
//! reconstructed from these private anchors or from serialized file metadata.
use super::*;
use std::sync::Mutex;

/// Captured from the actual O_EXCL descriptor before any fallible write. The
/// sealed version is installed only from that descriptor after successful sync.
/// It retains metadata, not one persistent descriptor per diagnostic chunk.
pub(crate) struct RetainedPrivateFile {
    leaf: String,
    directory: (u64, u64),
    original: (u64, u64),
    sealed: Mutex<Option<std::fs::Metadata>>,
}

fn identity(metadata: &std::fs::Metadata) -> (u64, u64) {
    (metadata.dev(), metadata.ino())
}
fn same_version(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    identity(a) == identity(b)
        && a.mode() == b.mode()
        && a.uid() == b.uid()
        && a.gid() == b.gid()
        && a.nlink() == b.nlink()
        && a.len() == b.len()
        && a.mtime() == b.mtime()
        && a.mtime_nsec() == b.mtime_nsec()
        && a.ctime() == b.ctime()
        && a.ctime_nsec() == b.ctime_nsec()
}
fn changed() -> Error {
    Error::new(
        "ownership_conflict",
        "original immutable file identity or version changed",
    )
}
impl RetainedPrivateFile {
    fn verify_directory(&self, directory: &PrivateDir) -> Result<()> {
        directory.revalidate()?;
        if identity(
            &directory
                .dir
                .metadata()
                .map_err(|e| Error::io("inspect original record directory", e))?,
        ) != self.directory
        {
            return Err(changed());
        }
        Ok(())
    }
    fn verify_named(&self, directory: &PrivateDir, version: &std::fs::Metadata) -> Result<()> {
        self.verify_directory(directory)?;
        let current = directory.open_file(&self.leaf, libc::O_RDONLY)?;
        let actual = current
            .metadata()
            .map_err(|e| Error::io("inspect original immutable leaf", e))?;
        if identity(&actual) != self.original || !same_version(&actual, version) {
            return Err(changed());
        }
        Ok(())
    }
}
impl PrivateDir {
    /// Private diagnostic variant of exclusive creation. The caller receives
    /// the actual creation identity before write/sync can fail, and must retain
    /// it with its original expected bytes. Existing public APIs are unchanged.
    pub(crate) fn create_bytes_exclusive_retained(
        &self,
        leaf: &str,
        bytes: &[u8],
        retain: impl FnOnce(Arc<RetainedPrivateFile>) -> Result<()>,
    ) -> Result<()> {
        if bytes.len() > 8 * 1024 * 1024 {
            return Err(Error::invalid("private file exceeds 8 MiB"));
        }
        let mut file = self.open_file(leaf, libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL)?;
        let created = file
            .metadata()
            .map_err(|e| Error::io("capture created immutable identity", e))?;
        let directory = self
            .dir
            .metadata()
            .map_err(|e| Error::io("capture immutable directory identity", e))?;
        let anchor = Arc::new(RetainedPrivateFile {
            leaf: leaf.to_owned(),
            directory: identity(&directory),
            original: identity(&created),
            sealed: Mutex::new(None),
        });
        retain(anchor.clone())?;
        file.write_all(bytes)
            .map_err(|e| Error::io("write retained immutable record", e))?;
        file.sync_all()
            .map_err(|e| Error::io("sync retained immutable record", e))?;
        check_owner(&file, false)?;
        let version = file
            .metadata()
            .map_err(|e| Error::io("seal written immutable identity", e))?;
        if identity(&version) != anchor.original || version.len() != bytes.len() as u64 {
            return Err(changed());
        }
        anchor.verify_named(self, &version)?;
        self.dir
            .sync_all()
            .map_err(|e| Error::io("sync retained immutable directory", e))?;
        // Recheck after durability; no first-read adoption or completion on error.
        let after = file
            .metadata()
            .map_err(|e| Error::io("recheck written immutable identity", e))?;
        if !same_version(&version, &after) {
            return Err(changed());
        }
        anchor.verify_named(self, &version)?;
        *anchor.sealed.lock().map_err(|_| {
            Error::new("ownership_conflict", "immutable identity lock unavailable")
        })? = Some(version);
        Ok(())
    }

    /// A single current descriptor supplies the bytes and stable-version checks;
    /// the original creation identity is never learned or replaced by this read.
    pub(crate) fn read_retained_bytes(
        &self,
        anchor: &RetainedPrivateFile,
        maximum: u64,
    ) -> Result<Vec<u8>> {
        if maximum == 0 || maximum > 8 * 1024 * 1024 {
            return Err(Error::invalid("invalid private file size bound"));
        }
        let version = anchor
            .sealed
            .lock()
            .map_err(|_| Error::new("ownership_conflict", "immutable identity lock unavailable"))?
            .clone()
            .ok_or_else(|| {
                Error::new(
                    "record_incomplete",
                    "original immutable write did not complete",
                )
            })?;
        anchor.verify_directory(self)?;
        let file = self.open_file(&anchor.leaf, libc::O_RDONLY)?;
        let before = file
            .metadata()
            .map_err(|e| Error::io("inspect retained immutable size", e))?;
        if identity(&before) != anchor.original || !same_version(&before, &version) {
            return Err(changed());
        }
        if before.len() > maximum {
            return Err(Error::new(
                "record_corrupt",
                "private file exceeds its size bound",
            ));
        }
        let mut bytes = Vec::new();
        (&file)
            .take(maximum + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| Error::io("read retained immutable record", e))?;
        if bytes.len() as u64 > maximum {
            return Err(Error::new(
                "record_corrupt",
                "private file grew beyond its size bound",
            ));
        }
        check_owner(&file, false)?;
        let after = file
            .metadata()
            .map_err(|e| Error::io("reinspect retained immutable record", e))?;
        if !same_version(&before, &after) || bytes.len() as u64 != version.len() {
            return Err(changed());
        }
        anchor.verify_named(self, &version)?;
        Ok(bytes)
    }
}

#[cfg(test)]
#[path = "storage_retained_tests.rs"]
mod tests;
