//! Exact submitted-input provenance and narrowly bounded thread recovery.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionResumeOrigin {
    pub(crate) predecessor: uuid::Uuid,
    pub(crate) recovery_digest: String,
    pub(crate) instruction_compatibility_digest: String,
    pub(crate) history_digest: String,
    pub(crate) historical_turns: BTreeSet<String>,
    pub(crate) thread: String,
    pub(crate) readback_digest: Option<String>,
}
impl SessionResumeOrigin {
    pub(super) fn validate(&self, state: &SessionState) -> Result<()> {
        if self.predecessor.is_nil()
            || self.predecessor.to_string() == state.binding.operation_id
            || state.thread_id.as_deref() != Some(&self.thread)
            || self.historical_turns.is_empty()
            || self.historical_turns.len() > 4096
            || self.historical_turns.iter().any(|id| id.is_empty())
            || state
                .completed_turns
                .keys()
                .any(|id| self.historical_turns.contains(id))
            || state
                .turn_id
                .iter()
                .chain(state.unresolved_turn.iter())
                .any(|id| self.historical_turns.contains(id))
        {
            return Err(Error::new(
                "corrupt_receipt",
                "resumed session changed its original operation/thread lineage",
            ));
        }
        for digest in [
            &self.recovery_digest,
            &self.instruction_compatibility_digest,
            &self.history_digest,
        ]
        .into_iter()
        .chain(self.readback_digest.as_ref())
        {
            crate::evidence::validate_digest(digest)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmissionHistory {
    records: Vec<Submission>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Submission {
    rpc_id: u64,
    method: String,
    thread: String,
    expected_turn: Option<String>,
    input_digest: Option<String>,
    frame_digest: String,
    outcome: SubmissionOutcome,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
enum SubmissionOutcome {
    Prepared,
    Acknowledged { turn: String },
    Rejected,
}
impl SubmissionHistory {
    pub(super) fn prepare(&mut self, rpc_id: u64, method: &str, params: &Value) -> Result<()> {
        if self.records.len() >= 4096 || self.records.iter().any(|record| record.rpc_id == rpc_id) {
            return Err(Error::new(
                "history_limit",
                "submission inventory is full or request ID is reused",
            ));
        }
        self.records.push(Submission {
            rpc_id,
            method: method.into(),
            thread: required_str(params, "threadId")?.into(),
            expected_turn: params
                .get("expectedTurnId")
                .and_then(Value::as_str)
                .map(str::to_owned),
            input_digest: params.get("input").map(input_digest).transpose()?,
            frame_digest: canonical_digest(
                1,
                &json!({"id":rpc_id,"method":method,"params":params}),
            )?,
            outcome: SubmissionOutcome::Prepared,
        });
        Ok(())
    }
    pub(super) fn observe_response(&mut self, rpc_id: u64, response: &Result<Value>) -> Result<()> {
        let record = self
            .records
            .iter_mut()
            .find(|record| record.rpc_id == rpc_id)
            .ok_or_else(|| Error::new("protocol_error", "response has no submitted input"))?;
        if record.outcome != SubmissionOutcome::Prepared {
            return Err(Error::new(
                "protocol_error",
                "submission already has a recorded response",
            ));
        }
        match response {
            Ok(value) => {
                let turn = if record.method == "turn/steer" {
                    required_str(value, "turnId")?
                } else {
                    required_str(
                        value.get("turn").ok_or_else(|| {
                            Error::new("protocol_error", "submission response lacks turn")
                        })?,
                        "id",
                    )?
                };
                if record
                    .expected_turn
                    .as_ref()
                    .is_some_and(|expected| expected != turn)
                {
                    return Err(Error::new(
                        "protocol_error",
                        "submission acknowledgement changed expected turn",
                    ));
                }
                record.outcome = SubmissionOutcome::Acknowledged { turn: turn.into() };
            }
            Err(error) if error.code == "provider_response" => {
                record.outcome = SubmissionOutcome::Rejected
            }
            Err(_) => (), // Lost/uncertain responses never become known no-effect.
        }
        Ok(())
    }
    pub(super) fn validate(&self, state: &SessionState) -> Result<()> {
        let mut ids = BTreeSet::new();
        if self.records.len() > 4096 {
            return Err(Error::new(
                "corrupt_receipt",
                "submission inventory exceeds bound",
            ));
        }
        for record in &self.records {
            if !ids.insert(record.rpc_id)
                || !["turn/start", "turn/steer", "review/start"].contains(&record.method.as_str())
                || state.thread_id.as_deref() != Some(&record.thread)
                || record
                    .expected_turn
                    .as_ref()
                    .is_some_and(|turn| turn.is_empty())
                || (record.method == "turn/steer") != record.expected_turn.is_some()
                || (record.method != "review/start") != record.input_digest.is_some()
                || matches!(&record.outcome, SubmissionOutcome::Acknowledged { turn } if turn.is_empty())
            {
                return Err(Error::new(
                    "corrupt_receipt",
                    "invalid submitted-input binding",
                ));
            }
            crate::evidence::validate_digest(&record.frame_digest)?;
            if let SubmissionOutcome::Acknowledged { turn } = &record.outcome
                && state.require_new_turn_identity(turn).is_err()
            {
                return Err(Error::new(
                    "corrupt_receipt",
                    "new-session input acknowledges a historical turn",
                ));
            }
            if let Some(digest) = &record.input_digest {
                crate::evidence::validate_digest(digest)?;
            }
        }
        Ok(())
    }
    pub(crate) fn preserves(&self, previous: &Self) -> Result<()> {
        if self.records.len() < previous.records.len() {
            return Err(Error::new(
                "record_corrupt",
                "submitted input history was truncated",
            ));
        }
        for (old, new) in previous.records.iter().zip(&self.records) {
            let mut normalized = new.clone();
            normalized.outcome = old.outcome.clone();
            if normalized != *old
                || old.outcome != SubmissionOutcome::Prepared && old.outcome != new.outcome
            {
                return Err(Error::new(
                    "record_corrupt",
                    "submitted input or acknowledged result changed",
                ));
            }
        }
        Ok(())
    }
    fn verify_turns(&self, turns: &[Value]) -> Result<()> {
        if self.records.len() != turns.len() {
            return Err(Error::new(
                "resume_incompatible",
                "recovery requires one acknowledged input for every retained turn",
            ));
        }
        for (submission, turn) in self.records.iter().zip(turns) {
            let SubmissionOutcome::Acknowledged { turn: acknowledged } = &submission.outcome else {
                return Err(Error::new(
                    "reconciliation_required",
                    "submitted input has no exact acknowledged turn",
                ));
            };
            if submission.method != "turn/start" || required_str(turn, "id")? != acknowledged {
                return Err(Error::new(
                    "resume_incompatible",
                    "review, steering or changed turn lineage cannot use this recovery profile",
                ));
            }
            let items = turn.get("items").and_then(Value::as_array).ok_or_else(|| {
                Error::new("resume_incompatible", "recovery requires full turn items")
            })?;
            let mut users = Vec::new();
            for item in items {
                match required_str(item, "type")? {
                    "userMessage" => users.push(item),
                    "agentMessage" | "reasoning" => (),
                    _ => {
                        return Err(Error::new(
                            "resume_incompatible",
                            "effectful or unsupported historical items require separate reconciliation",
                        ));
                    }
                }
            }
            if users.len() != 1
                || users[0]["content"]
                    .as_array()
                    .is_none_or(|inputs| inputs.iter().any(|input| input["type"] != "text"))
                || Some(input_digest(&users[0]["content"])?).as_ref()
                    != submission.input_digest.as_ref()
            {
                return Err(Error::new(
                    "resume_incompatible",
                    "provider input differs from the actual submitted input",
                ));
            }
        }
        Ok(())
    }
}

/// Protected journal provenance only. Native recovery also requires the actual
/// retained joined worker and original owned directory; this grants no permit.
#[derive(Clone, Debug)]
pub struct RetainedResumeSession {
    state: SessionState,
    admission_digest: String,
    reference: crate::control::RunReference,
    journal_digest: String,
    digest: String,
}
impl RetainedResumeSession {
    pub(crate) fn from_journal(
        state: SessionState,
        admission_digest: String,
        reference: crate::control::RunReference,
        journal_digest: String,
    ) -> Result<Self> {
        crate::evidence::validate_digest(&admission_digest)?;
        crate::evidence::validate_digest(&journal_digest)?;
        if reference.account != state.binding.account_id
            || reference.phase.work != state.binding.work_identity
            || reference.instruction_digest != state.binding.instruction_digest
            || state.max_turns > reference.max_turns
            || state.terminal.as_deref() != Some("failed")
            || state.turn_id.is_some()
            || state.unresolved_turn.is_some()
            || state.intervention.is_some()
            || state.input_required
            || state.correction_required != 0
            || state.correction_consumed != 0
            || state.provider_history.is_none()
            || state.submissions.is_none()
            || state.resume_origin.is_some()
            || state.completed_turns.is_empty()
            || state.completed_turns.len() != state.turns_started as usize
            || state
                .last_completed_turn
                .as_ref()
                .and_then(|id| state.completed_turns.get(id))
                .map(String::as_str)
                != Some("failed")
        {
            return Err(Error::new(
                "resume_incompatible",
                "only a fully observed uncorrected terminal failure can be recovered",
            ));
        }
        let digest = canonical_digest(
            1,
            &json!({"state":state,"admission":admission_digest,"reference":reference,"journal":journal_digest}),
        )?;
        Ok(Self {
            state,
            admission_digest,
            reference,
            journal_digest,
            digest,
        })
    }
    pub fn state(&self) -> &SessionState {
        &self.state
    }
    pub fn admission_digest(&self) -> &str {
        &self.admission_digest
    }
    pub fn reference(&self) -> &crate::control::RunReference {
        &self.reference
    }
    pub fn journal_digest(&self) -> &str {
        &self.journal_digest
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransientTurnFailure {
    ServerOverloaded,
    InternalServerError,
}

/// Created only by an authenticated adapter's exact retained-thread readback.
/// It records observations; admission, ownership and effective instructions are
/// separate opaque host capabilities and must still be checked by the consumer.
#[derive(Clone, Debug)]
pub struct VerifiedThreadRecovery {
    retained: RetainedResumeSession,
    response_digest: String,
    history_digest: String,
    submission_digest: String,
    transient_failure: TransientTurnFailure,
    digest: String,
}
impl VerifiedThreadRecovery {
    pub fn retained(&self) -> &RetainedResumeSession {
        &self.retained
    }
    pub fn response_digest(&self) -> &str {
        &self.response_digest
    }
    pub fn history_digest(&self) -> &str {
        &self.history_digest
    }
    pub fn submission_digest(&self) -> &str {
        &self.submission_digest
    }
    pub fn transient_failure(&self) -> &TransientTurnFailure {
        &self.transient_failure
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
}

// Kept private to the protocol adapter. Production constructs this only from
// the admitted semantic bridge; component tests exercise the wire sequence
// without constructing a worker admission or a joined-completion capability.
struct ResumeProtocol<'a> {
    recovery: &'a VerifiedThreadRecovery,
    prompt: &'a str,
    predecessor: uuid::Uuid,
    bridge_digest: &'a str,
}

impl RpcClient {
    /// Consume an actual semantic/metadata bridge without changing the original
    /// SessionBinding compatibility rule or relabelling historical turns.
    pub(crate) async fn resume_verified(
        &mut self,
        state: &mut SessionState,
        proof: &crate::worker_run::VerifiedSessionResume,
        isolation: &ValidatedIsolation,
        permissions: &PermissionConfiguration,
        broker: &mut dyn ToolBroker,
    ) -> Result<()> {
        self.validate_session(state, isolation, permissions)?;
        proof.verify_session(state)?;
        self.resume_protocol(
            state,
            ResumeProtocol {
                recovery: proof.recovery().observation(),
                prompt: proof.prompt(),
                predecessor: proof.predecessor(),
                bridge_digest: proof.digest(),
            },
            permissions,
            broker,
        )
        .await
    }
    async fn resume_protocol(
        &mut self,
        state: &mut SessionState,
        proof: ResumeProtocol<'_>,
        permissions: &PermissionConfiguration,
        broker: &mut dyn ToolBroker,
    ) -> Result<()> {
        let retained = proof.recovery.retained();
        let mut observation = ResumeObserver {
            current: state,
            sink: broker,
            original: &retained.state().binding,
        };
        let current = self
            .recover_terminal_failure(retained, &mut observation)
            .await?;
        if current.history_digest() != proof.recovery.history_digest()
            || current.submission_digest() != proof.recovery.submission_digest()
            || current.transient_failure() != proof.recovery.transient_failure()
        {
            return Err(Error::new(
                "resume_incompatible",
                "provider history changed after admitted metadata recovery",
            ));
        }
        let state = observation.current;
        state
            .resume_origin
            .as_mut()
            .ok_or_else(|| Error::new("resume_incompatible", "resume lineage disappeared"))?
            .readback_digest = Some(current.digest().into());
        observation.sink.observe_session(state)?;
        let thread = state
            .thread_id
            .clone()
            .ok_or_else(|| Error::new("resume_incompatible", "owned thread missing"))?;
        let mut config = permissions.settings.clone();
        config["model_reasoning_effort"] = json!(state.binding.profile.reasoning);
        config["review_model"] = json!(state.binding.profile.model);
        let response=self.request("thread/resume",json!({"threadId":thread,"cwd":state.binding.cwd,"model":state.binding.profile.model,"modelProvider":"openai","serviceTier":state.binding.profile.service_tier()?,"permissions":permissions.profile_id,"approvalPolicy":"never","approvalsReviewer":"user","config":config,"developerInstructions":proof.prompt,"runtimeWorkspaceRoots":[state.binding.cwd],"excludeTurns":true}),Some(state),observation.sink).await?;
        validate_thread_response(&response, state, permissions)?;
        if response.pointer("/thread/id").and_then(Value::as_str) != Some(&thread)
            || state.turn_id.is_some()
            || state.turns_started != 0
            || !state.completed_turns.is_empty()
        {
            return Err(Error::new(
                "reconciliation_required",
                "provider resume changed thread or unexpectedly launched work",
            ));
        }
        state.provider_instruction_sources = instruction_sources(&response)?;
        let findings = format!(
            "Verified continuation of the same task after a terminal transient provider failure. Prior operation: {}. Exact metadata and instruction bridge: {}. Current full history readback: {}. Historical completion and validation claims remain on their original operations.",
            proof.predecessor,
            proof.bridge_digest,
            current.digest()
        );
        state.reconciliation_digest = Some(canonical_digest(1, &findings)?);
        self.pending_reconciliation = Some(findings);
        self.allowed_tools = state.advertised_tools.clone();
        Ok(())
    }
    /// The native wrapper uses this closed metadata sequence so even a policy
    /// notification arriving during initialization is durably observed against
    /// the original session. The caller's gate precedes each outbound read.
    pub(crate) async fn recover_metadata(
        &mut self,
        retained: &RetainedResumeSession,
        permissions: &PermissionConfiguration,
        sink: &mut dyn ToolBroker,
        fresh: &dyn Fn() -> Result<()>,
    ) -> Result<VerifiedThreadRecovery> {
        if self.initialized || self.server_request_count != 0 {
            return Err(Error::new(
                "resume_incompatible",
                "metadata recovery requires a new owned transport",
            ));
        }
        let mut state = retained.state.clone();
        let mut broker = RecoveryBroker {
            sink,
            requested: false,
        };
        fresh()?;
        let initialized = self.request("initialize",json!({"clientInfo":{"name":"kegari-v0","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}}),Some(&mut state),&mut broker).await?;
        verify_quiet_recovery(
            &state,
            &retained.state,
            self.server_request_count,
            broker.requested,
        )?;
        required_str(&initialized, "userAgent")?;
        required_str(&initialized, "platformFamily")?;
        required_str(&initialized, "platformOs")?;
        let home = required_str(&initialized, "codexHome")?;
        if self
            .expected_home
            .as_ref()
            .is_some_and(|expected| Path::new(home) != expected)
        {
            return Err(Error::new(
                "identity_mismatch",
                "metadata provider used a different auth home",
            ));
        }
        fresh()?;
        self.send(&json!({"method":"initialized"})).await?;
        self.initialized = true;
        fresh()?;
        let account = self
            .request(
                "account/read",
                json!({"refreshToken":false}),
                Some(&mut state),
                &mut broker,
            )
            .await?;
        verify_quiet_recovery(
            &state,
            &retained.state,
            self.server_request_count,
            broker.requested,
        )?;
        if account.pointer("/account/type").and_then(Value::as_str) != Some("chatgpt")
            || account
                .pointer("/workspaceRouting/chatgptAccountId")
                .and_then(Value::as_str)
                != Some(&state.binding.account_id)
        {
            return Err(Error::new(
                "identity_mismatch",
                "metadata provider account differs from original session",
            ));
        }
        self.account_id = Some(state.binding.account_id.clone());
        fresh()?;
        let configuration = self
            .request(
                "config/read",
                json!({"includeLayers":true,"cwd":state.binding.cwd}),
                Some(&mut state),
                &mut broker,
            )
            .await?;
        self.validate_execution_configuration(permissions, &configuration)?;
        verify_quiet_recovery(
            &state,
            &retained.state,
            self.server_request_count,
            broker.requested,
        )?;
        fresh()?;
        self.recover_terminal_failure(retained, broker.sink).await
    }
    /// Read only the known thread. Never starts/resumes a session or runs tools.
    /// Production callers must launch this adapter under typed preparation and
    /// process-journal authority, then join it before consuming the observation.
    pub async fn recover_terminal_failure(
        &mut self,
        retained: &RetainedResumeSession,
        sink: &mut dyn ToolBroker,
    ) -> Result<VerifiedThreadRecovery> {
        let mut state = retained.state.clone();
        if self.account_id.as_deref() != Some(&state.binding.account_id)
            || state.thread_id.is_none()
        {
            return Err(Error::new(
                "identity_mismatch",
                "terminal recovery requires the recorded authenticated account and thread",
            ));
        }
        let server_requests = self.server_request_count;
        let mut broker = RecoveryBroker {
            sink,
            requested: false,
        };
        let response = self
            .request(
                "thread/read",
                json!({"threadId":state.thread_id,"includeTurns":true}),
                Some(&mut state),
                &mut broker,
            )
            .await?;
        let thread = response
            .get("thread")
            .ok_or_else(|| Error::new("provider_response", "thread recovery lacks thread"))?;
        if thread.get("id").and_then(Value::as_str) != state.thread_id.as_deref()
            || thread.get("cwd").and_then(Value::as_str) != state.binding.cwd.to_str()
            || thread.get("modelProvider").and_then(Value::as_str) != Some("openai")
        {
            return Err(Error::new(
                "identity_mismatch",
                "thread recovery changed recorded identity, cwd or provider",
            ));
        }
        let turns = thread
            .get("turns")
            .and_then(Value::as_array)
            .ok_or_else(|| Error::new("provider_response", "thread recovery lacks full history"))?;
        if turns.len() > 4096 {
            return Err(Error::new(
                "history_limit",
                "thread recovery exceeds turn bound",
            ));
        }
        // Preserve authoritative intervention before any ordinary mismatch.
        for turn in turns {
            if let Some(error) = turn.get("error").filter(|error| !error.is_null()) {
                let code = error
                    .get("codexErrorInfo")
                    .and_then(Value::as_str)
                    .unwrap_or("provider_error");
                if [
                    "cyberPolicy",
                    "misalignmentPolicyViolation",
                    "tooManyDenials",
                ]
                .contains(&code)
                    || error
                        .get("misalignment")
                        .is_some_and(|value| !value.is_null())
                {
                    state.block_provider("thread/read", &json!({"threadId":state.thread_id,"turnId":required_str(turn,"id")?,"error":error}), code)?;
                    broker.observe_session(&state)?;
                    return Err(Error::new(
                        "provider_intervention",
                        "recovery observed an authoritative provider intervention",
                    ));
                }
            }
        }
        if broker.requested
            || self.server_request_count != server_requests
            || state.intervention.is_some()
            || state.input_required
            || canonical_digest(1, &state)? != canonical_digest(1, &retained.state)?
            || !matches!(
                thread.pointer("/status/type").and_then(Value::as_str),
                Some("idle" | "notLoaded")
            )
        {
            return Err(Error::new(
                "reconciliation_required",
                "thread changed or requested work during metadata recovery",
            ));
        }
        state
            .provider_history
            .as_ref()
            .ok_or_else(|| Error::new("resume_incompatible", "missing retained full history"))?
            .verify_readback(turns)?;
        let submissions = state
            .submissions
            .as_ref()
            .ok_or_else(|| Error::new("resume_incompatible", "missing actual submitted inputs"))?;
        submissions.verify_turns(turns)?;
        let (last, previous) = turns
            .split_last()
            .ok_or_else(|| Error::new("resume_incompatible", "empty provider history"))?;
        if previous
            .iter()
            .any(|turn| turn["status"] != "completed" || !turn["error"].is_null())
            || last["status"] != "failed"
        {
            return Err(Error::new(
                "resume_incompatible",
                "recovery requires one final observed transient failure",
            ));
        }
        let transient_failure = match last
            .pointer("/error/codexErrorInfo")
            .and_then(Value::as_str)
        {
            Some("serverOverloaded") => TransientTurnFailure::ServerOverloaded,
            Some("internalServerError") => TransientTurnFailure::InternalServerError,
            _ => {
                return Err(Error::new(
                    "persistent_blocker",
                    "terminal failure is outside the narrow supported transient recovery profile",
                ));
            }
        };
        let response_digest = canonical_digest(1, &response)?;
        let history_digest = canonical_digest(1, turns)?;
        let submission_digest = canonical_digest(1, submissions)?;
        let digest = canonical_digest(
            1,
            &json!({"retained":retained.digest,"response":response_digest,"history":history_digest,"submissions":submission_digest,"failure":transient_failure}),
        )?;
        Ok(VerifiedThreadRecovery {
            retained: retained.clone(),
            response_digest,
            history_digest,
            submission_digest,
            transient_failure,
            digest,
        })
    }
}

struct ResumeObserver<'a> {
    current: &'a mut SessionState,
    sink: &'a mut dyn ToolBroker,
    original: &'a SessionBinding,
}
impl ToolBroker for ResumeObserver<'_> {
    fn observe_session(&mut self, observed: &SessionState) -> Result<()> {
        if &observed.binding != self.original {
            return Err(Error::new(
                "resume_incompatible",
                "provider readback crossed original session binding",
            ));
        }
        if let Some(intervention) = &observed.intervention {
            self.current.intervention = Some(intervention.clone());
            self.current.terminal = Some("blocked".into());
        }
        self.current.input_required |= observed.input_required;
        self.sink.observe_session(self.current)
    }
    fn call<'a>(
        &'a mut self,
        _: &'a ToolCall,
    ) -> Pin<Box<dyn Future<Output = Result<Value>> + Send + 'a>> {
        Box::pin(async {
            Err(Error::new(
                "unsupported_tool",
                "resume history observation cannot invoke tools",
            ))
        })
    }
}

fn verify_quiet_recovery(
    state: &SessionState,
    retained: &SessionState,
    requests: u64,
    requested: bool,
) -> Result<()> {
    if state.intervention.is_some() {
        return Err(Error::new(
            "provider_intervention",
            "metadata recovery observed an authoritative intervention",
        ));
    }
    if state.input_required {
        return Err(Error::new(
            "input_required",
            "metadata recovery observed an interactive requirement",
        ));
    }
    if requests != 0 || requested || canonical_digest(1, state)? != canonical_digest(1, retained)? {
        return Err(Error::new(
            "reconciliation_required",
            "metadata initialization observed a changed or requested session",
        ));
    }
    Ok(())
}

struct RecoveryBroker<'a> {
    sink: &'a mut dyn ToolBroker,
    requested: bool,
}
impl ToolBroker for RecoveryBroker<'_> {
    fn observe_session(&mut self, state: &SessionState) -> Result<()> {
        self.sink.observe_session(state)
    }
    fn call<'a>(
        &'a mut self,
        _: &'a ToolCall,
    ) -> Pin<Box<dyn Future<Output = Result<Value>> + Send + 'a>> {
        self.requested = true;
        Box::pin(async {
            Err(Error::new(
                "unsupported_tool",
                "metadata recovery cannot invoke worker tools",
            ))
        })
    }
}

fn input_digest(input: &Value) -> Result<String> {
    let inputs = input
        .as_array()
        .ok_or_else(|| Error::new("protocol_error", "submitted input is not an array"))?;
    if inputs.is_empty() || inputs.len() > 4096 {
        return Err(Error::new(
            "history_limit",
            "submitted input count is invalid",
        ));
    }
    if serde_json::to_vec(input)
        .map_err(|_| Error::invalid("input encoding"))?
        .len()
        > MAX_FRAME_BYTES
    {
        return Err(Error::new(
            "history_limit",
            "submitted input exceeds frame bound",
        ));
    }
    let mut normalized = input.clone();
    for input in normalized.as_array_mut().expect("validated array") {
        if input.get("type").and_then(Value::as_str) == Some("text") {
            required_str(input, "text")?;
            let object = input
                .as_object_mut()
                .ok_or_else(|| Error::new("protocol_error", "text input is not an object"))?;
            object.entry("text_elements").or_insert(json!([]));
            if !object["text_elements"].is_array() {
                return Err(Error::new(
                    "protocol_error",
                    "text input elements are invalid",
                ));
            }
        }
    }
    canonical_digest(1, &normalized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    struct Observed(Arc<Mutex<Vec<SessionState>>>);
    impl ToolBroker for Observed {
        fn observe_session(&mut self, state: &SessionState) -> Result<()> {
            self.0.lock().unwrap().push(state.clone());
            Ok(())
        }
        fn call<'a>(
            &'a mut self,
            _: &'a ToolCall,
        ) -> Pin<Box<dyn Future<Output = Result<Value>> + Send + 'a>> {
            Box::pin(async { panic!("metadata recovery invoked a privileged broker") })
        }
    }
    async fn read(reader: &mut BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>) -> Value {
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        serde_json::from_str(&line).unwrap()
    }
    async fn write(writer: &mut tokio::io::WriteHalf<tokio::io::DuplexStream>, value: Value) {
        writer
            .write_all(format!("{value}\n").as_bytes())
            .await
            .unwrap();
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn metadata_recovery_binds_actual_submission_and_full_terminal_history() {
        for scenario in [
            "current",
            "changed_history",
            "changed_input",
            "nontransient",
            "policy",
            "tool_request",
            "summary",
        ] {
            let (client, server) = tokio::io::duplex(32_768);
            let (reader, writer) = tokio::io::split(client);
            let (server_reader, mut server_writer) = tokio::io::split(server);
            let mut server_reader = BufReader::new(server_reader);
            let observations = Arc::new(Mutex::new(Vec::<SessionState>::new()));
            let persisted = observations.clone();
            let server = tokio::spawn(async move {
                let request = read(&mut server_reader).await;
                assert_eq!(request["method"], "initialize");
                write(&mut server_writer, json!({"id":request["id"],"result":{"userAgent":SUPPORTED_CODEX_VERSION,"platformFamily":"unix","platformOs":"macos","codexHome":"/private/tmp/fixture-home"}})).await;
                assert_eq!(read(&mut server_reader).await["method"], "initialized");
                let request = read(&mut server_reader).await;
                assert_eq!(request["method"], "account/read");
                write(&mut server_writer, json!({"id":request["id"],"result":{"account":{"type":"chatgpt"},"workspaceRouting":{"chatgptAccountId":"fixture-account"}}})).await;
                let request = read(&mut server_reader).await;
                assert_eq!(request["method"], "turn/start");
                let history = persisted
                    .lock()
                    .unwrap()
                    .last()
                    .unwrap()
                    .submissions
                    .clone()
                    .unwrap();
                assert_eq!(
                    history.records[0].frame_digest,
                    canonical_digest(1, &request).unwrap()
                );
                assert_eq!(history.records[0].outcome, SubmissionOutcome::Prepared);
                let text = if scenario == "changed_input" {
                    "unsubmitted text"
                } else {
                    "actual admitted input"
                };
                let code = if scenario == "nontransient" {
                    "badRequest"
                } else {
                    "serverOverloaded"
                };
                let turn = json!({"id":"turn-one","status":"failed","items":[{"id":"input-one","type":"userMessage","content":[{"type":"text","text":text}]},{"id":"reason-one","type":"reasoning","summary":[],"content":[]}],"error":{"message":"fixture failure","codexErrorInfo":code}});
                write(&mut server_writer, json!({"method":"turn/started","params":{"threadId":"thread-one","turn":{"id":"turn-one","status":"inProgress"}}})).await;
                for total in [80, 100] {
                    write(&mut server_writer,json!({"method":"thread/tokenUsage/updated","params":{"threadId":"thread-one","turnId":"turn-one","tokenUsage":{"total":{"totalTokens":total}}}})).await;
                }
                write(&mut server_writer, json!({"method":"turn/completed","params":{"threadId":"thread-one","turn":turn}})).await;
                write(
                    &mut server_writer,
                    json!({"id":request["id"],"result":{"turn":turn}}),
                )
                .await;
                let request = read(&mut server_reader).await;
                assert_eq!(request["method"], "thread/read");
                if scenario == "tool_request" {
                    write(&mut server_writer, json!({"id":"tool-one","method":"future/tool","params":{"threadId":"thread-one"}})).await;
                    let denial = read(&mut server_reader).await;
                    assert_eq!(denial["error"]["code"], -32601);
                }
                let mut thread = json!({"id":"thread-one","cwd":"/private/tmp/fixture","modelProvider":"openai","status":{"type":"idle"},"turns":[turn]});
                match scenario {
                    "changed_history" => {
                        thread["turns"][0]["items"][1]["summary"] = json!(["altered"])
                    }
                    "summary" => thread["turns"][0]["itemsView"] = json!("summary"),
                    "policy" => {
                        thread["turns"][0]["id"] = json!("unrecorded");
                        thread["turns"][0]["error"]["codexErrorInfo"] = json!("cyberPolicy");
                    }
                    _ => (),
                }
                write(
                    &mut server_writer,
                    json!({"id":request["id"],"result":{"thread":thread}}),
                )
                .await;
                thread
            });
            let mut rpc =
                RpcClient::from_transport(reader, writer, Duration::from_secs(2)).unwrap();
            rpc.initialize().await.unwrap();
            rpc.verify_account("fixture-account").await.unwrap();
            let mut state = SessionState::new(
                SessionBinding {
                    schema_version: 1,
                    account_id: "fixture-account".into(),
                    work_identity: "fixture-work".into(),
                    operation_id: uuid::Uuid::new_v4().to_string(),
                    generation: "fixture-generation".into(),
                    instruction_digest: canonical_digest(1, &"instructions").unwrap(),
                    isolation_digest: canonical_digest(1, &"isolation").unwrap(),
                    profile: CodexProfile {
                        model: "fixture-model".into(),
                        reasoning: "high".into(),
                        speed: "standard".into(),
                    },
                    cwd: PathBuf::from("/private/tmp/fixture"),
                },
                3,
            )
            .unwrap();
            state.thread_id = Some("thread-one".into());
            state.turns_started = 1;
            let mut broker = Observed(observations.clone());
            rpc.request("turn/start", json!({"threadId":"thread-one","input":[{"type":"text","text":"actual admitted input"}]}), Some(&mut state), &mut broker).await.unwrap();
            let serialized = serde_json::to_vec(&state).unwrap();
            let state = SessionState::restore(&serialized).unwrap();
            // Component fixture: actual protocol observations, no claim of a
            // native joined worker, filesystem recovery or admitted successor.
            let reference = crate::control::RunReference {
                workspace: uuid::Uuid::new_v4(),
                account: state.binding.account_id.clone(),
                normalized_state: None,
                phase: crate::scheduler::PhaseKey {
                    work: state.binding.work_identity.clone(),
                    generation: canonical_digest(1, &"generation").unwrap(),
                    phase: "implementation".into(),
                    retry_grant: None,
                },
                profile_digest: canonical_digest(1, &"profile").unwrap(),
                instruction_digest: state.binding.instruction_digest.clone(),
                max_attempts: 3,
                max_turns: state.max_turns,
                max_review_rounds: 3,
                attempt: 1,
            };
            let candidate = RetainedResumeSession::from_journal(
                state,
                canonical_digest(1, &"fixture receipt").unwrap(),
                reference,
                canonical_digest(1, &"fixture journal").unwrap(),
            )
            .unwrap();
            let old_digest = canonical_digest(1, candidate.state()).unwrap();
            let result = rpc.recover_terminal_failure(&candidate, &mut broker).await;
            let current_proof = result.as_ref().ok().cloned();
            if scenario == "current" {
                let proof = result.unwrap();
                assert_eq!(
                    proof.transient_failure(),
                    &TransientTurnFailure::ServerOverloaded
                );
                assert_eq!(proof.retained().digest(), candidate.digest());
            } else if scenario == "policy" {
                assert_eq!(result.unwrap_err().code, "provider_intervention");
                assert!(
                    observations
                        .lock()
                        .unwrap()
                        .last()
                        .unwrap()
                        .intervention
                        .is_some()
                );
            } else {
                assert!(result.is_err(), "{scenario}");
            }
            assert_eq!(canonical_digest(1, candidate.state()).unwrap(), old_digest);
            let thread = server.await.unwrap();
            if scenario == "current" {
                resumed_wire_sequence(current_proof.as_ref().unwrap(), thread.clone()).await;
                metadata_sequence(&candidate, thread).await;
                response_policy_sequence(current_proof.as_ref().unwrap()).await;
            }
        }
    }
    async fn metadata_sequence(candidate: &RetainedResumeSession, thread: Value) {
        for scenario in ["current", "setup_policy", "setup_tool", "wrong_account"] {
            let (client, server) = tokio::io::duplex(32_768);
            let (reader, writer) = tokio::io::split(client);
            let (sr, mut sw) = tokio::io::split(server);
            let mut sr = BufReader::new(sr);
            let permissions = PermissionConfiguration {
                profile_id: "fixture".into(),
                isolation_digest: candidate.state().binding.isolation_digest.clone(),
                settings: json!({"default_permissions":"fixture","permissions":{"fixture":{"filesystem":{":root":"deny"},"network":{"enabled":false}}},"approval_policy":"never","approvals_reviewer":"user","web_search":"disabled","forced_login_method":"chatgpt","features":{}}),
            };
            let configuration = permissions.settings.clone();
            let thread = thread.clone();
            let server = tokio::spawn(async move {
                let init = read(&mut sr).await;
                assert_eq!(init["method"], "initialize");
                if scenario == "setup_policy" {
                    write(&mut sw,json!({"method":"model/verification","params":{"threadId":"thread-one","turnId":"turn-one","verifications":["trustedAccessForCyber"]}})).await;
                }
                if scenario == "setup_tool" {
                    write(&mut sw,json!({"id":"setup-tool","method":"future/tool","params":{"threadId":"thread-one"}})).await;
                    assert_eq!(read(&mut sr).await["error"]["code"], -32601);
                }
                write(&mut sw,json!({"id":init["id"],"result":{"userAgent":"fixture","platformFamily":"unix","platformOs":"macos","codexHome":"/private/tmp/fixture-home"}})).await;
                if matches!(scenario, "setup_policy" | "setup_tool") {
                    return;
                }
                assert_eq!(read(&mut sr).await["method"], "initialized");
                let account = read(&mut sr).await;
                assert_eq!(account["method"], "account/read");
                write(&mut sw,json!({"id":account["id"],"result":{"account":{"type":"chatgpt"},"workspaceRouting":{"chatgptAccountId":if scenario=="wrong_account" {"wrong"}else{"fixture-account"}}}})).await;
                if scenario == "wrong_account" {
                    return;
                }
                let config = read(&mut sr).await;
                assert_eq!(config["method"], "config/read");
                write(
                    &mut sw,
                    json!({"id":config["id"],"result":{"config":configuration}}),
                )
                .await;
                let request = read(&mut sr).await;
                assert_eq!(request["method"], "thread/read");
                write(
                    &mut sw,
                    json!({"id":request["id"],"result":{"thread":thread}}),
                )
                .await;
            });
            let observed = Arc::new(Mutex::new(Vec::new()));
            let mut broker = Observed(observed.clone());
            let mut rpc =
                RpcClient::from_transport(reader, writer, Duration::from_secs(2)).unwrap();
            let reads = std::sync::atomic::AtomicU32::new(0);
            let fresh = || {
                reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            };
            let result = rpc
                .recover_metadata(candidate, &permissions, &mut broker, &fresh)
                .await;
            match scenario {
                "current" => {
                    assert!(result.is_ok());
                    assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 5);
                }
                "setup_policy" => {
                    assert_eq!(result.unwrap_err().code, "provider_intervention");
                    assert!(
                        observed
                            .lock()
                            .unwrap()
                            .last()
                            .unwrap()
                            .intervention
                            .is_some()
                    );
                }
                "setup_tool" => assert_eq!(result.unwrap_err().code, "reconciliation_required"),
                _ => assert_eq!(result.unwrap_err().code, "identity_mismatch"),
            }
            server.await.unwrap();
        }
    }

    async fn response_policy_sequence(proof: &VerifiedThreadRecovery) {
        struct ObservedGate {
            states: Arc<Mutex<Vec<SessionState>>>,
            expected_policy: bool,
            observed: std::sync::atomic::AtomicBool,
            uncertain: std::sync::atomic::AtomicBool,
        }
        impl RequestGate for ObservedGate {
            fn dispatch(
                &self,
                _: u64,
                _: &str,
                _: &Value,
                transmit: &mut dyn FnMut() -> Result<()>,
            ) -> Result<()> {
                transmit()
            }
            fn observed(&self, _: u64, _: &str, result: &Result<Value>) -> Result<()> {
                assert!(result.is_ok(), "the raw provider reply was observed");
                assert_eq!(
                    self.states
                        .lock()
                        .unwrap()
                        .last()
                        .unwrap()
                        .intervention
                        .is_some(),
                    self.expected_policy,
                    "policy must already be checkpointed before acknowledgement bookkeeping",
                );
                self.observed
                    .store(true, std::sync::atomic::Ordering::SeqCst);
                if self.expected_policy {
                    Err(Error::new(
                        "ownership_conflict",
                        "fixture owner was fenced after transmission",
                    ))
                } else {
                    Ok(())
                }
            }
            fn uncertain(&self, _: u64, _: &str) -> Result<()> {
                self.uncertain
                    .store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            }
        }
        // Actual duplex request/response ordering, with an injected owner-loss
        // bookkeeping failure. This does not create native ownership authority.
        for scenario in [
            "fresh",
            "historical",
            "malformed_fields",
            "inline_review",
            "missing_turn_id",
            "foreign_review",
            "missing_review_thread",
            "foreign_request",
        ] {
            let expected_policy = matches!(
                scenario,
                "fresh" | "historical" | "malformed_fields" | "inline_review"
            );
            let old = proof.retained().state();
            let mut binding = old.binding.clone();
            binding.operation_id = uuid::Uuid::new_v4().to_string();
            let mut state = SessionState::new(binding, old.max_turns).unwrap();
            state.thread_id = old.thread_id.clone();
            state.resume_origin = Some(SessionResumeOrigin {
                predecessor: old.binding.operation_id.parse().unwrap(),
                recovery_digest: proof.digest().into(),
                instruction_compatibility_digest: canonical_digest(1, &"wire component").unwrap(),
                history_digest: proof.history_digest().into(),
                historical_turns: old.completed_turns.keys().cloned().collect(),
                thread: "thread-one".into(),
                readback_digest: None,
            });
            let review = matches!(
                scenario,
                "inline_review" | "foreign_review" | "missing_review_thread"
            );
            let method = if review { "review/start" } else { "turn/start" };
            let turn_id = if scenario == "historical" {
                "turn-one"
            } else {
                "turn-new"
            };
            let mut response = json!({"turn":{"id":turn_id,"status":"failed","items":[],"error":{"message":"provider policy signal","codexErrorInfo":"cyberPolicy"}}});
            match scenario {
                "malformed_fields" => {
                    response["turn"]["status"] = json!([]);
                    response["turn"]["items"] = json!("malformed ordinary fields");
                }
                "missing_turn_id" => {
                    response["turn"].as_object_mut().unwrap().remove("id");
                }
                "inline_review" => response["reviewThreadId"] = json!("thread-one"),
                "foreign_review" => response["reviewThreadId"] = json!("another-thread"),
                _ => (),
            }
            let states = Arc::new(Mutex::new(Vec::<SessionState>::new()));
            let gate = Arc::new(ObservedGate {
                states: states.clone(),
                expected_policy,
                observed: std::sync::atomic::AtomicBool::new(false),
                uncertain: std::sync::atomic::AtomicBool::new(false),
            });
            let (client, server) = tokio::io::duplex(32_768);
            let (reader, writer) = tokio::io::split(client);
            let (sr, mut sw) = tokio::io::split(server);
            let server = tokio::spawn(async move {
                let mut sr = BufReader::new(sr);
                let init = read(&mut sr).await;
                write(&mut sw,json!({"id":init["id"],"result":{"userAgent":"fixture","platformFamily":"unix","platformOs":"macos","codexHome":"/private/tmp/fixture-home"}})).await;
                assert_eq!(read(&mut sr).await["method"], "initialized");
                let request = read(&mut sr).await;
                assert_eq!(request["method"], method);
                write(&mut sw, json!({"id":request["id"],"result":response})).await;
            });
            let mut rpc =
                RpcClient::from_transport(reader, writer, Duration::from_secs(2)).unwrap();
            rpc.initialize().await.unwrap();
            rpc.bind_request_gate(gate.clone()).unwrap();
            let mut params = json!({"threadId":if scenario == "foreign_request" {"another-thread"} else {"thread-one"},"input":[{"type":"text","text":"admitted input"}]});
            if review {
                params["delivery"] = json!("inline");
            }
            let mut broker = Observed(states.clone());
            let error = rpc
                .request(method, params, Some(&mut state), &mut broker)
                .await
                .unwrap_err();
            assert!(
                gate.observed.load(std::sync::atomic::Ordering::SeqCst)
                    && !gate.uncertain.load(std::sync::atomic::Ordering::SeqCst),
                "{scenario}: matched reply did not reach acknowledgement bookkeeping: {error:?}",
            );
            assert_eq!(state.intervention.is_some(), expected_policy, "{scenario}");
            if expected_policy {
                assert_eq!(error.code, "ownership_conflict");
                let hold = states
                    .lock()
                    .unwrap()
                    .last()
                    .unwrap()
                    .intervention
                    .clone()
                    .unwrap();
                assert_eq!(hold.thread_id, "thread-one");
                assert_eq!(hold.turn_id, turn_id);
                assert_eq!(state.terminal.as_deref(), Some("blocked"));
                assert!(
                    state
                        .submissions
                        .as_ref()
                        .unwrap()
                        .records
                        .iter()
                        .all(|record| record.outcome == SubmissionOutcome::Prepared)
                );
            } else {
                assert_eq!(
                    error.code,
                    if scenario == "missing_turn_id" {
                        "protocol_error"
                    } else {
                        "identity_mismatch"
                    }
                );
            }
            server.await.unwrap();
        }
    }

    async fn resumed_wire_sequence(proof: &VerifiedThreadRecovery, thread: Value) {
        // This tests the private protocol component with its real duplex
        // readback proof. It creates no worker permit, directory enrollment,
        // VerifiedSessionResume or native joined-completion capability.
        for scenario in [
            "current",
            "no_prior_usage",
            "changed_history",
            "policy",
            "replay_ack",
            "replay_started",
            "replay_completed_policy",
        ] {
            let old = proof.retained().state();
            let original_digest = canonical_digest(1, old).unwrap();
            let mut binding = old.binding.clone();
            binding.operation_id = uuid::Uuid::new_v4().to_string();
            binding.instruction_digest = canonical_digest(1, &"new assembly").unwrap();
            let mut state = SessionState::new(binding, old.max_turns).unwrap();
            state.thread_id = old.thread_id.clone();
            state.usage = old
                .usage
                .resumed_session(&old.binding.account_id, "thread-one");
            state.resume_origin = Some(SessionResumeOrigin {
                predecessor: old.binding.operation_id.parse().unwrap(),
                recovery_digest: proof.digest().into(),
                instruction_compatibility_digest: canonical_digest(1, &"wire component").unwrap(),
                history_digest: proof.history_digest().into(),
                historical_turns: old.completed_turns.keys().cloned().collect(),
                thread: "thread-one".into(),
                readback_digest: None,
            });
            let observations = Arc::new(Mutex::new(Vec::<SessionState>::new()));
            let persisted = observations.clone();
            let (client, server) = tokio::io::duplex(32_768);
            let (reader, writer) = tokio::io::split(client);
            let (sr, mut sw) = tokio::io::split(server);
            let mut sr = BufReader::new(sr);
            let mut readback = thread.clone();
            let server = tokio::spawn(async move {
                let init = read(&mut sr).await;
                assert_eq!(init["method"], "initialize");
                write(&mut sw,json!({"id":init["id"],"result":{"userAgent":"fixture","platformFamily":"unix","platformOs":"macos","codexHome":"/private/tmp/fixture-home"}})).await;
                assert_eq!(read(&mut sr).await["method"], "initialized");
                let account = read(&mut sr).await;
                assert_eq!(account["method"], "account/read");
                write(&mut sw,json!({"id":account["id"],"result":{"account":{"type":"chatgpt"},"workspaceRouting":{"chatgptAccountId":"fixture-account"}}})).await;
                let history = read(&mut sr).await;
                assert_eq!(history["method"], "thread/read");
                assert_eq!(history["params"]["includeTurns"], true);
                match scenario {
                    "changed_history" => {
                        readback["turns"][0]["items"][1]["summary"] =
                            json!(["changed since metadata preparation"])
                    }
                    "policy" => {
                        readback["turns"][0]["error"]["codexErrorInfo"] = json!("cyberPolicy")
                    }
                    _ => (),
                }
                write(
                    &mut sw,
                    json!({"id":history["id"],"result":{"thread":readback}}),
                )
                .await;
                if matches!(scenario, "changed_history" | "policy") {
                    return;
                }
                let resume = read(&mut sr).await;
                assert_eq!(resume["method"], "thread/resume");
                assert_eq!(resume["params"]["threadId"], "thread-one");
                assert_eq!(
                    resume["params"]["developerInstructions"],
                    "actual independently assembled current prompt"
                );
                assert_eq!(resume["params"]["excludeTurns"], true);
                let observed = persisted.lock().unwrap().last().cloned().unwrap();
                assert!(observed.resume_origin.unwrap().readback_digest.is_some());
                assert_eq!(observed.turns_started, 0);
                assert!(observed.completed_turns.is_empty());
                // A delayed cumulative report belongs to no new turn. It is
                // retained as unattributed prior usage, never charged anew.
                if scenario != "no_prior_usage" {
                    for total in [150, 150, 90] {
                        write(&mut sw,json!({"method":"thread/tokenUsage/updated","params":{"threadId":"thread-one","turnId":"turn-one","tokenUsage":{"total":{"totalTokens":total}}}})).await;
                    }
                }
                write(&mut sw,json!({"id":resume["id"],"result":{"thread":{"id":"thread-one"},"cwd":"/private/tmp/fixture","model":"fixture-model","modelProvider":"openai","reasoningEffort":"high","approvalPolicy":"never","activePermissionProfile":{"id":"fixture"},"instructionSources":["current supplied guidance"]}})).await;
                let next = read(&mut sr).await;
                assert_eq!(next["method"], "turn/start");
                let input = next["params"]["input"][0]["text"].as_str().unwrap();
                assert!(input.contains("Verified continuation of the same task"));
                assert!(input.contains("continue the admitted task"));
                if scenario == "replay_ack" {
                    write(&mut sw,json!({"id":next["id"],"result":{"turn":{"id":"turn-one","status":"inProgress"}}})).await;
                    return;
                }
                if scenario == "replay_started" {
                    write(&mut sw,json!({"method":"turn/started","params":{"threadId":"thread-one","turn":{"id":"turn-one","status":"inProgress"}}})).await;
                    return;
                }
                let turn = json!({"id":"turn-two","status":"completed","items":[{"id":"input-two","type":"userMessage","content":next["params"]["input"]}],"error":null});
                write(&mut sw,json!({"method":"turn/started","params":{"threadId":"thread-one","turn":{"id":"turn-two","status":"inProgress"}}})).await;
                if scenario == "replay_completed_policy" {
                    write(&mut sw,json!({"method":"turn/completed","params":{"threadId":"thread-one","turn":{"id":"turn-one","status":"failed","error":{"message":"historical policy signal","codexErrorInfo":"cyberPolicy"}}}})).await;
                    return;
                }
                // Prior-turn reports can arrive after new work begins. Their
                // timing is ambiguous, even though the cumulative delta is known.
                if scenario != "no_prior_usage" {
                    for total in [160, 160, 140] {
                        write(&mut sw,json!({"method":"thread/tokenUsage/updated","params":{"threadId":"thread-one","turnId":"turn-one","tokenUsage":{"total":{"totalTokens":total}}}})).await;
                    }
                }
                let first = if scenario == "no_prior_usage" {
                    160
                } else {
                    170
                };
                for total in [first, first, first - 5] {
                    write(&mut sw,json!({"method":"thread/tokenUsage/updated","params":{"threadId":"thread-one","turnId":"turn-two","tokenUsage":{"total":{"totalTokens":total}}}})).await;
                }
                // Even after the first known new turn, old-turn increments
                // remain unattributed; duplicate/lower reports never drift.
                for total in [first + 5, first + 5, first - 5] {
                    write(&mut sw,json!({"method":"thread/tokenUsage/updated","params":{"threadId":"thread-one","turnId":"turn-one","tokenUsage":{"total":{"totalTokens":total}}}})).await;
                }
                write(&mut sw,json!({"method":"thread/tokenUsage/updated","params":{"threadId":"thread-one","turnId":"turn-two","tokenUsage":{"total":{"totalTokens":first + 10}}}})).await;
                write(&mut sw,json!({"method":"turn/completed","params":{"threadId":"thread-one","turn":turn}})).await;
                for (turn_id, total) in [
                    ("turn-two", first + 15),
                    ("turn-one", first + 14),
                    ("turn-two", first + 20),
                ] {
                    write(&mut sw,json!({"method":"thread/tokenUsage/updated","params":{"threadId":"thread-one","turnId":turn_id,"tokenUsage":{"total":{"totalTokens":total}}}})).await;
                }
                write(&mut sw, json!({"id":next["id"],"result":{"turn":turn}})).await;
            });
            let mut rpc =
                RpcClient::from_transport(reader, writer, Duration::from_secs(2)).unwrap();
            rpc.initialize().await.unwrap();
            rpc.verify_account("fixture-account").await.unwrap();
            let mut broker = Observed(observations);
            let permissions = PermissionConfiguration {
                profile_id: "fixture".into(),
                isolation_digest: state.binding.isolation_digest.clone(),
                settings: json!({}),
            };
            let result = rpc
                .resume_protocol(
                    &mut state,
                    ResumeProtocol {
                        recovery: proof,
                        prompt: "actual independently assembled current prompt",
                        predecessor: old.binding.operation_id.parse().unwrap(),
                        bridge_digest: proof.digest(),
                    },
                    &permissions,
                    &mut broker,
                )
                .await;
            match scenario {
                "current" | "no_prior_usage" => {
                    result.unwrap();
                    assert!(state.usage.observed_tokens.is_empty());
                    assert_eq!(
                        state.usage.cumulative_baselines["fixture-account\0thread-one"],
                        if scenario == "no_prior_usage" {
                            100
                        } else {
                            150
                        }
                    );
                    if scenario == "no_prior_usage" {
                        assert!(state.usage.prior_unattributed_usage.is_empty());
                    } else {
                        assert_eq!(
                            state.usage.prior_unattributed_usage["fixture-account\0thread-one"],
                            PriorUnattributedUsage {
                                known_increase: 50,
                                retained_baseline_missing: false,
                                timing_unknown: false,
                                first_new_turn_boundary: None,
                            }
                        );
                    }
                    rpc.start_turn(
                        &mut state,
                        &permissions,
                        "continue the admitted task",
                        &mut broker,
                    )
                    .await
                    .unwrap();
                    assert_eq!(state.turns_started, 1);
                    assert_eq!(
                        state.completed_turns.keys().cloned().collect::<Vec<_>>(),
                        ["turn-two"]
                    );
                    assert_eq!(state.submissions.as_ref().unwrap().records.len(), 1);
                    assert_eq!(state.usage.observed_tokens["fixture-account"], 15);
                    assert_eq!(old.usage.observed_tokens["fixture-account"], 20);
                    assert_eq!(
                        state.usage.prior_unattributed_usage["fixture-account\0thread-one"],
                        PriorUnattributedUsage {
                            known_increase: if scenario == "no_prior_usage" { 65 } else { 75 },
                            retained_baseline_missing: false,
                            timing_unknown: true,
                            first_new_turn_boundary: Some(if scenario == "no_prior_usage" {
                                160
                            } else {
                                170
                            }),
                        }
                    );
                    assert_eq!(
                        state.usage.cumulative_baselines["fixture-account\0thread-one"],
                        if scenario == "no_prior_usage" {
                            180
                        } else {
                            190
                        }
                    );
                    let events = rpc
                        .drain_events()
                        .into_iter()
                        .filter(|event| event.method == "thread/tokenUsage/updated")
                        .collect::<Vec<_>>();
                    assert!(
                        events
                            .iter()
                            .filter(|event| event.turn_id.as_deref() == Some("turn-one"))
                            .all(|event| event.data["turn_id"] == "turn-one"
                                && event.data["observed_delta"] == 0
                                && event.data["baseline_only"] == true)
                    );
                    assert!(
                        events
                            .iter()
                            .any(|event| event.data["turn_id"] == "turn-one"
                                && event.data["cumulative_total"]
                                    == if scenario == "no_prior_usage" {
                                        165
                                    } else {
                                        175
                                    }
                                && event.data["timing_unknown"] == true)
                    );
                    assert_eq!(
                        events
                            .iter()
                            .filter(|event| event.turn_id.as_deref() == Some("turn-two"))
                            .map(|event| event.data["observed_delta"].as_u64().unwrap())
                            .collect::<Vec<_>>(),
                        [0, 0, 0, 5, 5, 5]
                    );
                    let boundary = events
                        .iter()
                        .find(|event| event.turn_id.as_deref() == Some("turn-two"))
                        .unwrap();
                    assert_eq!(boundary.data["baseline_only"], true);
                    assert_eq!(boundary.data["unqualified_resume_boundary"], true);
                    assert_eq!(boundary.data["timing_unknown"], true);
                    assert_eq!(
                        state.provider_instruction_sources,
                        ["current supplied guidance"]
                    );
                    SessionState::restore(&serde_json::to_vec(&state).unwrap()).unwrap();
                }
                "replay_ack" | "replay_started" | "replay_completed_policy" => {
                    result.unwrap();
                    let error = rpc
                        .start_turn(
                            &mut state,
                            &permissions,
                            "continue the admitted task",
                            &mut broker,
                        )
                        .await
                        .unwrap_err();
                    assert_eq!(error.code, "historical_turn_replay");
                    assert!(!state.completed_turns.contains_key("turn-one"));
                    assert_ne!(state.turn_id.as_deref(), Some("turn-one"));
                    assert!(
                        state
                            .submissions
                            .as_ref()
                            .unwrap()
                            .records
                            .iter()
                            .all(|record| record.outcome == SubmissionOutcome::Prepared)
                    );
                    assert_eq!(
                        state.intervention.is_some(),
                        scenario == "replay_completed_policy"
                    );
                    if scenario == "replay_completed_policy" {
                        assert_eq!(state.terminal.as_deref(), Some("blocked"));
                    }
                }
                "policy" => {
                    assert_eq!(result.unwrap_err().code, "provider_intervention");
                    assert!(state.intervention.is_some());
                    assert_eq!(state.terminal.as_deref(), Some("blocked"));
                }
                _ => assert_eq!(result.unwrap_err().code, "reconciliation_required"),
            }
            assert_eq!(canonical_digest(1, old).unwrap(), original_digest);
            server.await.unwrap();
        }
    }

    #[test]
    fn resumed_baseline_without_prior_counter_remains_explicitly_unknown() {
        let mut usage = UsageLedger::default().resumed_session("account", "thread");
        usage.observe_resume_baseline("account", "thread", 150, false);
        usage.observe_resume_baseline("account", "thread", 170, false);
        usage.observe_resume_baseline("account", "thread", 140, false);
        assert!(usage.observed_tokens.is_empty());
        assert!(usage.earlier_usage_unknown.contains("account\0thread"));
        assert_eq!(
            usage.prior_unattributed_usage["account\0thread"],
            PriorUnattributedUsage {
                known_increase: 20,
                retained_baseline_missing: true,
                timing_unknown: false,
                first_new_turn_boundary: None,
            }
        );
        assert_eq!(usage.cumulative_baselines["account\0thread"], 170);
        let serialized = serde_json::to_vec(&usage).unwrap();
        let mut restored: UsageLedger = serde_json::from_slice(&serialized).unwrap();
        assert_eq!(restored.observe("account", "thread", 180, false), 10);
        assert_eq!(
            restored.prior_unattributed_usage["account\0thread"],
            PriorUnattributedUsage {
                known_increase: 20,
                retained_baseline_missing: true,
                timing_unknown: false,
                first_new_turn_boundary: None,
            }
        );
    }
}
