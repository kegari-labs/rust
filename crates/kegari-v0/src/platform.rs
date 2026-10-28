//! macOS registration and process ownership. Metadata is evidence to inspect;
//! live OS locks and unreaped owned child handles establish exclusive control.
use crate::control::{ControlHost, Intent, IntentRecord, Lease};
use chrono::{DateTime, Utc};
use kegari_config::Config;
use kegari_digest::{bytes_digest, canonical_digest};
use kegari_storage::{ExclusiveLock, PrivateDir};
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
    sync::{Arc, Mutex},
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
        let mut child = crate::process_spawn::spawn_tokio(&mut command, Some(cutoff))?;
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeOwner {
    pub lease: Lease,
    pub process: ProcessIdentity,
    pub started_at: DateTime<Utc>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingOwner {
    pub lease: Lease,
    pub launcher: ProcessIdentity,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerRecord {
    pub runtime_id: Uuid,
    pub host_id: String,
    pub uid: u32,
    pub root: PathBuf,
    pub owner_generation: u64,
    pub binary: BinaryIdentity,
    pub runtime_build: String,
    pub setup_complete: bool,
    pub config_revision: Option<String>,
    pub control_process: Option<ProcessIdentity>,
    pub pending_owner: Option<PendingOwner>,
    pub orchestration: Option<RuntimeOwner>,
    pub updated_at: DateTime<Utc>,
}

pub struct Registration {
    root: PrivateDir,
    owner: Mutex<OwnerRecord>,
    probe: Arc<dyn ProcessProbe>,
    _control_lock: ExclusiveLock,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceShutdown {
    pub schema_version: u32,
    pub runtime_id: Uuid,
    pub stopped: bool,
    pub observed_at: DateTime<Utc>,
}
#[derive(Clone, Debug)]
pub enum ServiceStartState {
    AlreadyRunning(OwnerRecord),
    Ready(OwnerRecord),
}
fn check_shutdown_record(root: &PrivateDir, runtime: Uuid) -> Result<bool> {
    match root.read::<ServiceShutdown>("service-shutdown.json") {
        Ok(record) if record.schema_version == 1 && record.runtime_id == runtime => {
            Ok(record.stopped)
        }
        Ok(_) => Err(Error::new(
            "identity_mismatch",
            "service shutdown reservation differs from registered runtime",
        )),
        Err(error) if error.code == "record_missing" => Ok(false),
        Err(error) => Err(error),
    }
}
impl Registration {
    /// Explicit bootstrap. Acquires control.lock before any control record is
    /// created. This stage deliberately remains incomplete until verified
    /// credentials/configuration are committed by complete_setup.
    pub fn bootstrap(
        host: &HostIdentity,
        runtime: Uuid,
        binary: BinaryIdentity,
        request: Uuid,
    ) -> Result<(Self, ControlHost)> {
        Self::bootstrap_with_probe(host, runtime, binary, request, Arc::new(NativeProcessProbe))
    }
    pub fn bootstrap_with_probe(
        host: &HostIdentity,
        runtime: Uuid,
        binary: BinaryIdentity,
        request: Uuid,
        probe: Arc<dyn ProcessProbe>,
    ) -> Result<(Self, ControlHost)> {
        binary.revalidate(false)?;
        if runtime.is_nil() || request.is_nil() {
            return Err(Error::invalid("bootstrap identities must be non-nil"));
        }
        let root = PrivateDir::create(&host.state_root())?;
        let lock = root.lock("control.lock")?;
        for file in ["owner.json", "config.toml", "intent.json"] {
            match root.read_bytes(file, 8 * 1024 * 1024) {
                Ok(_) => {
                    return Err(Error::new(
                        "setup_incomplete",
                        "existing registration or partial setup must be recovered explicitly",
                    ));
                }
                Err(error) if error.code == "record_missing" => (),
                Err(error) => return Err(error),
            }
        }
        let owner = OwnerRecord {
            runtime_id: runtime,
            host_id: host.host_id.clone(),
            uid: host.uid,
            root: root.path().to_owned(),
            owner_generation: 1,
            binary,
            runtime_build: crate::BUILD_ID.into(),
            setup_complete: false,
            config_revision: None,
            control_process: Some(probe.current()?),
            pending_owner: None,
            orchestration: None,
            updated_at: Utc::now(),
        };
        root.write("owner.json", &owner)?;
        let control = ControlHost::setup_locked(
            PrivateDir::open(root.path())?,
            runtime,
            request,
            lock.clone(),
        )?;
        for directory in ["runs", "logs"] {
            root.child(directory)?;
        }
        Ok((
            Self {
                root,
                owner: Mutex::new(owner),
                probe,
                _control_lock: lock,
            },
            control,
        ))
    }
    pub fn recover(
        host: &HostIdentity,
        binary: &BinaryIdentity,
    ) -> Result<(Self, ControlHost, Config)> {
        Self::recover_with_probe(host, binary, Arc::new(NativeProcessProbe))
    }
    pub fn recover_with_probe(
        host: &HostIdentity,
        binary: &BinaryIdentity,
        probe: Arc<dyn ProcessProbe>,
    ) -> Result<(Self, ControlHost, Config)> {
        let root = PrivateDir::open(&host.state_root())?;
        let lock = root.lock("control.lock")?;
        let mut owner: OwnerRecord = root.read("owner.json")?;
        if owner.host_id != host.host_id
            || owner.uid != host.uid
            || owner.root != root.path()
            || owner.runtime_id.is_nil()
        {
            return Err(Error::new(
                "identity_mismatch",
                "registered runtime belongs to another host, root or OS account",
            ));
        }
        if !owner.setup_complete {
            return Err(Error::new(
                "setup_incomplete",
                "bootstrap has not verified and committed configuration and credentials",
            ));
        }
        if check_shutdown_record(&root, owner.runtime_id)? {
            return Err(Error::new(
                "service_stopped",
                "control service was explicitly stopped; use local service start",
            )
            .next("Run kegari-v0 service start on the execution host"));
        }
        if &owner.binary != binary || owner.runtime_build != crate::BUILD_ID {
            return Err(Error::new(
                "handover_required",
                "installed binary/build differs; explicit stopped and reconciled handover is required",
            ));
        }
        binary.revalidate(false)?;
        if let Some(process) = &owner.control_process
            && process.pid != std::process::id()
            && probe.observe(process)? == ProcessObservation::Running
        {
            return Err(Error::new(
                "ownership_conflict",
                "previous control host remains alive without the expected exclusive lock",
            ));
        }
        let bytes = root.read_bytes("config.toml", 8 * 1024 * 1024)?;
        let config = Config::parse(
            std::str::from_utf8(&bytes)
                .map_err(|_| Error::new("invalid_config", "configuration is not UTF-8"))?,
        )?;
        if config.runtime.id != owner.runtime_id
            || config.runtime.host_id != host.host_id
            || owner.config_revision.as_deref() != Some(&crate::config::config_revision(&config)?)
        {
            return Err(Error::new(
                "config_reconciliation",
                "configuration differs from its durable registered revision",
            ));
        }
        // A live prior orchestration is preserved for explicit attachment or
        // shutdown reconciliation. Its metadata is never cleared optimistically.
        let control = ControlHost::recover_locked(
            PrivateDir::open(root.path())?,
            owner.runtime_id,
            lock.clone(),
        )?;
        owner.control_process = Some(probe.current()?);
        owner.updated_at = Utc::now();
        root.write("owner.json", &owner)?;
        Ok((
            Self {
                root,
                owner: Mutex::new(owner),
                probe,
                _control_lock: lock,
            },
            control,
            config,
        ))
    }
    pub fn snapshot(&self) -> Result<OwnerRecord> {
        Ok(self
            .owner
            .lock()
            .map_err(|_| Error::new("ownership_conflict", "registration serialization failed"))?
            .clone())
    }
    pub fn root(&self) -> &PrivateDir {
        &self.root
    }
    /// The registration retains the acquired control.lock descriptor. Require
    /// the exact current process observation before native listener maintenance.
    pub fn verify_control_owner(&self) -> Result<()> {
        let owner = self.snapshot()?;
        let current = self.probe.current()?;
        if owner.control_process.as_ref() != Some(&current)
            || self.probe.observe(&current)? != ProcessObservation::Running
            || owner.uid != current.uid
            || owner.root != self.root.path()
        {
            return Err(Error::new(
                "ownership_conflict",
                "control listener maintenance requires the live registered owner",
            ));
        }
        Ok(())
    }
    /// Called by the control dispatcher's serialized STOPPED/no-work gate.
    /// KeepAlive/restart cannot bypass this durable reservation.
    pub fn reserve_service_shutdown(&self) -> Result<()> {
        let owner = self.snapshot()?;
        if owner.pending_owner.is_some() {
            return Err(Error::new(
                "ownership_conflict",
                "pending orchestration ownership requires reconciliation before shutdown",
            ));
        }
        if let Some(active) = &owner.orchestration
            && self.probe.observe(&active.process)? == ProcessObservation::Running
        {
            return Err(Error::new(
                "ownership_conflict",
                "orchestration is still running",
            ));
        }
        self.root.write(
            "service-shutdown.json",
            &ServiceShutdown {
                schema_version: 1,
                runtime_id: owner.runtime_id,
                stopped: true,
                observed_at: Utc::now(),
            },
        )
    }
    /// Explicit local start only. Clear the durable shutdown reservation only
    /// after acquiring control.lock and proving the recorded host has gone.
    pub fn prepare_service_start(
        host: &HostIdentity,
        probe: &dyn ProcessProbe,
        expected_binary: &BinaryIdentity,
    ) -> Result<ServiceStartState> {
        let root = PrivateDir::open(&host.state_root())?;
        let observed: OwnerRecord = root.read("owner.json")?;
        if observed.uid != host.uid
            || observed.host_id != host.host_id
            || observed.root != root.path()
            || !observed.setup_complete
        {
            return Err(Error::new(
                "identity_mismatch",
                "service start requires the complete registered host identity",
            ));
        }
        if &observed.binary != expected_binary || observed.runtime_build != crate::BUILD_ID {
            return Err(Error::new(
                "handover_required",
                "registered owner and installed service binary/build differ",
            ));
        }
        if let Some(process) = &observed.control_process
            && probe.observe(process)? == ProcessObservation::Running
        {
            if check_shutdown_record(&root, observed.runtime_id)? {
                return Err(Error::new(
                    "stop_in_progress",
                    "control host shutdown is still in progress",
                ));
            }
            return Ok(ServiceStartState::AlreadyRunning(observed));
        }
        let _lock = root.lock("control.lock")?;
        let current: OwnerRecord = root.read("owner.json")?;
        if canonical_digest(1, &current)? != canonical_digest(1, &observed)? {
            return Err(Error::new(
                "ownership_conflict",
                "registered owner changed while preparing service start",
            ));
        }
        if let Some(process) = &current.control_process
            && probe.observe(process)? == ProcessObservation::Running
        {
            return Ok(ServiceStartState::AlreadyRunning(current));
        }
        check_shutdown_record(&root, current.runtime_id)?;
        root.write(
            "service-shutdown.json",
            &ServiceShutdown {
                schema_version: 1,
                runtime_id: current.runtime_id,
                stopped: false,
                observed_at: Utc::now(),
            },
        )?;
        Ok(ServiceStartState::Ready(current))
    }
    /// Called only after identity/profile/Keychain verification. A crash between
    /// the TOML and owner-record replacement is detected on recovery, not treated
    /// as a completed configuration transaction.
    pub fn complete_setup(&self, config: &Config) -> Result<()> {
        let mut owner = self
            .owner
            .lock()
            .map_err(|_| Error::new("ownership_conflict", "registration serialization failed"))?;
        if owner.setup_complete {
            return Err(Error::new(
                "ownership_conflict",
                "setup already complete; use serialized configuration transaction",
            ));
        }
        config.validate()?;
        if config.runtime.id != owner.runtime_id || config.runtime.host_id != owner.host_id {
            return Err(Error::new(
                "identity_mismatch",
                "bootstrap configuration identity differs",
            ));
        }
        let text = toml::to_string_pretty(config)
            .map_err(|_| Error::invalid("configuration encoding failed"))?;
        self.root.write_bytes("config.toml", text.as_bytes())?;
        owner.config_revision = Some(crate::config::config_revision(config)?);
        owner.setup_complete = true;
        owner.updated_at = Utc::now();
        self.root.write("owner.json", &*owner)
    }
    /// The caller holds the serialized control transaction before this method.
    /// Recovery detects a crash between configuration and owner revision writes;
    /// it never guesses that a partially durable update completed.
    pub fn commit_configuration(&self, expected_revision: &str, config: &Config) -> Result<()> {
        let mut owner = self
            .owner
            .lock()
            .map_err(|_| Error::new("ownership_conflict", "registration serialization failed"))?;
        config.validate()?;
        if !owner.setup_complete || owner.config_revision.as_deref() != Some(expected_revision) {
            return Err(Error::new(
                "stale_config",
                "registered configuration revision changed",
            ));
        }
        if config.runtime.id != owner.runtime_id || config.runtime.host_id != owner.host_id {
            return Err(Error::new(
                "identity_mismatch",
                "configuration cannot replace runtime or host identity",
            ));
        }
        let current = self.root.read_bytes("config.toml", 8 * 1024 * 1024)?;
        let current = Config::parse(
            std::str::from_utf8(&current)
                .map_err(|_| Error::new("invalid_config", "configuration is not UTF-8"))?,
        )?;
        if crate::config::config_revision(&current)? != expected_revision {
            return Err(Error::new(
                "config_reconciliation",
                "stored configuration no longer matches registered revision",
            ));
        }
        let text = toml::to_string_pretty(config)
            .map_err(|_| Error::invalid("configuration encoding failed"))?;
        self.root.write_bytes("config.toml", text.as_bytes())?;
        owner.config_revision = Some(crate::config::config_revision(config)?);
        owner.updated_at = Utc::now();
        self.root.write("owner.json", &*owner)
    }
    /// A control-host restart may observe this record, but cannot acquire the
    /// runtime OS lock or spawn a replacement while the old owner is alive.
    pub fn prepare_owner(&self) -> Result<Lease> {
        let mut owner = self
            .owner
            .lock()
            .map_err(|_| Error::new("ownership_conflict", "registration serialization failed"))?;
        if owner.pending_owner.is_some() {
            return Err(Error::new(
                "reconciliation_required",
                "a prior orchestration startup is unresolved",
            ));
        }
        if let Some(previous) = &owner.orchestration {
            match self.probe.observe(&previous.process)? {
                ProcessObservation::Running => {
                    return Err(Error::new(
                        "ownership_conflict",
                        "previous orchestration process is still alive",
                    ));
                }
                ProcessObservation::DifferentIdentity => {
                    return Err(Error::new(
                        "ownership_conflict",
                        "prior PID was reused; reconcile preserved local work before replacement",
                    ));
                }
                ProcessObservation::Gone => {
                    return Err(Error::new(
                        "reconciliation_required",
                        "previous orchestration exited; local and remote effects require reconciliation",
                    ));
                }
            }
        }
        owner.owner_generation = owner
            .owner_generation
            .checked_add(1)
            .ok_or_else(|| Error::new("ownership_conflict", "owner generation exhausted"))?;
        let lease = Lease {
            runtime_id: owner.runtime_id,
            owner_generation: owner.owner_generation,
            owner_id: Uuid::new_v4(),
        };
        owner.pending_owner = Some(PendingOwner {
            lease: lease.clone(),
            launcher: self.probe.current()?,
        });
        owner.updated_at = Utc::now();
        self.root.write("owner.json", &*owner)?;
        Ok(lease)
    }
    pub fn record_orchestration(&self, lease: Lease, process: ProcessIdentity) -> Result<()> {
        let mut owner = self
            .owner
            .lock()
            .map_err(|_| Error::new("ownership_conflict", "registration serialization failed"))?;
        if owner.runtime_id != lease.runtime_id
            || owner.owner_generation != lease.owner_generation
            || owner.pending_owner.as_ref().map(|pending| &pending.lease) != Some(&lease)
            || owner.orchestration.is_some()
            || process.uid != owner.uid
            || process.executable != owner.binary.path
            || owner.control_process.as_ref().is_none_or(|control| {
                process.parent_pid != control.pid || process.pid == control.pid
            })
            || process.process_group != process.pid
            || self.probe.observe(&process)? != ProcessObservation::Running
        {
            return Err(Error::new(
                "ownership_conflict",
                "orchestration handshake does not match the reserved owner and executable",
            ));
        }
        if self.root.try_lock("runtime.lock")?.is_some() {
            return Err(Error::new(
                "ownership_conflict",
                "orchestration handshake has no held runtime lock",
            ));
        }
        owner.orchestration = Some(RuntimeOwner {
            lease,
            process,
            started_at: Utc::now(),
        });
        owner.pending_owner = None;
        owner.updated_at = Utc::now();
        self.root.write("owner.json", &*owner)
    }
    pub fn reconcile_exited(
        &self,
        lease: &Lease,
        local_work_clear: bool,
        remote_reconciled: bool,
    ) -> Result<()> {
        let mut owner = self
            .owner
            .lock()
            .map_err(|_| Error::new("ownership_conflict", "registration serialization failed"))?;
        let previous = owner
            .orchestration
            .as_ref()
            .ok_or_else(|| Error::new("ownership_conflict", "no retained orchestration owner"))?;
        if &previous.lease != lease
            || self.probe.observe(&previous.process)? == ProcessObservation::Running
            || !local_work_clear
            || !remote_reconciled
        {
            return Err(Error::new(
                "reconciliation_required",
                "previous process, owned work or remote actions remain unresolved",
            ));
        }
        // Prove no other process holds runtime.lock before clearing stale metadata.
        let _runtime_lock = self.root.lock("runtime.lock")?;
        owner.orchestration = None;
        owner.updated_at = Utc::now();
        self.root.write("owner.json", &*owner)
    }
}

impl Registration {
    /// A startup has no execution authority before the inherited-pipe handshake.
    /// Prove its launcher is stopped/current, its processes absent and lock free
    /// before releasing a crash-before-ack reservation.
    pub fn reconcile_pending_start(&self, lease: &Lease) -> Result<()> {
        let mut owner = self
            .owner
            .lock()
            .map_err(|_| Error::new("ownership_conflict", "registration serialization failed"))?;
        let pending = owner
            .pending_owner
            .as_ref()
            .ok_or_else(|| Error::new("unknown_resource", "no pending startup reservation"))?;
        if &pending.lease != lease || owner.orchestration.is_some() {
            return Err(Error::new(
                "ownership_conflict",
                "startup reservation differs from reconciliation target",
            ));
        }
        let current = self.probe.current()?;
        if pending.launcher != current
            && self.probe.observe(&pending.launcher)? == ProcessObservation::Running
        {
            return Err(Error::new(
                "ownership_conflict",
                "prior startup launcher remains alive",
            ));
        }
        let _lock = self.root.lock("runtime.lock")?;
        let processes = self.probe.matching_executable(&owner.binary.path)?;
        if processes.iter().any(|process| process != &current) {
            return Err(Error::new(
                "reconciliation_required",
                "a possible owned startup process remains; preserve reservation",
            ));
        }
        owner.pending_owner = None;
        owner.updated_at = Utc::now();
        self.root.write("owner.json", &*owner)
    }

    /// Safe to call inside ControlHost's serialized last-moment gate: this
    /// method never re-enters ControlHost or grants a new operation permit.
    pub fn verify_active_owner(&self, lease: &Lease) -> Result<()> {
        let owner = self
            .owner
            .lock()
            .map_err(|_| Error::new("ownership_conflict", "registration serialization failed"))?;
        let runtime = owner
            .orchestration
            .as_ref()
            .ok_or_else(|| Error::new("ownership_conflict", "no attached orchestration owner"))?;
        if &runtime.lease != lease
            || self.probe.observe(&runtime.process)? != ProcessObservation::Running
        {
            return Err(Error::new(
                "ownership_conflict",
                "orchestration process identity is no longer current",
            ));
        }
        match self.root.try_lock("runtime.lock")? {
            Some(_) => Err(Error::new(
                "ownership_conflict",
                "orchestration process no longer holds runtime.lock",
            )),
            None => Ok(()),
        }
    }
}

/// Held by the orchestration process, separately from the control host lock.
pub struct RuntimeLock {
    _lock: ExclusiveLock,
    pub lease: Lease,
}
impl RuntimeLock {
    pub fn acquire(root: &PrivateDir, lease: Lease) -> Result<Self> {
        Self::acquire_with_probe(root, lease, &NativeProcessProbe)
    }
    pub fn acquire_with_probe(
        root: &PrivateDir,
        lease: Lease,
        probe: &dyn ProcessProbe,
    ) -> Result<Self> {
        let lock = root.lock("runtime.lock")?;
        let owner: OwnerRecord = root.read("owner.json")?;
        let intent: IntentRecord = root.read("intent.json")?;
        if !owner.setup_complete
            || owner.runtime_id != lease.runtime_id
            || owner.owner_generation != lease.owner_generation
            || owner.pending_owner.as_ref().map(|pending| &pending.lease) != Some(&lease)
            || intent.runtime_id != lease.runtime_id
            || intent.desired != Intent::Running
        {
            return Err(Error::new(
                "requires_start",
                "runtime lock requires current registered RUNNING authority",
            ));
        }
        let process = probe.current()?;
        let pending = owner
            .pending_owner
            .as_ref()
            .expect("exact pending lease checked");
        if process.parent_pid != pending.launcher.pid
            || process.pid == pending.launcher.pid
            || process.process_group != process.pid
            || process.uid != owner.uid
            || process.executable != owner.binary.path
            || process.boot_id != pending.launcher.boot_id
        {
            return Err(Error::new(
                "ownership_conflict",
                "runtime lock is not held by the reserved launcher child",
            ));
        }
        Ok(Self { _lock: lock, lease })
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
        let mut child = crate::process_spawn::spawn_std(&mut command, None)
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServicePlan {
    pub label: String,
    pub uid: u32,
    pub binary: BinaryIdentity,
    pub plist_path: PathBuf,
    pub plist: String,
    pub digest: String,
}
pub fn service_plan(host: &HostIdentity, binary: BinaryIdentity) -> Result<ServicePlan> {
    binary.revalidate(true)?;
    let label = format!("com.kegari.v0.{}", host.uid);
    fn xml(text: &str) -> String {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;")
    }
    let path = binary
        .path
        .to_str()
        .ok_or_else(|| Error::invalid("installed path must be UTF-8"))?;
    let root = host.state_root();
    let log = root.join("logs/control-service.log");
    let log = log
        .to_str()
        .ok_or_else(|| Error::invalid("service log path must be UTF-8"))?;
    let plist = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict><key>Label</key><string>{}</string><key>ProgramArguments</key><array><string>{}</string><string>__control-host</string></array><key>RunAtLoad</key><true/><key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict><key>ProcessType</key><string>Background</string><key>Umask</key><integer>63</integer><key>StandardOutPath</key><string>{}</string><key>StandardErrorPath</key><string>{}</string></dict></plist>\n",
        xml(&label),
        xml(path),
        xml(log),
        xml(log)
    );
    let digest = canonical_digest(
        1,
        &serde_json::json!({"binary":binary,"uid":host.uid,"plist":plist}),
    )?;
    Ok(ServicePlan {
        plist_path: host
            .home
            .join("Library/LaunchAgents")
            .join(format!("{label}.plist")),
        label,
        uid: host.uid,
        binary,
        plist,
        digest,
    })
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

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstalledService {
    pub schema_version: u32,
    pub runtime_build: String,
    pub plan: ServicePlan,
    pub installed_at: DateTime<Utc>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildInfo {
    pub api_version: u32,
    pub runtime_version: String,
    pub runtime_build: String,
    pub schema_version: u32,
}
pub fn build_info() -> BuildInfo {
    BuildInfo {
        api_version: 1,
        runtime_version: env!("CARGO_PKG_VERSION").into(),
        runtime_build: crate::BUILD_ID.into(),
        schema_version: 1,
    }
}

/// This is invoked only by an explicit local service command. Its executable is
/// immutable and inspected before/after the bounded metadata subprocess.
pub async fn verify_installed_build(binary: &BinaryIdentity) -> Result<BuildInfo> {
    use tokio::io::AsyncReadExt;
    binary.revalidate(true)?;
    let mut command = tokio::process::Command::new(&binary.path);
    command
        .arg("__build-info")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .process_group(0);
    let cutoff = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut child =
        crate::process_spawn::spawn_tokio(&mut command, Some(cutoff)).map_err(|error| {
            if error.kind() == std::io::ErrorKind::TimedOut {
                Error::new("timeout", "installed build verification timed out")
            } else {
                Error::io("verify installed binary", error)
            }
        })?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| {
            Error::new(
                "incompatible_version",
                "installed binary output unavailable",
            )
        })?
        .take(65_537);
    let mut bytes = Vec::new();
    let observed = tokio::time::timeout_at(tokio::time::Instant::from_std(cutoff), async {
        stdout
            .read_to_end(&mut bytes)
            .await
            .map_err(|e| Error::io("read installed build information", e))?;
        if bytes.len() > 65_536 {
            return Err(Error::new(
                "incompatible_version",
                "installed build information exceeds bound",
            ));
        }
        let status = child
            .wait()
            .await
            .map_err(|e| Error::io("wait installed build verification", e))?;
        if !status.success() {
            return Err(Error::new(
                "incompatible_version",
                "installed binary rejected build verification",
            ));
        }
        Ok(())
    })
    .await;
    if !matches!(observed, Ok(Ok(()))) {
        if let Some(pid) = child.id() {
            // SAFETY: PID belongs to our unreaped child in its own process group.
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
        }
        let _ = child.kill().await;
    }
    observed.map_err(|_| Error::new("timeout", "installed build verification timed out"))??;
    binary.revalidate(true)?;
    let info: BuildInfo = serde_json::from_slice(&bytes).map_err(|_| {
        Error::new(
            "incompatible_version",
            "installed binary does not return exact build information",
        )
    })?;
    if info.api_version != 1
        || info.schema_version != 1
        || info.runtime_version != env!("CARGO_PKG_VERSION")
        || info.runtime_build != crate::BUILD_ID
    {
        return Err(Error::new(
            "handover_required",
            "installed binary differs from this verified runtime build; reconcile a supported handover before installation",
        ));
    }
    Ok(info)
}

fn service_directory(host: &HostIdentity, create: bool) -> Result<PathBuf> {
    let home = host
        .home
        .canonicalize()
        .map_err(|e| Error::io("resolve service account home", e))?;
    if home != host.home {
        return Err(Error::new(
            "unsafe_storage",
            "service account home must be canonical",
        ));
    }
    let mut directory = home;
    for leaf in ["Library", "LaunchAgents"] {
        directory.push(leaf);
        match std::fs::symlink_metadata(&directory) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && create => {
                std::fs::create_dir(&directory)
                    .map_err(|e| Error::io("create service directory", e))?;
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
                    .map_err(|e| Error::io("protect service directory", e))?;
            }
            Err(e) => return Err(Error::io("inspect service directory", e)),
            Ok(_) => {}
        }
        let metadata = std::fs::symlink_metadata(&directory)
            .map_err(|e| Error::io("inspect service directory", e))?;
        if !metadata.is_dir() || metadata.uid() != host.uid || metadata.mode() & 0o022 != 0 {
            return Err(Error::new(
                "unsafe_storage",
                "service directory must be owned, unlinked and not writable by other users",
            ));
        }
    }
    Ok(directory)
}
fn install_plist(host: &HostIdentity, plan: &ServicePlan) -> Result<()> {
    use std::io::Write;
    let directory = service_directory(host, true)?;
    if plan.plist_path.parent() != Some(&directory) {
        return Err(Error::new(
            "scope_violation",
            "service file escaped the account LaunchAgents directory",
        ));
    }
    match std::fs::symlink_metadata(&plan.plist_path) {
        Ok(meta) => {
            if !meta.is_file()
                || meta.nlink() != 1
                || meta.uid() != host.uid
                || meta.mode() & 0o077 != 0
            {
                return Err(Error::new(
                    "unsafe_storage",
                    "existing LaunchAgent is not a private owned regular file",
                ));
            }
            let existing = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
                .open(&plan.plist_path)
                .map_err(|e| Error::io("read existing LaunchAgent", e))?;
            let meta = existing
                .metadata()
                .map_err(|e| Error::io("inspect opened LaunchAgent", e))?;
            if !meta.is_file()
                || meta.nlink() != 1
                || meta.uid() != host.uid
                || meta.mode() & 0o077 != 0
            {
                return Err(Error::new(
                    "unsafe_storage",
                    "opened LaunchAgent changed its protected regular file identity",
                ));
            }
            let mut bytes = Vec::new();
            existing
                .take(65_537)
                .read_to_end(&mut bytes)
                .map_err(|e| Error::io("read existing LaunchAgent", e))?;
            if bytes != plan.plist.as_bytes() {
                return Err(Error::new(
                    "ownership_conflict",
                    "existing LaunchAgent differs; preserve it and reconcile installation explicitly",
                ));
            }
            return Ok(());
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(Error::io("inspect LaunchAgent", e)),
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&plan.plist_path)
        .map_err(|e| Error::io("create explicit LaunchAgent", e))?;
    file.write_all(plan.plist.as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|e| Error::io("persist LaunchAgent", e))?;
    File::open(directory)
        .and_then(|d| d.sync_all())
        .map_err(|e| Error::io("persist LaunchAgent directory", e))
}
/// Writes the explicit LaunchAgent only. Does not load it, start orchestration,
/// replace an unrelated plist, or alter desired intent.
pub async fn install_service(host: &HostIdentity, binary_path: &Path) -> Result<InstalledService> {
    let binary = BinaryIdentity::inspect(binary_path, true)?;
    verify_installed_build(&binary).await?;
    let root = PrivateDir::create(&host.state_root())?;
    let _install = root.lock("service-install.lock")?;
    match root.read::<OwnerRecord>("owner.json") {
        Ok(owner)
            if owner.binary != binary || owner.uid != host.uid || owner.host_id != host.host_id =>
        {
            return Err(Error::new(
                "handover_required",
                "registered control binary differs from requested installation",
            ));
        }
        Ok(_) => {}
        Err(error) if error.code == "record_missing" => {}
        Err(error) => return Err(error),
    }
    let plan = service_plan(host, binary)?;
    match root.read::<InstalledService>("service.json") {
        Ok(prior)
            if prior.schema_version == 1
                && prior.runtime_build == crate::BUILD_ID
                && prior.plan.digest == plan.digest =>
        {
            install_plist(host, &plan)?;
            return Ok(prior);
        }
        Ok(_) => {
            return Err(Error::new(
                "handover_required",
                "existing service registration differs from requested installation",
            ));
        }
        Err(error) if error.code == "record_missing" => {}
        Err(error) => return Err(error),
    }
    root.child("logs")?;
    install_plist(host, &plan)?;
    let installed = InstalledService {
        schema_version: 1,
        runtime_build: crate::BUILD_ID.into(),
        plan,
        installed_at: Utc::now(),
    };
    root.write("service.json", &installed)?;
    Ok(installed)
}
pub fn installed_service(host: &HostIdentity) -> Result<InstalledService> {
    let root = PrivateDir::open(&host.state_root())?;
    let installed: InstalledService = root.read("service.json")?;
    let expected = service_plan(host, installed.plan.binary.clone())?;
    if installed.schema_version != 1
        || installed.runtime_build != crate::BUILD_ID
        || installed.plan.digest != expected.digest
        || canonical_digest(1, &installed.plan)? != canonical_digest(1, &expected)?
    {
        return Err(Error::new(
            "identity_mismatch",
            "service registration or binary changed",
        ));
    }
    if expected.plist_path.parent() != Some(service_directory(host, false)?.as_path()) {
        return Err(Error::new(
            "scope_violation",
            "installed LaunchAgent directory changed",
        ));
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(&expected.plist_path)
        .map_err(|e| Error::io("open installed LaunchAgent", e))?;
    let metadata = file
        .metadata()
        .map_err(|e| Error::io("inspect installed LaunchAgent", e))?;
    if !metadata.is_file()
        || metadata.uid() != host.uid
        || metadata.nlink() != 1
        || metadata.mode() & 0o077 != 0
    {
        return Err(Error::new(
            "unsafe_storage",
            "installed LaunchAgent is not an owner-only regular file",
        ));
    }
    let mut bytes = Vec::new();
    file.take(65_537)
        .read_to_end(&mut bytes)
        .map_err(|e| Error::io("read installed LaunchAgent", e))?;
    if bytes != expected.plist.as_bytes() {
        return Err(Error::new(
            "identity_mismatch",
            "installed LaunchAgent differs from its registered plan",
        ));
    }
    Ok(installed)
}
async fn launchctl(args: &[String]) -> Result<bool> {
    let mut command = tokio::process::Command::new("/bin/launchctl");
    command
        .args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let cutoff = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let status = tokio::time::timeout_at(tokio::time::Instant::from_std(cutoff), async {
        let mut child = crate::process_spawn::spawn_tokio(&mut command, Some(cutoff))?;
        child.wait().await
    })
    .await
    .map_err(|_| {
        Error::new(
            "operation_uncertain",
            "launchd action timed out; inspect service state before retry",
        )
    })?
    .map_err(|error| {
        if error.kind() == std::io::ErrorKind::TimedOut {
            Error::new(
                "operation_uncertain",
                "launchd action timed out; inspect service state before retry",
            )
        } else {
            Error::io("invoke explicit launchd action", error)
        }
    })?;
    Ok(status.success())
}
/// Explicit local service start; never force-restarts a running service.
pub async fn start_service(host: &HostIdentity) -> Result<ServiceStartState> {
    let installed = installed_service(host)?;
    verify_installed_build(&installed.plan.binary).await?;
    let state =
        Registration::prepare_service_start(host, &NativeProcessProbe, &installed.plan.binary)?;
    let owner = match &state {
        ServiceStartState::AlreadyRunning(owner) | ServiceStartState::Ready(owner) => owner,
    };
    if owner.binary != installed.plan.binary {
        return Err(Error::new(
            "handover_required",
            "registered owner and installed service binaries differ",
        ));
    }
    if matches!(state, ServiceStartState::AlreadyRunning(_)) {
        return Ok(state);
    }
    let domain = format!("gui/{}", host.uid);
    let target = format!("{domain}/{}", installed.plan.label);
    let loaded = launchctl(&["print".into(), target.clone()]).await?;
    let args = if loaded {
        vec!["kickstart".into(), target]
    } else {
        vec![
            "bootstrap".into(),
            domain,
            installed.plan.plist_path.to_string_lossy().into_owned(),
        ]
    };
    if !launchctl(&args).await? {
        return Err(Error::new(
            "operation_failed",
            "launchd did not acknowledge explicit service start; inspect registered service state",
        ));
    }
    Ok(state)
}
/// Only after the API's serialized shutdown acknowledgment. Never sends a
/// second mutation or kills a host while its orchestration is draining.
pub async fn finish_service_stop(host: &HostIdentity) -> Result<()> {
    let root = PrivateDir::open(&host.state_root())?;
    let owner: OwnerRecord = root.read("owner.json")?;
    if !check_shutdown_record(&root, owner.runtime_id)? {
        return Err(Error::new(
            "access_denied",
            "control host must durably reserve shutdown before launchd removal",
        ));
    }
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    if let Some(process) = &owner.control_process {
        while process.observe()? == ProcessObservation::Running {
            if tokio::time::Instant::now() >= deadline {
                return Err(Error::new(
                    "operation_uncertain",
                    "control host shutdown acknowledged but process still exits; do not replay shutdown",
                ));
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }
    // Serialize launchd removal against a new explicit start and host recovery.
    // The previous host has released this lock before we can acquire it.
    let _lock = root.lock("control.lock")?;
    let current: OwnerRecord = root.read("owner.json")?;
    if current.runtime_id != owner.runtime_id || !check_shutdown_record(&root, owner.runtime_id)? {
        return Err(Error::new(
            "ownership_conflict",
            "service was restarted during shutdown reconciliation",
        ));
    }
    let installed = match installed_service(host) {
        Ok(value) => value,
        Err(error) if error.code == "record_missing" => return Ok(()),
        Err(error) => return Err(error),
    };
    let target = format!("gui/{}/{}", host.uid, installed.plan.label);
    if launchctl(&["print".into(), target.clone()]).await?
        && !launchctl(&["bootout".into(), target]).await?
    {
        return Err(Error::new(
            "operation_uncertain",
            "host exited but launchd service removal needs reconciliation",
        ));
    }
    Ok(())
}

/// Handover-only replacement while its caller retains control.lock/runtime.lock
/// and has revalidated the opaque, qualified local handover plan. It preserves
/// the old service record and never loads or starts the replacement service.
pub(crate) fn replace_service_for_handover(
    host: &HostIdentity,
    root: &PrivateDir,
    previous: &BinaryIdentity,
    replacement: &BinaryIdentity,
    replacement_build: &str,
) -> Result<()> {
    let owner: OwnerRecord = root.read("owner.json")?;
    if owner.binary != *previous || !check_shutdown_record(root, owner.runtime_id)? {
        return Err(Error::new(
            "handover_blocked",
            "old owner or durable supervisor fence changed",
        ));
    }
    let installed = match installed_service(host) {
        Ok(installed) => installed,
        Err(error) if error.code == "record_missing" => return Ok(()),
        Err(error) => return Err(error),
    };
    if installed.plan.binary != *previous {
        return Err(Error::new(
            "handover_blocked",
            "installed supervisor differs from old owner",
        ));
    }
    let plan = service_plan(host, replacement.clone())?;
    let directory = service_directory(host, false)?;
    let old = root.child("handover-history")?;
    old.write(
        &format!("service-{}.json", owner.owner_generation),
        &installed,
    )?;
    let temporary = directory.join(format!(".kegari-handover-{}.plist", Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(&temporary)
        .map_err(|e| Error::io("create replacement LaunchAgent", e))?;
    use std::io::Write;
    file.write_all(plan.plist.as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|e| Error::io("persist replacement LaunchAgent", e))?;
    // Recheck the exact prior registered plist immediately before replacement.
    installed_service(host)?;
    std::fs::rename(&temporary, &plan.plist_path)
        .map_err(|e| Error::io("replace explicitly fenced LaunchAgent", e))?;
    File::open(directory)
        .and_then(|d| d.sync_all())
        .map_err(|e| Error::io("persist LaunchAgent handover", e))?;
    root.write(
        "service.json",
        &InstalledService {
            schema_version: 1,
            runtime_build: replacement_build.into(),
            plan,
            installed_at: Utc::now(),
        },
    )
}

#[cfg(test)]
mod owned_child_cleanup_tests {
    use super::*;
    #[test]
    fn failed_owned_handshake_drop_kills_and_reaps_only_its_direct_child() {
        let child = crate::process_spawn::spawn_std(
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
