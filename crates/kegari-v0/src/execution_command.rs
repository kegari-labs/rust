//! Literal discovered commands produce observations, never validation capabilities.
use super::*;
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandRequest {
    pub argv: Vec<String>,
    pub cwd: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandPreparationRecord {
    pub schema_version: u32,
    pub invocation_id: Uuid,
    pub original_binding: ExecutionBinding,
    pub binding: ExecutionBinding,
    pub correction: Option<CorrectionExecutionProvenance>,
    pub request: CommandRequest,
    pub observation: isolation::CommandObservation,
    pub limits: ExecutionLimits,
    pub prepared_at: DateTime<Utc>,
    pub deadline_at: DateTime<Utc>,
    pub before: SourceSnapshot,
    pub before_digest: String,
    pub evidence_class: ExecutionEvidenceClass,
}
impl CommandPreparationRecord {
    /// Fixed typed JSON encoding, including the original legacy floating-point
    /// media limit. This is descriptive identity, not a deserialized authority.
    pub fn digest(&self) -> Result<String> {
        Ok(bytes_digest(&encode(self)?))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandExecutionRecord {
    pub schema_version: u32,
    pub preparation: CommandPreparationRecord,
    pub preparation_digest: String,
    pub dispatch_requested_at: DateTime<Utc>,
    /// Time the original boundary returned its started handle, not a kernel timestamp.
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
    pub after: SourceSnapshot,
    pub after_digest: String,
    pub outcome: CommandOutcome,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum CommandJournalEntry {
    Prepared {
        invocation_id: Uuid,
        operation_id: Uuid,
        preparation_digest: String,
        preparation: Box<CommandPreparationRecord>,
    },
    NotStarted {
        invocation_id: Uuid,
        operation_id: Uuid,
        preparation_digest: String,
        error: Error,
    },
    DispatchRequested {
        invocation_id: Uuid,
        operation_id: Uuid,
        preparation_digest: String,
        at: DateTime<Utc>,
    },
    Started {
        invocation_id: Uuid,
        operation_id: Uuid,
        preparation_digest: String,
        at: DateTime<Utc>,
    },
    Completed {
        invocation_id: Uuid,
        operation_id: Uuid,
        preparation_digest: String,
        record: Box<CommandExecutionRecord>,
    },
    Uncertain {
        invocation_id: Uuid,
        operation_id: Uuid,
        preparation_digest: String,
        error: Error,
        outcome: Option<CommandOutcome>,
    },
}

pub struct PreparedCommand {
    record: CommandPreparationRecord,
    digest: String,
    effective: Option<EffectiveExecutionBinding>,
    isolation: ValidatedIsolation,
    deadline: Instant,
    source: Anchors,
    executable: Anchors,
}
pub struct RunningCommand {
    prepared: PreparedCommand,
    command: Box<dyn StartedCommand>,
    dispatch_requested_at: DateTime<Utc>,
    started_at: DateTime<Utc>,
    persistence_error: Option<Error>,
}
impl RunningCommand {
    pub fn cancellation(&self) -> Arc<dyn CommandCancellation> {
        self.command.cancellation()
    }
}

fn encode(value: &impl Serialize) -> Result<Vec<u8>> {
    serde_json::to_vec(value).map_err(|_| Error::new("command_record", "encode command record"))
}
fn current(deadline: Instant) -> Result<()> {
    if Instant::now() >= deadline {
        Err(Error::new("timeout", "original command deadline expired"))
    } else {
        Ok(())
    }
}

// Metadata is captured outside Control around the actual content reads. The
// gate only rechecks retained entries; parent directory stamps detect additions
// and removals. Never follow source links or treat a missing entry as unchanged.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Stamp {
    device: u64,
    inode: u64,
    mode: u32,
    links: u64,
    bytes: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}
fn stamp(path: &Path) -> Result<Stamp> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let m = std::fs::symlink_metadata(path)
            .map_err(|e| Error::io("inspect original command input", e))?;
        Ok(Stamp {
            device: m.dev(),
            inode: m.ino(),
            mode: m.mode(),
            links: m.nlink(),
            bytes: m.len(),
            modified: (m.mtime(), m.mtime_nsec()),
            changed: (m.ctime(), m.ctime_nsec()),
        })
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err(Error::new(
            "unsupported_platform",
            "command input identity requires Unix metadata",
        ))
    }
}
// Ancestor identity must remain exact, but unrelated sibling creation is not a
// change to the selected executable. Leaf versions retain all metadata.
fn path_stamp(path: &Path, full: bool) -> Result<Stamp> {
    let mut observed = stamp(path)?;
    if !full {
        observed.links = 0;
        observed.bytes = 0;
        observed.modified = (0, 0);
        observed.changed = (0, 0);
    }
    Ok(observed)
}
struct Anchors(BTreeMap<PathBuf, (Stamp, bool)>);
impl Anchors {
    fn tree(root: &Path, limit: usize, deadline: Instant) -> Result<Self> {
        let mut retained = BTreeMap::new();
        let mut pending = vec![root.to_path_buf()];
        while let Some(path) = pending.pop() {
            current(deadline)?;
            if path.as_os_str().len() > 4096 || retained.len() >= limit {
                return Err(Error::new(
                    "source_limit",
                    "command input metadata exceeds bound",
                ));
            }
            let original = stamp(&path)?;
            let metadata = std::fs::symlink_metadata(&path)
                .map_err(|e| Error::io("inspect command input kind", e))?;
            if metadata.is_dir() {
                for entry in
                    std::fs::read_dir(&path).map_err(|e| Error::io("list command inputs", e))?
                {
                    if retained
                        .len()
                        .saturating_add(pending.len())
                        .saturating_add(1)
                        >= limit
                    {
                        return Err(Error::new(
                            "source_limit",
                            "command input metadata exceeds bound",
                        ));
                    }
                    pending.push(
                        entry
                            .map_err(|e| Error::io("read command input entry", e))?
                            .path(),
                    );
                }
            } else if !metadata.is_file() && !metadata.is_symlink() {
                return Err(Error::new(
                    "source_limit",
                    "command input contains unsupported file kind",
                ));
            }
            if stamp(&path)? != original {
                return Err(Error::new(
                    "stale_command",
                    "command input changed during observation",
                ));
            }
            retained.insert(path, (original, true));
        }
        Ok(Self(retained))
    }
    fn paths(paths: &[&Path]) -> Result<Self> {
        let mut retained = BTreeMap::new();
        for path in paths {
            // Alias/parent versions are retained as well as the canonical target.
            for parent in path.ancestors() {
                let full = parent == *path;
                let original = path_stamp(parent, full)?;
                retained
                    .entry(parent.to_path_buf())
                    .and_modify(|entry: &mut (Stamp, bool)| {
                        if full {
                            *entry = (original.clone(), true);
                        }
                    })
                    .or_insert((original, full));
            }
        }
        Ok(Self(retained))
    }
    fn verify(&self, deadline: Option<Instant>) -> Result<()> {
        for (path, (original, full)) in &self.0 {
            if let Some(deadline) = deadline {
                current(deadline)?;
            }
            if &path_stamp(path, *full)? != original {
                return Err(Error::new(
                    "stale_command",
                    "an original command input changed",
                ));
            }
        }
        Ok(())
    }
    fn verify_git(&self, repository: &Path) -> Result<()> {
        let git = repository.join(".git");
        for (path, (original, full)) in &self.0 {
            if path.starts_with(&git) && &path_stamp(path, *full)? != original {
                return Err(Error::new(
                    "stale_command",
                    "command changed admitted Git metadata",
                ));
            }
        }
        Ok(())
    }
}

impl LocalExecutor<'_> {
    pub fn prepare_command(
        &self,
        binding: &ExecutionBinding,
        request: &CommandRequest,
        invocation_id: Uuid,
        effective: Option<&EffectiveExecutionBinding>,
        mut persist: impl FnMut(&CommandJournalEntry) -> Result<()>,
    ) -> Result<PreparedCommand> {
        let started = Instant::now();
        let prepared_at = Utc::now();
        self.limits.validate()?;
        let deadline = started + Duration::from_millis(self.limits.command_timeout_ms);
        if request.argv.is_empty()
            || request.argv.len() > 128
            || request.argv[0].is_empty()
            || request
                .argv
                .iter()
                .any(|arg| arg.len() > 4096 || arg.contains('\0'))
            || request
                .argv
                .iter()
                .try_fold(0usize, |sum, arg| sum.checked_add(arg.len() + 1))
                .is_none_or(|sum| sum > 65_536)
            || invocation_id.is_nil()
            || request.cwd.is_empty()
            || request.cwd.len() > 4096
            || request.cwd.contains('\0')
        {
            return Err(Error::new(
                "invalid_command",
                "command requires a bounded cwd and nonnil invocation",
            ));
        }
        let original = self.check_ownership_binding(binding, effective)?;
        self.preflight(original)?;
        self.isolation.revalidate_resource_contents()?;
        let isolation = self.isolation.with_cwd(Path::new(&request.cwd))?;
        let source = Anchors::tree(
            &self.isolation.plan().repository,
            self.limits.source_files,
            deadline,
        )?;
        let before = observed_source_snapshot_until(
            self.git,
            self.isolation,
            &self.limits,
            &BTreeSet::new(),
            Some(deadline),
        )?;
        if before.revision != original.revision {
            return Err(Error::new(
                "stale_revision",
                "command source HEAD differs from admission",
            ));
        }
        source.verify(Some(deadline))?;
        // First resolve using the real domain, retain aliases before reading bytes,
        // then compare the actual rendered selection and original metadata again.
        let selected = isolation.resolve_executable(
            request
                .argv
                .first()
                .ok_or_else(|| Error::new("invalid_command", "command argv is empty"))?,
        )?;
        let canonical = selected
            .canonicalize()
            .map_err(|e| Error::io("resolve command executable", e))?;
        let executable =
            Anchors::paths(&[&selected, &canonical, Path::new("/usr/bin/sandbox-exec")])?;
        let observation = isolation::observe_command(&isolation, &request.argv)?;
        if observation.selected_executable != selected
            || observation.canonical_executable.path != canonical
        {
            return Err(Error::new(
                "stale_command",
                "command executable changed during observation",
            ));
        }
        executable.verify(Some(deadline))?;
        let record = CommandPreparationRecord {
            schema_version: 1,
            invocation_id,
            original_binding: original.clone(),
            binding: binding.clone(),
            correction: effective
                .map(EffectiveExecutionBinding::provenance)
                .transpose()?,
            request: request.clone(),
            observation,
            limits: self.limits.clone(),
            prepared_at,
            deadline_at: prepared_at
                + chrono::Duration::milliseconds(self.limits.command_timeout_ms as i64),
            before_digest: before.digest()?,
            before,
            evidence_class: self.boundary.evidence_class(),
        };
        let digest = record.digest()?;
        persist(&CommandJournalEntry::Prepared {
            invocation_id,
            operation_id: original.operation_id,
            preparation_digest: digest.clone(),
            preparation: Box::new(record.clone()),
        })?;
        Ok(PreparedCommand {
            record,
            digest,
            effective: effective.cloned(),
            isolation,
            deadline,
            source,
            executable,
        })
    }

    pub fn preflight_command(&self, prepared: &PreparedCommand) -> Result<()> {
        current(prepared.deadline)?;
        let original =
            self.check_ownership_binding(&prepared.record.binding, prepared.effective.as_ref())?;
        if original != &prepared.record.original_binding
            || encode(&self.limits)? != encode(&prepared.record.limits)?
            || self.boundary.evidence_class() != prepared.record.evidence_class
        {
            return Err(Error::new(
                "execution_binding",
                "prepared command owner, limits or boundary changed",
            ));
        }
        self.launch_identity(original)?;
        prepared.isolation.verify_cwd_derivation(self.isolation)?;
        prepared.source.verify(Some(prepared.deadline))?;
        prepared.executable.verify(Some(prepared.deadline))?;
        self.boundary.preflight(&prepared.isolation)?;
        current(prepared.deadline)
    }

    /// Consumes an actual unstarted preparation. A decoded event cannot call it.
    pub fn refuse_command(
        &self,
        prepared: PreparedCommand,
        error: Error,
        mut persist: impl FnMut(&CommandJournalEntry) -> Result<()>,
    ) -> Result<()> {
        persist(&CommandJournalEntry::NotStarted {
            invocation_id: prepared.record.invocation_id,
            operation_id: prepared.record.original_binding.operation_id,
            preparation_digest: prepared.digest,
            error,
        })
    }

    pub fn start_command(
        &self,
        prepared: PreparedCommand,
        mut persist: impl FnMut(&CommandJournalEntry) -> Result<()>,
    ) -> Result<RunningCommand> {
        if let Err(error) = self.preflight_command(&prepared) {
            self.refuse_command(prepared, error.clone(), &mut persist)?;
            return Err(error);
        }
        let dispatch_requested_at = Utc::now();
        persist(&CommandJournalEntry::DispatchRequested {
            invocation_id: prepared.record.invocation_id,
            operation_id: prepared.record.original_binding.operation_id,
            preparation_digest: prepared.digest.clone(),
            at: dispatch_requested_at,
        })?;
        // Persistence can consume time or observe an external change. Repeat the
        // process-free gate without renewing the original cutoff.
        let started = self.preflight_command(&prepared).and_then(|()| {
            self.boundary.start_until(
                &prepared.isolation,
                &prepared.record.request.argv,
                prepared.deadline,
                prepared.record.limits.output_bytes,
            )
        });
        let command = match started {
            Ok(command) => command,
            Err(error) => return Err(uncertain(&prepared, error, None, &mut persist)),
        };
        let started_at = Utc::now();
        let persistence_error = persist(&CommandJournalEntry::Started {
            invocation_id: prepared.record.invocation_id,
            operation_id: prepared.record.original_binding.operation_id,
            preparation_digest: prepared.digest.clone(),
            at: started_at,
        })
        .err();
        if persistence_error.is_some() {
            let _ = command.cancellation().cancel();
        }
        // No fallible early return is allowed after the actual owner was received.
        Ok(RunningCommand {
            prepared,
            command,
            dispatch_requested_at,
            started_at,
            persistence_error,
        })
    }

    pub async fn complete_command(
        &self,
        running: RunningCommand,
        mut persist: impl FnMut(&CommandJournalEntry) -> Result<()>,
    ) -> Result<CommandExecutionRecord> {
        let RunningCommand {
            prepared,
            command,
            dispatch_requested_at,
            started_at,
            persistence_error,
        } = running;
        let cancellation = command.cancellation();
        let outcome = match command.wait().await {
            Ok(outcome) => outcome,
            Err(error) => return Err(uncertain(&prepared, error, None, &mut persist)),
        };
        let collected =
            (|| {
                if !cancellation.exited()? {
                    return Err(Error::new(
                        "operation_uncertain",
                        "original command closure is not established",
                    ));
                }
                if let Some(error) = persistence_error {
                    return Err(error);
                }
                self.launch_identity(self.check_ownership_binding(
                    &prepared.record.binding,
                    prepared.effective.as_ref(),
                )?)?;
                prepared.isolation.verify_cwd_derivation(self.isolation)?;
                self.isolation.revalidate_resource_contents()?;
                prepared.executable.verify(None)?;
                if outcome.schema_version != 1
                    || outcome.argv != prepared.record.request.argv
                    || outcome.cwd != prepared.isolation.plan().cwd
                    || outcome.isolation_digest != prepared.isolation.digest()
                    || (outcome.exit_code.is_some() && outcome.termination_signal.is_some())
                    || outcome.stdout.len().saturating_add(outcome.stderr.len())
                        > prepared.record.limits.output_bytes
                {
                    return Err(Error::new(
                        "command_observation",
                        "actual command outcome contradicts its preparation",
                    ));
                }
                prepared
                    .source
                    .verify_git(&self.isolation.plan().repository)?;
                // This is bounded owned evidence collection after actual closure, not
                // a new command allowance. Timed-out outcomes must remain recordable.
                let collection_deadline = Instant::now()
                    + Duration::from_millis(prepared.record.limits.command_timeout_ms);
                let after_anchors = Anchors::tree(
                    &self.isolation.plan().repository,
                    prepared.record.limits.source_files,
                    collection_deadline,
                )?;
                let after = observed_source_snapshot_until(
                    self.git,
                    self.isolation,
                    &prepared.record.limits,
                    &BTreeSet::new(),
                    Some(collection_deadline),
                )?;
                after_anchors.verify(Some(collection_deadline))?;
                if after.revision != prepared.record.original_binding.revision {
                    return Err(Error::new(
                        "stale_revision",
                        "command changed the admitted source HEAD",
                    ));
                }
                for name in prepared
                    .record
                    .before
                    .files
                    .keys()
                    .chain(after.files.keys())
                {
                    if prepared.record.before.files.get(name) != after.files.get(name) {
                        let path = self.isolation.plan().repository.join(name);
                        if !self.isolation.plan().source_writes.iter().any(|grant| {
                            path == grant.path || (grant.directory && path.starts_with(&grant.path))
                        }) {
                            return Err(Error::new(
                                "scope_violation",
                                "command changed source outside its original write grants",
                            ));
                        }
                    }
                }
                Ok(after)
            })();
        let after = match collected {
            Ok(after) => after,
            Err(error) => return Err(uncertain(&prepared, error, Some(outcome), &mut persist)),
        };
        let after_digest = match after.digest() {
            Ok(digest) => digest,
            Err(error) => return Err(uncertain(&prepared, error, Some(outcome), &mut persist)),
        };
        let record = CommandExecutionRecord {
            schema_version: 1,
            preparation: prepared.record.clone(),
            preparation_digest: prepared.digest.clone(),
            dispatch_requested_at,
            started_at,
            completed_at: Utc::now(),
            after_digest,
            after,
            outcome,
        };
        if let Err(error) = persist(&CommandJournalEntry::Completed {
            invocation_id: record.preparation.invocation_id,
            operation_id: record.preparation.original_binding.operation_id,
            preparation_digest: prepared.digest.clone(),
            record: Box::new(record.clone()),
        }) {
            return Err(uncertain(
                &prepared,
                error,
                Some(record.outcome),
                &mut persist,
            ));
        }
        Ok(record)
    }
}

fn uncertain(
    prepared: &PreparedCommand,
    error: Error,
    outcome: Option<CommandOutcome>,
    persist: &mut impl FnMut(&CommandJournalEntry) -> Result<()>,
) -> Error {
    match persist(&CommandJournalEntry::Uncertain {
        invocation_id: prepared.record.invocation_id,
        operation_id: prepared.record.original_binding.operation_id,
        preparation_digest: prepared.digest.clone(),
        error: error.clone(),
        outcome,
    }) {
        Ok(()) => error,
        Err(journal) => Error::new(
            "operation_uncertain",
            "command observation and uncertainty persistence failed",
        )
        .detail(serde_json::json!({"original":error,"journal":journal})),
    }
}
