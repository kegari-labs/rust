//! Acceptance of actual native Codex results. This module never runs commands.
use super::*;
use crate::validation::{ValidationPlan, ValidationPlanRecord};
use kegari_codex::{
    NativeExecutionItem, NativeExecutionObservation, NativeExecutionStatus, NativeItemPhase,
    PROTOCOL_PROFILE,
};

/// Only declared output files are observed; this is not a command environment
/// inventory or an execution grant.
#[derive(Clone, Debug, Default)]
pub(crate) struct CodexArtifactSnapshot {
    files: BTreeMap<PathBuf, Option<CodexArtifactStamp>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CodexArtifactStamp {
    bytes: u64,
    digest: String,
    modified: (u64, u32),
    identity: Option<(u64, u64)>,
    changed: Option<(i64, i64)>,
}
impl CodexArtifactStamp {
    fn fresh_after(&self, before: Option<&Self>) -> bool {
        before.is_none_or(|before| {
            self.bytes != before.bytes
                || self.digest != before.digest
                || self.modified != before.modified
                || self.identity != before.identity
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodexArtifactFreshness {
    pub path: PathBuf,
    before: Option<CodexArtifactStamp>,
    after: CodexArtifactStamp,
}

fn artifact_metadata(
    metadata: &std::fs::Metadata,
) -> Result<(u64, (u64, u32), Option<(u64, u64)>, Option<(i64, i64)>)> {
    let modified = metadata
        .modified()
        .map_err(|e| Error::io("read artifact modification identity", e))?
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| {
            Error::new(
                "artifact_invalid",
                "artifact timestamp predates the supported epoch",
            )
        })?;
    #[cfg(unix)]
    let (identity, changed) = {
        use std::os::unix::fs::MetadataExt;
        (
            Some((metadata.dev(), metadata.ino())),
            Some((metadata.ctime(), metadata.ctime_nsec())),
        )
    };
    #[cfg(not(unix))]
    let (identity, changed) = (None, None);
    Ok((
        metadata.len(),
        (modified.as_secs(), modified.subsec_nanos()),
        identity,
        changed,
    ))
}

fn observe_artifact(path: &Path, limit: u64) -> Result<Option<CodexArtifactStamp>> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(Error::io("inspect declared native artifact", error)),
    };
    let before = artifact_metadata(&metadata)?;
    let bytes = file_bytes(path, limit)?;
    let after = artifact_metadata(
        &std::fs::symlink_metadata(path)
            .map_err(|e| Error::io("recheck declared native artifact", e))?,
    )?;
    if before != after || bytes.len() as u64 != before.0 {
        return Err(Error::new(
            "artifact_changed",
            "declared artifact changed during observation",
        ));
    }
    Ok(Some(CodexArtifactStamp {
        bytes: before.0,
        digest: bytes_digest(&bytes),
        modified: before.1,
        identity: before.2,
        changed: before.3,
    }))
}

impl CodexArtifactSnapshot {
    pub(crate) fn observe_after(&self, isolation: &ValidatedIsolation, limit: u64) -> Result<Self> {
        let mut files = BTreeMap::new();
        let mut total = 0u64;
        for path in self.files.keys() {
            if isolation.artifact(path)? != *path {
                return Err(Error::new(
                    "artifact_conflict",
                    "declared artifact path changed",
                ));
            }
            let observed = observe_artifact(path, limit)?;
            total = total.saturating_add(observed.as_ref().map_or(0, |stamp| stamp.bytes));
            if total > limit {
                return Err(Error::new(
                    "artifact_limit",
                    "declared native outputs exceed the aggregate observation bound",
                ));
            }
            files.insert(path.clone(), observed);
        }
        Ok(Self { files })
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

/// Capture candidates matching this native command. An unrelated inspection
/// command does not acquire artifact requirements merely because a check exists.
pub(crate) fn capture_codex_artifacts(
    workflow: &Workflow,
    plan: Option<&ValidationPlan>,
    isolation: &ValidatedIsolation,
    limits: &ExecutionLimits,
    event: &NativeExecutionObservation,
) -> Result<CodexArtifactSnapshot> {
    let NativeExecutionItem::CommandExecution(command) = &event.item else {
        return Ok(CodexArtifactSnapshot::default());
    };
    let mut paths = BTreeSet::new();
    for check in workflow
        .front_matter
        .checks
        .local
        .values()
        .chain(workflow.front_matter.checks.integrated.values())
    {
        if command_matches(&command.command, &check.argv)
            && command.cwd == isolation.plan().cwd.to_string_lossy()
            && !check.result_artifact.is_empty()
        {
            paths.insert(isolation.artifact(Path::new(&check.result_artifact))?);
        }
    }
    if let Some(plan) = plan {
        for requirement in &plan.record().requirements {
            let scope = isolation.with_cwd(Path::new(&requirement.cwd))?;
            if command_matches(&command.command, &requirement.argv)
                && command.cwd == scope.plan().cwd.to_string_lossy()
            {
                for path in &requirement.artifacts {
                    paths.insert(scope.artifact(Path::new(path))?);
                }
            }
        }
    }
    if paths.len() > 1024 {
        return Err(Error::new(
            "artifact_limit",
            "declared native outputs exceed the existing obligation bound",
        ));
    }
    let mut files = BTreeMap::new();
    let mut total = 0u64;
    for path in paths {
        let observed = observe_artifact(&path, limits.artifact_bytes)?;
        total = total.saturating_add(observed.as_ref().map_or(0, |stamp| stamp.bytes));
        if total > limits.artifact_bytes {
            return Err(Error::new(
                "artifact_limit",
                "declared native outputs exceed the aggregate observation bound",
            ));
        }
        files.insert(path, observed);
    }
    Ok(CodexArtifactSnapshot { files })
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodexCheckProvenance {
    pub schema_version: u32,
    pub protocol_profile: String,
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub command: String,
    pub status: NativeExecutionStatus,
    pub started_item_digest: String,
    pub completed_item_digest: String,
    pub transcript: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<ValidationPlanRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifact_freshness: Vec<CodexArtifactFreshness>,
}

pub(crate) struct CodexCheckInput<'a> {
    pub binding: &'a ExecutionBinding,
    pub workflow: &'a Workflow,
    pub name: &'a str,
    pub kind: CheckKind,
    pub plan: Option<&'a ValidationPlan>,
    pub effective: Option<&'a EffectiveExecutionBinding>,
    pub isolation: &'a ValidatedIsolation,
    pub limits: &'a ExecutionLimits,
    pub secret_values: &'a [String],
    pub before: SourceSnapshot,
    pub after: SourceSnapshot,
    pub started: &'a NativeExecutionObservation,
    pub completed: &'a NativeExecutionObservation,
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
    pub artifacts_before: &'a CodexArtifactSnapshot,
    pub artifacts_after: &'a CodexArtifactSnapshot,
    pub producer_uncertain: bool,
}

pub(crate) fn observe_codex_source(
    git: &Git,
    isolation: &ValidatedIsolation,
    limits: &ExecutionLimits,
) -> Result<SourceSnapshot> {
    limits.validate()?;
    observed_source_snapshot(git, isolation, limits, &BTreeSet::new())
}

fn shell_word(value: &str) -> String {
    if !value.is_empty()
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_./-:@%+=,".contains(&c))
    {
        value.into()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

fn command_matches(command: &str, argv: &[String]) -> bool {
    if argv.is_empty() {
        return false;
    }
    let encoded = argv
        .iter()
        .map(|s| shell_word(s))
        .collect::<Vec<_>>()
        .join(" ");
    command == encoded
        || (argv.len() == 3
            && matches!(
                Path::new(&argv[0]).file_name().and_then(|v| v.to_str()),
                Some("sh" | "bash" | "zsh")
            )
            && matches!(argv[1].as_str(), "-c" | "-lc")
            && command == argv[2])
}

impl CodexCheckProvenance {
    pub(super) fn validate_record(&self, record: &CheckExecutionRecord) -> Result<()> {
        if !matches!(self.schema_version, 1 | 2)
            || (self.schema_version == 1 && !self.artifact_freshness.is_empty())
            || self.protocol_profile != PROTOCOL_PROFILE
            || [&self.thread_id, &self.turn_id, &self.item_id, &self.command]
                .iter()
                .any(|v| v.is_empty())
            || !matches!(
                self.status,
                NativeExecutionStatus::Completed | NativeExecutionStatus::Failed
            )
            || (record.successful && self.status != NativeExecutionStatus::Completed)
            || record.command.exit_code.is_none()
            || !command_matches(&self.command, &record.command.argv)
            || !record.artifacts.iter().any(|a| a.path == self.transcript)
            || record.before.revision != record.binding.revision
        {
            return Err(Error::new(
                "check_record_invalid",
                "Codex result identity or terminal evidence is inconsistent",
            ));
        }
        validate_digest(&self.started_item_digest)?;
        validate_digest(&self.completed_item_digest)?;
        let mut paths = BTreeSet::new();
        for freshness in &self.artifact_freshness {
            validate_digest(&freshness.after.digest)?;
            if let Some(before) = &freshness.before {
                validate_digest(&before.digest)?;
            }
            if !paths.insert(&freshness.path)
                || !freshness.after.fresh_after(freshness.before.as_ref())
                || !record.artifacts.iter().any(|artifact| {
                    artifact.path == freshness.path
                        && artifact.digest == freshness.after.digest
                        && artifact.bytes == freshness.after.bytes
                })
            {
                return Err(Error::new(
                    "check_record_invalid",
                    "native artifact lacks its distinct fresh output identity",
                ));
            }
        }
        if record
            .artifacts
            .iter()
            .any(|artifact| artifact.path != self.transcript && !paths.contains(&artifact.path))
        {
            return Err(Error::new(
                "check_record_invalid",
                "native artifact has no observed freshness",
            ));
        }
        if let Some(plan) = &self.plan {
            plan.validate_shape()?;
            let requirement = plan
                .requirements
                .iter()
                .find(|r| r.id == record.check_name)
                .ok_or_else(|| {
                    Error::new(
                        "check_record_invalid",
                        "Codex result is outside its validation plan",
                    )
                })?;
            if plan.binding != record.binding
                || plan.workflow_digest != record.workflow_digest
                || plan.correction != record.correction
                || plan.kind != record.kind
                || plan.source_digest != record.before.digest()?
                || plan.created_at > record.started_at
                || requirement.argv != record.command.argv
            {
                return Err(Error::new(
                    "check_record_invalid",
                    "Codex result differs from its enrolled source obligation",
                ));
            }
        }
        Ok(())
    }
}

pub(crate) fn accept_codex_check(input: CodexCheckInput<'_>) -> Result<CheckCompletion> {
    let CodexCheckInput {
        binding,
        workflow,
        name,
        kind,
        plan,
        effective,
        isolation,
        limits,
        secret_values,
        before,
        after,
        started,
        completed,
        started_at,
        completed_at,
        artifacts_before,
        artifacts_after,
        producer_uncertain,
    } = input;
    limits.validate()?;
    binding.validate(isolation)?;
    crate::validation::validate_execution_kind(&kind, binding)?;
    if !matches!(
        isolation.plan().role,
        ExecutionRole::Child | ExecutionRole::Validation
    ) {
        return Err(Error::new(
            "scope_violation",
            "this role cannot accept repository execution",
        ));
    }
    if started.phase != NativeItemPhase::Started
        || completed.phase != NativeItemPhase::Completed
        || started.operation_id != binding.operation_id.to_string()
        || started.operation_id != completed.operation_id
        || started.thread_id != completed.thread_id
        || started.turn_id != completed.turn_id
        || started.item_id != completed.item_id
        || completed_at < started_at
    {
        return Err(Error::new(
            "execution_identity",
            "native result lacks its matching admitted start",
        ));
    }
    let (NativeExecutionItem::CommandExecution(begin), NativeExecutionItem::CommandExecution(end)) =
        (&started.item, &completed.item)
    else {
        return Err(Error::new(
            "execution_identity",
            "validation requires actual native command results",
        ));
    };
    if begin.command != end.command
        || begin.cwd != end.cwd
        || begin.id != end.id
        || end.id != completed.item_id
    {
        return Err(Error::new(
            "execution_identity",
            "native command result differs from its observed command identity",
        ));
    }
    let correction = effective
        .map(EffectiveExecutionBinding::provenance)
        .transpose()?;
    if effective.is_some_and(|value| value.binding() != binding) {
        return Err(Error::new(
            "correction_execution_binding",
            "native result differs from its effective correction",
        ));
    }
    let (argv, cwd, declared_artifacts) = if let Some(plan) = plan {
        let record = plan.record();
        let required = plan.requirement(name)?;
        if record.binding != *binding
            || record.workflow_digest != workflow.digest
            || record.kind != kind
            || record.correction != correction
            || record.source_digest != before.digest()?
            || record.created_at > started_at
            || plan.digest() != canonical_digest(1, record)?
        {
            return Err(Error::new(
                "stale_validation_plan",
                "Codex command differs from the enrolled source or obligation",
            ));
        }
        if !required.services.is_empty() {
            return Err(Error::new(
                "codex_service_result_unavailable",
                "declared services require their supported Codex lifecycle results",
            ));
        }
        let scope = isolation.with_cwd(Path::new(&required.cwd))?;
        let artifacts = required
            .artifacts
            .iter()
            .map(|path| scope.artifact(Path::new(path)))
            .collect::<Result<Vec<_>>>()?;
        (required.argv.clone(), scope.plan().cwd.clone(), artifacts)
    } else {
        let checks = match kind {
            CheckKind::Local => &workflow.front_matter.checks.local,
            CheckKind::Integrated => &workflow.front_matter.checks.integrated,
            _ => {
                return Err(Error::invalid(
                    "Codex execution cannot manufacture hosted CI",
                ));
            }
        };
        let required = checks
            .get(name)
            .ok_or_else(|| Error::new("unknown_check", "check is absent from the workflow"))?;
        if !required.services.is_empty() {
            return Err(Error::new(
                "codex_service_result_unavailable",
                "configured services need native lifecycle evidence",
            ));
        }
        let artifacts = if required.result_artifact.is_empty() {
            vec![]
        } else {
            vec![isolation.artifact(Path::new(&required.result_artifact))?]
        };
        (
            required.argv.clone(),
            isolation.plan().cwd.clone(),
            artifacts,
        )
    };
    if end.cwd != cwd.to_string_lossy()
        || !command_matches(&end.command, &argv)
        || before.revision != binding.revision
    {
        return Err(Error::new(
            "execution_identity",
            "native command, cwd or source differs from the selected check",
        ));
    }
    // Only a command bound to the selected obligation may establish an
    // unresolved attempt. A missing or mismatched item is not an attempt.
    if end.exit_code.is_none()
        || !matches!(
            end.status,
            NativeExecutionStatus::Completed | NativeExecutionStatus::Failed
        )
    {
        return Err(Error::new(
            "codex_result_incomplete",
            "selected native command has no known terminal result",
        ));
    }
    if producer_uncertain {
        return Err(Error::new(
            "artifact_producer_uncertain",
            "declared output overlaps another native writer or unresolved command",
        ));
    }
    let mut artifacts = Vec::new();
    let mut artifact_freshness = Vec::new();
    let mut artifact_total = 0u64;
    for path in declared_artifacts {
        let previous = artifacts_before.files.get(&path).ok_or_else(|| {
            Error::new(
                "artifact_freshness_missing",
                "artifact was not declared when the native command started",
            )
        })?;
        let produced = artifacts_after
            .files
            .get(&path)
            .and_then(Option::as_ref)
            .ok_or_else(|| {
                Error::new(
                    "artifact_missing",
                    "native command produced no declared artifact",
                )
            })?;
        if !produced.fresh_after(previous.as_ref()) {
            return Err(Error::new(
                "artifact_stale",
                "declared artifact is unchanged from before the native command",
            ));
        }
        if observe_artifact(&path, limits.artifact_bytes)?.as_ref() != Some(produced) {
            return Err(Error::new(
                "artifact_changed",
                "declared artifact changed after command completion",
            ));
        }
        artifact_total = artifact_total
            .checked_add(produced.bytes)
            .ok_or_else(|| Error::new("artifact_limit", "artifact size overflow"))?;
        if artifact_total > limits.artifact_bytes {
            return Err(Error::new(
                "artifact_limit",
                "combined native artifacts exceed their retained bound",
            ));
        }
        let bytes = file_bytes(&path, limits.artifact_bytes)?;
        if secret_values
            .iter()
            .filter(|value| !value.is_empty())
            .any(|secret| {
                bytes
                    .windows(secret.len())
                    .any(|part| part == secret.as_bytes())
            })
        {
            return Err(Error::new(
                "artifact_secret",
                "declared artifact contains protected material",
            ));
        }
        artifacts.push(ArtifactProof {
            path: path.clone(),
            digest: produced.digest.clone(),
            bytes: produced.bytes,
            validation_operation: binding.operation_id,
            candidate_vector_digest: binding.candidate_vector_digest.clone(),
        });
        artifact_freshness.push(CodexArtifactFreshness {
            path,
            before: previous.clone(),
            after: produced.clone(),
        });
    }
    let mut stdout = end.aggregated_output.clone().unwrap_or_default();
    for secret in secret_values.iter().filter(|s| !s.is_empty()) {
        stdout = stdout.replace(secret, "[REDACTED]");
    }
    let output_truncated = stdout.len() > limits.output_bytes;
    if output_truncated {
        let mut end = limits.output_bytes;
        while !stdout.is_char_boundary(end) {
            end -= 1;
        }
        stdout.truncate(end);
    }
    let command = CommandOutcome {
        schema_version: 1,
        argv,
        cwd,
        isolation_digest: binding.isolation_digest.clone(),
        exit_code: end.exit_code,
        termination_signal: None,
        timed_out: false,
        stdout,
        stderr: String::new(),
        output_truncated,
        elapsed_ms: u64::try_from(end.duration_ms.unwrap_or(0)).unwrap_or(0),
    };
    let blocker = if before != after {
        Some("source changed while the native check ran".into())
    } else if end.status != NativeExecutionStatus::Completed
        || command.exit_code != Some(0)
        || output_truncated
    {
        Some("native check failed or output exceeded its retained bound".into())
    } else {
        None
    };
    let address = canonical_digest(
        1,
        &(
            binding.operation_id,
            &completed.thread_id,
            &completed.turn_id,
            &completed.item_id,
        ),
    )?;
    let transcript = isolation.plan().scratch.join(format!(
        "codex-validation-{}.json",
        address.rsplit(':').next().unwrap_or_default()
    ));
    if isolation.artifact(&transcript)? != transcript {
        return Err(Error::new(
            "artifact_conflict",
            "native transcript escaped admitted scratch",
        ));
    }
    let codex = CodexCheckProvenance {
        schema_version: 2,
        protocol_profile: PROTOCOL_PROFILE.into(),
        thread_id: completed.thread_id.clone(),
        turn_id: completed.turn_id.clone(),
        item_id: completed.item_id.clone(),
        command: end.command.clone(),
        status: end.status,
        started_item_digest: canonical_digest(1, started)?,
        completed_item_digest: canonical_digest(1, completed)?,
        transcript: transcript.clone(),
        plan: plan.map(|p| p.record().clone()),
        artifact_freshness,
    };
    let bytes = serde_json::to_vec(&serde_json::json!({"schema_version":1,"kind":"codex_validation_result",
        "operation_id":binding.operation_id,"source_digest":after.digest()?,"codex":codex,"command":command}))
        .map_err(|_| Error::invalid("encode native validation transcript"))?;
    if artifact_total.saturating_add(bytes.len() as u64) > limits.artifact_bytes {
        return Err(Error::new(
            "artifact_invalid",
            "native transcript exceeds output bound",
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
        .open(&transcript)
        .map_err(|e| Error::io("create native result transcript", e))?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|e| Error::io("retain native result transcript", e))?;
    artifacts.push(ArtifactProof {
        path: transcript,
        digest: bytes_digest(&bytes),
        bytes: bytes.len() as u64,
        validation_operation: binding.operation_id,
        candidate_vector_digest: binding.candidate_vector_digest.clone(),
    });
    let record = CheckExecutionRecord {
        schema_version: 5,
        codex: Some(codex),
        discovered: None,
        correction,
        bundle: None,
        binding: binding.clone(),
        workflow_digest: workflow.digest.clone(),
        check_name: name.into(),
        kind,
        before,
        after,
        artifacts,
        command,
        started_at,
        completed_at,
        evidence_class: ExecutionEvidenceClass::CodexExecution,
        successful: blocker.is_none(),
        blocker,
    };
    record.validate_shape()?;
    if record.successful {
        Ok(CheckCompletion::Passed(VerifiedCheckExecution {
            record,
            effective: effective.cloned(),
            validation_plan: plan.cloned(),
        }))
    } else {
        Ok(CheckCompletion::Failed(FailedCheckExecution {
            record,
            effective: effective.cloned(),
            validation_plan: plan.cloned(),
        }))
    }
}
