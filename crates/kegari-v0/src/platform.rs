//! macOS registration and process ownership. Metadata is evidence to inspect;
//! live OS locks and unreaped owned child handles establish exclusive control.
use crate::control::{ControlHost, Intent, IntentRecord, Lease};
use chrono::{DateTime, Utc};
use kegari_config::Config;
use kegari_digest::canonical_digest;
use kegari_platform::{
    BinaryIdentity, HostIdentity, NativeProcessProbe, ProcessIdentity, ProcessObservation,
    ProcessProbe,
};
use kegari_storage::{ExclusiveLock, PrivateDir};
use kegari_types::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

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
        kegari_process::spawn::spawn_tokio(&mut command, Some(cutoff)).map_err(|error| {
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
        let mut child = kegari_process::spawn::spawn_tokio(&mut command, Some(cutoff))?;
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
