//! Assignment-bound local/integrated validation discovery is enrolled from actual settled worker reads.
//! Public records remain descriptive; only this retained owner holds a plan.
use super::*;
use crate::validation::{
    DiscoveryJournalReference, DiscoveryReadContent, DiscoveryReadReference, ValidationEnrollment,
    ValidationPlan, ValidationProposal,
};

/// Original worker-produced membership seal. Public histories are descriptive
/// copies; they cannot omit a later failed attempt or erase an unresolved state.
/// No constructor or deserialization path accepts caller-supplied records.
#[derive(Debug)]
pub struct ValidationHistorySeal {
    operation: Uuid,
    plan: Option<String>,
    attempts: Option<String>,
    error: Option<String>,
    artifacts: Option<String>,
}
impl ValidationHistorySeal {
    pub(crate) fn digest(&self) -> Result<String> {
        if self.plan.is_none()
            || self.attempts.is_none()
            || self.error.is_none()
            || self.artifacts.is_none()
        {
            return Err(Error::new(
                "validation_history_binding",
                "original validation history cannot be sealed",
            ));
        }
        canonical_digest(
            1,
            &json!({"schema_version":2,"operation":self.operation,
            "plan":self.plan,"attempts":self.attempts,"error":self.error,"artifacts":self.artifacts}),
        )
    }

    pub(crate) fn verify(
        &self,
        operation: Uuid,
        plan: Option<&ValidationPlan>,
        attempts: &[crate::execution::CheckExecutionRecord],
        error: Option<&Error>,
    ) -> Result<()> {
        let expected_plan = plan_inventory_digest(plan)?;
        let expected_attempts = canonical_digest(1, attempts)?;
        let expected_error = canonical_digest(1, &error)?;
        if self.operation != operation
            || self.plan.as_deref() != Some(expected_plan.as_str())
            || self.attempts.as_deref() != Some(expected_attempts.as_str())
            || self.error.as_deref() != Some(expected_error.as_str())
        {
            return Err(Error::new(
                "validation_history_binding",
                "validation history differs from its original worker inventory or remains unsealable",
            ));
        }
        Ok(())
    }
    pub(crate) fn verify_artifacts(&self, artifacts: &[RetainedArtifact]) -> Result<()> {
        let expected = artifact_inventory_digest(artifacts)?;
        if self.artifacts.as_deref() != Some(expected.as_str()) {
            return Err(Error::new(
                "validation_history_binding",
                "retained artifact inventory differs from its original worker seal",
            ));
        }
        Ok(())
    }
}
fn artifact_inventory_digest(artifacts: &[RetainedArtifact]) -> Result<String> {
    canonical_digest(
        1,
        &artifacts
            .iter()
            .map(RetainedArtifact::inventory_digest)
            .collect::<Result<Vec<_>>>()?,
    )
}
fn plan_inventory_digest(plan: Option<&ValidationPlan>) -> Result<String> {
    canonical_digest(
        1,
        &plan.map(|plan| json!({"digest":plan.digest(),"history":plan.history()})),
    )
}

#[derive(Default)]
pub(super) struct DiscoveryState {
    reads: BTreeMap<String, DiscoveryReadReference>,
    order: Vec<String>,
    pending_reads: BTreeMap<Uuid, PendingRead>,
    pending_plans: BTreeMap<Uuid, ValidationPlan>,
    pending_enrollments: BTreeSet<Uuid>,
    current: Option<ValidationPlan>,
    pending_attempts: BTreeSet<Uuid>,
    attempts: Arc<Mutex<BTreeMap<Uuid, crate::execution::CheckExecutionRecord>>>,
    failure: Option<Error>,
}
struct PendingRead {
    turn: String,
    instruction: String,
    input: String,
    result: String,
    content: DiscoveryReadContent,
}

pub(super) fn proposal_tool() -> Value {
    let schema = json!({"type":"object","additionalProperties":false,
        "properties":{"previous":{"type":["string","null"]},"reason":{"type":"string"},
        "supporting_reads":{"type":"array","items":{"type":"string"},"minItems":1,"maxItems":64},
        "requirements":{"type":"array","minItems":1,"maxItems":64,"items":{
            "type":"object","additionalProperties":false,"properties":{
                "id":{"type":"string"},"statement":{"type":"string"},"rationale":{"type":"string"},
                "argv":{"type":"array","items":{"type":"string"}},"cwd":{"type":"string"},
                "services":{"type":"array"},"artifacts":{"type":"array","items":{"type":"string"}}},
            "required":["id","statement","rationale","argv","cwd","services","artifacts"]}}},
        "required":["previous","reason","supporting_reads","requirements"]});
    json!({"type":"function","name":"validation_propose","description":"Enroll or revise obligations for the actual host-assigned local or integrated validation scope from actual settled source/context reads. Use context_read validation_evidence for reference IDs. Every prior obligation must remain; changed commands require a reason. Does not grant permissions or pass a gate.","inputSchema":schema})
}
#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum RunArgs {
    Configured(ConfiguredArgs),
    Discovered(DiscoveredArgs),
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ConfiguredArgs {
    pub name: String,
    pub kind: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DiscoveredArgs {
    pub plan_digest: String,
    pub obligation_id: String,
}
pub(super) enum SelectedRun {
    Configured {
        name: String,
        kind: CheckKind,
    },
    Discovered {
        plan: ValidationPlan,
        obligation: String,
    },
}
impl SelectedRun {
    pub(super) fn attempt(&self, invocation: Uuid) -> Option<Value> {
        match self {
            Self::Discovered { plan, obligation } => Some(
                json!({"plan_digest":plan.digest(),"obligation_id":obligation,"invocation_id":invocation}),
            ),
            Self::Configured { .. } => None,
        }
    }
}
impl ScopedWorker {
    fn discovery_kind(&self) -> Result<CheckKind> {
        let execution = self.execution.as_ref().ok_or_else(|| {
            Error::new(
                "validation_scope",
                "discovery requires an actual execution owner",
            )
        })?;
        let work = self.contract.authority.work().ok_or_else(|| {
            Error::new(
                "validation_scope",
                "independent review cannot enroll execution obligations",
            )
        })?;
        if !self
            .contract
            .assembled
            .manifest
            .role
            .permits_repository_execution()
            || !self
                .contract
                .assembled
                .manifest
                .permissions
                .repository_code_execution
            || work != &execution.binding.work
            || work.child.is_none()
        {
            return Err(Error::new(
                "validation_scope",
                "discovery requires actual authorized child execution",
            ));
        }
        classify_discovery_scope(
            self.contract.assembled.manifest.role,
            &self.contract.assembled.manifest.phase,
            self.contract.isolation.plan().role,
            execution.binding.candidate_vector_digest.as_deref(),
            &self.contract.context,
        )
    }
    fn supports_discovery(&self) -> bool {
        self.discovery_kind().is_ok()
    }
    fn require_discovery(&self) -> Result<CheckKind> {
        let kind = self.discovery_kind()?;
        if let Some(error) = self.validation_history_error() {
            return Err(error);
        }
        Ok(kind)
    }
    pub(super) fn select_validation(&self, args: RunArgs) -> Result<SelectedRun> {
        match args {
            RunArgs::Configured(args) => Ok(SelectedRun::Configured {
                name: args.name,
                kind: match args.kind.as_str() {
                    "local" => CheckKind::Local,
                    "integrated" => CheckKind::Integrated,
                    _ => {
                        return Err(Error::new(
                            "invalid_input",
                            "only local/integrated named checks are executable",
                        ));
                    }
                },
            }),
            RunArgs::Discovered(args) => {
                let kind = self.require_discovery()?;
                let plan = self.validation.current.as_ref().ok_or_else(|| {
                    Error::new(
                        "validation_plan_missing",
                        "no current host-enrolled validation plan",
                    )
                })?;
                if plan.digest() != args.plan_digest || plan.record().kind != kind {
                    return Err(Error::new(
                        "validation_plan_changed",
                        "execution must use the current host-enrolled plan",
                    ));
                }
                plan.requirement(&args.obligation_id)?;
                Ok(SelectedRun::Discovered {
                    plan: plan.clone(),
                    obligation: args.obligation_id,
                })
            }
        }
    }
    fn retain_read(
        &mut self,
        call: &ToolCall,
        id: Uuid,
        value: &Value,
        content: DiscoveryReadContent,
    ) -> Result<()> {
        if !self.supports_discovery() {
            return Ok(());
        }
        if self.validation.pending_reads.len() + self.validation.reads.len() >= MAX_CALLS {
            return Err(Error::new(
                "tool_limit",
                "validation read inventory exhausted",
            ));
        }
        self.validation.pending_reads.insert(
            id,
            PendingRead {
                turn: call.turn_id.clone(),
                instruction: self.contract.assembled.manifest_digest.clone(),
                input: self.correction_requirement().input_digest,
                result: canonical_digest(1, value)?,
                content,
            },
        );
        Ok(())
    }
    pub(super) fn retain_context_read(
        &mut self,
        call: &ToolCall,
        id: Uuid,
        key: &str,
        value: &Value,
    ) -> Result<()> {
        if !self.supports_discovery() {
            return Ok(());
        }
        self.retain_read(
            call,
            id,
            value,
            DiscoveryReadContent::Context {
                key: key.into(),
                digest: canonical_digest(1, value)?,
                bytes: serde_json::to_vec(value)
                    .map_err(|_| Error::invalid("context read encoding"))?
                    .len() as u64,
            },
        )
    }
    pub(super) fn settle_validation_call(
        &mut self,
        id: Uuid,
        finished: &BrokerJournalReference,
        result: &Result<Value>,
    ) -> Result<()> {
        if let Some(pending) = self.validation.pending_reads.remove(&id)
            && let Ok(value) = result
        {
            if pending.result != canonical_digest(1, value)? || finished.kind != "tool_finished" {
                return Err(Error::new(
                    "validation_read",
                    "completed read differs from original callback",
                ));
            }
            let reference = DiscoveryReadReference {
                schema_version: 1,
                operation_id: self.permit.operation_id,
                call_id: id,
                turn: pending.turn,
                instruction_digest: pending.instruction,
                correction_input_digest: pending.input,
                result_digest: pending.result,
                journal: DiscoveryJournalReference {
                    leaf: finished.leaf.clone(),
                    digest: finished.digest.clone(),
                    bytes: finished.bytes,
                },
                content: pending.content,
            };
            let identity = reference.digest()?;
            self.journal.append(
                "validation_read",
                &json!({"id":identity,"reference":reference}),
            )?;
            self.validation.order.push(identity.clone());
            self.validation.reads.insert(identity, reference);
        }
        if result.is_ok()
            && let Some(plan) = self.validation.pending_plans.get(&id).cloned()
        {
            self.journal.append(
                "validation_plan_settled",
                &json!({"plan_digest":plan.digest(),"call_id":id,"finished":finished}),
            )?;
            self.validation.pending_plans.remove(&id);
            self.validation.current = Some(plan);
        }
        Ok(())
    }
    pub(super) fn validation_evidence(&self) -> Result<Value> {
        let mut references = Vec::new();
        // A bounded newest-first index. Earlier IDs remain usable and are never
        // removed from the protected original inventory by this display limit.
        for id in self.validation.order.iter().rev().take(16) {
            let reference = &self.validation.reads[id];
            let mut candidate = references.clone();
            candidate.push(json!({"id":id,"reference":reference}));
            if serde_json::to_vec(&candidate)
                .map_err(|_| Error::invalid("validation read index encoding"))?
                .len()
                > MAX_TOOL_BYTES / 2
            {
                break;
            }
            references = candidate;
        }
        let shown = references.len();
        let continuation = self.contract.validation_prefix.as_ref().map(|prefix| {
            let mut prior_results = Vec::new();
            let total = prefix.history().iter().map(|segment| segment.attempts.len()).sum::<usize>();
            for record in prefix.history().iter().rev().flat_map(|segment| segment.attempts.iter().rev()).take(16) {
                let summary = json!({"record_digest":canonical_digest(1, record)?,"operation":record.binding.operation_id,
                    "source":record.binding.revision,"obligation":record.check_name,"successful":record.successful,
                    "exit_code":record.command.exit_code,"timed_out":record.command.timed_out,"blocker":record.blocker,
                    "item_id":record.codex.as_ref().map(|provenance|&provenance.item_id)});
                let mut next = prior_results.clone(); next.push(summary);
                if serde_json::to_vec(&next).map_err(|_|Error::invalid("encode historical validation summary"))?.len() > MAX_TOOL_BYTES / 4 { break; }
                prior_results = next;
            }
            Ok::<_, Error>(json!({"history_digest":prefix.digest(),
                "required_obligations":prefix.history().last().ok_or_else(||Error::invalid("empty validation prefix"))?.current()?.requirements.iter().map(|requirement|&requirement.id).collect::<Vec<_>>(),
                "total_prior_results":total,"omitted_prior_results":total.saturating_sub(prior_results.len()),"prior_results_newest_first":prior_results,
                "rule":"These are historical results, not checks for this admission. Discover a fresh source-bound plan, preserve all obligation IDs and record current actual results. The complete original history and artifacts remain retained for acceptance and independent review."}))
        }).transpose()?;
        bounded_value(
            json!({"schema_version":1,"reads":references,"total_reads":self.validation.reads.len(),
            "omitted_reads":self.validation.reads.len().saturating_sub(shown),
            "continuation":continuation,
            "current_plan":self.validation.current.as_ref().map(|plan| json!({"digest":plan.digest(),"ordinal":plan.record().ordinal,
                "source_digest":plan.record().source_digest,"requirements":plan.record().requirements.iter().map(|r|&r.id).collect::<Vec<_>>()})),
            "history":self.validation.current.as_ref().map(|plan| plan.history().iter().map(|record| canonical_digest(1,record)).collect::<Result<Vec<_>>>()).transpose()?}),
        )
    }
    pub(crate) fn seal_validation_history(&self) -> ValidationHistorySeal {
        let error = self.validation_history_error();
        ValidationHistorySeal {
            operation: self.permit.operation_id,
            plan: plan_inventory_digest(self.validation.current.as_ref()).ok(),
            attempts: self.validation.attempts.lock().ok().and_then(|records| {
                canonical_digest(1, &records.values().collect::<Vec<_>>()).ok()
            }),
            error: canonical_digest(1, &error).ok(),
            artifacts: self
                .retained
                .lock()
                .ok()
                .and_then(|artifacts| artifact_inventory_digest(&artifacts).ok()),
        }
    }
    pub(super) fn current_validation_plan(&self) -> Option<&ValidationPlan> {
        self.validation.current.as_ref()
    }
    pub(crate) fn take_validation_plan(&mut self) -> Option<ValidationPlan> {
        self.validation.current.take()
    }
    pub(crate) fn take_validation_artifacts(&mut self) -> Vec<RetainedArtifact> {
        self.retained
            .lock()
            .map(|mut artifacts| std::mem::take(&mut *artifacts))
            .unwrap_or_default()
    }
    pub(crate) fn validation_history_error(&self) -> Option<Error> {
        self.validation.failure.clone().or_else(|| {
            (!self.validation.pending_attempts.is_empty()
                || !self.validation.pending_plans.is_empty()
                || !self.validation.pending_enrollments.is_empty()
                || self.validation.attempts.is_poisoned()
                || self.retained.is_poisoned())
            .then(|| {
                Error::new(
                    "validation_history_uncertain",
                    "original validation plan or attempt has unresolved persistence or execution",
                )
            })
        })
    }
    pub(crate) fn take_validation_attempts(
        &mut self,
    ) -> Vec<crate::execution::CheckExecutionRecord> {
        self.validation
            .attempts
            .lock()
            .map(|mut records| std::mem::take(&mut *records).into_values().collect())
            .unwrap_or_default()
    }
    pub(super) fn start_validation_attempt(
        &mut self,
        id: Uuid,
    ) -> Arc<Mutex<BTreeMap<Uuid, crate::execution::CheckExecutionRecord>>> {
        self.validation.pending_attempts.insert(id);
        self.validation.attempts.clone()
    }
    pub(super) fn finish_validation_attempt(
        &mut self,
        id: Uuid,
        dispatched: bool,
        result: &Result<()>,
    ) {
        if result.is_ok() || !dispatched {
            self.validation.pending_attempts.remove(&id);
        } else if let Err(error) = result {
            // Includes known service-prefix failure: cleanup is not a successful
            // validation attempt and cannot make an earlier pass current again.
            self.validation.failure = Some(error.clone());
        }
    }
    pub(super) async fn propose_validation(&mut self, call: &ToolCall, id: Uuid) -> Result<Value> {
        let kind = self.require_discovery()?;
        let proposal: ValidationProposal = parse(&call.arguments)?;
        if let Some(prefix) = &self.contract.validation_prefix {
            prefix.require_obligations(&proposal)?;
        }
        if proposal.supporting_reads.is_empty() || proposal.supporting_reads.len() > 64 {
            return Err(Error::new(
                "validation_read",
                "proposal requires 1 to 64 actual settled reads",
            ));
        }
        validate_assignment_requirements(
            &self.contract.workflow,
            &self.contract.isolation,
            &self.contract.context,
            &proposal,
        )?;
        let references = proposal
            .supporting_reads
            .iter()
            .map(|id| {
                self.validation.reads.get(id).cloned().ok_or_else(|| {
                    Error::new(
                        "validation_read",
                        "supporting read is not an actual settled owner observation",
                    )
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let execution = self
            .execution
            .clone()
            .ok_or_else(|| Error::new("unsupported_tool", "owned execution unavailable"))?;
        let contract = self.contract.clone();
        let expected = effective_authority(
            self.host.as_ref(),
            self.permit.operation_id,
            &contract.authority,
        )?;
        let effective = self
            .applied_transition
            .clone()
            .map(|transition| {
                crate::execution::EffectiveExecutionBinding::new(
                    &execution.binding,
                    &execution.ownership,
                    &contract.isolation,
                    transition,
                )
            })
            .transpose()?;
        let context_digest = self.correction_requirement().input_digest;
        let journal = self.journal.clone();
        let control = self.control.clone();
        let host = self.host.clone();
        let permit = self.permit.clone();
        let previous = self.validation.current.clone();
        self.validation.pending_enrollments.insert(id);
        let write_attempted = Arc::new(AtomicBool::new(false));
        let writing = write_attempted.clone();
        let enrolled = tokio::task::spawn_blocking(move || {
            let binding = effective
                .as_ref()
                .map_or(&execution.binding, |effective| effective.binding());
            if expected.work() != Some(&binding.work) {
                return Err(Error::new(
                    "correction_execution_binding",
                    "validation discovery requires the acknowledged current input",
                ));
            }
            for reference in &references {
                verify_read(
                    &journal,
                    reference,
                    permit.operation_id,
                    &contract.assembled.manifest_digest,
                    &context_digest,
                )?;
            }
            let source = execution.observe_source(&contract.isolation)?;
            let enrollment = ValidationEnrollment {
                kind,
                original_binding: execution.binding.clone(),
                binding: binding.clone(),
                correction: effective.as_ref().map(|e| e.provenance()).transpose()?,
                workflow_digest: contract.workflow.digest.clone(),
                instruction_digest: contract.assembled.manifest_digest.clone(),
                tool_contract_digest: contract.tools.digest().into(),
                context_digest,
                source_digest: source.digest()?,
            };
            control.validate_permit_observed(&permit, |_| {
                host.verify_owner(&permit.lease)?;
                expected.compare(&effective_authority(
                    host.as_ref(),
                    permit.operation_id,
                    &contract.authority,
                )?)?;
                let plan =
                    ValidationPlan::enroll(enrollment, proposal, references, previous.as_ref())?;
                writing.store(true, Ordering::SeqCst);
                journal.append(
                    "validation_plan",
                    &json!({"call_id":id,"plan_digest":plan.digest(),"record":plan.record()}),
                )?;
                Ok(plan)
            })
        })
        .await
        .map_err(|_| {
            Error::new(
                "operation_uncertain",
                "validation discovery dispatcher interrupted",
            )
        })
        .and_then(|result| result);
        if enrolled.is_ok() || !write_attempted.load(Ordering::SeqCst) {
            self.validation.pending_enrollments.remove(&id);
        }
        let plan = enrolled?;
        let result = json!({"plan_digest":plan.digest(),"ordinal":plan.record().ordinal,"requirements":plan.record().requirements.iter().map(|r|&r.id).collect::<Vec<_>>()});
        self.validation.pending_plans.insert(id, plan);
        bounded_value(result)
    }
}
fn verify_read(
    journal: &Journal,
    reference: &DiscoveryReadReference,
    operation: Uuid,
    instruction: &str,
    input: &str,
) -> Result<()> {
    reference.digest()?;
    // Old read records stay decodable for historical evidence. They cannot
    // become observations from Codex's native repository tools.
    if !matches!(reference.content, DiscoveryReadContent::Context { .. }) {
        return Err(Error::new(
            "validation_read_legacy",
            "legacy Kegari source/resource reads cannot support a Codex-native validation plan",
        ));
    }
    if reference.operation_id != operation
        || reference.instruction_digest != instruction
        || reference.correction_input_digest != input
    {
        return Err(Error::new(
            "validation_read",
            "read belongs to another operation or correction input",
        ));
    }
    let event: Value = journal
        .dir
        .read_bounded(&reference.journal.leaf, 256 * 1024)?;
    let bytes = serde_json::to_vec(&event).map_err(|_| Error::invalid("read journal encoding"))?;
    if bytes.len() as u64 != reference.journal.bytes
        || canonical_digest(1, &event)? != reference.journal.digest
        || event["kind"] != "tool_finished"
        || event["record"]["request_id"] != json!(reference.call_id)
    {
        return Err(Error::new(
            "validation_read",
            "original completed read journal changed",
        ));
    }
    let result: Result<Value> = serde_json::from_value(event["record"]["result"].clone())
        .map_err(|_| Error::new("validation_read", "completed read result is malformed"))?;
    let value = result
        .map_err(|_| Error::new("validation_read", "failed read cannot support validation"))?;
    if canonical_digest(1, &value)? != reference.result_digest {
        return Err(Error::new(
            "validation_read",
            "completed read result differs from original bytes",
        ));
    }
    Ok(())
}

pub(super) fn validate_assignment_requirements(
    workflow: &Workflow,
    isolation: &ValidatedIsolation,
    context: &Value,
    proposal: &ValidationProposal,
) -> Result<()> {
    if let Some(selected) =
        context.pointer("/evidence/delivery_assignment/validation_assignment/configured_checks")
    {
        let selected: Vec<String> = serde_json::from_value(selected.clone()).map_err(|_| {
            Error::new(
                "validation_plan",
                "host validation assignment has malformed selected checks",
            )
        })?;
        let integrated = context
            .pointer("/evidence/delivery_assignment/validation_assignment/kind")
            .and_then(Value::as_str)
            == Some("integrated");
        validate_selected_requirements(workflow, isolation, &selected, proposal, integrated)?;
    }
    Ok(())
}
#[cfg(test)]
pub(super) fn validate_configured_requirements(
    workflow: &Workflow,
    isolation: &ValidatedIsolation,
    selected: &[String],
    proposal: &ValidationProposal,
) -> Result<()> {
    validate_selected_requirements(workflow, isolation, selected, proposal, false)
}
fn validate_selected_requirements(
    workflow: &Workflow,
    isolation: &ValidatedIsolation,
    selected: &[String],
    proposal: &ValidationProposal,
    integrated: bool,
) -> Result<()> {
    for name in selected {
        let checks = if integrated {
            &workflow.front_matter.checks.integrated
        } else {
            &workflow.front_matter.checks.local
        };
        let command = checks.get(name).ok_or_else(|| {
            Error::new(
                "validation_plan",
                "selected local obligation is absent from the actual workflow",
            )
        })?;
        let requirement = proposal
            .requirements
            .iter()
            .find(|requirement| &requirement.id == name)
            .ok_or_else(|| {
                Error::new(
                    "validation_plan",
                    "proposal omits a known selected local obligation",
                )
            })?;
        let derived = isolation.with_cwd(Path::new(&requirement.cwd))?;
        if requirement.argv != command.argv
            || requirement.services != command.services
            || derived.plan().cwd != isolation.plan().cwd
        {
            return Err(Error::new(
                "validation_plan",
                "known selected local obligation must retain its exact command, services and cwd",
            ));
        }
    }
    Ok(())
}
pub(super) fn refuse_cold_history(event: &Value) -> Result<()> {
    if event["kind"].as_str().is_some_and(|kind| {
        matches!(
            kind,
            "validation_plan"
                | "validation_plan_settled"
                | "validation_attempt"
                | "validation_attempt_ack"
                | "validation_check_chunk"
                | "validation_check_entry"
        )
    }) {
        return Err(Error::new(
            "validation_recovery_pending",
            "retained validation obligations require their original host plan owner; cold recovery cannot reconstruct it",
        ));
    }
    Ok(())
}

// Pure classification of already retained host facts, never an authority
// constructor. The production caller first checks its actual permit work owner.
pub(super) fn classify_discovery_scope(
    role: AgentRole,
    phase: &str,
    isolation: ExecutionRole,
    vector: Option<&str>,
    context: &Value,
) -> Result<CheckKind> {
    let assignment = context.pointer("/evidence/delivery_assignment/validation_assignment");
    match vector {
        None if phase != "parent_integrated_validation"
            && assignment
                .and_then(|a| a.get("kind"))
                .and_then(Value::as_str)
                != Some("integrated") =>
        {
            Ok(CheckKind::Local)
        }
        Some(vector)
            if role == AgentRole::Validation
                && isolation == ExecutionRole::Validation
                && phase == "parent_integrated_validation"
                && assignment
                    .and_then(|a| a.get("kind"))
                    .and_then(Value::as_str)
                    == Some("integrated")
                && assignment
                    .and_then(|a| a.get("vector"))
                    .and_then(Value::as_str)
                    == Some(vector) =>
        {
            crate::evidence::validate_digest(vector)?;
            Ok(CheckKind::Integrated)
        }
        _ => Err(Error::new(
            "validation_scope",
            "integrated discovery requires the exact host E2E assignment, validation role and original candidate vector",
        )),
    }
}
