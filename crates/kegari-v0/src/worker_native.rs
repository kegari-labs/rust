//! Codex execution observations, bound to the existing admitted operation.
//! This module never launches repository commands or adopts their processes.
use super::*;
use crate::execution::{
    CheckCompletion, CodexArtifactSnapshot, CodexCheckInput, SourceSnapshot,
    capture_codex_artifacts,
};
use kegari_codex::{
    NativeExecutionItem, NativeExecutionObservation, NativeExecutionStatus, NativeItemPhase,
};

#[derive(Default)]
pub(super) struct NativeResults {
    commands: BTreeMap<(String, String, String), NativeCommand>,
    bootstrap: BTreeMap<
        (String, String, String),
        (
            NativeExecutionObservation,
            Option<NativeExecutionObservation>,
        ),
    >,
    file_generation: u64,
    active_file_changes: BTreeSet<(String, String, String)>,
}
struct NativeCommand {
    started: NativeExecutionObservation,
    before: SourceSnapshot,
    started_at: chrono::DateTime<Utc>,
    generation: u64,
    artifacts_before: CodexArtifactSnapshot,
    overlapped: bool,
    completed: Option<(
        NativeExecutionObservation,
        SourceSnapshot,
        chrono::DateTime<Utc>,
        CodexArtifactSnapshot,
    )>,
    accepted: BTreeSet<String>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct OutputCursor {
    thread_id: String,
    turn_id: String,
    item_id: String,
    source_digest: Option<String>,
    output_digest: String,
}
impl OutputCursor {
    fn for_output(
        event: &NativeExecutionObservation,
        source_digest: Option<String>,
        output: &str,
    ) -> Self {
        Self {
            thread_id: event.thread_id.clone(),
            turn_id: event.turn_id.clone(),
            item_id: event.item_id.clone(),
            source_digest,
            output_digest: bytes_digest(output.as_bytes()),
        }
    }
}
fn encoded_len(value: &Value) -> Result<usize> {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .map_err(|_| Error::invalid("native result encoding"))
}
fn missing_output() -> Error {
    Error::new(
        "execution_result_missing",
        "completed command output is unavailable for this exact thread, turn and item",
    )
}

pub(super) fn record_tool() -> Value {
    json!({"type":"function","name":"validation_record",
        "description":"Associate an actual completed Codex command item with a required check. This records observed evidence; it never executes a command. Read execution_results for item IDs. Missing, stale, unknown or mismatched results cannot pass.",
        "inputSchema":{"type":"object","additionalProperties":false,"properties":{
            "item_id":{"type":"string"},"turn_id":{"type":"string"},
            "name":{"type":"string"},"kind":{"type":"string","enum":["local","integrated"]},
            "plan_digest":{"type":"string"},"obligation_id":{"type":"string"}},
            "required":["item_id"],"oneOf":[{"required":["name","kind"]},{"required":["plan_digest","obligation_id"]}]}})
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordArgs {
    item_id: String,
    #[serde(default)]
    turn_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    plan_digest: Option<String>,
    #[serde(default)]
    obligation_id: Option<String>,
}
impl ScopedWorker {
    pub(super) fn observe_native_execution(
        &mut self,
        state: &SessionState,
        event: &NativeExecutionObservation,
    ) -> Result<()> {
        self.verify_session_contract(state)?;
        if event.operation_id != self.permit.operation_id.to_string()
            || state.thread_id.as_deref() != Some(event.thread_id.as_str())
            || !self
                .contract
                .instructions()
                .manifest
                .permissions
                .repository_code_execution
        {
            return Err(Error::new(
                "scope_violation",
                "native repository operation is outside this admitted execution role",
            ));
        }
        let key = (
            event.thread_id.clone(),
            event.turn_id.clone(),
            event.item_id.clone(),
        );
        if let Some(bootstrap) = &self.contract.unborn {
            bootstrap.revalidate(&self.contract.isolation)?;
            match event.phase {
                NativeItemPhase::Started => {
                    if self.native.bootstrap.len() >= MAX_CALLS
                        || self.native.bootstrap.contains_key(&key)
                    {
                        return Err(Error::new(
                            "protocol_error",
                            "bootstrap item identity reused or observation bound exhausted",
                        ));
                    }
                    bootstrap.started(&key)?;
                    self.native
                        .bootstrap
                        .insert(key.clone(), (event.clone(), None));
                }
                NativeItemPhase::Completed => {
                    let (start, completed) =
                        self.native.bootstrap.get_mut(&key).ok_or_else(|| {
                            Error::new(
                                "execution_result_missing",
                                "bootstrap result lacks its observed start",
                            )
                        })?;
                    let same = match (&start.item, &event.item) {
                        (
                            NativeExecutionItem::CommandExecution(a),
                            NativeExecutionItem::CommandExecution(b),
                        ) => a.command == b.command && a.cwd == b.cwd,
                        (
                            NativeExecutionItem::FileChange(_),
                            NativeExecutionItem::FileChange(_),
                        ) => true,
                        _ => false,
                    };
                    if completed.is_some() || !same {
                        return Err(Error::new(
                            "execution_result_mismatch",
                            "bootstrap completion changed its item identity",
                        ));
                    }
                    *completed = Some(event.clone());
                }
            }
            let mut value = serde_json::to_value(event)
                .map_err(|_| Error::invalid("bootstrap observation encoding"))?;
            redact_value(&mut value, &self.contract.known_secrets);
            self.journal.append("codex_bootstrap_observed",&json!({"observation":value,"baseline":Value::Null,"branch":bootstrap.proof.branch(),"checks_accepted":false}))?;
            if event.phase == NativeItemPhase::Completed {
                let known = match &event.item {
                    NativeExecutionItem::CommandExecution(command) => {
                        matches!(
                            command.status,
                            NativeExecutionStatus::Completed | NativeExecutionStatus::Failed
                        ) && command.exit_code.is_some()
                    }
                    NativeExecutionItem::FileChange(files) => matches!(
                        files.status,
                        NativeExecutionStatus::Completed
                            | NativeExecutionStatus::Failed
                            | NativeExecutionStatus::Declined
                    ),
                };
                bootstrap.completed(&key, known)?;
            }
            return Ok(());
        }
        let execution = self.execution.as_ref().ok_or_else(|| {
            Error::new(
                "worker_binding",
                "native repository result requires owned source",
            )
        })?;
        match (&event.item, event.phase) {
            (NativeExecutionItem::CommandExecution(_), NativeItemPhase::Started) => {
                if self.native.commands.contains_key(&key)
                    || self.native.commands.len() >= MAX_CALLS
                {
                    return Err(Error::new(
                        "protocol_error",
                        "native command identity was reused or result bound exhausted",
                    ));
                }
                let before = execution.observe_source(&self.contract.isolation)?;
                if before.revision != execution.binding.revision {
                    return Err(Error::new(
                        "stale_revision",
                        "native command source differs from admitted repository",
                    ));
                }
                let artifacts_before = capture_codex_artifacts(
                    &self.contract.workflow,
                    self.current_validation_plan(),
                    &self.contract.isolation,
                    &execution.limits,
                    event,
                )?;
                let mut overlapped = !self.native.active_file_changes.is_empty();
                for pending in self
                    .native
                    .commands
                    .values_mut()
                    .filter(|pending| pending.completed.is_none())
                {
                    pending.overlapped = true;
                    overlapped = true;
                }
                self.native.commands.insert(
                    key,
                    NativeCommand {
                        started: event.clone(),
                        before,
                        started_at: Utc::now(),
                        generation: self.native.file_generation,
                        artifacts_before,
                        overlapped,
                        completed: None,
                        accepted: BTreeSet::new(),
                    },
                );
            }
            (NativeExecutionItem::CommandExecution(command), NativeItemPhase::Completed) => {
                let prior = self.native.commands.get(&key).ok_or_else(|| {
                    Error::new(
                        "execution_result_missing",
                        "completed command lacks its observed start and source binding",
                    )
                })?;
                let NativeExecutionItem::CommandExecution(start) = &prior.started.item else {
                    unreachable!()
                };
                if prior.completed.is_some()
                    || command.command != start.command
                    || command.cwd != start.cwd
                {
                    return Err(Error::new(
                        "execution_result_mismatch",
                        "native command completion changed its original command or cwd",
                    ));
                }
                let after = execution.observe_source(&self.contract.isolation)?;
                let artifacts_after = prior
                    .artifacts_before
                    .observe_after(&self.contract.isolation, execution.limits.artifact_bytes)?;
                self.native
                    .commands
                    .get_mut(&key)
                    .expect("looked up above")
                    .completed = Some((event.clone(), after, Utc::now(), artifacts_after));
            }
            (NativeExecutionItem::FileChange(_), phase) => {
                if phase == NativeItemPhase::Started {
                    self.native.active_file_changes.insert(key);
                } else {
                    self.native.active_file_changes.remove(&key);
                }
                for pending in self
                    .native
                    .commands
                    .values_mut()
                    .filter(|pending| pending.completed.is_none())
                {
                    pending.overlapped = true;
                }
                self.native.file_generation =
                    self.native.file_generation.checked_add(1).ok_or_else(|| {
                        Error::new("tool_limit", "native file generation overflow")
                    })?;
            }
        }
        // This is a workflow observation, not per-command admission or a process journal.
        let mut value = serde_json::to_value(event)
            .map_err(|_| Error::invalid("native observation encoding"))?;
        redact_value(&mut value, &self.contract.known_secrets);
        value["observation_digest"] = json!(canonical_digest(1, event)?);
        self.journal.append("codex_execution_observed", &value)?;
        Ok(())
    }

    pub(super) fn execution_results(&self, args: &ContextRead) -> Result<Value> {
        if let Some(cursor) = &args.output_cursor {
            if args.results_digest.is_some() {
                return Err(Error::invalid(
                    "output pages cannot use a result-list cursor",
                ));
            }
            return self.execution_output_page(
                cursor,
                args.offset
                    .ok_or_else(|| Error::invalid("output page requires a byte offset"))?,
            );
        }
        let completed: Vec<_> = if self.contract.is_unborn_bootstrap() {
            self.native
                .bootstrap
                .values()
                .rev()
                .filter_map(|(_, event)| event.as_ref().map(|event| (event, None, None)))
                .collect()
        } else {
            self.native
                .commands
                .values()
                .rev()
                .filter_map(|command| {
                    command.completed.as_ref().map(|(event, source, _, _)| {
                        (
                            event,
                            Some(source),
                            Some(
                                command.before == *source
                                    && command.generation == self.native.file_generation,
                            ),
                        )
                    })
                })
                .collect()
        };
        // Completed observations cannot be replaced. Their exact identities bind
        // list offsets; a newly completed item requires restarting the list.
        let identities: Vec<_> = completed
            .iter()
            .map(|(event, _, _)| {
                (
                    &event.operation_id,
                    &event.thread_id,
                    &event.turn_id,
                    &event.item_id,
                )
            })
            .collect();
        let results_digest = canonical_digest(1, &identities)?;
        let offset = args.offset.unwrap_or(0);
        if offset > completed.len()
            || offset == completed.len() && !completed.is_empty()
            || offset > 0 && args.results_digest.is_none()
            || args
                .results_digest
                .as_ref()
                .is_some_and(|digest| *digest != results_digest)
        {
            return Err(Error::new(
                "stale_context",
                "result-list cursor is stale or outside its bound",
            ));
        }
        let mut entries = Vec::new();
        let mut bytes = 0usize;
        for (event, source, unchanged) in completed.iter().skip(offset) {
            let source_digest = source.map(SourceSnapshot::digest).transpose()?;
            let mut value = match &event.item {
                NativeExecutionItem::CommandExecution(result) => {
                    let mut value = json!({"thread_id":event.thread_id,"turn_id":event.turn_id,"item_id":event.item_id,
                        "command":result.command,"cwd":result.cwd,"status":result.status,"exit_code":result.exit_code,
                        "output":result.aggregated_output,"source_digest":source_digest,"source_unchanged":unchanged});
                    redact_value(&mut value, &self.contract.known_secrets);
                    if let Some(output) = value["output"].as_str() {
                        let cursor = OutputCursor::for_output(event, source_digest, output);
                        let output_bytes = output.len();
                        value["output_cursor"] = serde_json::to_value(cursor)
                            .map_err(|_| Error::invalid("output cursor encoding"))?;
                        value["output_bytes"] = json!(output_bytes);
                    }
                    value
                }
                NativeExecutionItem::FileChange(_) => {
                    let mut value = serde_json::to_value(event)
                        .map_err(|_| Error::invalid("native result encoding"))?;
                    redact_value(&mut value, &self.contract.known_secrets);
                    value
                }
            };
            if encoded_len(&value)? > MAX_TOOL_BYTES / 2 {
                value["output"] = json!({"paged":"pass output_cursor with offset zero"});
            }
            if encoded_len(&value)? > MAX_TOOL_BYTES / 2 {
                // Large command text or patch summaries must not hide the output
                // cursor or make every later result inaccessible.
                value = json!({"thread_id":event.thread_id,"turn_id":event.turn_id,"item_id":event.item_id,
                    "source_digest":value["source_digest"],"output_cursor":value["output_cursor"],
                    "output_bytes":value["output_bytes"],"observation_digest":canonical_digest(1,event)?,
                    "summary_omitted":"native item metadata exceeds the context page bound"});
            }
            let length = encoded_len(&value)?;
            if bytes + length > MAX_TOOL_BYTES - 2048 {
                break;
            }
            bytes += length;
            entries.push(value);
        }
        let next = offset + entries.len();
        if next == offset && next < completed.len() {
            return Err(Error::new(
                "tool_limit",
                "native result identity exceeds the context bound",
            ));
        }
        bounded_value(
            json!({"schema_version":2,"execution_layer":"codex_app_server",
            "operation_id":self.permit.operation_id,"baseline_absent":self.contract.is_unborn_bootstrap(),
            "results_digest":results_digest,"total":completed.len(),"offset":offset,
            "next_offset":if next == completed.len() {None} else {Some(next)},"entries":entries}),
        )
    }

    fn execution_output_page(&self, cursor: &OutputCursor, offset: usize) -> Result<Value> {
        let key = (
            cursor.thread_id.clone(),
            cursor.turn_id.clone(),
            cursor.item_id.clone(),
        );
        let (event, source_digest) = if self.contract.is_unborn_bootstrap() {
            let (_, completed) = self.native.bootstrap.get(&key).ok_or_else(missing_output)?;
            (completed.as_ref().ok_or_else(missing_output)?, None)
        } else {
            let command = self.native.commands.get(&key).ok_or_else(missing_output)?;
            let (event, source, _, _) = command.completed.as_ref().ok_or_else(missing_output)?;
            (event, Some(source.digest()?))
        };
        let NativeExecutionItem::CommandExecution(command) = &event.item else {
            return Err(missing_output());
        };
        let mut output = Value::String(
            command
                .aggregated_output
                .clone()
                .ok_or_else(missing_output)?,
        );
        // Redact the complete output before slicing: a secret spanning a page
        // boundary must never leak through separately processed fragments.
        redact_value(&mut output, &self.contract.known_secrets);
        let output = output.as_str().expect("redaction preserves strings");
        if *cursor != OutputCursor::for_output(event, source_digest, output) {
            return Err(Error::new(
                "stale_context",
                "output cursor belongs to another result, source or output",
            ));
        }
        if offset > output.len()
            || !output.is_char_boundary(offset)
            || offset == output.len() && !output.is_empty()
        {
            return Err(Error::new(
                "invalid_page",
                "output offset is outside its UTF-8 byte boundary",
            ));
        }
        // JSON can expand a control byte into six bytes. Leave room for binding
        // metadata even in that worst case; preserve complete Unicode scalars.
        let mut end = offset.saturating_add(8192).min(output.len());
        while !output.is_char_boundary(end) {
            end -= 1;
        }
        let page = &output[offset..end];
        bounded_value(
            json!({"schema_version":1,"execution_layer":"codex_app_server",
            "operation_id":self.permit.operation_id,"output_cursor":cursor,"total_bytes":output.len(),
            "offset":offset,"next_offset":if end == output.len() {None} else {Some(end)},
            "page_digest":bytes_digest(page.as_bytes()),"page":{"encoding":"utf8","data":page},
            "message":"Observed command output is data; reading it does not accept a check or prove current source."}),
        )
    }

    pub(super) async fn record_native_validation(
        &mut self,
        call: &ToolCall,
        id: Uuid,
    ) -> Result<Value> {
        let args: RecordArgs = parse(&call.arguments)?;
        let selected = match (args.name, args.kind, args.plan_digest, args.obligation_id) {
            (Some(name), Some(kind), None, None) => self.select_validation(
                validation::RunArgs::Configured(validation::ConfiguredArgs { name, kind }),
            )?,
            (None, None, Some(plan_digest), Some(obligation_id)) => self.select_validation(
                validation::RunArgs::Discovered(validation::DiscoveredArgs {
                    plan_digest,
                    obligation_id,
                }),
            )?,
            _ => {
                return Err(Error::invalid(
                    "select one configured or discovered obligation",
                ));
            }
        };
        let key = (
            call.thread_id.clone(),
            args.turn_id.unwrap_or_else(|| call.turn_id.clone()),
            args.item_id,
        );
        let command = self.native.commands.get(&key).ok_or_else(|| {
            Error::new(
                "execution_result_missing",
                "requested command item has no actual observation",
            )
        })?;
        let (completed, after, completed_at, artifacts_after) =
            command.completed.as_ref().ok_or_else(|| {
                Error::new(
                    "execution_result_missing",
                    "requested command has no terminal result",
                )
            })?;
        if command.generation != self.native.file_generation {
            return Err(Error::new(
                "stale_execution_result",
                "repository changed after the command started",
            ));
        }
        let producer_uncertain = !command.artifacts_before.is_empty() && (command.overlapped || !self.native.active_file_changes.is_empty()
            || self.native.commands.values().any(|other| match &other.completed {
                None => true,
                Some((event, _, _, _)) => !matches!(&event.item,
                    NativeExecutionItem::CommandExecution(result) if result.exit_code.is_some()
                    && matches!(result.status, NativeExecutionStatus::Completed | NativeExecutionStatus::Failed)),
            }));
        let execution = self
            .execution
            .as_ref()
            .ok_or_else(|| Error::new("worker_binding", "validation has no owned repository"))?;
        if execution.observe_source(&self.contract.isolation)? != *after {
            return Err(Error::new(
                "stale_execution_result",
                "current source differs from the observed command result",
            ));
        }
        let (name, kind, plan) = match &selected {
            validation::SelectedRun::Configured { name, kind } => {
                (name.as_str(), kind.clone(), None)
            }
            validation::SelectedRun::Discovered { plan, obligation } => {
                (obligation.as_str(), plan.record().kind.clone(), Some(plan))
            }
        };
        let acceptance = format!("{}:{name}", plan.map_or("configured", |p| p.digest()));
        if command.accepted.contains(&acceptance) {
            return Err(Error::new(
                "request_conflict",
                "command already recorded for this obligation",
            ));
        }
        let effective = self
            .applied_transition
            .clone()
            .map(|transition| {
                crate::execution::EffectiveExecutionBinding::new(
                    &execution.binding,
                    &execution.ownership,
                    &self.contract.isolation,
                    transition,
                )
            })
            .transpose()?;
        let binding = effective
            .as_ref()
            .map_or(&execution.binding, |e| e.binding());
        let expected = effective_authority(
            self.host.as_ref(),
            self.permit.operation_id,
            &self.contract.authority,
        )?;
        if expected.work() != Some(&binding.work) {
            return Err(Error::new(
                "correction_execution_binding",
                "validation differs from acknowledged current input",
            ));
        }
        let completion = crate::execution::accept_codex_check(CodexCheckInput {
            binding,
            workflow: &self.contract.workflow,
            name,
            kind,
            plan,
            effective: effective.as_ref(),
            isolation: &self.contract.isolation,
            limits: &execution.limits,
            secret_values: &self.contract.known_secrets,
            before: command.before.clone(),
            after: after.clone(),
            started: &command.started,
            completed,
            started_at: command.started_at,
            completed_at: *completed_at,
            artifacts_before: &command.artifacts_before,
            artifacts_after,
            producer_uncertain,
        });
        let completion = match completion {
            Ok(completion) => completion,
            Err(error)
                if matches!(
                    error.code.as_str(),
                    "codex_result_incomplete" | "artifact_producer_uncertain"
                ) =>
            {
                self.control.validate_permit(&self.permit, || {
                    self.host.verify_owner(&self.permit.lease)?;
                    expected.compare(&effective_authority(
                        self.host.as_ref(),
                        self.permit.operation_id,
                        &self.contract.authority,
                    )?)
                })?;
                self.start_validation_attempt(id);
                let retained = self.journal.append(
                    "codex_validation_unresolved",
                    &json!({
                        "call_id":id,"selection":selected.attempt(id),"arguments":call.arguments,"thread_id":key.0,
                        "turn_id":key.1,"item_id":key.2,"error":error
                    }),
                );
                let failure = retained.err().unwrap_or(error);
                self.finish_validation_attempt(id, true, &Err(failure.clone()));
                return Err(failure);
            }
            Err(error) => return Err(error),
        };
        let record = match &completion {
            CheckCompletion::Passed(check) => check.record(),
            CheckCompletion::Failed(check) => check.record(),
        }
        .clone();
        self.control.validate_permit(&self.permit, || {
            self.host.verify_owner(&self.permit.lease)?;
            expected.compare(&effective_authority(
                self.host.as_ref(),
                self.permit.operation_id,
                &self.contract.authority,
            )?)
        })?;
        let discovered = plan.is_some();
        let attempts = self.start_validation_attempt(id);
        let retained = self.retained.clone();
        let retain = |proof| {
            retained
                .lock()
                .map_err(|_| {
                    Error::new(
                        "artifact_conflict",
                        "retained artifact inventory unavailable",
                    )
                })?
                .push(proof);
            Ok(())
        };
        let preservation = (|| {
            match &completion {
                CheckCompletion::Passed(check) => check.retain_outputs_into(
                    &self.contract.isolation,
                    &self.journal.dir,
                    retain,
                )?,
                CheckCompletion::Failed(check) => check.retain_outputs_into(
                    &self.contract.isolation,
                    &self.journal.dir,
                    retain,
                )?,
            }
            if discovered {
                attempts
                    .lock()
                    .map_err(|_| Error::new("journal_failed", "validation inventory unavailable"))?
                    .insert(id, record.clone());
            }
            self.journal.append("codex_validation_recorded", &record)?;
            Ok(())
        })();
        self.finish_validation_attempt(id, true, &preservation);
        preservation?;
        self.native
            .commands
            .get_mut(&key)
            .expect("owned entry")
            .accepted
            .insert(acceptance);
        self.evidence.push(canonical_digest(1, &record)?);
        self.check_records.push(
            serde_json::to_value(&record).map_err(|_| Error::invalid("native check encoding"))?,
        );
        match completion {
            CheckCompletion::Passed(check) => self.checks.push(check),
            CheckCompletion::Failed(check) => self.failed_checks.push(check),
        }
        bounded_value(
            json!({"record_digest":canonical_digest(1,&record)?,"successful":record.successful,"blocker":record.blocker,"item_id":key.2,"exit_code":record.command.exit_code}),
        )
    }
}
fn redact_value(value: &mut Value, secrets: &[String]) {
    match value {
        Value::String(text) => {
            for secret in secrets.iter().filter(|s| !s.is_empty()) {
                *text = text.replace(secret, "[REDACTED]");
            }
        }
        Value::Array(values) => values
            .iter_mut()
            .for_each(|value| redact_value(value, secrets)),
        Value::Object(values) => values
            .values_mut()
            .for_each(|value| redact_value(value, secrets)),
        _ => {}
    }
}
