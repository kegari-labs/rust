//! Durable process identity references. A retained PID is observation data, never
//! an owned handle or permission to signal/adopt a process after host recovery.
use crate::{
    control::Lease,
    digest::canonical_digest,
    error::{Error, Result},
    platform::{
        BinaryIdentity, NativeProcessProbe, ProcessIdentity, ProcessObservation, ProcessProbe,
    },
    storage::{ExclusiveLock, PrivateDir},
};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    os::fd::AsRawFd,
    os::unix::process::CommandExt,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SpawnBinding {
    pub operation_id: Uuid,
    pub effect_id: Uuid,
    pub lease: Lease,
    pub work_key: String,
    pub scope_digest: String,
    pub cwd: PathBuf,
    pub executable: BinaryIdentity,
    pub observed_executables: Vec<BinaryIdentity>,
    pub argv_digest: String,
    pub kind: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProcessState {
    Starting,
    Running {
        process: ProcessIdentity,
    },
    GroupExited {
        process: Option<ProcessIdentity>,
        exit_code: Option<i32>,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessRecord {
    pub schema_version: u32,
    pub binding: SpawnBinding,
    pub host: ProcessIdentity,
    pub guard: String,
    pub state: ProcessState,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryState {
    GuardHeld,
    StartingUnknown,
    ExactProcessRunning,
    LeaderGoneTreeUnverified,
    GroupExitRecordedTreeUnverified,
}
#[derive(Clone, Debug, Serialize)]
pub struct ProcessRecovery {
    pub operation_id: Uuid,
    pub effect_id: Uuid,
    pub state: RecoveryState,
    pub process: Option<ProcessIdentity>,
    /// This journal does not prove containment of detached descendants.
    pub all_descendants_exited: bool,
}
pub struct ProcessJournal {
    directory: PrivateDir,
    runtime: Uuid,
    probe: Arc<dyn ProcessProbe>,
}
impl ProcessJournal {
    pub fn new(directory: PrivateDir, runtime: Uuid) -> Result<Self> {
        Self::with_probe(directory, runtime, Arc::new(NativeProcessProbe))
    }
    pub fn with_probe(
        directory: PrivateDir,
        runtime: Uuid,
        probe: Arc<dyn ProcessProbe>,
    ) -> Result<Self> {
        if runtime.is_nil() {
            return Err(Error::invalid("process journal needs runtime identity"));
        }
        Ok(Self {
            directory,
            runtime,
            probe,
        })
    }
    /// Persist before native spawn while holding the operation's current gate.
    /// Any record already present means recovery, never permission to replay.
    pub fn begin(self: &Arc<Self>, binding: SpawnBinding) -> Result<SpawnGuard> {
        if binding.operation_id.is_nil()
            || binding.effect_id.is_nil()
            || binding.lease.runtime_id != self.runtime
            || binding.work_key.is_empty()
            || binding.kind.is_empty()
            || binding.kind.len() > 128
            || !binding.cwd.is_absolute()
        {
            return Err(Error::invalid("process identity binding is incomplete"));
        }
        for digest in [
            &binding.scope_digest,
            &binding.argv_digest,
            &binding.executable.digest,
        ] {
            crate::evidence::validate_digest(digest)?;
        }
        binding.executable.revalidate(false)?;
        if binding.observed_executables.len() > 8 {
            return Err(Error::invalid("process launcher targets exceed bound"));
        }
        for executable in &binding.observed_executables {
            executable.revalidate(false)?;
        }
        if binding
            .cwd
            .canonicalize()
            .map_err(|error| Error::io("resolve process cwd", error))?
            != binding.cwd
        {
            return Err(Error::new(
                "scope_violation",
                "process cwd must be canonical",
            ));
        }
        let operation = self.directory.child(&binding.operation_id.to_string())?;
        let record_name = format!("{}.json", binding.effect_id);
        match operation.read::<ProcessRecord>(&record_name) {
            Ok(_) => {
                return Err(Error::new(
                    "operation_uncertain",
                    "process effect already has a durable identity record",
                ));
            }
            Err(error) if error.code == "record_missing" => {}
            Err(error) => return Err(error),
        }
        if operation.records()?.len() >= 4096 {
            return Err(Error::new(
                "process_limit",
                "operation exhausted bounded process records",
            ));
        }
        let guard_name = format!("{}.guard", binding.effect_id);
        let lock = operation.lock(&guard_name)?;
        let inherited = lock.inherited_file()?;
        let record = ProcessRecord {
            schema_version: 1,
            host: self.probe.current()?,
            guard: guard_name,
            binding,
            state: ProcessState::Starting,
        };
        operation.write(&record_name, &record)?;
        Ok(SpawnGuard {
            journal: self.clone(),
            directory: operation,
            record_name,
            record,
            lock: Some(lock),
            inherited: Some(inherited),
            configured: AtomicBool::new(false),
        })
    }
    pub fn observe(&self, operation_id: Uuid) -> Result<Vec<ProcessRecovery>> {
        if operation_id.is_nil() {
            return Err(Error::invalid("process recovery needs operation identity"));
        }
        let path = self.directory.path().join(operation_id.to_string());
        match std::fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(error) => return Err(Error::io("inspect process operation directory", error)),
            Ok(_) => {}
        }
        let operation = PrivateDir::open(&path)?;
        let mut observed = Vec::new();
        for leaf in operation.records()? {
            let record: ProcessRecord = operation.read(&leaf)?;
            if record.schema_version != 1
                || record.binding.lease.runtime_id != self.runtime
                || record.binding.operation_id != operation_id
                || leaf != format!("{}.json", record.binding.effect_id)
                || record.guard != format!("{}.guard", record.binding.effect_id)
            {
                return Err(Error::new(
                    "record_corrupt",
                    "process record crosses runtime, operation or effect identity",
                ));
            }
            let process = match &record.state {
                ProcessState::Running { process } => Some(process.clone()),
                ProcessState::GroupExited { process, .. } => process.clone(),
                ProcessState::Starting => None,
            };
            let guard_metadata = std::fs::symlink_metadata(operation.path().join(&record.guard))
                .map_err(|error| Error::io("inspect retained process guard", error))?;
            use std::os::unix::fs::MetadataExt;
            if !guard_metadata.is_file()
                || guard_metadata.uid() != unsafe { libc::geteuid() }
                || guard_metadata.mode() & 0o077 != 0
                || guard_metadata.nlink() != 1
            {
                return Err(Error::new(
                    "unsafe_storage",
                    "retained process guard is not an owned private regular file",
                ));
            }
            let state = match operation.lock(&record.guard) {
                Err(error) if error.code == "ownership_conflict" => RecoveryState::GuardHeld,
                Err(error) => return Err(error),
                Ok(_guard) => match &record.state {
                    ProcessState::Starting => RecoveryState::StartingUnknown,
                    ProcessState::Running { process } => match self.probe.observe(process)? {
                        ProcessObservation::Running => RecoveryState::ExactProcessRunning,
                        ProcessObservation::Gone | ProcessObservation::DifferentIdentity => {
                            RecoveryState::LeaderGoneTreeUnverified
                        }
                    },
                    ProcessState::GroupExited { process, .. } => {
                        if let Some(process) = process
                            && self.probe.observe(process)? == ProcessObservation::Running
                        {
                            return Err(Error::new(
                                "ownership_conflict",
                                "recorded group exit conflicts with the exact running process",
                            ));
                        }
                        RecoveryState::GroupExitRecordedTreeUnverified
                    }
                },
            };
            observed.push(ProcessRecovery {
                operation_id,
                effect_id: record.binding.effect_id,
                state,
                process,
                all_descendants_exited: false,
            });
        }
        Ok(observed)
    }
    pub fn operations(&self) -> Result<Vec<Uuid>> {
        let mut operations = Vec::new();
        for entry in std::fs::read_dir(self.directory.path())
            .map_err(|error| Error::io("enumerate process operation references", error))?
        {
            let entry =
                entry.map_err(|error| Error::io("read process operation reference", error))?;
            let id = entry
                .file_name()
                .to_str()
                .and_then(|name| Uuid::parse_str(name).ok())
                .ok_or_else(|| Error::new("record_corrupt", "unexpected process-journal entry"))?;
            PrivateDir::open(&entry.path())?;
            operations.push(id);
        }
        operations.sort();
        Ok(operations)
    }
}
/// Owned launch capability. It cannot be reconstructed from a record, and its
/// inherited descriptor keeps the actual OS lock alive across parent death.
pub struct SpawnGuard {
    journal: Arc<ProcessJournal>,
    directory: PrivateDir,
    record_name: String,
    record: ProcessRecord,
    lock: Option<ExclusiveLock>,
    inherited: Option<File>,
    configured: AtomicBool,
}
impl SpawnGuard {
    pub fn binding(&self) -> &SpawnBinding {
        &self.record.binding
    }
    pub fn configure_std(&self, command: &mut std::process::Command) -> Result<()> {
        if self.configured.swap(true, Ordering::SeqCst) {
            return Err(Error::new(
                "ownership_conflict",
                "process guard was already assigned to a spawn",
            ));
        }
        let fd = self
            .inherited
            .as_ref()
            .ok_or_else(|| Error::new("ownership_conflict", "process guard already closed"))?
            .as_raw_fd();
        // SAFETY: the descriptor is owned by this capability throughout spawn;
        // only async-signal-safe fcntl runs after fork. No global FD policy changes.
        unsafe {
            command.pre_exec(move || {
                let flags = libc::fcntl(fd, libc::F_GETFD);
                if flags < 0 || libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        Ok(())
    }
    pub fn configure_tokio(&self, command: &mut tokio::process::Command) -> Result<()> {
        self.configure_std(command.as_std_mut())
    }
    pub fn spawned_std(&mut self, child: &std::process::Child) -> Result<ProcessIdentity> {
        self.record_spawned(child.id())
    }
    pub fn spawned_tokio(&mut self, child: &tokio::process::Child) -> Result<ProcessIdentity> {
        self.record_spawned(child.id().ok_or_else(|| {
            Error::new("process_exit", "owned child exited before identity capture")
        })?)
    }
    fn record_spawned(&mut self, pid: u32) -> Result<ProcessIdentity> {
        if !self.configured.load(Ordering::SeqCst)
            || !matches!(self.record.state, ProcessState::Starting)
        {
            return Err(Error::new(
                "ownership_conflict",
                "process identity must follow its one guarded spawn",
            ));
        }
        let mut observed = Vec::new();
        for binary in std::iter::once(&self.record.binding.executable)
            .chain(self.record.binding.observed_executables.iter())
        {
            observed.extend(self.journal.probe.matching_executable(&binary.path)?);
        }
        observed.sort_by_key(|process| {
            (
                process.pid,
                process.started_seconds,
                process.started_microseconds,
            )
        });
        observed.dedup();
        let matching: Vec<_> = observed
            .into_iter()
            .filter(|process| {
                process.pid == pid
                    && process.parent_pid == self.record.host.pid
                    && process.uid == self.record.host.uid
                    && process.boot_id == self.record.host.boot_id
                    && std::iter::once(&self.record.binding.executable)
                        .chain(self.record.binding.observed_executables.iter())
                        .any(|binary| binary.path == process.executable)
            })
            .collect();
        if matching.len() != 1 {
            return Err(Error::new(
                "operation_uncertain",
                "owned child OS identity cannot be established; retain starting guard",
            ));
        }
        let process = matching
            .into_iter()
            .next()
            .ok_or_else(|| Error::new("operation_uncertain", "owned child identity disappeared"))?;
        self.record.state = ProcessState::Running {
            process: process.clone(),
        };
        self.directory.write(&self.record_name, &self.record)?;
        Ok(process)
    }
    /// Call only after the actual owned Child was reaped and its owned group was
    /// observed clear. Detached-descendant containment remains explicitly unproven.
    pub fn group_exited(&mut self, status: std::process::ExitStatus) -> Result<()> {
        let process = match &self.record.state {
            ProcessState::Running { process } => Some(process.clone()),
            ProcessState::Starting => None,
            ProcessState::GroupExited { .. } => {
                return Err(Error::new(
                    "ownership_conflict",
                    "process exit was already recorded",
                ));
            }
        };
        let mut next = self.record.clone();
        next.state = ProcessState::GroupExited {
            process,
            exit_code: status.code(),
        };
        self.directory.write(&self.record_name, &next)?;
        self.record = next;
        self.inherited.take();
        self.lock.take();
        Ok(())
    }
    pub fn binding_digest(&self) -> Result<String> {
        canonical_digest(1, &self.record.binding)
    }
}
