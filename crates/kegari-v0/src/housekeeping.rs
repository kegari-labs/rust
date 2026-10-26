//! Explicit retention, cleanup and bounded host diagnostics. Nothing is removed
//! because an issue closes, a worker exits, a release succeeds or disk fills.
use crate::{
    control::{
        Actor, ControlHost, ControlSnapshot, MutationRequest, Receipt, ReceiptPhase, ReceiptStore,
    },
    digest::{bytes_digest, canonical_digest},
    error::{Error, Result},
    execution::{
        ExecutionBinding, ExecutionLimits, HookInvocation, HookLedger, HookPhase, LocalExecutor,
        OwnedDirectory,
    },
    git::Git,
    isolation::IsolationPlan,
    platform::{
        BinaryIdentity, HostIdentity, OwnerRecord, ProcessIdentity, ProcessObservation,
        ProcessProbe, ServiceShutdown,
    },
    providers::{GithubClient, RepositoryIdentity},
    storage::PrivateDir,
    workflow::Workflow,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            ffi::OsStrExt,
            fs::{MetadataExt, OpenOptionsExt},
        },
    },
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};
use uuid::Uuid;

#[path = "housekeeping_families.rs"]
mod families;
pub use families::{EnrolledDirectoryReuse, VerifiedDirectoryReuse};
#[path = "housekeeping_hooks.rs"]
mod hook_locations;

const LOG_BYTES: u64 = 10 * 1024 * 1024;
const LOG_FILES: usize = 10;
const MAX_FILES: usize = 100_000;
const MAX_TREE_BYTES: u64 = 1024 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Debug,
    Info,
    Warning,
    Error,
}
/// No arbitrary provider JSON/prompt payload field is accepted by the log API.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogEvent {
    pub severity: Severity,
    pub runtime_id: Uuid,
    pub owner_generation: Option<u64>,
    pub workspace_id: Option<Uuid>,
    pub work: Option<String>,
    pub operation_id: Option<Uuid>,
    pub session_id: Option<Uuid>,
    pub revision: Option<String>,
    pub event: String,
    pub outcome: String,
    pub reason: Option<String>,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct LogHealth {
    pub failed_events: u64,
    pub last_error: Option<String>,
    pub last_failure: Option<DateTime<Utc>>,
    pub last_success: Option<DateTime<Utc>>,
}
pub struct LogSink {
    directory: PrivateDir,
    state: Mutex<LogHealth>,
    redactions: Mutex<Vec<Vec<u8>>>,
    limit: u64,
}
impl LogSink {
    pub fn new(directory: PrivateDir) -> Self {
        Self {
            directory,
            state: Mutex::new(LogHealth::default()),
            redactions: Mutex::new(vec![]),
            limit: LOG_BYTES,
        }
    }
    /// Operators may choose a smaller bound; the specified maximum is immutable.
    pub fn with_limit(directory: PrivateDir, maximum: u64) -> Result<Self> {
        if !(1024..=LOG_BYTES).contains(&maximum) {
            return Err(Error::invalid(
                "diagnostic file limit must be 1KiB through 10MiB",
            ));
        }
        let mut sink = Self::new(directory);
        sink.limit = maximum;
        Ok(sink)
    }
    pub fn register_secret(&self, secret: &crate::secrets::Secret) -> Result<()> {
        self.redactions
            .lock()
            .map_err(|_| Error::new("log_unavailable", "redaction state unavailable"))?
            .push(secret.bytes().to_vec());
        Ok(())
    }
    pub fn health(&self) -> LogHealth {
        self.state
            .lock()
            .map(|s| s.clone())
            .unwrap_or_else(|_| LogHealth {
                failed_events: 1,
                last_error: Some("log_state_unavailable".into()),
                last_failure: Some(Utc::now()),
                last_success: None,
            })
    }
    /// Logging failure is observable through health, never a scheduling panic.
    pub fn emit(&self, event: &LogEvent) -> bool {
        let Ok(mut health) = self.state.lock() else {
            return false;
        };
        let result = self.write(event);
        match result {
            Ok(()) => {
                health.last_success = Some(Utc::now());
                true
            }
            Err(error) => {
                health.failed_events = health.failed_events.saturating_add(1);
                health.last_error = Some(error.code);
                health.last_failure = Some(Utc::now());
                false
            }
        }
    }
    fn write(&self, event: &LogEvent) -> Result<()> {
        if event.runtime_id.is_nil() {
            return Err(Error::invalid("log event requires runtime identity"));
        }
        let secrets = self
            .redactions
            .lock()
            .map_err(|_| Error::new("log_unavailable", "redaction state unavailable"))?;
        let clean = |text: &str, max: usize| redact(text, &secrets, max);
        let value = json!({"at":Utc::now(),"severity":event.severity,"runtime_id":event.runtime_id,"owner_generation":event.owner_generation,"workspace_id":event.workspace_id,"work":event.work.as_ref().map(|s|clean(s,256)),"operation_id":event.operation_id,"session_id":event.session_id,"revision":event.revision.as_ref().map(|s|clean(s,256)),"event":clean(&event.event,128),"outcome":clean(&event.outcome,128),"reason":event.reason.as_ref().map(|s|clean(s,1024))});
        let mut bytes =
            serde_json::to_vec(&value).map_err(|_| Error::invalid("log event encoding failed"))?;
        bytes.push(b'\n');
        if bytes.len() as u64 > self.limit {
            return Err(Error::new(
                "log_event_too_large",
                "redacted event exceeds the configured file bound",
            ));
        }
        // Section guards explicitly unlock at scope exit, including across
        // incidental fork inheritance. Bound contention with actual concurrent
        // writers instead of losing an event or blocking scheduling indefinitely.
        let deadline = std::time::Instant::now() + Duration::from_millis(100);
        let _lock = loop {
            match self.directory.section_lock("log.lock") {
                Ok(lock) => break lock,
                Err(error)
                    if error.code == "ownership_conflict"
                        && std::time::Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(1))
                }
                Err(error) => return Err(error),
            }
        };
        let current = PrivateDir::open(self.directory.path())?;
        let path = current.path().join("events.jsonl");
        let mut file = private_append(&path)?;
        if file
            .metadata()
            .map_err(|e| Error::io("inspect log length", e))?
            .len()
            .saturating_add(bytes.len() as u64)
            > self.limit
        {
            drop(file);
            for index in (1..LOG_FILES).rev() {
                let source = current.path().join(if index == 1 {
                    "events.jsonl".into()
                } else {
                    format!("events.{}.jsonl", index - 1)
                });
                let destination = current.path().join(format!("events.{index}.jsonl"));
                match OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
                    .open(&destination)
                {
                    Ok(existing) => {
                        validate_file(&existing, true)?;
                        if index == LOG_FILES - 1 {
                            std::fs::remove_file(&destination)
                                .map_err(|e| Error::io("rotate oldest diagnostic log", e))?;
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => {
                        return Err(Error::io("inspect existing rotated diagnostic", error));
                    }
                }
                match OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
                    .open(&source)
                {
                    Ok(existing) => {
                        validate_file(&existing, true)?;
                        std::fs::rename(&source, &destination)
                            .map_err(|e| Error::io("rotate diagnostic log", e))?;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(Error::io("inspect rotating diagnostic log", e)),
                }
            }
            file = private_append(&path)?;
        }
        file.write_all(&bytes)
            .map_err(|e| Error::io("write structured diagnostic", e))
    }
}
impl Drop for LogSink {
    fn drop(&mut self) {
        if let Ok(values) = self.redactions.get_mut() {
            for bytes in values {
                for byte in bytes {
                    unsafe {
                        std::ptr::write_volatile(byte, 0);
                    }
                }
            }
        }
    }
}
fn redact(text: &str, secrets: &[Vec<u8>], maximum: usize) -> String {
    let mut value = text.replace(['\r', '\n', '\0'], " ");
    for secret in secrets {
        if let Ok(secret) = std::str::from_utf8(secret)
            && !secret.is_empty()
        {
            value = value.replace(secret, "[REDACTED]");
        }
    }
    let lower = value.to_ascii_lowercase();
    if [
        "bearer ",
        "authorization:",
        "api_key=",
        "apikey=",
        "access_token=",
        "refresh_token=",
        "password=",
        "secret=",
        "-----begin private key",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
    {
        return "[REDACTED credential-bearing diagnostic]".into();
    }
    if value.len() > maximum {
        let mut end = maximum;
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        value.truncate(end);
        value.push('…');
    }
    value
}
fn private_append(path: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| Error::io("open structured diagnostic", e))?;
    validate_file(&file, true)?;
    Ok(file)
}
fn validate_file(file: &File, private: bool) -> Result<()> {
    let m = file
        .metadata()
        .map_err(|e| Error::io("inspect retained file", e))?;
    if !m.is_file()
        || m.nlink() != 1
        || m.uid() != unsafe { libc::geteuid() }
        || m.mode() & if private { 0o077 } else { 0o022 } != 0
    {
        return Err(Error::new(
            "unsafe_storage",
            "file is not owned, regular, unlinked and permission bounded",
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublishedReference {
    pub reference: String,
    pub commit: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetentionLocation {
    /// A complete owned snapshot, including Git objects, outside execution roots.
    pub directory: PathBuf,
    pub repository: RepositoryIdentity,
    /// Actual provider reads must reproduce every local branch/HEAD below.
    pub published_refs: Vec<PublishedReference>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExecutionRegistration {
    schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    family_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hook_location_digest: Option<String>,
    binding: ExecutionBinding,
    isolation: IsolationPlan,
    marker: PathBuf,
    directory_digest: String,
    workflow_path: PathBuf,
    workflow_digest: String,
    processes: Vec<ProcessIdentity>,
    retention: Option<RetentionLocation>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetainedFile {
    pub directory: bool,
    pub relative: PathBuf,
    pub bytes: u64,
    pub digest: String,
    pub mode: u32,
    pub device: u64,
    pub inode: u64,
}
#[derive(Clone, Debug, Serialize)]
pub struct CleanupPlan {
    pub schema_version: u32,
    pub runtime_id: Uuid,
    pub operation_id: Uuid,
    pub source: PathBuf,
    pub retained: PathBuf,
    pub registration_digest: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub directory_family_digest: Option<String>,
    pub inventory_digest: String,
    pub receipt_digest: String,
    pub workflow_digest: String,
    pub files: Vec<RetainedFile>,
    pub published_refs: Vec<PublishedReference>,
    pub token: String,
}
impl CleanupPlan {
    pub fn digest(&self) -> &str {
        &self.token
    }
}
/// Opaque authority for one exact explicit cleanup. Display plans and receipt
/// JSON cannot construct this capability or authorize a hook/removal.
pub struct VerifiedCleanupExecution {
    plan: CleanupPlan,
    receipt: Receipt,
    token: Arc<crate::control::HostProcessToken>,
    control: Arc<ControlHost>,
    actor: Actor,
    current_actor: CurrentOperator,
    generation: u64,
    parent_command: Option<Uuid>,
    directory: PrivateDir,
    housekeeping: Arc<Housekeeping>,
}
impl VerifiedCleanupExecution {
    pub fn plan(&self) -> &CleanupPlan {
        &self.plan
    }
    pub fn receipt(&self) -> &Receipt {
        &self.receipt
    }
    pub(crate) fn command_identity(
        &self,
    ) -> Result<(crate::isolation::ValidatedIsolation, Vec<String>, Duration)> {
        let registration: ExecutionRegistration = self
            .directory
            .read(&format!("{}.json", self.plan.operation_id))?;
        if canonical_digest(1, &registration)? != self.plan.registration_digest {
            return Err(Error::new(
                "stale_cleanup_plan",
                "cleanup command registration changed",
            ));
        }
        let workflow = Workflow::load(&registration.workflow_path)?;
        if workflow.digest != self.plan.workflow_digest {
            return Err(Error::new(
                "stale_config",
                "cleanup command workflow changed",
            ));
        }
        let script = workflow
            .front_matter
            .hooks
            .before_remove
            .as_ref()
            .ok_or_else(|| Error::new("scope_violation", "sealed cleanup has no removal hook"))?;
        let isolation = registration.isolation.validate()?;
        OwnedDirectory::reopen(
            &registration.marker,
            &registration.directory_digest,
            &isolation,
        )?;
        Ok((
            isolation,
            vec!["/bin/sh".into(), "-c".into(), script.clone()],
            Duration::from_millis(workflow.front_matter.hooks.timeout_ms),
        ))
    }
    pub(crate) fn token(&self) -> Arc<crate::control::HostProcessToken> {
        self.token.clone()
    }
    pub(crate) fn validate_snapshot(&self, state: &ControlSnapshot) -> Result<()> {
        (self.current_actor)(&self.actor)?;
        if state.intent.generation != self.generation
            || self
                .parent_command
                .is_some_and(|id| !state.background_commands.contains(&id))
        {
            return Err(Error::new(
                "stale_generation",
                "cleanup authority or command lineage changed",
            ));
        }
        cleanup_control_clear(state, Some(self.receipt.operation_id), self.parent_command)?;
        let registration = self.housekeeping.registration(self.plan.operation_id)?;
        let family = self.housekeeping.directory_family(&registration)?;
        let members = family
            .as_ref()
            .map(|f| f.registrations.clone())
            .unwrap_or_else(|| {
                BTreeMap::from([(registration.binding.operation_id, registration.clone())])
            });
        if canonical_digest(1, &registration)? != self.plan.registration_digest
            || family.as_ref().map(|f| &f.digest) != self.plan.directory_family_digest.as_ref()
            || self
                .housekeeping
                .cleanup_observations(&registration, &members, Some(self))?
                != self.plan.receipt_digest
        {
            return Err(Error::new(
                "stale_cleanup_plan",
                "directory family changed after cleanup authorization",
            ));
        }
        Ok(())
    }
    fn apply_effect(&self, effect: impl FnOnce() -> Result<()>) -> Result<()> {
        self.control
            .with_host_process(&self.token, |state| self.validate_snapshot(state), effect)
    }
    pub(crate) fn validate_current(&self) -> Result<()> {
        self.control.with_host_process(
            &self.token,
            |state| self.validate_snapshot(state),
            || Ok(()),
        )
    }
}
fn cleanup_control_clear(
    control: &ControlSnapshot,
    cleanup_request: Option<Uuid>,
    parent_command: Option<Uuid>,
) -> Result<()> {
    if !control.active_operations.is_empty()
        || !control.pending_external_effects.is_empty()
        || control
            .unresolved_operations
            .keys()
            .any(|id| Some(*id) != cleanup_request)
        || control
            .pending_local_processes
            .iter()
            .any(|id| Some(*id) != cleanup_request)
        || control
            .background_commands
            .iter()
            .any(|id| Some(*id) != parent_command)
        || (control.intent.desired == crate::control::Intent::Stopped
            && control.orchestration_alive)
        || control.service_stopping
        || control.persistence_error.is_some()
    {
        return Err(Error::new(
            "cleanup_blocked",
            "active, queued, uncertain or draining work must be reconciled before cleanup",
        ));
    }
    Ok(())
}
/// Read authority derived from a protected cleanup registration and a verified
/// complete retention snapshot. A public display plan cannot mint this proof.
#[derive(Clone)]
pub struct VerifiedCleanupReadRoot {
    directory: Arc<PrivateDir>,
    registration: ExecutionRegistration,
    registration_digest: String,
    root: PathBuf,
    identity: (u64, u64, u32, u32),
}
impl VerifiedCleanupReadRoot {
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn repository(&self) -> &crate::git::RepositoryId {
        &self.registration.binding.repository
    }
    pub fn runtime_id(&self) -> Uuid {
        self.registration.binding.runtime_id
    }
    pub fn operation_id(&self) -> Uuid {
        self.registration.binding.operation_id
    }
    pub fn digest(&self) -> Result<String> {
        canonical_digest(
            1,
            &json!({"registration":self.registration_digest,"root":self.root,"identity":self.identity}),
        )
    }
    pub fn revalidate_filesystem(&self) -> Result<()> {
        let actual: ExecutionRegistration = self
            .directory
            .read(&format!("{}.json", self.operation_id()))?;
        if canonical_digest(1, &actual)? != self.registration_digest {
            return Err(Error::new(
                "stale_cleanup_plan",
                "cleanup registration changed",
            ));
        }
        let boundary = actual.isolation.validate()?;
        OwnedDirectory::reopen(&actual.marker, &actual.directory_digest, &boundary)?;
        let dir = PrivateDir::open(&self.root)?;
        let meta = std::fs::symlink_metadata(dir.path())
            .map_err(|e| Error::io("inspect cleanup Git root", e))?;
        if (meta.dev(), meta.ino(), meta.uid(), meta.mode()) != self.identity {
            return Err(Error::new(
                "ownership_conflict",
                "cleanup Git root was replaced",
            ));
        }
        Ok(())
    }
}
pub type CleanupFuture<'a, T> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<T>> + Send + 'a>>;
/// A trusted host adapter. Production uses NativeCleanupProcesses; fixture
/// adapters are explicit and cannot issue host-local process capabilities.
pub trait CleanupProcesses: Send + Sync {
    fn read<'a>(
        &'a self,
        root: VerifiedCleanupReadRoot,
        query: crate::git::CleanupRead,
        cleanup: Option<Arc<VerifiedCleanupExecution>>,
    ) -> CleanupFuture<'a, String>;
    fn before_remove<'a>(&'a self, request: CleanupHook<'a>) -> CleanupFuture<'a, ()>;
}
pub struct CleanupHook<'a> {
    pub proof: Arc<VerifiedCleanupExecution>,
    pub root: VerifiedCleanupReadRoot,
    pub binding: &'a ExecutionBinding,
    pub isolation: &'a crate::isolation::ValidatedIsolation,
    pub ownership: &'a OwnedDirectory,
    pub workflow: &'a Workflow,
    pub directory: PrivateDir,
    pub ledger_name: String,
}
#[derive(Clone)]
pub struct Housekeeping {
    root: Arc<PrivateDir>,
    control: Arc<ControlHost>,
    probe: Arc<dyn ProcessProbe>,
    processes: Arc<OnceLock<Arc<dyn CleanupProcesses>>>,
    git_executable: PathBuf,
}
pub type CurrentOperator = Arc<dyn Fn(&Actor) -> Result<()> + Send + Sync>;
impl Housekeeping {
    pub fn new(
        root: PrivateDir,
        control: Arc<ControlHost>,
        probe: Arc<dyn ProcessProbe>,
        git_executable: PathBuf,
    ) -> Result<Self> {
        Git::from_pinned_executable(&git_executable)?;
        root.child("cleanup")?;
        Ok(Self {
            root: Arc::new(root),
            control,
            probe,
            processes: Arc::new(OnceLock::new()),
            git_executable,
        })
    }
    pub(crate) fn git_executable(&self) -> &Path {
        &self.git_executable
    }
    pub fn install_processes(&self, processes: Arc<dyn CleanupProcesses>) -> Result<()> {
        self.processes.set(processes).map_err(|_| {
            Error::new(
                "authority_already_bound",
                "cleanup process authority cannot be replaced",
            )
        })
    }
    pub fn with_processes(self, processes: Arc<dyn CleanupProcesses>) -> Result<Self> {
        self.install_processes(processes)?;
        Ok(self)
    }
    fn read_root(
        &self,
        registration: &ExecutionRegistration,
        path: &Path,
    ) -> Result<VerifiedCleanupReadRoot> {
        if path != registration.isolation.repository
            && registration
                .retention
                .as_ref()
                .is_none_or(|r| r.directory != path)
        {
            return Err(Error::new(
                "scope_violation",
                "cleanup read root is not a registered source or retention copy",
            ));
        }
        let directory = PrivateDir::open(path)?;
        let meta = std::fs::symlink_metadata(directory.path())
            .map_err(|e| Error::io("inspect cleanup root identity", e))?;
        let proof = VerifiedCleanupReadRoot {
            directory: Arc::new(self.root.child("cleanup")?),
            registration: registration.clone(),
            registration_digest: canonical_digest(1, registration)?,
            root: path.into(),
            identity: (meta.dev(), meta.ino(), meta.uid(), meta.mode()),
        };
        proof.revalidate_filesystem()?;
        Ok(proof)
    }
    /// Called at actual host-owned directory creation, before any execution.
    pub fn register_execution(
        &self,
        binding: &ExecutionBinding,
        isolation: &crate::isolation::ValidatedIsolation,
        directory: &OwnedDirectory,
        workflow: &Workflow,
    ) -> Result<()> {
        binding.validate(isolation)?;
        let identity = directory.identity();
        if identity.runtime_id != binding.runtime_id
            || identity.work != binding.work
            || identity.repository != binding.repository
        {
            return Err(Error::new(
                "execution_ownership",
                "cleanup registration differs from owned execution",
            ));
        }
        OwnedDirectory::reopen(directory.marker(), directory.digest(), isolation)?;
        let storage = self.root.child("cleanup")?;
        let _registration_lock = storage.section_lock("families.lock")?;
        let name = format!("{}.json", binding.operation_id);
        match storage.read::<ExecutionRegistration>(&name) {
            Ok(old) => {
                if old.binding == *binding
                    && old.directory_digest == directory.digest()
                    && old.marker == directory.marker()
                    && old.workflow_path == workflow.path
                    && old.workflow_digest == workflow.digest
                    && canonical_digest(1, &old.isolation)?
                        == canonical_digest(1, isolation.plan())?
                {
                    self.directory_family(&old)?;
                    return Ok(());
                }
                return Err(Error::new(
                    "ownership_conflict",
                    "execution cleanup registration already differs",
                ));
            }
            Err(e) if e.code == "record_missing" => {}
            Err(e) => return Err(e),
        }
        let registration = ExecutionRegistration {
            schema_version: 2,
            family_id: Some(identity.directory_id),
            hook_location_digest: None,
            binding: binding.clone(),
            isolation: isolation.plan().clone(),
            marker: directory.marker().into(),
            directory_digest: directory.digest().into(),
            workflow_path: workflow.path.clone(),
            workflow_digest: workflow.digest.clone(),
            processes: vec![],
            retention: None,
        };
        self.reject_registration_alias(&registration)?;
        // An interrupted two-record write cannot produce a complete family.
        // Its exact creation can retry, while cleanup rejects missing members.
        self.create_directory_family(&registration, directory)?;
        storage.write(&name, &registration)
    }

    pub fn record_process(&self, operation: Uuid, process: ProcessIdentity) -> Result<()> {
        self.update_registration(operation, |registration| {
            if self.probe.observe(&process)? != ProcessObservation::Running {
                return Err(Error::new(
                    "ownership_conflict",
                    "cleanup process registration requires actual live owned process identity",
                ));
            }
            if !registration.processes.contains(&process) {
                registration.processes.push(process);
            }
            Ok(())
        })
    }
    /// Records only preservation references. Inspection still verifies every byte,
    /// process, receipt and provider reference; this method grants no eligibility.
    pub fn retain(&self, operation: Uuid, location: RetentionLocation) -> Result<()> {
        self.update_registration(operation, |registration| {
            validate_retained_path(registration, &location.directory)?;
            if registration.binding.repository.id != location.repository.id.to_string() {
                return Err(Error::new(
                    "identity_mismatch",
                    "retention repository differs",
                ));
            }
            registration.retention = Some(location);
            Ok(())
        })
    }
    // Serialize the entire read-modify-write with cleanup admission and other
    // registration updates. Callers must not already hold Control's mutex.
    fn update_registration(
        &self,
        operation: Uuid,
        update: impl FnOnce(&mut ExecutionRegistration) -> Result<()>,
    ) -> Result<()> {
        self.control.serialize_observation(|state| {
            self.registration_updates_allowed(state)?;
            let storage = self.root.child("cleanup")?;
            let _guard = storage.section_lock("families.lock")?;
            let mut registration = self.registration(operation)?;
            update(&mut registration)?;
            self.save(&registration)
        })
    }
    fn registration_updates_allowed(&self, state: &ControlSnapshot) -> Result<()> {
        for operation in state
            .pending_local_processes
            .iter()
            .chain(state.unresolved_operations.keys())
        {
            if self.control.operation(*operation)?.kind == "cleanup" {
                return Err(Error::new(
                    "cleanup_blocked",
                    "registration changes must wait for explicit cleanup reconciliation",
                ));
            }
        }
        Ok(())
    }
    pub fn record_hook_ledger(&self, operation: Uuid, ledger: &HookLedger) -> Result<()> {
        self.control.serialize_observation(|state| {
            self.registration_updates_allowed(state)?;
            let storage = self.root.child("cleanup")?;
            let _guard = storage.section_lock("families.lock")?;
            let registration = self.registration(operation)?;
            if ledger.binding_digest != canonical_digest(1, &registration.binding)? {
                return Err(Error::new(
                    "execution_identity",
                    "cleanup hook ledger differs from execution",
                ));
            }
            if self.hook_location(&registration)?.is_some() {
                if canonical_digest(1, ledger)?
                    != canonical_digest(1, &self.current_hook_ledger(&registration)?)?
                {
                    return Err(Error::new(
                        "ownership_conflict",
                        "copied ledger cannot replace actual worker hook history",
                    ));
                }
                return Ok(());
            }
            storage.write(&format!("hooks-{operation}.json"), ledger)
        })
    }
    fn registration(&self, operation: Uuid) -> Result<ExecutionRegistration> {
        let value: ExecutionRegistration = self
            .root
            .child("cleanup")?
            .read(&format!("{operation}.json"))?;
        if !matches!(value.schema_version, 1 | 2)
            || value.binding.operation_id != operation
            || (value.schema_version == 2) != value.family_id.is_some()
        {
            return Err(Error::new(
                "record_corrupt",
                "cleanup registration identity differs",
            ));
        }
        Ok(value)
    }
    fn save(&self, registration: &ExecutionRegistration) -> Result<()> {
        self.root.child("cleanup")?.write(
            &format!("{}.json", registration.binding.operation_id),
            registration,
        )
    }
    pub async fn inspect(&self, operation: Uuid, github: &GithubClient) -> Result<CleanupPlan> {
        let snapshot = self.control.snapshot()?;
        self.inspect_observed(operation, github, &snapshot, None, None, None)
            .await
    }
    async fn inspect_observed(
        &self,
        operation: Uuid,
        github: &GithubClient,
        control: &ControlSnapshot,
        cleanup_request: Option<Uuid>,
        parent_command: Option<Uuid>,
        cleanup: Option<Arc<VerifiedCleanupExecution>>,
    ) -> Result<CleanupPlan> {
        let registration = self.registration(operation)?;
        let family = self.directory_family(&registration)?;
        let members: Vec<_> = family
            .as_ref()
            .map(|f| f.registrations.values().collect())
            .unwrap_or_else(|| vec![&registration]);
        if control.intent.runtime_id != registration.binding.runtime_id {
            return Err(Error::new(
                "identity_mismatch",
                "cleanup runtime differs from registration",
            ));
        }
        cleanup_control_clear(control, cleanup_request, parent_command)?;
        for process in members.iter().flat_map(|member| &member.processes) {
            if self.probe.observe(process)? == ProcessObservation::Running {
                return Err(Error::new(
                    "cleanup_blocked",
                    "owned process still references execution files",
                ));
            }
        }
        let receipts = ReceiptStore::new(self.root.child("receipts")?).all()?;
        if receipts.iter().any(|r| {
            Some(r.request_id) != cleanup_request
                && Some(r.request_id) != parent_command
                && matches!(r.phase, ReceiptPhase::Prepared | ReceiptPhase::Sent)
        }) {
            return Err(Error::new(
                "cleanup_blocked",
                "nonterminal receipt preserves execution dependencies",
            ));
        }
        let receipt = receipts
            .iter()
            .find(|r| r.operation_id == operation)
            .ok_or_else(|| {
                Error::new(
                    "cleanup_blocked",
                    "execution has no retained terminal operation receipt",
                )
            })?;
        if receipt.runtime_id != registration.binding.runtime_id {
            return Err(Error::new(
                "identity_mismatch",
                "execution receipt runtime differs",
            ));
        }
        let member_map = members
            .iter()
            .map(|m| (m.binding.operation_id, (*m).clone()))
            .collect();
        let receipt_digest =
            self.cleanup_observations(&registration, &member_map, cleanup.as_deref())?;
        let isolation = registration.isolation.clone().validate()?;
        OwnedDirectory::reopen(
            &registration.marker,
            &registration.directory_digest,
            &isolation,
        )?;
        let workflow = Workflow::load(&registration.workflow_path)?;
        if workflow.digest != registration.workflow_digest {
            return Err(Error::new(
                "stale_config",
                "cleanup requires the exact recorded workflow, including its removal hook",
            ));
        }
        let retained = registration.retention.as_ref().ok_or_else(|| {
            Error::new(
                "cleanup_blocked",
                "no complete retained recovery copy is registered",
            )
        })?;
        validate_retained_path(&registration, &retained.directory)?;
        let source = &registration.isolation.repository;
        for path in [source.join(".git"), retained.directory.join(".git")] {
            if !std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir()) {
                return Err(Error::new(
                    "cleanup_blocked",
                    "cleanup requires a self-contained clone and retained Git object directory",
                ));
            }
        }
        let files = inventory(source)?;
        let preserved = inventory(&retained.directory)?;
        if content_inventory(&files) != content_inventory(&preserved) {
            return Err(Error::new(
                "cleanup_blocked",
                "retained recovery snapshot does not reproduce every local file and Git object",
            ));
        }
        let status = self
            .git_read(
                self.read_root(&registration, source)?,
                crate::git::CleanupRead::Status,
                cleanup.clone(),
            )
            .await?;
        if !status.is_empty() {
            return Err(Error::new(
                "cleanup_blocked",
                "dirty or untracked source work must be preserved",
            ));
        }
        let heads = self
            .git_read(
                self.read_root(&registration, source)?,
                crate::git::CleanupRead::Heads,
                cleanup.clone(),
            )
            .await?;
        let head = self
            .git_read(
                self.read_root(&registration, source)?,
                crate::git::CleanupRead::Head,
                cleanup.clone(),
            )
            .await?;
        let commits: BTreeSet<_> = heads
            .lines()
            .chain(head.lines())
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        if commits.is_empty() || retained.published_refs.is_empty() {
            return Err(Error::new(
                "cleanup_blocked",
                "local commits lack retained provider publication references",
            ));
        }
        let mut published = BTreeSet::new();
        for reference in &retained.published_refs {
            if github
                .reference(&retained.repository, &reference.reference)
                .await?
                != reference.commit
            {
                return Err(Error::new(
                    "cleanup_blocked",
                    "provider publication reference changed",
                ));
            }
            published.insert(reference.commit.clone());
        }
        if !commits.is_subset(&published) {
            return Err(Error::new(
                "cleanup_blocked",
                "local branch or detached commit has not been verified published",
            ));
        }
        let fsck = self
            .git_read(
                self.read_root(&registration, source)?,
                crate::git::CleanupRead::Fsck,
                cleanup.clone(),
            )
            .await?;
        if fsck.lines().any(|line| {
            line.starts_with("dangling commit ") || line.starts_with("unreachable commit ")
        }) {
            return Err(Error::new(
                "cleanup_blocked",
                "unreferenced commit history must be published or explicitly reconciled before cleanup",
            ));
        }
        self.git_read(
            self.read_root(&registration, &retained.directory)?,
            crate::git::CleanupRead::Fsck,
            cleanup,
        )
        .await?;
        let mut plan = CleanupPlan {
            schema_version: 1,
            runtime_id: registration.binding.runtime_id,
            operation_id: operation,
            source: source.clone(),
            retained: retained.directory.clone(),
            registration_digest: canonical_digest(1, &registration)?,
            directory_family_digest: family.as_ref().map(|f| f.digest.clone()),
            inventory_digest: canonical_digest(1, &files)?,
            receipt_digest,
            workflow_digest: workflow.digest,
            files,
            published_refs: retained.published_refs.clone(),
            token: String::new(),
        };
        plan.token = canonical_digest(1, &plan)?;
        Ok(plan)
    }
    /// The caller's role callback consults the current host configuration while
    /// ControlHost holds its mutation mutex; cached role booleans are insufficient.
    pub async fn apply(
        self: Arc<Self>,
        request: MutationRequest,
        operation: Uuid,
        token: String,
        github: GithubClient,
        current_actor: CurrentOperator,
    ) -> Result<Receipt> {
        self.apply_internal(request, operation, token, github, current_actor, None)
            .await
    }
    pub async fn apply_for_command(
        self: Arc<Self>,
        request: MutationRequest,
        operation: Uuid,
        token: String,
        github: GithubClient,
        current_actor: CurrentOperator,
        parent: Receipt,
    ) -> Result<Receipt> {
        self.apply_internal(
            request,
            operation,
            token,
            github,
            current_actor,
            Some(parent),
        )
        .await
    }
    async fn apply_internal(
        self: Arc<Self>,
        mut request: MutationRequest,
        operation: Uuid,
        token: String,
        github: GithubClient,
        current_actor: CurrentOperator,
        parent: Option<Receipt>,
    ) -> Result<Receipt> {
        current_actor(&request.actor)?;
        let parent_command = if let Some(parent) = parent {
            let recorded = self.control.operation(parent.request_id)?;
            let expected: BTreeMap<_, _> = request
                .expected
                .iter()
                .filter(|(key, _)| {
                    ["config_revision", "control_generation"].contains(&key.as_str())
                })
                .collect();
            let digest = canonical_digest(
                1,
                &json!({"kind":"command/Cleanup","targets":recorded.canonical_targets,"expected":expected,"payload":{"arguments":request.payload,"secret_digest":null},"principal":request.actor.principal}),
            )?;
            if recorded.phase != ReceiptPhase::Prepared
                || recorded.kind != "command/Cleanup"
                || recorded.runtime_id != request.runtime_id
                || recorded.actor != request.actor.identity
                || recorded.payload_digest != digest
                || parent.payload_digest != digest
                || request.request_id != crate::app::command_effect_id(parent.request_id)
            {
                return Err(Error::new(
                    "request_conflict",
                    "cleanup effect differs from its durable background command lineage",
                ));
            }
            request
                .expected
                .insert("parent_command_id".into(), parent.request_id.to_string());
            request
                .expected
                .insert("parent_payload_digest".into(), digest);
            Some(parent.request_id)
        } else {
            None
        };
        let registration = self.registration(operation)?;
        request.kind = "cleanup".into();
        request.canonical_targets = vec![
            request.runtime_id.to_string(),
            operation.to_string(),
            registration
                .isolation
                .repository
                .to_string_lossy()
                .into_owned(),
        ];
        request
            .expected
            .insert("cleanup_plan".into(), token.clone());
        request
            .expected
            .insert("source_operation".into(), operation.to_string());
        match self.control.operation(request.request_id) {
            Ok(_) => {
                let control = self.control.clone();
                return tokio::task::spawn_blocking(move || {
                    control.mutate_observed(
                        request,
                        None,
                        Some("recover"),
                        |_| {
                            Err(Error::new(
                                "reconciliation_required",
                                "known cleanup cannot repeat",
                            ))
                        },
                        |_| {
                            Err(Error::new(
                                "reconciliation_required",
                                "known cleanup cannot repeat",
                            ))
                        },
                    )
                })
                .await
                .map_err(|_| Error::new("control_failed", "cleanup replay failed"))?;
            }
            Err(error) if error.code == "record_missing" => {}
            Err(error) => return Err(error),
        }
        let initial = self
            .inspect_observed(
                operation,
                &github,
                &self.control.snapshot()?,
                None,
                parent_command,
                None,
            )
            .await?;
        if initial.token != token {
            return Err(Error::new(
                "stale_cleanup_plan",
                "cleanup plan differs from current exact eligibility",
            ));
        }
        if Workflow::load(&registration.workflow_path)?
            .front_matter
            .hooks
            .before_remove
            .is_some()
        {
            crate::isolation::repository_command_prerequisite(&registration.isolation.validate()?)?;
        }
        let actor = request.actor.clone();
        let state = self.control.snapshot()?;
        let generation = state.intent.generation;
        if request
            .expected
            .get("control_generation")
            .map(|value| {
                value
                    .parse::<u64>()
                    .map_err(|_| Error::invalid("expected control generation is malformed"))
            })
            .transpose()?
            .is_some_and(|expected| expected != generation)
        {
            return Err(Error::new(
                "stale_generation",
                "control intent changed before cleanup",
            ));
        }
        let (owned, receipt) = self.control.begin_cleanup_effect(request, |state| {
            current_actor(&actor)?;
            if state.intent.generation != generation
                || parent_command.is_some_and(|id| !state.background_commands.contains(&id))
            {
                return Err(Error::new(
                    "stale_generation",
                    "cleanup control intent or command lineage changed",
                ));
            }
            cleanup_control_clear(state, None, parent_command)?;
            let current = self.registration(operation)?;
            let family = self.directory_family(&current)?;
            let members = family
                .as_ref()
                .map(|f| f.registrations.clone())
                .unwrap_or_else(|| BTreeMap::from([(operation, current.clone())]));
            if canonical_digest(1, &current)? != initial.registration_digest
                || Workflow::load(&current.workflow_path)?.digest != initial.workflow_digest
                || family.as_ref().map(|f| &f.digest) != initial.directory_family_digest.as_ref()
                || self.cleanup_observations(&current, &members, None)? != initial.receipt_digest
            {
                return Err(Error::new(
                    "stale_cleanup_plan",
                    "cleanup source registration or workflow changed before admission",
                ));
            }
            Ok(())
        })?;
        let proof = Arc::new(VerifiedCleanupExecution {
            plan: initial,
            receipt,
            token: Arc::new(owned),
            control: self.control.clone(),
            actor,
            current_actor,
            generation,
            parent_command,
            directory: self.root.child("cleanup")?,
            housekeeping: self.clone(),
        });
        proof.validate_current()?;
        let registration = self.registration(operation)?;
        let workflow = Workflow::load(&registration.workflow_path)?;
        if workflow.front_matter.hooks.before_remove.is_some() {
            self.before_remove(&registration, &workflow, proof.clone())
                .await?;
        }
        proof.validate_current()?;
        let fresh = self
            .inspect_observed(
                operation,
                &github,
                &self.control.snapshot()?,
                Some(proof.receipt.operation_id),
                parent_command,
                Some(proof.clone()),
            )
            .await?;
        if fresh.token != token {
            return Err(Error::new(
                "stale_cleanup_plan",
                "hook or intervening observation changed preservation predicates",
            ));
        }
        proof.validate_current()?;
        let cleanup = self.clone();
        let action = proof.clone();
        let observation = tokio::task::spawn_blocking(move || {
            action.validate_current()?;
            let registration = cleanup.registration(operation)?;
            let members: Vec<_> = cleanup.directory_family(&registration)?.map(|family| family.registrations.into_keys().collect()).unwrap_or_else(|| vec![operation]);
            let directory = cleanup.root.child("cleanup")?;
            let content_digest = canonical_digest(1, &content_inventory(&fresh.files))?;
            for member in &members {
                let tombstone = CleanupTombstone { schema_version: 1, runtime_id: fresh.runtime_id, operation_id: *member, cleanup_request: action.receipt.operation_id, plan_token: token.clone(), retained: fresh.retained.clone(), content_digest: content_digest.clone(), removed: false };
                directory.write(&format!("retained-{member}.json"), &tombstone)?;
            }
            remove_checked_tree(&fresh.source, &fresh.files, &action)?;
            for member in &members {
                let tombstone = CleanupTombstone { schema_version: 1, runtime_id: fresh.runtime_id, operation_id: *member, cleanup_request: action.receipt.operation_id, plan_token: token.clone(), retained: fresh.retained.clone(), content_digest: content_digest.clone(), removed: true };
                directory.write(&format!("retained-{member}.json"), &tombstone)?;
            }
            Ok::<_, Error>(json!({"cleanup_plan":token,"source_operation":operation,"family_operations":members,"removed_path":fresh.source,"retained_recovery":fresh.retained}))
        }).await.map_err(|_| Error::new("operation_uncertain", "cleanup task ended without a verified completion"))??;
        self.control
            .complete_host_process(&proof.token, || Ok(()), observation)
    }
    async fn before_remove(
        &self,
        registration: &ExecutionRegistration,
        workflow: &Workflow,
        proof: Arc<VerifiedCleanupExecution>,
    ) -> Result<()> {
        proof.validate_current()?;
        let boundary = registration.isolation.clone().validate()?;
        let ownership = OwnedDirectory::reopen(
            &registration.marker,
            &registration.directory_digest,
            &boundary,
        )?;
        let (hook_directory, ledger_name) = self.hook_storage(registration)?;
        self.processes
            .get()
            .ok_or_else(|| {
                Error::new(
                    "process_authority_required",
                    "cleanup requires the native owned process adapter",
                )
            })?
            .before_remove(CleanupHook {
                proof,
                root: self.read_root(registration, &registration.isolation.repository)?,
                binding: &registration.binding,
                isolation: &boundary,
                ownership: &ownership,
                workflow,
                directory: hook_directory,
                ledger_name,
            })
            .await
    }
    /// Recovery/correction callers resolve an explicitly removed operation to its
    /// complete retained copy, checking its content before using the new path.
    pub fn retained_directory(&self, operation: Uuid) -> Result<PathBuf> {
        let record: CleanupTombstone = self
            .root
            .child("cleanup")?
            .read(&format!("retained-{operation}.json"))?;
        if record.schema_version != 1
            || record.operation_id != operation
            || record.runtime_id != self.control.snapshot()?.intent.runtime_id
        {
            return Err(Error::new(
                "record_corrupt",
                "retained cleanup identity differs",
            ));
        }
        let files = inventory(&record.retained)?;
        if canonical_digest(1, &content_inventory(&files))? != record.content_digest {
            return Err(Error::new(
                "artifact_conflict",
                "retained recovery copy changed",
            ));
        }
        Ok(record.retained)
    }
    async fn git_read(
        &self,
        root: VerifiedCleanupReadRoot,
        query: crate::git::CleanupRead,
        cleanup: Option<Arc<VerifiedCleanupExecution>>,
    ) -> Result<String> {
        self.processes
            .get()
            .ok_or_else(|| {
                Error::new(
                    "process_authority_required",
                    "cleanup requires a journaled host process adapter",
                )
            })?
            .read(root, query, cleanup)
            .await
    }
}

/// Actual control-owned cleanup adapter. All metadata and hook starts retain the
/// same process journal and cancellation registry used by the runtime driver.
pub struct NativeCleanupProcesses {
    services: crate::host_process::HostProcessServices,
    git: Git,
    binary: BinaryIdentity,
}
impl NativeCleanupProcesses {
    pub fn for_housekeeping(
        services: crate::host_process::HostProcessServices,
        housekeeping: &Housekeeping,
    ) -> Result<Self> {
        Self::new(services, housekeeping.git_executable())
    }
    pub fn new(
        services: crate::host_process::HostProcessServices,
        executable: &Path,
    ) -> Result<Self> {
        Ok(Self {
            services,
            git: Git::from_pinned_executable(executable)?,
            binary: BinaryIdentity::inspect(executable, true)?,
        })
    }
    fn scope(
        &self,
        root: VerifiedCleanupReadRoot,
        cleanup: Option<Arc<VerifiedCleanupExecution>>,
    ) -> Result<Arc<crate::host_process::LocalProcessPermit>> {
        if let Some(cleanup) = cleanup {
            crate::host_process::LocalProcessPermit::cleanup(
                self.services.clone(),
                cleanup,
                root,
                self.binary.clone(),
            )
        } else {
            let selected = crate::git::cleanup_host_read_root(
                &root,
                &self.services.configuration.snapshot()?,
            )?;
            crate::host_process::LocalProcessPermit::metadata(
                self.services.clone(),
                crate::host_process::MetadataRequest {
                    request_id: Uuid::new_v4(),
                    work_key: format!("cleanup-read:{}", root.operation_id()),
                    roots: vec![selected],
                    git_binary: self.binary.clone(),
                },
            )
        }
    }
}
impl CleanupProcesses for NativeCleanupProcesses {
    fn read<'a>(
        &'a self,
        root: VerifiedCleanupReadRoot,
        query: crate::git::CleanupRead,
        cleanup: Option<Arc<VerifiedCleanupExecution>>,
    ) -> CleanupFuture<'a, String> {
        Box::pin(async move {
            let authority = self.scope(root.clone(), cleanup)?;
            let git = self.git.clone();
            let scope = authority.clone();
            let observed = tokio::task::spawn_blocking(move || {
                let git = git.with_host_execution_scope(crate::git::HostGitExecutionScope {
                    limits: crate::git::GitLimits::default(),
                    authority: scope,
                })?;
                git.cleanup_read(root.root(), query)
            })
            .await
            .map_err(|_| {
                Error::new(
                    "operation_uncertain",
                    "cleanup Git task ended without joining its owned children",
                )
            })?;
            authority
                .finish(json!({"cleanup_read":format!("{query:?}"),"success":observed.is_ok()}))?;
            observed
        })
    }
    fn before_remove<'a>(&'a self, request: CleanupHook<'a>) -> CleanupFuture<'a, ()> {
        Box::pin(async move {
            let (expected, _, _) = request.proof.command_identity()?;
            if expected.digest() != request.isolation.digest() {
                return Err(Error::new(
                    "scope_violation",
                    "removal hook isolation differs from its retained execution",
                ));
            }
            crate::isolation::repository_command_prerequisite(&expected)?;
            let authority = self.scope(request.root, Some(request.proof.clone()))?;
            let git = self.git.clone();
            let scope = authority.clone();
            let git = tokio::task::spawn_blocking(move || {
                git.with_host_execution_scope(crate::git::HostGitExecutionScope {
                    limits: crate::git::GitLimits::default(),
                    authority: scope,
                })
            })
            .await
            .map_err(|_| {
                Error::new("operation_uncertain", "cleanup Git preparation task ended")
            })??;
            let boundary = CleanupCommandBoundary {
                authority: authority.clone(),
            };
            let executor = LocalExecutor {
                isolation: request.isolation,
                git: &git,
                boundary: &boundary,
                limits: ExecutionLimits {
                    output_bytes: 16 * 1024,
                    ..Default::default()
                },
                ownership: request.ownership,
            };
            let name = &request.ledger_name;
            let mut ledger: HookLedger = request.directory.read(name)?;
            let outcome = executor
                .run_hook(
                    &mut ledger,
                    HookInvocation {
                        binding: request.binding,
                        workflow: request.workflow,
                        phase: HookPhase::BeforeRemove,
                        session: None,
                        eligible_cleanup_plan: Some(request.proof.plan().digest()),
                    },
                    |next| request.directory.write(name, next),
                )
                .await;
            authority.finish(json!({"before_remove_observed":outcome.is_ok()}))?;
            let outcome = outcome?;
            if !outcome.allows_next_phase || outcome.diagnostic_only {
                return Err(Error::new(
                    "cleanup_blocked",
                    "before_remove hook did not succeed",
                ));
            }
            Ok(())
        })
    }
}
struct CleanupCommandBoundary {
    authority: Arc<crate::host_process::LocalProcessPermit>,
}
impl crate::execution::CommandBoundary for CleanupCommandBoundary {
    fn evidence_class(&self) -> crate::execution::ExecutionEvidenceClass {
        crate::execution::ExecutionEvidenceClass::ConfinedExecution
    }
    fn start(
        &self,
        isolation: &crate::isolation::ValidatedIsolation,
        argv: &[String],
        timeout: Duration,
        output_limit: usize,
    ) -> Result<Box<dyn crate::execution::StartedCommand>> {
        Ok(Box::new(self.authority.start_cleanup_command(
            isolation,
            argv,
            timeout,
            output_limit,
        )?))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CleanupTombstone {
    schema_version: u32,
    runtime_id: Uuid,
    operation_id: Uuid,
    cleanup_request: Uuid,
    plan_token: String,
    retained: PathBuf,
    content_digest: String,
    removed: bool,
}

fn remove_checked_tree(
    root: &Path,
    expected: &[RetainedFile],
    authority: &VerifiedCleanupExecution,
) -> Result<()> {
    authority.validate_current()?;
    if inventory(root)? != expected {
        return Err(Error::new(
            "stale_cleanup_plan",
            "execution inventory changed immediately before deletion",
        ));
    }
    let parent = root
        .parent()
        .ok_or_else(|| Error::invalid("cleanup root has no parent"))?;
    let leaf = root
        .file_name()
        .ok_or_else(|| Error::invalid("cleanup root has no name"))?;
    let parent_file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(parent)
        .map_err(|e| Error::io("open cleanup parent", e))?;
    let leaf = std::ffi::CString::new(leaf.as_bytes())
        .map_err(|_| Error::invalid("cleanup filename contains NUL"))?;
    // SAFETY: checked parent descriptor and NUL-terminated child name; no links.
    let fd = unsafe {
        libc::openat(
            parent_file.as_raw_fd(),
            leaf.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(Error::io(
            "open cleanup root",
            std::io::Error::last_os_error(),
        ));
    }
    let directory = unsafe { File::from_raw_fd(fd) };
    let metadata = directory
        .metadata()
        .map_err(|e| Error::io("inspect cleanup root", e))?;
    let root_proof = expected
        .iter()
        .find(|entry| entry.relative.as_os_str().is_empty() && entry.directory)
        .ok_or_else(|| Error::new("stale_cleanup_plan", "cleanup root identity absent"))?;
    if metadata.dev() != root_proof.device || metadata.ino() != root_proof.inode {
        return Err(Error::new(
            "ownership_conflict",
            "cleanup root changed before descriptor admission",
        ));
    }
    let mut entries = BTreeMap::new();
    for file in expected {
        entries.insert(file.relative.clone(), file);
    }
    unlink_children(&directory, Path::new(""), &entries, authority)?;
    let mut current_stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe {
        libc::fstatat(
            parent_file.as_raw_fd(),
            leaf.as_ptr(),
            current_stat.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } != 0
    {
        return Err(Error::io(
            "recheck cleanup root",
            std::io::Error::last_os_error(),
        ));
    }
    let current_metadata = unsafe { current_stat.assume_init() };
    if current_metadata.st_dev as u64 != metadata.dev() || current_metadata.st_ino != metadata.ino()
    {
        return Err(Error::new(
            "ownership_conflict",
            "cleanup root was replaced; retained remaining directory",
        ));
    }
    authority.apply_effect(|| {
        if unsafe { libc::unlinkat(parent_file.as_raw_fd(), leaf.as_ptr(), libc::AT_REMOVEDIR) }
            != 0
        {
            return Err(Error::io(
                "remove verified cleanup directory",
                std::io::Error::last_os_error(),
            ));
        }
        Ok(())
    })?;
    parent_file
        .sync_all()
        .map_err(|e| Error::io("persist cleanup removal", e))
}
fn unlink_children(
    directory: &File,
    prefix: &Path,
    expected: &BTreeMap<PathBuf, &RetainedFile>,
    authority: &VerifiedCleanupExecution,
) -> Result<()> {
    // fdopendir owns its duplicated descriptor; readdir names never escape this
    // parent because every open/unlink remains descriptor-relative and no-follow.
    let duplicate = unsafe { libc::dup(directory.as_raw_fd()) };
    if duplicate < 0 {
        return Err(Error::io(
            "duplicate cleanup directory",
            std::io::Error::last_os_error(),
        ));
    }
    let stream = unsafe { libc::fdopendir(duplicate) };
    if stream.is_null() {
        unsafe { libc::close(duplicate) };
        return Err(Error::io(
            "enumerate cleanup directory",
            std::io::Error::last_os_error(),
        ));
    }
    let result = (|| {
        loop {
            authority.validate_current()?;
            let entry = unsafe { libc::readdir(stream) };
            if entry.is_null() {
                break;
            }
            let name = unsafe { std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()) };
            if name.to_bytes() == b"." || name.to_bytes() == b".." {
                continue;
            }
            let relative = prefix.join(std::ffi::OsStr::from_bytes(name.to_bytes()));
            let mut st = std::mem::MaybeUninit::<libc::stat>::uninit();
            if unsafe {
                libc::fstatat(
                    directory.as_raw_fd(),
                    name.as_ptr(),
                    st.as_mut_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            } != 0
            {
                return Err(Error::io(
                    "recheck cleanup entry",
                    std::io::Error::last_os_error(),
                ));
            }
            let st = unsafe { st.assume_init() };
            if st.st_uid != unsafe { libc::geteuid() } {
                return Err(Error::new(
                    "ownership_conflict",
                    "cleanup entry owner changed",
                ));
            }
            if st.st_mode & libc::S_IFMT == libc::S_IFDIR {
                let proof = expected.get(&relative).ok_or_else(|| {
                    Error::new(
                        "stale_cleanup_plan",
                        "unplanned directory appeared during removal",
                    )
                })?;
                if !proof.directory || st.st_dev as u64 != proof.device || st.st_ino != proof.inode
                {
                    return Err(Error::new(
                        "stale_cleanup_plan",
                        "cleanup directory identity changed",
                    ));
                }
                let fd = unsafe {
                    libc::openat(
                        directory.as_raw_fd(),
                        name.as_ptr(),
                        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                    )
                };
                if fd < 0 {
                    return Err(Error::io(
                        "open cleanup child directory",
                        std::io::Error::last_os_error(),
                    ));
                }
                let child = unsafe { File::from_raw_fd(fd) };
                let identity = child
                    .metadata()
                    .map_err(|e| Error::io("recheck opened cleanup directory", e))?;
                if identity.dev() != proof.device || identity.ino() != proof.inode {
                    return Err(Error::new(
                        "ownership_conflict",
                        "cleanup child changed while opening",
                    ));
                }
                unlink_children(&child, &relative, expected, authority)?;
                let mut latest = std::mem::MaybeUninit::<libc::stat>::uninit();
                if unsafe {
                    libc::fstatat(
                        directory.as_raw_fd(),
                        name.as_ptr(),
                        latest.as_mut_ptr(),
                        libc::AT_SYMLINK_NOFOLLOW,
                    )
                } != 0
                {
                    return Err(Error::io(
                        "recheck emptied cleanup directory",
                        std::io::Error::last_os_error(),
                    ));
                }
                let latest = unsafe { latest.assume_init() };
                if latest.st_dev as u64 != proof.device || latest.st_ino != proof.inode {
                    return Err(Error::new(
                        "ownership_conflict",
                        "cleanup child replaced before unlink",
                    ));
                }
                authority.apply_effect(|| {
                    if unsafe {
                        libc::unlinkat(directory.as_raw_fd(), name.as_ptr(), libc::AT_REMOVEDIR)
                    } != 0
                    {
                        return Err(Error::io(
                            "remove cleanup child directory",
                            std::io::Error::last_os_error(),
                        ));
                    }
                    Ok(())
                })?;
            } else {
                let proof = expected.get(&relative).ok_or_else(|| {
                    Error::new(
                        "stale_cleanup_plan",
                        "unplanned file appeared during removal",
                    )
                })?;
                if proof.directory
                    || st.st_mode & libc::S_IFMT != libc::S_IFREG
                    || st.st_dev as u64 != proof.device
                    || st.st_ino != proof.inode
                    || st.st_size as u64 != proof.bytes
                    || st.st_nlink != 1
                    || u32::from(st.st_mode) & 0o7777 != proof.mode
                {
                    return Err(Error::new(
                        "stale_cleanup_plan",
                        "cleanup file identity changed; preserve remaining files",
                    ));
                }
                // Hash through the opened descriptor outside the control gate.
                // The short unlink gate then checks that neither the descriptor
                // nor its directory entry changed since that exact hash.
                let removal = PreparedFileRemoval::inspect(directory, name, proof)?;
                authority.apply_effect(|| removal.unlink(directory, name))?;
            }
        }
        Ok(())
    })();
    unsafe { libc::closedir(stream) };
    result
}

#[derive(PartialEq, Eq)]
struct CleanupFileState {
    device: u64,
    inode: u64,
    bytes: u64,
    mode: u32,
    uid: u32,
    links: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}
impl CleanupFileState {
    fn read(file: &File) -> Result<Self> {
        let metadata = file
            .metadata()
            .map_err(|e| Error::io("recheck cleanup file descriptor", e))?;
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            bytes: metadata.len(),
            mode: metadata.mode(),
            uid: metadata.uid(),
            links: metadata.nlink(),
            modified: (metadata.mtime(), metadata.mtime_nsec()),
            changed: (metadata.ctime(), metadata.ctime_nsec()),
        })
    }
}
struct PreparedFileRemoval {
    file: File,
    state: CleanupFileState,
}
impl PreparedFileRemoval {
    fn open(directory: &File, name: &std::ffi::CStr) -> Result<File> {
        let fd = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
            )
        };
        if fd < 0 {
            return Err(Error::io(
                "open cleanup file for final verification",
                std::io::Error::last_os_error(),
            ));
        }
        Ok(unsafe { File::from_raw_fd(fd) })
    }
    fn inspect(directory: &File, name: &std::ffi::CStr, proof: &RetainedFile) -> Result<Self> {
        let mut file = Self::open(directory, name)?;
        validate_file(&file, false)?;
        let state = CleanupFileState::read(&file)?;
        if proof.directory
            || state.device != proof.device
            || state.inode != proof.inode
            || state.bytes != proof.bytes
            || state.bytes > MAX_TREE_BYTES
            || state.mode & 0o7777 != proof.mode
        {
            return Err(Error::new(
                "stale_cleanup_plan",
                "cleanup file differs from retained identity or mode",
            ));
        }
        let mut hash = Sha256::new();
        let mut bytes = 0u64;
        let mut buffer = [0u8; 65536];
        loop {
            let count = file
                .read(&mut buffer)
                .map_err(|e| Error::io("verify retained cleanup bytes before unlink", e))?;
            if count == 0 {
                break;
            }
            bytes += count as u64;
            if bytes > proof.bytes {
                return Err(Error::new(
                    "stale_cleanup_plan",
                    "cleanup file grew after preservation",
                ));
            }
            hash.update(&buffer[..count]);
        }
        if bytes != proof.bytes
            || format!("sha256:{}", hex::encode(hash.finalize())) != proof.digest
            || CleanupFileState::read(&file)? != state
        {
            return Err(Error::new(
                "stale_cleanup_plan",
                "cleanup file bytes or metadata changed after preservation",
            ));
        }
        Ok(Self { file, state })
    }
    fn unlink(self, directory: &File, name: &std::ffi::CStr) -> Result<()> {
        let entry = Self::open(directory, name)?;
        if CleanupFileState::read(&self.file)? != self.state
            || CleanupFileState::read(&entry)? != self.state
        {
            return Err(Error::new(
                "stale_cleanup_plan",
                "cleanup file changed after final byte verification; preserve remaining files",
            ));
        }
        if unsafe { libc::unlinkat(directory.as_raw_fd(), name.as_ptr(), 0) } != 0 {
            return Err(Error::io(
                "remove verified cleanup file",
                std::io::Error::last_os_error(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod cleanup_unlink_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn edits_between_inventory_hash_and_unlink_preserve_unretained_work() {
        for after_hash in [false, true] {
            for mode_only in [false, true] {
                let fixture = tempfile::tempdir().unwrap();
                std::fs::set_permissions(fixture.path(), std::fs::Permissions::from_mode(0o700))
                    .unwrap();
                let root = fixture.path().canonicalize().unwrap();
                let source = root.join("source");
                let retained = root.join("retained");
                std::fs::create_dir(&source).unwrap();
                std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o700)).unwrap();
                let path = source.join("file");
                std::fs::write(&path, b"retained").unwrap();
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
                std::fs::copy(&path, &retained).unwrap();
                let expected = inventory(&source).unwrap();
                let proof = expected.iter().find(|entry| !entry.directory).unwrap();
                let directory = File::open(&source).unwrap();
                let name = std::ffi::CString::new("file").unwrap();
                let prepared = after_hash
                    .then(|| PreparedFileRemoval::inspect(&directory, &name, proof).unwrap());
                if mode_only {
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o400))
                        .unwrap();
                } else {
                    // An in-place edit of exactly the retained length defeats
                    // inode/size-only validation, even though the bytes differ.
                    std::fs::write(&path, b"modified").unwrap();
                }
                let current = std::fs::metadata(&path).unwrap();
                assert_eq!(current.ino(), proof.inode);
                assert_eq!(current.len(), proof.bytes);
                let result = match prepared {
                    Some(prepared) => prepared.unlink(&directory, &name),
                    None => PreparedFileRemoval::inspect(&directory, &name, proof)
                        .and_then(|prepared| prepared.unlink(&directory, &name)),
                };
                assert_eq!(result.unwrap_err().code, "stale_cleanup_plan");
                assert_eq!(std::fs::read(&retained).unwrap(), b"retained");
                assert_eq!(
                    std::fs::read(&path).unwrap(),
                    if mode_only { b"retained" } else { b"modified" }
                );
                assert_eq!(
                    std::fs::metadata(&path).unwrap().mode() & 0o7777,
                    if mode_only { 0o400 } else { 0o600 }
                );
            }
        }
    }

    #[test]
    fn unchanged_descriptor_verified_file_is_removed() {
        let fixture = tempfile::tempdir().unwrap();
        std::fs::set_permissions(fixture.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let root = fixture.path().canonicalize().unwrap();
        let path = root.join("file");
        std::fs::write(&path, b"retained").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let expected = inventory(&root).unwrap();
        let proof = expected.iter().find(|entry| !entry.directory).unwrap();
        let directory = File::open(&root).unwrap();
        let name = std::ffi::CString::new("file").unwrap();
        PreparedFileRemoval::inspect(&directory, &name, proof)
            .unwrap()
            .unlink(&directory, &name)
            .unwrap();
        assert!(!path.exists());
    }
}

fn validate_retained_path(registration: &ExecutionRegistration, path: &Path) -> Result<()> {
    PrivateDir::open(path)?;
    let root = &registration.isolation.execution_root;
    if path.starts_with(root) || root.starts_with(path) || path == registration.isolation.repository
    {
        return Err(Error::new(
            "scope_violation",
            "retained recovery copy must be outside the execution tree",
        ));
    }
    Ok(())
}
fn content_inventory(files: &[RetainedFile]) -> Vec<(&Path, bool, u64, &str, u32)> {
    files
        .iter()
        .map(|f| {
            (
                f.relative.as_path(),
                f.directory,
                f.bytes,
                f.digest.as_str(),
                f.mode,
            )
        })
        .collect()
}
fn inventory(root: &Path) -> Result<Vec<RetainedFile>> {
    let canonical = root
        .canonicalize()
        .map_err(|e| Error::io("resolve cleanup inventory", e))?;
    if canonical != root {
        return Err(Error::new(
            "unsafe_storage",
            "cleanup inventory root must be canonical and unlinked",
        ));
    }
    let root = canonical;
    let mut pending = vec![root.clone()];
    let mut files = vec![];
    let mut total = 0u64;
    while let Some(path) = pending.pop() {
        let m = std::fs::symlink_metadata(&path)
            .map_err(|e| Error::io("inspect cleanup inventory", e))?;
        if m.uid() != unsafe { libc::geteuid() }
            || m.mode() & 0o022 != 0
            || m.file_type().is_symlink()
        {
            return Err(Error::new(
                "unsafe_storage",
                "cleanup inventory includes a link, foreign owner or writable shared path",
            ));
        }
        if m.is_dir() {
            if files.len() >= MAX_FILES {
                return Err(Error::new(
                    "cleanup_blocked",
                    "cleanup directory inventory exceeds bound",
                ));
            }
            files.push(RetainedFile {
                directory: true,
                relative: path
                    .strip_prefix(&root)
                    .map_err(|_| Error::invalid("cleanup directory containment failed"))?
                    .into(),
                bytes: 0,
                digest: bytes_digest(b"owned-directory"),
                mode: m.mode() & 0o7777,
                device: m.dev(),
                inode: m.ino(),
            });
            let mut entries = std::fs::read_dir(&path)
                .map_err(|e| Error::io("enumerate cleanup inventory", e))?
                .map(|e| e.map(|e| e.path()))
                .collect::<std::io::Result<Vec<_>>>()
                .map_err(|e| Error::io("read cleanup directory entry", e))?;
            entries.sort();
            pending.extend(entries);
        } else {
            if files.len() >= MAX_FILES || !m.is_file() || m.nlink() != 1 {
                return Err(Error::new(
                    "cleanup_blocked",
                    "cleanup inventory exceeds file bound or contains unsupported linked/special files",
                ));
            }
            total = total
                .checked_add(m.len())
                .filter(|n| *n <= MAX_TREE_BYTES)
                .ok_or_else(|| {
                    Error::new("cleanup_blocked", "cleanup inventory exceeds byte bound")
                })?;
            let mut file = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
                .open(&path)
                .map_err(|e| Error::io("open cleanup inventory file", e))?;
            validate_file(&file, false)?;
            let actual = file
                .metadata()
                .map_err(|e| Error::io("check cleanup file identity", e))?;
            if actual.dev() != m.dev() || actual.ino() != m.ino() {
                return Err(Error::new(
                    "ownership_conflict",
                    "cleanup file changed during inventory",
                ));
            }
            let mut hash = Sha256::new();
            let mut count = 0u64;
            let mut buffer = [0u8; 65536];
            loop {
                let n = file
                    .read(&mut buffer)
                    .map_err(|e| Error::io("hash cleanup inventory", e))?;
                if n == 0 {
                    break;
                }
                count += n as u64;
                if count > m.len() {
                    return Err(Error::new(
                        "ownership_conflict",
                        "cleanup file grew during inventory",
                    ));
                }
                hash.update(&buffer[..n]);
            }
            if count != m.len() {
                return Err(Error::new(
                    "ownership_conflict",
                    "cleanup file changed during inventory",
                ));
            }
            files.push(RetainedFile {
                directory: false,
                relative: path
                    .strip_prefix(&root)
                    .map_err(|_| Error::invalid("cleanup containment failed"))?
                    .into(),
                bytes: count,
                digest: format!("sha256:{}", hex::encode(hash.finalize())),
                mode: m.mode() & 0o7777,
                device: m.dev(),
                inode: m.ino(),
            });
        }
    }
    files.sort_by(|a, b| a.relative.cmp(&b.relative));
    Ok(files)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoverInfo {
    pub schema_version: u32,
    pub api_version: u32,
    pub config_schema: u32,
    pub receipt_schema: u32,
    /// Explicit process journal compatibility, including schema-3 check members.
    /// Older readers must not silently discard retained bundle ancestry.
    pub process_schema: u32,
    /// The replacement must understand managed policy fields in configuration
    /// and retained isolation records; omission cannot imply that capability.
    pub managed_execution_schema: u32,
    pub runtime_version: String,
    pub runtime_build: String,
    pub instruction_schema: u32,
    pub asset_version: String,
    pub assets: BTreeMap<String, String>,
}
pub fn handover_info() -> Result<HandoverInfo> {
    Ok(HandoverInfo {
        schema_version: 3,
        api_version: 1,
        config_schema: 1,
        receipt_schema: 1,
        process_schema: 3,
        managed_execution_schema: 2,
        runtime_version: env!("CARGO_PKG_VERSION").into(),
        runtime_build: crate::BUILD_ID.into(),
        instruction_schema: crate::instructions::ASSEMBLY_SCHEMA,
        asset_version: crate::instructions::ASSET_VERSION.into(),
        assets: crate::instructions::verify_assets()?,
    })
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QualificationArtifact {
    pub path: PathBuf,
    pub digest: String,
}
/// The host signs this only after an explicit human-reviewed qualification
/// action. Builder tests cannot produce operational qualification by running.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeQualification {
    pub schema_version: u32,
    pub runtime_id: Uuid,
    pub binary_digest: String,
    pub runtime_build: String,
    pub instruction_digest: String,
    pub reviewed_by: String,
    pub reviewed_at: DateTime<Utc>,
    pub deterministic: QualificationArtifact,
    pub actual_agents: QualificationArtifact,
    pub integration: QualificationArtifact,
    pub human_calibration: QualificationArtifact,
}
#[derive(Clone)]
pub struct VerifiedRuntimeQualification {
    record: RuntimeQualification,
    digest: String,
    key_id: String,
}
pub fn verify_runtime_qualification(
    signed: &crate::evidence::Signed<RuntimeQualification>,
    key: &crate::evidence::AttestationKey,
) -> Result<VerifiedRuntimeQualification> {
    key.verify("runtime_qualification", signed)?;
    let record = &signed.payload;
    if record.schema_version != 1
        || record.runtime_id.is_nil()
        || record.reviewed_by.trim().is_empty()
        || record.runtime_build.is_empty()
        || record.reviewed_at > Utc::now()
    {
        return Err(Error::new(
            "qualification_required",
            "qualification identity/review is incomplete",
        ));
    }
    crate::evidence::validate_digest(&record.binary_digest)?;
    crate::evidence::validate_digest(&record.instruction_digest)?;
    for artifact in [
        &record.deterministic,
        &record.actual_agents,
        &record.integration,
        &record.human_calibration,
    ] {
        let bytes = bounded_owned_file(&artifact.path, 16 * 1024 * 1024)?;
        if bytes_digest(&bytes) != artifact.digest {
            return Err(Error::new(
                "qualification_required",
                "qualification evidence reference changed",
            ));
        }
    }
    Ok(VerifiedRuntimeQualification {
        record: record.clone(),
        digest: canonical_digest(1, signed)?,
        key_id: signed.key_id.clone(),
    })
}
#[derive(Clone, Debug, Serialize)]
pub struct HandoverPlan {
    pub runtime_id: Uuid,
    pub previous_binary: BinaryIdentity,
    pub replacement: BinaryIdentity,
    pub replacement_info: HandoverInfo,
    pub owner_digest: String,
    pub config_digest: String,
    pub intent_digest: String,
    pub shutdown_digest: String,
    pub receipts_digest: String,
    pub qualification_digest: String,
    pub attestation_key_id: String,
    pub token: String,
}
struct HandoverState {
    owner: OwnerRecord,
    config: crate::config::Config,
    intent: crate::control::IntentRecord,
    shutdown: ServiceShutdown,
    receipts: Vec<Receipt>,
}
fn handover_state(
    host: &HostIdentity,
    root: &PrivateDir,
    probe: &dyn ProcessProbe,
) -> Result<HandoverState> {
    let owner: OwnerRecord = root.read("owner.json")?;
    let intent: crate::control::IntentRecord = root.read("intent.json")?;
    let shutdown: ServiceShutdown = root.read("service-shutdown.json")?;
    if owner.uid != host.uid
        || owner.host_id != host.host_id
        || owner.root != host.state_root()
        || !owner.setup_complete
        || owner.runtime_id != intent.runtime_id
        || intent.desired != crate::control::Intent::Stopped
        || shutdown.schema_version != 1
        || shutdown.runtime_id != owner.runtime_id
        || !shutdown.stopped
        || owner.pending_owner.is_some()
    {
        return Err(Error::new(
            "handover_blocked",
            "handover requires this complete runtime, durable STOPPED, supervisor fencing and reconciled pending ownership",
        ));
    }
    for process in owner
        .control_process
        .iter()
        .chain(owner.orchestration.iter().map(|owner| &owner.process))
    {
        if probe.observe(process)? == ProcessObservation::Running {
            return Err(Error::new(
                "handover_blocked",
                "old owned process remains live",
            ));
        }
    }
    if !probe.matching_executable(&owner.binary.path)?.is_empty() {
        return Err(Error::new(
            "handover_blocked",
            "complete process inventory still contains the old executable",
        ));
    }
    let bytes = root.read_bytes("config.toml", 8 * 1024 * 1024)?;
    let config = crate::config::Config::parse(
        std::str::from_utf8(&bytes)
            .map_err(|_| Error::invalid("persisted configuration is not UTF-8"))?,
    )?;
    if config.runtime.id != owner.runtime_id
        || config.runtime.host_id != owner.host_id
        || owner.config_revision.as_deref() != Some(&config.revision()?)
    {
        return Err(Error::new(
            "handover_blocked",
            "persisted configuration differs from registered owner",
        ));
    }
    let mut receipts = ReceiptStore::new(root.child("receipts")?).all()?;
    receipts.sort_by_key(|r| r.request_id);
    if receipts.iter().any(|r| {
        r.runtime_id != owner.runtime_id
            || matches!(r.phase, ReceiptPhase::Prepared | ReceiptPhase::Sent)
    }) {
        return Err(Error::new(
            "handover_blocked",
            "uncertain actions or incompatible receipts must be reconciled before handover",
        ));
    }
    Ok(HandoverState {
        owner,
        config,
        intent,
        shutdown,
        receipts,
    })
}
async fn replacement_info(binary: &BinaryIdentity) -> Result<HandoverInfo> {
    use tokio::io::AsyncReadExt;
    binary.revalidate(true)?;
    let mut command = tokio::process::Command::new(&binary.path);
    command
        .arg("__handover-info")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .process_group(0);
    let cutoff = std::time::Instant::now() + Duration::from_secs(5);
    let mut child =
        crate::process_spawn::spawn_tokio(&mut command, Some(cutoff)).map_err(|error| {
            if error.kind() == std::io::ErrorKind::TimedOut {
                Error::new("timeout", "replacement metadata probe timed out")
            } else {
                Error::io("inspect immutable replacement metadata", error)
            }
        })?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::new("incompatible_version", "replacement metadata unavailable"))?
        .take(65537);
    let mut bytes = vec![];
    let observed = tokio::time::timeout_at(tokio::time::Instant::from_std(cutoff), async {
        stdout
            .read_to_end(&mut bytes)
            .await
            .map_err(|e| Error::io("read replacement metadata", e))?;
        let status = child
            .wait()
            .await
            .map_err(|e| Error::io("wait replacement metadata", e))?;
        if !status.success() || bytes.len() > 65536 {
            return Err(Error::new(
                "incompatible_version",
                "replacement metadata failed or exceeded bounds",
            ));
        }
        serde_json::from_slice::<HandoverInfo>(&bytes).map_err(|_| {
            Error::new(
                "incompatible_version",
                "replacement metadata schema is unsupported",
            )
        })
    })
    .await;
    if !matches!(observed, Ok(Ok(_))) {
        if let Some(pid) = child.id() {
            unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
        }
        let _ = child.kill().await;
    }
    let info =
        observed.map_err(|_| Error::new("timeout", "replacement metadata probe timed out"))??;
    binary.revalidate(true)?;
    if info.schema_version != 3
        || info.api_version != 1
        || info.config_schema != 1
        || info.receipt_schema != 1
        || info.process_schema != 3
        || info.managed_execution_schema != 2
        || info.instruction_schema != crate::instructions::ASSEMBLY_SCHEMA
        || info.runtime_build.is_empty()
        || info.runtime_version.is_empty()
        || info.asset_version.is_empty()
        || info.assets.is_empty()
    {
        return Err(Error::new(
            "incompatible_version",
            "replacement cannot read the retained formats or lacks versioned assets",
        ));
    }
    for digest in info.assets.values() {
        crate::evidence::validate_digest(digest)?;
    }
    if info.assets.keys().collect::<BTreeSet<_>>()
        != crate::instructions::verify_assets()?
            .keys()
            .collect::<BTreeSet<_>>()
    {
        return Err(Error::new(
            "incompatible_version",
            "replacement omits or renames required role assets without a supported instruction schema",
        ));
    }
    Ok(info)
}
pub async fn inspect_handover(
    host: &HostIdentity,
    replacement: BinaryIdentity,
    qualification: &VerifiedRuntimeQualification,
    probe: &dyn ProcessProbe,
) -> Result<HandoverPlan> {
    let info = replacement_info(&replacement).await?;
    let root = PrivateDir::open(&host.state_root())?;
    let _control = root.lock("control.lock")?;
    let _runtime = root.lock("runtime.lock")?;
    let state = handover_state(host, &root, probe)?;
    if qualification.record.runtime_id != state.owner.runtime_id
        || qualification.record.binary_digest != replacement.digest
        || qualification.record.runtime_build != info.runtime_build
        || qualification.record.instruction_digest != canonical_digest(1, &info)?
    {
        return Err(Error::new(
            "qualification_required",
            "reviewed qualification does not bind this exact replacement and instruction set",
        ));
    }
    if qualification.key_id != format!("keychain:{}:attestation-v1", state.owner.runtime_id)
        || !state.config.control.local_users.values().any(|operator| {
            operator.id == qualification.record.reviewed_by
                && operator
                    .roles
                    .contains(&crate::config::ControlRole::Configure)
        })
    {
        return Err(Error::new(
            "qualification_required",
            "qualification must retain this runtime's attestation key and a mapped human reviewer",
        ));
    }
    for artifact in [
        &qualification.record.deterministic,
        &qualification.record.actual_agents,
        &qualification.record.integration,
        &qualification.record.human_calibration,
    ] {
        if bytes_digest(&bounded_owned_file(&artifact.path, 16 * 1024 * 1024)?) != artifact.digest {
            return Err(Error::new(
                "qualification_required",
                "qualification evidence changed since verification",
            ));
        }
    }
    let mut plan = HandoverPlan {
        runtime_id: state.owner.runtime_id,
        previous_binary: state.owner.binary.clone(),
        replacement,
        replacement_info: info,
        owner_digest: canonical_digest(1, &state.owner)?,
        config_digest: state.config.revision()?,
        intent_digest: canonical_digest(1, &state.intent)?,
        shutdown_digest: canonical_digest(1, &state.shutdown)?,
        receipts_digest: canonical_digest(1, &state.receipts)?,
        qualification_digest: qualification.digest.clone(),
        attestation_key_id: qualification.key_id.clone(),
        token: String::new(),
    };
    plan.token = canonical_digest(1, &plan)?;
    Ok(plan)
}
/// Explicit local application only: writes no provider records and never loads
/// launchd or starts orchestration. The supervisor remains durably stopped.
pub async fn apply_handover(
    host: &HostIdentity,
    plan: &HandoverPlan,
    qualification: &VerifiedRuntimeQualification,
    probe: &dyn ProcessProbe,
    actor: Actor,
    request_id: Uuid,
) -> Result<Receipt> {
    actor.require("configure")?;
    actor.require("recover")?;
    if !matches!(actor.principal,Some(crate::control::Principal::Unix{uid}) if uid==host.uid) {
        return Err(Error::new(
            "access_denied",
            "handover requires this execution host's local OS principal",
        ));
    }
    let mutation = MutationRequest {
        request_id,
        runtime_id: plan.runtime_id,
        actor: actor.clone(),
        kind: "handover".into(),
        canonical_targets: vec![plan.runtime_id.to_string()],
        expected: BTreeMap::from([("handover_plan".into(), plan.token.clone())]),
        payload: json!({"replacement":plan.replacement,"qualification":plan.qualification_digest}),
    };
    {
        let root = PrivateDir::open(&host.state_root())?;
        let _control = root.lock("control.lock")?;
        let config_bytes = root.read_bytes("config.toml", 8 * 1024 * 1024)?;
        let config = crate::config::Config::parse(
            std::str::from_utf8(&config_bytes)
                .map_err(|_| Error::invalid("persisted config is not UTF-8"))?,
        )?;
        crate::app::verify_current_actor(&config, &actor, "configure")?;
        crate::app::verify_current_actor(&config, &actor, "recover")?;
        let store = ReceiptStore::new(root.child("receipts")?);
        match store.get(request_id) {
            Ok(_) => return Ok(store.prepare(&mutation, 0)?.0),
            Err(error) if error.code == "record_missing" => {}
            Err(error) => return Err(error),
        }
    }
    let fresh = inspect_handover(host, plan.replacement.clone(), qualification, probe).await?;
    if fresh.token != plan.token {
        return Err(Error::new(
            "stale_handover_plan",
            "handover facts changed after inspection",
        ));
    }
    let root = PrivateDir::open(&host.state_root())?;
    let _control = root.lock("control.lock")?;
    let _runtime = root.lock("runtime.lock")?;
    let state = handover_state(host, &root, probe)?;
    crate::app::verify_current_actor(&state.config, &actor, "configure")?;
    crate::app::verify_current_actor(&state.config, &actor, "recover")?;
    if canonical_digest(1, &state.owner)? != plan.owner_digest
        || state.config.revision()? != plan.config_digest
        || canonical_digest(1, &state.intent)? != plan.intent_digest
        || canonical_digest(1, &state.shutdown)? != plan.shutdown_digest
        || canonical_digest(1, &state.receipts)? != plan.receipts_digest
    {
        return Err(Error::new(
            "stale_handover_plan",
            "handover state changed before serialized replacement",
        ));
    }
    plan.replacement.revalidate(true)?;
    let store = ReceiptStore::new(root.child("receipts")?);
    let (mut receipt, new) = store.prepare(&mutation, state.owner.owner_generation)?;
    if !new {
        return Ok(receipt);
    }
    root.child("handover-history")?
        .write(&format!("plan-{request_id}.json"), plan)?;
    store.mark_sent(&mut receipt)?;
    let effect = (|| {
        crate::platform::replace_service_for_handover(
            host,
            &root,
            &plan.previous_binary,
            &plan.replacement,
            &plan.replacement_info.runtime_build,
        )?;
        let mut next = state.owner;
        next.binary = plan.replacement.clone();
        next.runtime_build = plan.replacement_info.runtime_build.clone();
        next.owner_generation = next
            .owner_generation
            .checked_add(1)
            .ok_or_else(|| Error::new("ownership_conflict", "owner generation overflow"))?;
        next.control_process = None;
        next.orchestration = None;
        next.updated_at = Utc::now();
        root.write("owner.json", &next)?;
        store.acknowledge(&mut receipt,BTreeMap::from([("runtime_build".into(),next.runtime_build.clone())]),json!({"handover_plan":plan.token,"owner_generation":next.owner_generation,"desired":"stopped","supervisor":"stopped","attestation_key_retained":plan.attestation_key_id}))?;
        Ok(receipt)
    })();
    effect.map_err(|error: Error| {
        Error::new(
            "operation_uncertain",
            "handover replacement requires receipt/state reconciliation before activation",
        )
        .detail(json!({"operation_id":request_id,"cause":error.code}))
    })
}
fn bounded_owned_file(path: &Path, maximum: u64) -> Result<Vec<u8>> {
    if !path.is_absolute() {
        return Err(Error::invalid("retained evidence path must be absolute"));
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| Error::io("open retained qualification evidence", e))?;
    validate_file(&file, false)?;
    let mut bytes = vec![];
    file.take(maximum + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| Error::io("read retained qualification evidence", e))?;
    if bytes.len() as u64 > maximum {
        return Err(Error::new(
            "qualification_required",
            "qualification evidence exceeds bound",
        ));
    }
    Ok(bytes)
}

/// Read-only preparation identity. These records preserve assets and ownership;
/// they never manufacture a child execution binding or grant cleanup permission.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReadOnlyIdentity {
    Coordination {
        workspace: Uuid,
        parent: crate::authority::IssueKey,
    },
    ChildReasoning {
        workspace: Uuid,
        work: crate::evidence::WorkIdentity,
    },
    IndependentReview(crate::git::IndependentCloneIdentity),
}
pub struct ReadOnlyPreparation<'a> {
    pub preparation: &'a crate::control::Permit,
    pub operation: Uuid,
    pub authority: &'a crate::worker::WorkerAuthority,
    pub identity: ReadOnlyIdentity,
    pub independent_clone: Option<&'a crate::git::VerifiedIndependentClone>,
    pub isolation: &'a crate::isolation::ValidatedIsolation,
    pub workflow: &'a Workflow,
}
#[derive(Clone, Debug)]
pub struct PreservedPreparation {
    operation: Uuid,
    digest: String,
}
impl PreservedPreparation {
    pub fn operation_id(&self) -> Uuid {
        self.operation
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadOnlyRecord {
    schema_version: u32,
    runtime: Uuid,
    operation: Uuid,
    preparation: crate::control::Permit,
    identity: ReadOnlyIdentity,
    authority_digest: String,
    isolation: IsolationPlan,
    isolation_digest: String,
    workflow_digest: String,
    root_device: u64,
    root_inode: u64,
    repository_device: u64,
    repository_inode: u64,
    clone_proof_digest: Option<String>,
}
impl Housekeeping {
    pub fn register_readonly_preparation(
        &self,
        input: ReadOnlyPreparation<'_>,
    ) -> Result<PreservedPreparation> {
        use crate::{isolation::ExecutionRole, worker::WorkerAuthority};
        input.isolation.revalidate()?;
        let plan = input.isolation.plan();
        if input.operation.is_nil()
            || input.preparation.work_key != input.authority.work_key()?
            || !plan.source_writes.is_empty()
            || !plan.resource_writes.is_empty()
            || !plan.executables.is_empty()
            || plan.cwd != plan.repository
            || plan.repository.parent() != Some(plan.execution_root.as_path())
        {
            return Err(Error::new(
                "scope_violation",
                "read-only preservation requires exact operation authority, separate owned substrate and no repository execution/write grant",
            ));
        }
        if !matches!(input.identity, ReadOnlyIdentity::IndependentReview(_))
            && input.independent_clone.is_some()
        {
            return Err(Error::new(
                "scope_violation",
                "non-review preservation cannot carry a clone authority",
            ));
        }
        let workflow_digest = match (&input.identity, input.authority) {
            (
                ReadOnlyIdentity::Coordination { workspace, parent },
                WorkerAuthority::Authorized(gate),
            ) if plan.role == ExecutionRole::Coordination
                && gate.work().child.is_none()
                && *workspace == gate.workspace_id()
                && gate.purpose() == crate::reconcile::GatePurpose::Coordination =>
            {
                if *parent
                    != crate::authority::IssueKey::new(
                        &gate.work().linear_endpoint,
                        gate.work().linear_workspace,
                        gate.work().parent,
                    )?
                {
                    return Err(Error::new(
                        "identity_mismatch",
                        "coordination preservation parent differs",
                    ));
                }
                gate.workflow_digest()
            }
            (
                ReadOnlyIdentity::ChildReasoning { workspace, work },
                WorkerAuthority::Authorized(gate),
            ) if plan.role == ExecutionRole::Child
                && work.child.is_some()
                && *workspace == gate.workspace_id()
                && work == gate.work()
                && gate.child_scope().is_some_and(|scope| {
                    scope.write_repository.is_none() && scope.write_paths.is_empty()
                }) =>
            {
                gate.workflow_digest()
            }
            (ReadOnlyIdentity::IndependentReview(identity), WorkerAuthority::Independent(gate))
                if plan.role == ExecutionRole::IndependentReview
                    && identity.runtime_id == input.preparation.lease.runtime_id
                    && identity.workspace_id == gate.workspace_id()
                    && identity.repository.id == gate.repository_id().to_string()
                    && identity.pull_request == gate.pull_request().number
                    && identity.head.commit == gate.pull_request().head_sha
                    && identity.base.commit == gate.pull_request().base_sha
                    && identity.review_identity == gate.review_identity()
                    && identity.repository_path == plan.repository =>
            {
                let proof = input.independent_clone.ok_or_else(|| {
                    Error::new(
                        "execution_ownership",
                        "independent preservation needs the guarded exact clone observation",
                    )
                })?;
                if proof.identity() != identity || proof.root() != plan.execution_root {
                    return Err(Error::new(
                        "execution_ownership",
                        "independent clone proof belongs to another substrate",
                    ));
                }
                proof.revalidate_filesystem()?;
                gate.workflow_digest()
            }
            _ => {
                return Err(Error::new(
                    "scope_violation",
                    "read-only preservation role or canonical provider identity differs from the sealed admission",
                ));
            }
        };
        if workflow_digest != input.workflow.digest {
            return Err(Error::new(
                "stale_result",
                "preservation workflow differs from admitted snapshot",
            ));
        }
        let root = PrivateDir::open(&plan.execution_root)?;
        let repository = PrivateDir::open(&plan.repository)?;
        let root_meta = std::fs::symlink_metadata(root.path())
            .map_err(|e| Error::io("inspect preparation root", e))?;
        let repository_meta = std::fs::symlink_metadata(repository.path())
            .map_err(|e| Error::io("inspect preparation repository", e))?;
        if !matches!(input.identity, ReadOnlyIdentity::IndependentReview(_))
            && std::fs::read_dir(repository.path())
                .map_err(|e| Error::io("inspect read-only substrate", e))?
                .next()
                .is_some()
        {
            return Err(Error::new(
                "ownership_conflict",
                "reasoning substrate is not an empty newly owned directory",
            ));
        }
        let record = ReadOnlyRecord {
            schema_version: 1,
            runtime: input.preparation.lease.runtime_id,
            operation: input.operation,
            preparation: input.preparation.clone(),
            identity: input.identity,
            authority_digest: input.authority.digest()?,
            isolation: plan.clone(),
            isolation_digest: input.isolation.digest().into(),
            workflow_digest: workflow_digest.into(),
            root_device: root_meta.dev(),
            root_inode: root_meta.ino(),
            repository_device: repository_meta.dev(),
            repository_inode: repository_meta.ino(),
            clone_proof_digest: input
                .independent_clone
                .map(|proof| proof.digest())
                .transpose()?,
        };
        let digest = canonical_digest(1, &record)?;
        self.control.validate_permit(input.preparation, || {
            let storage = self.root.child("readonly-preparations")?;
            let name = format!("{}.json", record.operation);
            match storage.read::<ReadOnlyRecord>(&name) {
                Ok(prior) if canonical_digest(1, &prior)? == digest => Ok(()),
                Ok(_) => Err(Error::new(
                    "ownership_conflict",
                    "read-only preparation is already registered with different provenance",
                )),
                Err(e) if e.code == "record_missing" => storage.write(&name, &record),
                Err(e) => Err(e),
            }
        })?;
        Ok(PreservedPreparation {
            operation: record.operation,
            digest,
        })
    }
    fn readonly_record(&self, operation: Uuid) -> Result<ReadOnlyRecord> {
        let record: ReadOnlyRecord = self
            .root
            .child("readonly-preparations")?
            .read(&format!("{operation}.json"))?;
        if record.schema_version != 1
            || record.operation != operation
            || record.runtime != self.control.snapshot()?.intent.runtime_id
        {
            return Err(Error::new(
                "record_corrupt",
                "read-only preservation record differs from runtime/operation",
            ));
        }
        let isolation = record.isolation.validate()?;
        if isolation.digest() != record.isolation_digest {
            return Err(Error::new(
                "ownership_conflict",
                "read-only preparation isolation changed",
            ));
        }
        let root = PrivateDir::open(&record.isolation.execution_root)?;
        let repository = PrivateDir::open(&record.isolation.repository)?;
        let r = std::fs::symlink_metadata(root.path())
            .map_err(|e| Error::io("inspect preserved root", e))?;
        let p = std::fs::symlink_metadata(repository.path())
            .map_err(|e| Error::io("inspect preserved substrate", e))?;
        if (r.dev(), r.ino(), p.dev(), p.ino())
            != (
                record.root_device,
                record.root_inode,
                record.repository_device,
                record.repository_inode,
            )
        {
            return Err(Error::new(
                "ownership_conflict",
                "preserved directory identity changed",
            ));
        }
        Ok(record)
    }
    pub fn record_preserved_process(
        &self,
        preserved: &PreservedPreparation,
        process: ProcessIdentity,
    ) -> Result<()> {
        let record = self.readonly_record(preserved.operation)?;
        if canonical_digest(1, &record)? != preserved.digest
            || self.probe.observe(&process)? != ProcessObservation::Running
        {
            return Err(Error::new(
                "ownership_conflict",
                "preservation process is not actually live or capability differs",
            ));
        }
        let storage = self.root.child("readonly-preparations")?;
        let name = format!("processes-{}.json", record.operation);
        let mut processes: Vec<ProcessIdentity> = match storage.read(&name) {
            Ok(v) => v,
            Err(e) if e.code == "record_missing" => vec![],
            Err(e) => return Err(e),
        };
        if !processes.contains(&process) {
            processes.push(process);
            storage.write(&name, &processes)?;
        }
        Ok(())
    }
    pub fn inspect_preserved_preparation(&self, operation: Uuid) -> Result<serde_json::Value> {
        let record = self.readonly_record(operation)?;
        let processes: Vec<ProcessIdentity> = match self
            .root
            .child("readonly-preparations")?
            .read(&format!("processes-{operation}.json"))
        {
            Ok(v) => v,
            Err(e) if e.code == "record_missing" => vec![],
            Err(e) => return Err(e),
        };
        let observed: Vec<_> = processes
            .iter()
            .map(|p| {
                self.probe
                    .observe(p)
                    .map(|state| json!({"process":p,"state":format!("{state:?}")}))
            })
            .collect::<Result<_>>()?;
        Ok(
            json!({"operation_id":operation,"runtime_id":record.runtime,"identity":record.identity,"path":record.isolation.execution_root,"isolation_digest":record.isolation_digest,"workflow_digest":record.workflow_digest,"registration_digest":canonical_digest(1,&record)?,"processes":observed,"preservation":"required","cleanup_eligible":false,"next_action":"Retain this reasoning/review substrate and its immutable preparation records; cleanup requires its own complete verified plan"}),
        )
    }
}
