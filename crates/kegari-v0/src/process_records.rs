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
    pub lease: Option<Lease>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_owner: Option<crate::host_process::HostProcessIdentity>,
    pub work_key: String,
    pub scope_digest: String,
    pub cwd: PathBuf,
    pub executable: BinaryIdentity,
    pub observed_executables: Vec<BinaryIdentity>,
    pub argv_digest: String,
    pub kind: String,
}
impl SpawnBinding {
    fn runtime_id(&self) -> Result<Uuid> {
        match (&self.lease, &self.control_owner) {
            (Some(lease), None) => Ok(lease.runtime_id),
            (None, Some(owner))
                if owner.operation_id == self.operation_id
                    && owner.work_key == self.work_key
                    && owner.scope_digest == self.scope_digest =>
            {
                Ok(owner.runtime_id)
            }
            _ => Err(Error::new(
                "scope_violation",
                "process record needs exactly one worker or control-host authority",
            )),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProcessState {
    Starting,
    Running {
        process: ProcessIdentity,
    },
    GroupExited {
        process: Option<ProcessIdentity>,
        exit_code: Option<i32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        owned_group: Option<Uuid>,
        #[serde(default, skip_serializing_if = "is_false")]
        guard_release_verified: bool,
    },
}
fn is_false(value: &bool) -> bool {
    !value
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProcessRecord {
    pub schema_version: u32,
    pub binding: SpawnBinding,
    pub host: ProcessIdentity,
    pub guard: String,
    pub guard_device: u64,
    pub guard_inode: u64,
    pub state: ProcessState,
}
impl ProcessRecord {
    fn validate_schema(&self) -> Result<()> {
        if !matches!(self.schema_version, 1 | 2) {
            return Err(Error::new(
                "record_corrupt",
                "unsupported process record schema",
            ));
        }
        if let ProcessState::GroupExited {
            owned_group,
            guard_release_verified,
            ..
        } = &self.state
            && ((self.schema_version == 2) != owned_group.is_some()
                || owned_group.is_some_and(|id| id.is_nil())
                || (self.schema_version == 1 && *guard_release_verified))
        {
            return Err(Error::new(
                "record_corrupt",
                "group exit provenance differs from its schema",
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryState {
    GuardHeld,
    StartingUnknown,
    ExactProcessRunning,
    LeaderGoneTreeUnverified,
    GroupExitRecordedTreeUnverified,
    OwnedGroupExitRecorded,
    GroupExitGuardUnverified,
}
#[derive(Clone, Debug, Serialize)]
pub struct ProcessRecovery {
    pub operation_id: Uuid,
    pub effect_id: Uuid,
    pub binding: SpawnBinding,
    pub state: RecoveryState,
    pub process: Option<ProcessIdentity>,
    /// Original owned group exit was recorded by the schema-2 exit capability,
    /// with guard release verified without observed contrary ownership. This is historical
    /// group evidence, not authority to adopt a process or retire an operation.
    pub owned_group_exited: bool,
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
        if binding.control_owner.is_some() || binding.lease.is_none() {
            return Err(Error::new(
                "access_denied",
                "host-local process requires a sealed host permit",
            ));
        }
        self.begin_owned(binding)
    }
    pub(crate) fn begin_control(
        self: &Arc<Self>,
        binding: SpawnBinding,
        permit: &crate::host_process::LocalProcessPermit,
    ) -> Result<SpawnGuard> {
        permit.validate_journal_binding(&binding)?;
        self.begin_owned(binding)
    }
    fn begin_owned(self: &Arc<Self>, binding: SpawnBinding) -> Result<SpawnGuard> {
        if binding.operation_id.is_nil()
            || binding.effect_id.is_nil()
            || binding.runtime_id()? != self.runtime
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
        if bounded_records(&operation)?.len() >= 4096 {
            return Err(Error::new(
                "process_limit",
                "operation exhausted bounded process records",
            ));
        }
        let guard_name = format!("{}.guard", binding.effect_id);
        let lock = operation
            .lock(&guard_name)
            .map_err(|error| error.detail(serde_json::json!({"stage":"acquire_guard"})))?;
        // A previous claimant may have completed between the first read and
        // acquiring this lock. The durable record is authoritative even after
        // its process released the guard; an existing effect is never replayed.
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
        let (guard_device, guard_inode) = lock.identity()?;
        let inherited = lock.inherited_file()?;
        let host = self.probe.current()?;
        if binding
            .control_owner
            .as_ref()
            .is_some_and(|owner| owner.process != host)
        {
            return Err(Error::new(
                "ownership_conflict",
                "host-local process belongs to another actual control process",
            ));
        }
        let record = ProcessRecord {
            schema_version: 2,
            host,
            guard: guard_name,
            guard_device,
            guard_inode,
            binding,
            state: ProcessState::Starting,
        };
        operation
            .write(&record_name, &record)
            .map_err(|error| error.detail(serde_json::json!({"stage":"persist_starting"})))?;
        Ok(SpawnGuard {
            journal: self.clone(),
            directory: operation,
            record_name,
            record,
            lock: Some(lock),
            inherited: Some(inherited),
            configured: AtomicBool::new(false),
            owned_group: None,
            guard_conflict: false,
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
        for leaf in bounded_records(&operation)? {
            let record: ProcessRecord = operation.read_bounded(&leaf, 64 * 1024)?;
            record.validate_schema()?;
            if record.binding.runtime_id()? != self.runtime
                || record.binding.operation_id != operation_id
                || leaf != format!("{}.json", record.binding.effect_id)
                || record.guard != format!("{}.guard", record.binding.effect_id)
                || record
                    .binding
                    .control_owner
                    .as_ref()
                    .is_some_and(|owner| owner.process != record.host)
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
            let state = match operation.observe_lock_existing(
                &record.guard,
                record.guard_device,
                record.guard_inode,
            ) {
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
                    ProcessState::GroupExited {
                        process,
                        guard_release_verified,
                        ..
                    } => {
                        if let Some(process) = process
                            && self.probe.observe(process)? == ProcessObservation::Running
                        {
                            return Err(Error::new(
                                "ownership_conflict",
                                "recorded group exit conflicts with the exact running process",
                            ));
                        }
                        if record.schema_version == 2 && *guard_release_verified {
                            RecoveryState::OwnedGroupExitRecorded
                        } else if record.schema_version == 2 {
                            RecoveryState::GroupExitGuardUnverified
                        } else {
                            RecoveryState::GroupExitRecordedTreeUnverified
                        }
                    }
                },
            };
            let owned_group_exited = state == RecoveryState::OwnedGroupExitRecorded;
            observed.push(ProcessRecovery {
                operation_id,
                effect_id: record.binding.effect_id,
                binding: record.binding,
                state,
                process,
                owned_group_exited,
                all_descendants_exited: false,
            });
        }
        Ok(observed)
    }
    /// Historical data only. No PID probe, adoption, signal or exit inference.
    pub(crate) fn retained_records(&self, operation_id: Uuid) -> Result<Vec<ProcessRecord>> {
        let path = self.directory.path().join(operation_id.to_string());
        match std::fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(error) => return Err(Error::io("inspect retained process directory", error)),
            Ok(_) => {}
        }
        let directory = PrivateDir::open(&path)?;
        let mut result = Vec::new();
        for leaf in bounded_records(&directory)? {
            let record: ProcessRecord = directory.read_bounded(&leaf, 64 * 1024)?;
            record.validate_schema()?;
            if record.binding.runtime_id()? != self.runtime
                || record.binding.operation_id != operation_id
                || leaf != format!("{}.json", record.binding.effect_id)
                || record.guard != format!("{}.guard", record.binding.effect_id)
            {
                return Err(Error::new(
                    "record_corrupt",
                    "retained process identity changed",
                ));
            }
            let _guard = directory.observe_lock_existing(
                &record.guard,
                record.guard_device,
                record.guard_inode,
            )?;
            result.push(record);
        }
        Ok(result)
    }
    pub fn operations(&self) -> Result<Vec<Uuid>> {
        let mut operations = Vec::new();
        for entry in std::fs::read_dir(self.directory.path())
            .map_err(|error| Error::io("enumerate process operation references", error))?
        {
            if operations.len() >= 65_536 {
                return Err(Error::new(
                    "process_limit",
                    "retained process operations exceed bounded recovery inventory",
                ));
            }
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
    owned_group: Option<(Uuid, u32)>,
    guard_conflict: bool,
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
    pub fn spawned_std(
        &mut self,
        child: &std::process::Child,
        group: &crate::process_group::OwnedGroup,
    ) -> Result<ProcessIdentity> {
        self.record_spawned(child.id(), group)
    }
    pub fn spawned_tokio(
        &mut self,
        child: &tokio::process::Child,
        group: &crate::process_group::OwnedGroup,
    ) -> Result<ProcessIdentity> {
        self.record_spawned(
            child.id().ok_or_else(|| {
                Error::new("process_exit", "owned child exited before identity capture")
            })?,
            group,
        )
    }
    fn record_spawned(
        &mut self,
        pid: u32,
        group: &crate::process_group::OwnedGroup,
    ) -> Result<ProcessIdentity> {
        if !self.configured.load(Ordering::SeqCst)
            || !matches!(self.record.state, ProcessState::Starting)
            || self.owned_group.is_some()
        {
            return Err(Error::new(
                "ownership_conflict",
                "process identity must follow its one guarded spawn",
            ));
        }
        // Bind the actual creation capability before fallible OS metadata
        // capture. A quick child still needs a same-spawn exit token; its PID
        // alone, even if later reused, never matches another group's token.
        self.owned_group = Some(group.bind_child(pid)?);
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
        let mut next = self.record.clone();
        next.state = ProcessState::Running {
            process: process.clone(),
        };
        self.directory.write(&self.record_name, &next)?;
        self.record = next;
        Ok(process)
    }
    /// Persist only the exit capability from this guard's actual owned spawn.
    /// Retain the capability on errors so journal/guard checks can be retried.
    /// Detached-descendant containment remains explicitly unproven.
    pub fn group_exited(&mut self, exit: &crate::process_group::OwnedGroupExit) -> Result<()> {
        let (group, pid) = exit.binding();
        if self.owned_group != Some((group, pid)) {
            return Err(Error::new(
                "ownership_conflict",
                "exit evidence belongs to another owned spawn",
            ));
        }
        if self.guard_conflict {
            return Err(Error::new(
                "operation_uncertain",
                "a holder survived owned group exit; releasing its descriptor does not establish its process exit",
            ));
        }
        let process = match &self.record.state {
            ProcessState::Running { process } => Some(process.clone()),
            ProcessState::Starting => None,
            ProcessState::GroupExited {
                process,
                exit_code,
                owned_group,
                ..
            } => {
                if *owned_group != Some(group) || *exit_code != exit.status().code() {
                    return Err(Error::new(
                        "ownership_conflict",
                        "recorded exit differs from owned evidence",
                    ));
                }
                process.clone()
            }
        };
        if process.as_ref().is_some_and(|process| process.pid != pid) {
            return Err(Error::new(
                "ownership_conflict",
                "captured process differs from owned exit",
            ));
        }
        if !matches!(self.record.state, ProcessState::GroupExited { .. }) {
            let mut next = self.record.clone();
            next.state = ProcessState::GroupExited {
                process,
                exit_code: exit.status().code(),
                owned_group: Some(group),
                guard_release_verified: false,
            };
            self.directory.write(&self.record_name, &next)?;
            self.record = next;
        } else {
            let retained: ProcessRecord =
                self.directory.read_bounded(&self.record_name, 64 * 1024)?;
            retained.validate_schema()?;
            if retained != self.record {
                return Err(Error::new(
                    "record_corrupt",
                    "retained owned exit changed before retirement completed",
                ));
            }
        }
        self.inherited.take();
        self.lock.take();
        // A surviving inherited guard is contrary evidence even after the
        // owned group closes. Do not erase this known lifecycle uncertainty.
        let released = self.directory.observe_lock_existing(
            &self.record.guard,
            self.record.guard_device,
            self.record.guard_inode,
        );
        let _released = match released {
            Ok(released) => released,
            Err(error) if error.code == "ownership_conflict" => {
                self.guard_conflict = true;
                // A previously observed holder can close the descriptor and
                // remain alive. Never infer its exit from a later free lock.
                let mut next = self.record.clone();
                if let ProcessState::GroupExited {
                    guard_release_verified,
                    ..
                } = &mut next.state
                    && *guard_release_verified
                {
                    *guard_release_verified = false;
                    self.directory.write(&self.record_name, &next)?;
                    self.record = next;
                }
                return Err(Error::new(
                    "operation_uncertain",
                    "owned group exited but its inherited process guard remains held",
                ));
            }
            Err(error) => return Err(error),
        };
        let mut next = self.record.clone();
        if let ProcessState::GroupExited {
            guard_release_verified,
            ..
        } = &mut next.state
            && !*guard_release_verified
        {
            *guard_release_verified = true;
            self.directory.write(&self.record_name, &next)?;
            self.record = next;
        }
        Ok(())
    }
    pub fn binding_digest(&self) -> Result<String> {
        canonical_digest(1, &self.record.binding)
    }
}

fn bounded_records(directory: &PrivateDir) -> Result<Vec<String>> {
    let mut records = Vec::new();
    for (index, entry) in std::fs::read_dir(directory.path())
        .map_err(|error| Error::io("enumerate process records", error))?
        .enumerate()
    {
        if index >= 8192 {
            return Err(Error::new(
                "process_limit",
                "retained process records exceed bounded recovery inventory",
            ));
        }
        let entry = entry.map_err(|error| Error::io("read process record entry", error))?;
        let leaf = entry
            .file_name()
            .into_string()
            .map_err(|_| Error::new("record_corrupt", "non-UTF8 process record entry"))?;
        if leaf.ends_with(".json") {
            if records.len() >= 4096 {
                return Err(Error::new(
                    "process_limit",
                    "retained process records exceed bounded recovery inventory",
                ));
            }
            records.push(leaf);
        } else if !leaf.ends_with(".guard") {
            return Err(Error::new(
                "record_corrupt",
                "unexpected retained process record entry",
            ));
        }
    }
    records.sort();
    Ok(records)
}
