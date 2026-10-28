//! OS identity, protected executable inspection and native host helpers.
//! Application registration and service policy remain with callers.
use kegari_digest::bytes_digest;
use kegari_types::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::{
    ffi::{CStr, CString},
    fs::{File, OpenOptions},
    io::Read,
    os::unix::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
        process::CommandExt,
    },
    path::{Component, Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct HostIdentity {
    pub host_id: String,
    pub boot_id: Uuid,
    pub uid: u32,
    pub user: String,
    pub home: PathBuf,
}
#[derive(Debug, Clone)]
pub struct AccountHome {
    pub uid: u32,
    pub user: String,
    pub home: PathBuf,
}
/// Client preferences need the OS account home, not host/boot ownership probes.
/// A remote-only client must not require a local coordinator or trust `$HOME`.
pub fn account_home() -> Result<AccountHome> {
    // SAFETY: geteuid is scalar; getpwuid_r writes only the supplied buffers.
    let uid = unsafe { libc::geteuid() };
    let mut passwd = std::mem::MaybeUninit::<libc::passwd>::uninit();
    let mut buffer = vec![0u8; 65_536];
    let mut result = std::ptr::null_mut();
    if unsafe {
        libc::getpwuid_r(
            uid,
            passwd.as_mut_ptr(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut result,
        )
    } != 0
        || result.is_null()
    {
        return Err(Error::new("prerequisite", "OS account home is unavailable"));
    }
    // SAFETY: successful getpwuid_r populated passwd; strings live in buffer.
    let passwd = unsafe { passwd.assume_init() };
    let user = unsafe { CStr::from_ptr(passwd.pw_name) }
        .to_str()
        .map_err(|_| Error::invalid("OS username is not UTF-8"))?
        .to_owned();
    let home = PathBuf::from(
        unsafe { CStr::from_ptr(passwd.pw_dir) }
            .to_str()
            .map_err(|_| Error::invalid("OS home is not UTF-8"))?,
    );
    if !home.is_absolute() {
        return Err(Error::new(
            "prerequisite",
            "OS account home is not absolute",
        ));
    }
    Ok(AccountHome { uid, user, home })
}

/// Launch the invoking device's browser through the shared spawn boundary.
/// The CLI validates the destination before calling this OS-only adapter.
pub async fn launch_client_browser(url: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    let mut command = tokio::process::Command::new("/usr/bin/open");
    #[cfg(not(target_os = "macos"))]
    let mut command = tokio::process::Command::new("xdg-open");
    command
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let cutoff = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let result = tokio::time::timeout_at(tokio::time::Instant::from_std(cutoff), async {
        let mut child = kegari_process::spawn::spawn_tokio(&mut command, Some(cutoff))?;
        child.wait().await
    })
    .await
    .map_err(|_| Error::new("timeout", "client browser launch timed out"))?
    .map_err(|error| {
        if error.kind() == std::io::ErrorKind::TimedOut {
            Error::new("timeout", "client browser launch timed out")
        } else {
            Error::new(
                "target_unavailable",
                "client browser launcher is unavailable",
            )
        }
    })?;
    if !result.success() {
        return Err(Error::new(
            "target_unavailable",
            "client browser launch failed",
        ));
    }
    Ok(())
}
impl HostIdentity {
    pub fn discover() -> Result<Self> {
        native::host()
    }
    /// Production callers use the OS account's home, never a caller-supplied
    /// HOME/--root override, so a second registration is not an ownership escape.
    pub fn state_root(&self) -> PathBuf {
        self.home.join(".kegari")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BinaryIdentity {
    pub path: PathBuf,
    pub digest: String,
    pub bytes: u64,
}
fn checked_file(path: &Path, immutable: bool) -> Result<File> {
    if !path.is_absolute()
        || path
            .components()
            .any(|p| matches!(p, Component::ParentDir | Component::CurDir))
    {
        return Err(Error::invalid("binary must be an absolute normalized path"));
    }
    let mut ancestor = PathBuf::new();
    for part in path.components() {
        ancestor.push(part.as_os_str());
        let metadata = std::fs::symlink_metadata(&ancestor)
            .map_err(|e| Error::io("inspect executable path", e))?;
        if metadata.file_type().is_symlink() || metadata.mode() & 0o022 != 0 {
            // Root-owned sticky temporary directories are allowed for test
            // artifacts; the executable and its immediate parent stay protected.
            let protected_temporary = metadata.is_dir()
                && metadata.uid() == 0
                && metadata.mode() & 0o1000 != 0
                && ancestor != path
                && Some(ancestor.as_path()) != path.parent();
            if !protected_temporary {
                return Err(Error::new(
                    "unsafe_storage",
                    "executable path is linked or writable by another OS user",
                ));
            }
        }
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| Error::io("open executable", e))?;
    let metadata = file
        .metadata()
        .map_err(|e| Error::io("read executable metadata", e))?;
    // SAFETY: geteuid has no memory preconditions.
    let uid = unsafe { libc::geteuid() };
    if !metadata.is_file()
        || metadata.mode() & 0o111 == 0
        || (metadata.uid() != uid && metadata.uid() != 0)
        || metadata.mode() & 0o022 != 0
        || (immutable && metadata.mode() & 0o222 != 0)
    {
        return Err(Error::new(
            "unsafe_storage",
            "installed binary must be an owned executable with immutable contents",
        ));
    }
    Ok(file)
}
impl BinaryIdentity {
    pub fn inspect(path: &Path, immutable: bool) -> Result<Self> {
        let file = checked_file(path, immutable)?;
        let bytes = file
            .metadata()
            .map_err(|e| Error::io("read executable size", e))?
            .len();
        if bytes > 512 * 1024 * 1024 {
            return Err(Error::invalid("executable exceeds inspection bound"));
        }
        let mut data = Vec::new();
        file.take(512 * 1024 * 1024 + 1)
            .read_to_end(&mut data)
            .map_err(|e| Error::io("hash executable", e))?;
        if data.len() as u64 != bytes {
            return Err(Error::new(
                "ownership_conflict",
                "executable changed during inspection",
            ));
        }
        Ok(Self {
            path: path.to_owned(),
            digest: bytes_digest(&data),
            bytes,
        })
    }
    pub fn revalidate(&self, immutable: bool) -> Result<()> {
        if *self != Self::inspect(&self.path, immutable)? {
            return Err(Error::new(
                "identity_mismatch",
                "registered executable bytes changed",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub parent_pid: u32,
    pub process_group: u32,
    pub uid: u32,
    pub boot_id: Uuid,
    pub started_seconds: u64,
    pub started_microseconds: u64,
    pub executable: PathBuf,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessObservation {
    Running,
    Gone,
    DifferentIdentity,
}
impl ProcessIdentity {
    pub fn current() -> Result<Self> {
        native::process(std::process::id())?
            .ok_or_else(|| Error::new("ownership_conflict", "current process identity unavailable"))
    }
    pub fn observe(&self) -> Result<ProcessObservation> {
        Ok(match native::process(self.pid)? {
            None => ProcessObservation::Gone,
            Some(current) if current == *self => ProcessObservation::Running,
            Some(_) => ProcessObservation::DifferentIdentity,
        })
    }
}
pub trait ProcessProbe: Send + Sync {
    fn current(&self) -> Result<ProcessIdentity>;
    fn observe(&self, process: &ProcessIdentity) -> Result<ProcessObservation>;
    fn matching_executable(&self, path: &Path) -> Result<Vec<ProcessIdentity>>;
}
pub struct NativeProcessProbe;
impl ProcessProbe for NativeProcessProbe {
    fn current(&self) -> Result<ProcessIdentity> {
        ProcessIdentity::current()
    }
    fn observe(&self, process: &ProcessIdentity) -> Result<ProcessObservation> {
        process.observe()
    }
    fn matching_executable(&self, path: &Path) -> Result<Vec<ProcessIdentity>> {
        native::matching_executable(path)
    }
}

/// A locally spawned child is not reaped until this handle decides whether to
/// signal it; its PID therefore cannot be reused between check and signal.
/// Recovered orphan PIDs do not receive this capability.
pub struct OwnedChild {
    child: Child,
    identity: ProcessIdentity,
    reaped: Option<ExitStatus>,
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if self.reaped.is_some() {
            return;
        }
        if let Ok(Some(status)) = self.child.try_wait() {
            self.reaped = Some(status);
            return;
        }
        // This handle owns a direct child that has not been reaped. No retained
        // PID is adopted, and dropping an unsuccessful handshake cannot orphan
        // a scheduling process. Normal shutdown has already reaped it.
        if self.child.kill().is_ok() {
            self.reaped = self.child.wait().ok();
        } else if let Ok(status) = self.child.try_wait() {
            self.reaped = status;
        }
    }
}
impl OwnedChild {
    pub fn spawn(binary: &BinaryIdentity, args: &[String], home: &Path) -> Result<Self> {
        binary.revalidate(false)?;
        let mut command = Command::new(&binary.path);
        command
            .args(args)
            .env_clear()
            .env("HOME", home)
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // SAFETY: after fork this closure performs only the async-signal-safe
        // setsid syscall and constructs an OS error; it takes no parent locks.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
        let mut child = kegari_process::spawn::spawn_std(&mut command, None)
            .map_err(|e| Error::io("spawn registered orchestration", e))?;
        let identity = match native::process(child.id()) {
            Ok(Some(identity))
                if identity.executable == binary.path
                    && identity.parent_pid == std::process::id()
                    && identity.process_group == child.id() =>
            {
                identity
            }
            result => {
                // This is still our unreaped child. Kill just that child if its
                // startup identity cannot be proved; never adopt another PID.
                let _ = child.kill();
                let _ = child.wait();
                return Err(match result {
                    Err(error) => error,
                    _ => Error::new(
                        "ownership_conflict",
                        "spawned process identity did not match",
                    ),
                });
            }
        };
        Ok(Self {
            child,
            identity,
            reaped: None,
        })
    }
    pub fn identity(&self) -> &ProcessIdentity {
        &self.identity
    }
    pub fn take_io(
        &mut self,
    ) -> Result<(
        std::process::ChildStdin,
        std::process::ChildStdout,
        std::process::ChildStderr,
    )> {
        match (
            self.child.stdin.take(),
            self.child.stdout.take(),
            self.child.stderr.take(),
        ) {
            (Some(input), Some(output), Some(error)) => Ok((input, output, error)),
            _ => Err(Error::new(
                "ownership_conflict",
                "child protocol streams were already taken",
            )),
        }
    }
    pub fn try_wait(&mut self) -> Result<Option<ExitStatus>> {
        if self.reaped.is_none() {
            self.reaped = self
                .child
                .try_wait()
                .map_err(|e| Error::io("observe owned process", e))?;
        }
        Ok(self.reaped)
    }
    pub fn signal_group(&mut self, force: bool) -> Result<bool> {
        if self.try_wait()?.is_some() {
            return Ok(false);
        }
        if self.identity.observe()? != ProcessObservation::Running
            || self.identity.process_group != self.child.id()
        {
            return Err(Error::new(
                "ownership_conflict",
                "owned process group identity changed",
            ));
        }
        // SAFETY: this process group was created for our still-unreaped child;
        // no PID-only recovered process can enter this path.
        if unsafe {
            libc::kill(
                -(self.child.id() as i32),
                if force { libc::SIGKILL } else { libc::SIGTERM },
            )
        } < 0
        {
            return Err(Error::io(
                "signal owned process group",
                std::io::Error::last_os_error(),
            ));
        }
        Ok(true)
    }
}

pub fn free_bytes(path: &Path) -> Result<u64> {
    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| Error::invalid("NUL in storage path"))?;
    let mut value = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: path is NUL terminated and value is a valid writable statvfs.
    if unsafe { libc::statvfs(path.as_ptr(), value.as_mut_ptr()) } != 0 {
        return Err(Error::io(
            "inspect free storage",
            std::io::Error::last_os_error(),
        ));
    }
    // SAFETY: statvfs succeeded and initialized value.
    let value = unsafe { value.assume_init() };
    (value.f_bavail as u64)
        .checked_mul(value.f_frsize)
        .ok_or_else(|| Error::new("prerequisite", "free storage measurement overflow"))
}

#[cfg(target_os = "macos")]
mod native {
    use super::*;
    pub fn matching_executable(path: &Path) -> Result<Vec<ProcessIdentity>> {
        // A saturated/incomplete inventory never proves absence. No external
        // `ps` output, names or PID-only guesses are used for ownership.
        let uid = unsafe { libc::geteuid() };
        for capacity in [1024usize, 4096, 16384, 65536] {
            let mut pids = vec![0i32; capacity];
            let bytes = (capacity * std::mem::size_of::<i32>()) as i32;
            // SAFETY: bounded initialized PID output buffer and read-only query.
            let length = unsafe {
                // PROC_UID_ONLY = 4 in Apple's bsd/sys/proc_info.h.
                libc::proc_listpids(4, uid, pids.as_mut_ptr().cast(), bytes)
            };
            if length <= 0 {
                return Err(Error::new(
                    "ownership_conflict",
                    "complete OS process inventory is unavailable",
                ));
            }
            if length >= bytes {
                continue;
            }
            if !(length as usize).is_multiple_of(std::mem::size_of::<i32>()) {
                return Err(Error::new(
                    "ownership_conflict",
                    "invalid OS process inventory length",
                ));
            }
            pids.truncate(length as usize / std::mem::size_of::<i32>());
            let mut found = Vec::new();
            for pid in pids.into_iter().filter(|pid| *pid > 0) {
                if let Some(value) = process(pid as u32)?
                    && value.executable == path
                {
                    found.push(value);
                }
            }
            return Ok(found);
        }
        Err(Error::new(
            "ownership_conflict",
            "OS process inventory exceeded its inspection bound",
        ))
    }
    fn boot_id() -> Result<Uuid> {
        let mut bytes = [0u8; 128];
        let mut length = bytes.len();
        // SAFETY: fixed sysctl name and bounded writable output; no new value.
        if unsafe {
            libc::sysctlbyname(
                c"kern.bootsessionuuid".as_ptr(),
                bytes.as_mut_ptr().cast(),
                &mut length,
                std::ptr::null_mut(),
                0,
            )
        } != 0
            || length > bytes.len()
        {
            return Err(Error::new(
                "prerequisite",
                "OS boot identity is unavailable",
            ));
        }
        let text = std::str::from_utf8(&bytes[..length])
            .map_err(|_| Error::new("prerequisite", "invalid OS boot identity"))?;
        Uuid::parse_str(text.trim_end_matches('\0'))
            .map_err(|_| Error::new("prerequisite", "invalid OS boot identity"))
    }
    pub fn host() -> Result<HostIdentity> {
        let mut id = [0u8; 16];
        let timeout = libc::timespec {
            tv_sec: 5,
            tv_nsec: 0,
        };
        // SAFETY: output UUID and timeout are valid live buffers.
        if unsafe { libc::gethostuuid(id.as_mut_ptr(), &timeout) } != 0 {
            return Err(Error::new(
                "prerequisite",
                "OS host identity is unavailable",
            ));
        }
        // SAFETY: geteuid has no preconditions; getpwuid_r writes only its buffers.
        let uid = unsafe { libc::geteuid() };
        let mut passwd = std::mem::MaybeUninit::<libc::passwd>::uninit();
        let mut buffer = vec![0u8; 65536];
        let mut result = std::ptr::null_mut();
        if unsafe {
            libc::getpwuid_r(
                uid,
                passwd.as_mut_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut result,
            )
        } != 0
            || result.is_null()
        {
            return Err(Error::new("prerequisite", "OS account home is unavailable"));
        }
        // SAFETY: successful getpwuid_r populated passwd; returned strings live in buffer.
        let passwd = unsafe { passwd.assume_init() };
        let user = unsafe { CStr::from_ptr(passwd.pw_name) }
            .to_str()
            .map_err(|_| Error::invalid("OS username must be UTF-8"))?
            .to_owned();
        let home = unsafe { CStr::from_ptr(passwd.pw_dir) }
            .to_str()
            .map_err(|_| Error::invalid("OS home must be UTF-8"))?;
        Ok(HostIdentity {
            host_id: format!("macos:{}", Uuid::from_bytes(id)),
            boot_id: boot_id()?,
            uid,
            user,
            home: PathBuf::from(home),
        })
    }
    pub fn process(pid: u32) -> Result<Option<ProcessIdentity>> {
        if pid == 0 || pid > i32::MAX as u32 {
            return Err(Error::invalid("invalid process identity"));
        }
        let mut value = std::mem::MaybeUninit::<libc::proc_bsdinfo>::uninit();
        // SAFETY: exact-sized proc_bsdinfo output and valid scalar arguments.
        let length = unsafe {
            libc::proc_pidinfo(
                pid as i32,
                libc::PROC_PIDTBSDINFO,
                0,
                value.as_mut_ptr().cast(),
                std::mem::size_of::<libc::proc_bsdinfo>() as i32,
            )
        };
        if length == 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ESRCH) {
                return Ok(None);
            }
            return Err(Error::io("inspect owned process", error));
        }
        if length != std::mem::size_of::<libc::proc_bsdinfo>() as i32 {
            return Err(Error::new(
                "ownership_conflict",
                "incomplete OS process identity",
            ));
        }
        // SAFETY: exact successful byte count initialized the entire struct.
        let value = unsafe { value.assume_init() };
        let mut path = vec![0u8; 4096];
        // SAFETY: path is a valid writable output buffer.
        let length =
            unsafe { libc::proc_pidpath(pid as i32, path.as_mut_ptr().cast(), path.len() as u32) };
        if length <= 0 || length as usize > path.len() {
            return Err(Error::new(
                "ownership_conflict",
                "owned executable path is unavailable",
            ));
        }
        let path = std::str::from_utf8(&path[..length as usize])
            .map_err(|_| Error::new("ownership_conflict", "owned executable path is not UTF-8"))?
            .trim_end_matches('\0');
        Ok(Some(ProcessIdentity {
            pid,
            parent_pid: value.pbi_ppid,
            process_group: value.pbi_pgid,
            uid: value.pbi_uid,
            boot_id: boot_id()?,
            started_seconds: value.pbi_start_tvsec,
            started_microseconds: value.pbi_start_tvusec,
            executable: PathBuf::from(path),
        }))
    }
}
#[cfg(not(target_os = "macos"))]
mod native {
    use super::*;
    pub fn matching_executable(_: &Path) -> Result<Vec<ProcessIdentity>> {
        Err(Error::new(
            "prerequisite",
            "Kegari v0 ownership inspection requires macOS",
        ))
    }
    pub fn host() -> Result<HostIdentity> {
        Err(Error::new("prerequisite", "Kegari v0 host requires macOS"))
    }
    pub fn process(_: u32) -> Result<Option<ProcessIdentity>> {
        Err(Error::new(
            "prerequisite",
            "Kegari v0 ownership inspection requires macOS",
        ))
    }
}

#[cfg(test)]
mod owned_child_cleanup_tests {
    use super::*;
    #[test]
    fn failed_owned_handshake_drop_kills_and_reaps_only_its_direct_child() {
        let child = kegari_process::spawn::spawn_std(
            Command::new("/bin/sleep")
                .arg("30")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null()),
            None,
        )
        .unwrap();
        let pid = child.id();
        // Synthetic metadata: this tests owned-handle cleanup, not native birth
        // identity qualification (which remains a separate blocked check).
        let identity = ProcessIdentity {
            pid,
            parent_pid: std::process::id(),
            process_group: pid,
            uid: unsafe { libc::geteuid() },
            boot_id: Uuid::new_v4(),
            started_seconds: 0,
            started_microseconds: 0,
            executable: PathBuf::from("/bin/sleep"),
        };
        drop(OwnedChild {
            child,
            identity,
            reaped: None,
        });
        let mut status = 0;
        assert_eq!(
            unsafe { libc::waitpid(pid as i32, &mut status, libc::WNOHANG) },
            -1
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ECHILD)
        );
    }
}
