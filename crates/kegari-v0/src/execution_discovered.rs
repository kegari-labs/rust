//! Validation enrolls a retained discovered obligation before execution. Generic
//! command records and decoded plan records cannot manufacture these capabilities.
use super::*;
use crate::validation::{ValidationPlan, ValidationPlanRecord};
use std::time::Instant;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveredServiceObservation {
    pub name: String,
    pub start: isolation::CommandObservation,
    pub readiness: isolation::CommandObservation,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveredCheckProvenance {
    pub schema_version: u32,
    pub plan: ValidationPlanRecord,
    pub plan_digest: String,
    pub invocation_id: Uuid,
    pub observation: isolation::CommandObservation,
    pub service_observations: Vec<DiscoveredServiceObservation>,
    pub prepared_at: DateTime<Utc>,
    pub deadline_at: DateTime<Utc>,
    pub command_timeout_ms: u64,
    pub output_bytes: usize,
    pub transcript: PathBuf,
    pub artifacts: Vec<PathBuf>,
}
#[derive(Debug)]
pub(super) struct PreparedDiscovered {
    pub(super) plan: ValidationPlan,
    pub(super) provenance: DiscoveredCheckProvenance,
    pub(super) isolation: ValidatedIsolation,
    pub(super) deadline: Instant,
    source: command::Anchors,
    executables: Vec<command::Anchors>,
    limits: ExecutionLimits,
}
fn current(deadline: Instant) -> Result<()> {
    if Instant::now() >= deadline {
        Err(Error::new(
            "timeout",
            "original discovered validation deadline expired",
        ))
    } else {
        Ok(())
    }
}
fn observe(
    isolation: &ValidatedIsolation,
    argv: &[String],
    deadline: Instant,
) -> Result<(isolation::CommandObservation, command::Anchors)> {
    current(deadline)?;
    let selected = isolation.resolve_executable(
        argv.first()
            .ok_or_else(|| Error::invalid("validation argv is empty"))?,
    )?;
    let canonical = selected
        .canonicalize()
        .map_err(|e| Error::io("resolve validation executable", e))?;
    let anchors =
        command::Anchors::paths(&[&selected, &canonical, Path::new("/usr/bin/sandbox-exec")])?;
    let observation = isolation::observe_command(isolation, argv)?;
    if observation.selected_executable != selected
        || observation.canonical_executable.path != canonical
    {
        return Err(Error::new(
            "stale_command",
            "validation executable changed during observation",
        ));
    }
    anchors.verify(Some(deadline))?;
    Ok((observation, anchors))
}
fn matching_plan(
    executor: &LocalExecutor<'_>,
    binding: &ExecutionBinding,
    effective: Option<&EffectiveExecutionBinding>,
    plan: &ValidationPlan,
) -> Result<()> {
    let original = executor.check_ownership_binding(binding, effective)?;
    let record = plan.record();
    if record.original_binding != *original
        || record.binding != *binding
        || record.correction
            != effective
                .map(EffectiveExecutionBinding::provenance)
                .transpose()?
        || record.kind != CheckKind::Local
        || binding.work.child.is_none()
        || binding.candidate_vector_digest.is_some()
        || plan.digest() != canonical_digest(1, record)?
    {
        return Err(Error::new(
            "validation_plan_binding",
            "discovered obligation differs from actual original/effective child execution",
        ));
    }
    executor.launch_identity(original)
}
impl LocalExecutor<'_> {
    /// Observe the actual clean complete candidate outside the Control gate.
    /// No user artifact declaration can exclude a source path from this identity.
    pub fn validation_source(
        &self,
        binding: &ExecutionBinding,
        effective: Option<&EffectiveExecutionBinding>,
    ) -> Result<SourceSnapshot> {
        let original = self.check_ownership_binding(binding, effective)?;
        self.preflight(original)?;
        self.isolation.revalidate_resource_contents()?;
        source_snapshot(self.git, self.isolation, &self.limits, &BTreeSet::new())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn prepare_discovered_check(
        &self,
        binding: &ExecutionBinding,
        plan: &ValidationPlan,
        id: &str,
        invocation_id: Uuid,
        previous_outputs: &[RetainedArtifact],
        effective: Option<&EffectiveExecutionBinding>,
        mut persist: impl FnMut(&CheckJournalEntry) -> Result<()>,
    ) -> Result<PreparedCheck> {
        let started = Instant::now();
        let prepared_at = Utc::now();
        self.limits.validate()?;
        let deadline = started + Duration::from_millis(self.limits.command_timeout_ms);
        if invocation_id.is_nil() || invocation_id == binding.operation_id {
            return Err(Error::invalid(
                "validation requires a distinct nonnil invocation identity",
            ));
        }
        matching_plan(self, binding, effective, plan)?;
        let requirement = plan.requirement(id)?;
        let original = self.check_ownership_binding(binding, effective)?;
        self.preflight(original)?;
        self.isolation.revalidate_resource_contents()?;
        let isolation = self.isolation.with_cwd(Path::new(&requirement.cwd))?;
        let source = command::Anchors::tree(
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
        self.git.verify_execution_tree(
            &self.isolation.plan().repository,
            &before.revision,
            &BTreeSet::new(),
        )?;
        if before.revision != original.revision || before.digest()? != plan.record().source_digest {
            return Err(Error::new(
                "stale_validation_plan",
                "discovered validation source differs from its retained complete candidate",
            ));
        }
        source.verify(Some(deadline))?;
        let (observation, main_anchor) = observe(&isolation, &requirement.argv, deadline)?;
        let mut executables = vec![main_anchor];
        let mut service_observations = Vec::new();
        for service in &requirement.services {
            let (start, anchor) = observe(&isolation, &service.argv, deadline)?;
            executables.push(anchor);
            let (readiness, anchor) = observe(&isolation, &service.readiness.argv, deadline)?;
            executables.push(anchor);
            service_observations.push(DiscoveredServiceObservation {
                name: service.name.clone(),
                start,
                readiness,
            });
        }
        let transcript = isolation
            .plan()
            .scratch
            .join(format!("validation-transcript-{invocation_id}.json"));
        match std::fs::symlink_metadata(&transcript) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(Error::io("inspect reserved validation transcript", e)),
            Ok(_) => {
                return Err(Error::new(
                    "artifact_conflict",
                    "reserved validation transcript already exists",
                ));
            }
        }
        let mut outputs = BTreeSet::from([transcript.clone()]);
        for requested in &requirement.artifacts {
            let requested = Path::new(requested);
            let path = if requested.is_absolute() {
                requested.to_path_buf()
            } else {
                isolation.plan().cwd.join(requested)
            };
            let path = isolation.artifact(&path)?;
            if path == isolation.plan().scratch
                || !path.starts_with(&isolation.plan().scratch)
                || !outputs.insert(path)
            {
                return Err(Error::new(
                    "artifact_conflict",
                    "discovered artifacts require unique paths inside original scratch",
                ));
            }
        }
        let mut prior = BTreeMap::new();
        for retained in previous_outputs {
            retained.verify(self.limits.artifact_bytes)?;
        }
        for path in &outputs {
            let observed = artifact_stamp(path, self.limits.artifact_bytes)?;
            if let Some(observed) = &observed
                && !previous_outputs.iter().any(|retained| {
                    retained.original.path == *path && retained.original.digest == observed.digest
                })
            {
                return Err(Error::new(
                    "artifact_conflict",
                    "prior validation output must be retained before replacement",
                ));
            }
            prior.insert(path.clone(), observed);
        }
        let resolved = crate::workflow::ResolvedCheck {
            cwd: isolation.plan().cwd.clone(),
            argv: requirement.argv.clone(),
            artifact_roots: vec![isolation.plan().scratch.clone()],
            result_artifact: transcript.clone(),
            services: requirement.services.clone(),
        };
        let provenance = DiscoveredCheckProvenance {
            schema_version: 1,
            plan: plan.record().clone(),
            plan_digest: plan.digest().into(),
            invocation_id,
            observation,
            service_observations,
            prepared_at,
            deadline_at: prepared_at
                + chrono::Duration::milliseconds(self.limits.command_timeout_ms as i64),
            command_timeout_ms: self.limits.command_timeout_ms,
            output_bytes: self.limits.output_bytes,
            transcript,
            artifacts: outputs.iter().cloned().collect(),
        };
        current(deadline)?;
        persist(&CheckJournalEntry::Started {
            binding: Box::new(binding.clone()),
            discovered: Some(Box::new(provenance.clone())),
            correction: effective
                .map(EffectiveExecutionBinding::provenance)
                .transpose()?,
            workflow_digest: plan.record().workflow_digest.clone(),
            check_name: id.into(),
            source_digest: before.digest()?,
            started_at: prepared_at,
        })?;
        Ok(PreparedCheck {
            discovered: Some(PreparedDiscovered {
                plan: plan.clone(),
                provenance,
                isolation,
                deadline,
                source,
                executables,
                limits: self.limits.clone(),
            }),
            binding: binding.clone(),
            effective: effective.cloned(),
            workflow_digest: plan.record().workflow_digest.clone(),
            check_name: id.into(),
            kind: CheckKind::Local,
            resolved,
            outputs,
            prior,
            before,
            started_at: prepared_at,
            evidence_class: self.boundary.evidence_class(),
        })
    }
}
impl PreparedDiscovered {
    pub(super) fn preflight(
        &self,
        executor: &LocalExecutor<'_>,
        binding: &ExecutionBinding,
        effective: Option<&EffectiveExecutionBinding>,
    ) -> Result<()> {
        current(self.deadline)?;
        matching_plan(executor, binding, effective, &self.plan)?;
        if serde_json::to_value(&executor.limits)
            .map_err(|_| Error::invalid("encode execution limits"))?
            != serde_json::to_value(&self.limits)
                .map_err(|_| Error::invalid("encode retained execution limits"))?
        {
            return Err(Error::new(
                "validation_plan_binding",
                "discovered execution bounds changed after preparation",
            ));
        }
        self.isolation.verify_cwd_derivation(executor.isolation)?;
        self.source.verify(Some(self.deadline))?;
        for anchor in &self.executables {
            anchor.verify(Some(self.deadline))?;
        }
        current(self.deadline)
    }
    pub(super) fn finish(
        &self,
        executor: &LocalExecutor<'_>,
        command: &CommandOutcome,
        after: &SourceSnapshot,
    ) -> Result<()> {
        self.isolation.verify_cwd_derivation(executor.isolation)?;
        self.source
            .verify_git(&executor.isolation.plan().repository)?;
        for anchor in &self.executables {
            anchor.verify(None)?;
        }
        if command.schema_version != 1
            || command.argv != self.provenance.observation.requested_argv
            || command.cwd != self.provenance.observation.cwd
            || command.isolation_digest != self.isolation.digest()
            || command.termination_signal.is_some_and(|signal| signal <= 0)
            || (command.exit_code.is_some() && command.termination_signal.is_some())
            || command.stdout.len().saturating_add(command.stderr.len()) > self.limits.output_bytes
        {
            return Err(Error::new(
                "execution_identity",
                "discovered command differs from its original invocation",
            ));
        }
        // Every producer reaches this only after its actual command/service
        // owners closed. The host owns this transcript; project output formats
        // are never interpreted, nor is exit zero semantic sufficiency.
        let transcript = serde_json::json!({
            "schema_version":1,"kind":"observed_validation_command",
            "invocation_id":self.provenance.invocation_id,"plan_digest":self.provenance.plan_digest,
            "source_digest":after.digest()?,"command":command,
        });
        let bytes = serde_json::to_vec(&transcript)
            .map_err(|_| Error::invalid("encode actual validation transcript"))?;
        if bytes.len() as u64 > self.limits.artifact_bytes {
            return Err(Error::new(
                "artifact_invalid",
                "validation transcript exceeds original artifact bound",
            ));
        }
        if self.isolation.artifact(&self.provenance.transcript)? != self.provenance.transcript {
            return Err(Error::new(
                "artifact_conflict",
                "reserved validation transcript path changed",
            ));
        }
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        }
        let mut file = options
            .open(&self.provenance.transcript)
            .map_err(|e| Error::io("create exclusive validation transcript", e))?;
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|e| Error::io("persist actual validation transcript", e))?;
        Ok(())
    }
}

impl DiscoveredCheckProvenance {
    pub(super) fn validate_record(&self, record: &CheckExecutionRecord) -> Result<()> {
        self.plan.validate_shape()?;
        let requirement = self
            .plan
            .requirements
            .iter()
            .find(|r| r.id == record.check_name)
            .ok_or_else(|| {
                Error::new(
                    "check_record_invalid",
                    "discovered check is outside its retained plan",
                )
            })?;
        let expected_deadline = (1..=86_400_000)
            .contains(&self.command_timeout_ms)
            .then(|| {
                self.prepared_at
                    .checked_add_signed(chrono::Duration::milliseconds(
                        self.command_timeout_ms as i64,
                    ))
            })
            .flatten();
        let observed = &self.observation;
        let scratch = observed
            .environment
            .get("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| {
                Error::new(
                    "check_record_invalid",
                    "discovered observation lacks original scratch identity",
                )
            })?;
        let expected_transcript =
            scratch.join(format!("validation-transcript-{}.json", self.invocation_id));
        if self.schema_version != 1
            || self.plan_digest != canonical_digest(1, &self.plan)?
            || self.invocation_id.is_nil()
            || self.invocation_id == record.binding.operation_id
            || record.binding != self.plan.binding
            || record.correction != self.plan.correction
            || record.workflow_digest != self.plan.workflow_digest
            || record.kind != self.plan.kind
            || record.kind != CheckKind::Local
            || record.before.digest()? != self.plan.source_digest
            || record.before.revision != record.binding.revision
            || self.prepared_at != record.started_at
            || self.plan.created_at > self.prepared_at
            || self.output_bytes == 0
            || self.output_bytes > 10 * 1024 * 1024
            || record
                .command
                .stdout
                .len()
                .saturating_add(record.command.stderr.len())
                > self.output_bytes
            || observed.schema_version != 1
            || expected_deadline != Some(self.deadline_at)
            || observed.requested_argv != requirement.argv
            || observed.requested_argv != record.command.argv
            || observed.original_isolation_digest != record.binding.isolation_digest
            || !observed.cwd.is_absolute()
            || !observed.selected_executable.is_absolute()
            || !observed.canonical_executable.path.is_absolute()
            || !observed.launcher.path.is_absolute()
            || !scratch.is_absolute()
            || self.transcript != expected_transcript
            || self.artifacts.is_empty()
            || self.artifacts.len() > 17
            || self.artifacts.windows(2).any(|pair| pair[0] >= pair[1])
            || !self.artifacts.contains(&self.transcript)
            || self
                .artifacts
                .iter()
                .any(|path| path == &scratch || !path.starts_with(&scratch))
            || !record
                .artifacts
                .iter()
                .any(|artifact| artifact.path == self.transcript)
            || record
                .artifacts
                .iter()
                .any(|artifact| !self.artifacts.contains(&artifact.path))
            || (record.successful && record.artifacts.len() != self.artifacts.len())
            || record.artifacts.iter().any(|artifact| {
                !artifact.path.starts_with(&scratch)
                    || artifact.path == scratch
                    || artifact.validation_operation != record.binding.operation_id
                    || artifact.candidate_vector_digest != record.binding.candidate_vector_digest
            })
            || self.service_observations.len() != requirement.services.len()
            || record
                .bundle
                .as_ref()
                .is_some_and(|bundle| bundle.services.len() != requirement.services.len())
            || record.bundle.is_some() != !requirement.services.is_empty()
        {
            return Err(Error::new(
                "check_record_invalid",
                "discovered check provenance contradicts original plan or invocation",
            ));
        }
        for digest in [
            &observed.isolation_digest,
            &observed.launch_digest,
            &observed.original_isolation_digest,
            &observed.canonical_executable.digest,
            &observed.launcher.digest,
        ] {
            crate::evidence::validate_digest(digest)?;
        }
        if !observed
            .backend_policy_digest
            .strip_prefix("sha256:v5:")
            .is_some_and(|value| {
                value.len() == 64
                    && value
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            })
        {
            return Err(Error::new(
                "check_record_invalid",
                "discovered backend policy identity is malformed",
            ));
        }
        for (index, (service, captured)) in requirement
            .services
            .iter()
            .zip(&self.service_observations)
            .enumerate()
        {
            let result = record
                .bundle
                .as_ref()
                .and_then(|bundle| bundle.services.get(index))
                .ok_or_else(|| {
                    Error::new(
                        "check_record_invalid",
                        "discovered service result is missing",
                    )
                })?;
            if captured.name != service.name
                || result.name != service.name
                || result.readiness.argv != service.readiness.argv
                || result.terminal.argv != service.argv
            {
                return Err(Error::new(
                    "check_record_invalid",
                    "discovered service membership changed",
                ));
            }
            for (capture, argv) in [
                (&captured.start, &service.argv),
                (&captured.readiness, &service.readiness.argv),
            ] {
                if capture.schema_version != 1
                    || capture.requested_argv != *argv
                    || capture.cwd != observed.cwd
                    || capture.environment != observed.environment
                    || capture.isolation_digest != observed.isolation_digest
                    || capture.original_isolation_digest != observed.original_isolation_digest
                    || capture.backend_policy_digest != observed.backend_policy_digest
                    || !capture.selected_executable.is_absolute()
                    || !capture.canonical_executable.path.is_absolute()
                    || !capture.launcher.path.is_absolute()
                {
                    return Err(Error::new(
                        "check_record_invalid",
                        "discovered service observation changed original boundary",
                    ));
                }
                for digest in [
                    &capture.launch_digest,
                    &capture.canonical_executable.digest,
                    &capture.launcher.digest,
                ] {
                    crate::evidence::validate_digest(digest)?;
                }
            }
        }
        Ok(())
    }
}
pub(super) fn verify_capability(
    record: &CheckExecutionRecord,
    plan: Option<&ValidationPlan>,
) -> Result<()> {
    match (&record.discovered, plan) {
        (None, None) => Ok(()),
        (Some(provenance), Some(plan))
            if provenance.plan_digest == plan.digest()
                && canonical_digest(1, &provenance.plan)?
                    == canonical_digest(1, plan.record())? =>
        {
            Ok(())
        }
        _ => Err(Error::new(
            "validation_plan_binding",
            "discovered check lacks its original private validation enrollment",
        )),
    }
}

#[cfg(test)]
#[path = "execution_discovered_tests.rs"]
mod tests;
