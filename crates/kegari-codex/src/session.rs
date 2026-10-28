//! Protocol session state and observed history; no native execution authority.
use super::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionState {
    pub schema_version: u32,
    pub binding: SessionBinding,
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    /// Known acknowledged turn whose terminal provider outcome was not
    /// observed before interruption. Process exit cannot erase its identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unresolved_turn: Option<String>,
    pub turns_started: u32,
    pub max_turns: u32,
    pub correction_required: u64,
    pub correction_consumed: u64,
    pub intervention: Option<ProviderIntervention>,
    pub input_required: bool,
    pub terminal: Option<String>,
    pub completed_turns: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_completed_turn: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_history: Option<RecordedThreadHistory>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submissions: Option<SubmissionHistory>,
    /// Immutable predecessor provenance; historical turns and totals stay in
    /// their original protected session rather than being relabelled here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_origin: Option<resume::SessionResumeOrigin>,
    pub provider_instruction_sources: Vec<String>,
    pub advertised_tools: BTreeSet<String>,
    pub reconciliation_digest: Option<String>,
    pub usage: UsageLedger,
}
impl SessionState {
    pub fn new(binding: SessionBinding, max_turns: u32) -> Result<Self> {
        binding.validate()?;
        if max_turns == 0 {
            return Err(Error::invalid("max_turns must be positive"));
        }
        Ok(Self {
            schema_version: 1,
            binding,
            thread_id: None,
            turn_id: None,
            unresolved_turn: None,
            turns_started: 0,
            max_turns,
            correction_required: 0,
            correction_consumed: 0,
            intervention: None,
            input_required: false,
            terminal: None,
            completed_turns: BTreeMap::new(),
            last_completed_turn: None,
            provider_history: Some(RecordedThreadHistory::default()),
            submissions: Some(SubmissionHistory::default()),
            resume_origin: None,
            provider_instruction_sources: Vec::new(),
            advertised_tools: BTreeSet::new(),
            reconciliation_digest: None,
            usage: UsageLedger::default(),
        })
    }
    pub fn restore(bytes: &[u8]) -> Result<Self> {
        let state: Self = serde_json::from_slice(bytes)
            .map_err(|_| Error::new("corrupt_receipt", "invalid provider recovery state"))?;
        state.binding.validate()?;
        if state.schema_version != 1
            || state.max_turns == 0
            || state.turns_started > state.max_turns
            || state.correction_consumed > state.correction_required
            || state.unresolved_turn.as_ref().is_some_and(|id| {
                id.is_empty()
                    || state.completed_turns.contains_key(id)
                    || state.turn_id.as_ref().is_some_and(|active| active != id)
            })
            || state.completed_turns.len() > state.max_turns as usize
            || state
                .last_completed_turn
                .as_ref()
                .is_some_and(|id| !state.completed_turns.contains_key(id))
        {
            return Err(Error::new(
                "corrupt_receipt",
                "inconsistent provider recovery state",
            ));
        }
        if let Some(history) = &state.provider_history {
            history.validate(&state.completed_turns, state.last_completed_turn.as_deref())?;
        }
        if let Some(submissions) = &state.submissions {
            submissions.validate(&state)?;
        }
        if let Some(origin) = &state.resume_origin {
            origin.validate(&state)?;
        }
        Ok(state)
    }
    pub fn next_turn_allowed(&self) -> Result<()> {
        if self.intervention.is_some() {
            return Err(Error::new(
                "provider_intervention",
                "provider resolution required before any retry, new turn or replacement session",
            ));
        }
        if self.input_required {
            return Err(Error::new(
                "input_required",
                "work requires planning resolution",
            ));
        }
        if self.turn_id.is_some() {
            return Err(Error::new(
                "turn_active",
                "an existing turn must finish or be interrupted",
            ));
        }
        if self.unresolved_turn.is_some() {
            return Err(Error::new(
                "reconciliation_required",
                "known interrupted provider turn has no terminal reconciliation",
            ));
        }
        if self.turns_started >= self.max_turns {
            return Err(Error::new(
                "turn_budget_exhausted",
                "session turn bound reached",
            ));
        }
        Ok(())
    }
    pub fn observe_correction(&mut self, watermark: u64) {
        self.correction_required = self.correction_required.max(watermark);
    }
    pub fn acknowledge_correction(&mut self, watermark: u64) -> Result<()> {
        if watermark > self.correction_required {
            return Err(Error::new(
                "stale_result",
                "worker claims an unknown correction watermark",
            ));
        }
        self.correction_consumed = self.correction_consumed.max(watermark);
        Ok(())
    }
    pub fn completion_is_current(&self) -> bool {
        self.terminal.as_deref() == Some("completed")
            && self.unresolved_turn.is_none()
            && self.correction_consumed == self.correction_required
            && self.intervention.is_none()
            && !self.input_required
    }
    pub fn interruption(&mut self, reason: &str) -> SessionEvent {
        if self.intervention.is_none() {
            self.terminal = Some("interrupted".into());
        }
        if let Some(turn) = self.turn_id.take() {
            self.unresolved_turn = Some(turn);
        }
        SessionEvent::new(
            if self.intervention.is_some() {
                EventKind::ProviderIntervention
            } else {
                EventKind::ProcessExit
            },
            reason,
            self.thread_id.clone(),
            self.unresolved_turn.clone(),
            json!({"outcome":if self.intervention.is_some(){"blocked"}else{"interrupted"}}),
        )
    }
    pub(super) fn block_provider(
        &mut self,
        method: &str,
        params: &Value,
        reason: &str,
    ) -> Result<()> {
        let thread = required_str(params, "threadId")?;
        let turn = required_str(params, "turnId")?;
        if self.thread_id.as_deref() != Some(thread) {
            return Err(Error::new(
                "protocol_error",
                "provider event belongs to another thread",
            ));
        }
        if self.intervention.is_none() {
            self.intervention=Some(ProviderIntervention{schema_version:1,account_id:self.binding.account_id.clone(),thread_id:thread.into(),turn_id:turn.into(),source_method:method.into(),reason_code:reason.into(),required_resolution:"Resolve through the provider's supported verification/policy process; record verified resolution before readmission. A new thread or account cannot bypass this hold.".into(),observed_at:chrono::Utc::now().to_rfc3339(),source_digest:canonical_digest(1,params)?});
        }
        self.terminal = Some("blocked".into());
        Ok(())
    }
    pub(super) fn require_new_turn_identity(&self, id: &str) -> Result<()> {
        if self
            .resume_origin
            .as_ref()
            .is_some_and(|origin| origin.historical_turns.contains(id))
        {
            return Err(Error::new(
                "historical_turn_replay",
                "provider reused a retained historical turn in the new session",
            ));
        }
        Ok(())
    }
    /// Provider observation only. Retry authority additionally requires the
    /// actual joined completion and its unchanged protected admission receipt.
    pub fn terminal_transient_failure(&self) -> Result<Option<TransientTurnFailure>> {
        if self.terminal.as_deref() != Some("failed")
            || self.turn_id.is_some()
            || self.unresolved_turn.is_some()
            || self.intervention.is_some()
            || self.input_required
        {
            return Ok(None);
        }
        let Some(last) = self.last_completed_turn.as_deref().filter(|last| {
            self.completed_turns
                .get(*last)
                .is_some_and(|status| status == "failed")
        }) else {
            return Ok(None);
        };
        self.provider_history
            .as_ref()
            .map(|history| history.terminal_transient_failure(last))
            .transpose()
            .map(Option::flatten)
    }
    pub(super) fn observe_turn_policy(&mut self, method: &str, turn: &Value) -> Result<()> {
        if let Some(error) = turn.get("error").filter(|v| !v.is_null()) {
            let code = error
                .get("codexErrorInfo")
                .and_then(Value::as_str)
                .unwrap_or("");
            if [
                "cyberPolicy",
                "misalignmentPolicyViolation",
                "tooManyDenials",
            ]
            .contains(&code)
                || error.get("misalignment").is_some_and(|v| !v.is_null())
            {
                let params = json!({"threadId":self.thread_id,"turnId":required_str(turn,"id")?,"error":error});
                self.block_provider(method, &params, code)?;
            }
        }
        Ok(())
    }
    pub fn notification(&mut self, method: &str, params: &Value) -> Result<SessionEvent> {
        let thread = params
            .get("threadId")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let turn = params
            .get("turnId")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if let Some(id) = &thread
            && self.thread_id.as_ref().is_some_and(|t| t != id)
        {
            return Err(Error::new(
                "protocol_error",
                "notification crosses bound thread",
            ));
        }
        let mut kind = EventKind::Notification;
        let mut data = Value::Null;
        match method {
            "model/verification" => {
                let verifications = params
                    .get("verifications")
                    .and_then(Value::as_array)
                    .ok_or_else(|| {
                        Error::new(
                            "protocol_error",
                            "verification notification missing classifications",
                        )
                    })?;
                if verifications.is_empty() {
                    return Err(Error::new(
                        "protocol_error",
                        "empty verification does not establish resolution",
                    ));
                }
                if !verifications.iter().all(Value::is_string) {
                    return Err(Error::new(
                        "protocol_error",
                        "invalid verification classifications",
                    ));
                }
                self.block_provider(method, params, "verification_required")?;
                kind = EventKind::ProviderIntervention;
            }
            "error" => {
                required_str(params, "threadId")?;
                required_str(params, "turnId")?;
                if !params.get("willRetry").is_some_and(Value::is_boolean) {
                    return Err(Error::new(
                        "protocol_error",
                        "error event missing willRetry",
                    ));
                }
                let error = params
                    .get("error")
                    .ok_or_else(|| Error::new("protocol_error", "error event missing error"))?;
                required_str(error, "message")?;
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
                    || error.get("misalignment").is_some_and(|v| !v.is_null())
                {
                    self.block_provider(method, params, code)?;
                    kind = EventKind::ProviderIntervention;
                } else {
                    kind = EventKind::TurnFailed;
                    data = json!({"code":code,"will_retry":params["willRetry"]});
                }
            }
            "model/safetyBuffering/updated" => {
                required_str(params, "threadId")?;
                required_str(params, "turnId")?;
                required_str(params, "model")?;
                if !params.get("showBufferingUi").is_some_and(Value::is_boolean)
                    || !params.get("reasons").is_some_and(Value::is_array)
                    || !params.get("useCases").is_some_and(Value::is_array)
                {
                    return Err(Error::new("protocol_error", "malformed buffering event"));
                }
                data = json!({"transient_buffering":params["showBufferingUi"]});
            }
            "turn/started" => {
                let t = params
                    .get("turn")
                    .ok_or_else(|| Error::new("protocol_error", "turn event lacks turn"))?;
                let id = required_str(t, "id")?;
                self.observe_turn_policy(method, t)?;
                self.require_new_turn_identity(id)?;
                if self.completed_turns.contains_key(id) {
                    return Ok(SessionEvent::new(
                        EventKind::Notification,
                        method,
                        thread,
                        turn,
                        json!({"duplicate":true}),
                    ));
                }
                if self.turn_id.as_deref().is_some_and(|old| old != id) {
                    return Err(Error::new("protocol_error", "unexpected concurrent turn"));
                }
                if self.unresolved_turn.as_deref().is_some_and(|old| old != id) {
                    return Err(Error::new(
                        "protocol_error",
                        "new turn cannot replace an unresolved known turn",
                    ));
                }
                self.turn_id = Some(id.into());
                kind = EventKind::TurnStarted;
            }
            "turn/completed" => {
                let t = params
                    .get("turn")
                    .ok_or_else(|| Error::new("protocol_error", "completion lacks turn"))?;
                let id = required_str(t, "id")?;
                // Actual same-thread policy signals remain authoritative even
                // when the surrounding turn is duplicate, late or malformed.
                // Ordinary outcome/history validation must not erase this hold.
                self.observe_turn_policy(method, t)?;
                self.require_new_turn_identity(id)?;
                let status = required_str(t, "status")?;
                if let Some(previous) = self.completed_turns.get(id) {
                    if previous != status {
                        return Err(Error::new(
                            "protocol_error",
                            "duplicate terminal event changed its outcome",
                        ));
                    }
                    if let Some(history) = &mut self.provider_history {
                        history.observe(t)?;
                    }
                    return Ok(SessionEvent::new(
                        if self.intervention.is_some() {
                            EventKind::ProviderIntervention
                        } else {
                            EventKind::Notification
                        },
                        method,
                        thread,
                        turn,
                        json!({"duplicate":true}),
                    ));
                }
                if self.turn_id.as_deref() != Some(id)
                    && self.unresolved_turn.as_deref() != Some(id)
                {
                    return Err(Error::new(
                        "protocol_error",
                        "completion is not for the active turn",
                    ));
                }
                let status = required_str(t, "status")?;
                if !["completed", "failed", "interrupted"].contains(&status) {
                    return Err(Error::new(
                        "protocol_error",
                        "completion has nonterminal status",
                    ));
                }
                if let Some(history) = &mut self.provider_history {
                    history.observe(t)?;
                }
                if self.intervention.is_none() {
                    self.terminal = Some(status.into());
                }
                self.turn_id = None;
                self.unresolved_turn = None;
                self.completed_turns.insert(id.into(), status.into());
                self.last_completed_turn = Some(id.into());
                kind = if self.intervention.is_some() {
                    EventKind::ProviderIntervention
                } else {
                    match status {
                        "completed" => EventKind::TurnCompleted,
                        "interrupted" => EventKind::TurnCancelled,
                        _ => EventKind::TurnFailed,
                    }
                };
                data = json!({"status":status,"turn_id":id});
            }
            "thread/tokenUsage/updated" => {
                let t = required_str(params, "threadId")?;
                let usage_turn = required_str(params, "turnId")?;
                let total = params
                    .pointer("/tokenUsage/total/totalTokens")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| {
                        Error::new("protocol_error", "invalid cumulative token counter")
                    })?;
                let known_turn = self.turn_id.as_deref() == Some(usage_turn)
                    || self.unresolved_turn.as_deref() == Some(usage_turn)
                    || self.completed_turns.contains_key(usage_turn);
                let key = format!("{}\0{t}", self.binding.account_id);
                let unqualified_boundary = self.resume_origin.is_some()
                    && known_turn
                    && self
                        .usage
                        .prior_unattributed_usage
                        .get(&key)
                        .is_none_or(|usage| usage.first_new_turn_boundary.is_none());
                if self.resume_origin.is_some() && (!known_turn || unqualified_boundary) {
                    let timing_unknown = self.turns_started != 0 || unqualified_boundary;
                    let monotone = self
                        .usage
                        .cumulative_baselines
                        .get(&key)
                        .is_none_or(|previous| total >= *previous);
                    self.usage.observe_resume_baseline(
                        &self.binding.account_id,
                        t,
                        total,
                        timing_unknown,
                    );
                    if unqualified_boundary && monotone {
                        self.usage
                            .prior_unattributed_usage
                            .entry(key.clone())
                            .or_default()
                            .first_new_turn_boundary = Some(total);
                    }
                    data = json!({"turn_id":usage_turn,"observed_delta":0,"cumulative_total":total,"baseline_only":true,"timing_unknown":timing_unknown,"unqualified_resume_boundary":unqualified_boundary,"prior_unattributed_usage":self.usage.prior_unattributed_usage.get(&key)});
                } else {
                    let delta = self
                        .usage
                        .observe(&self.binding.account_id, t, total, false);
                    data = json!({"turn_id":usage_turn,"observed_delta":delta,"cumulative_total":total,"baseline_only":false});
                }
            }
            "model/rerouted" => {
                self.input_required = true;
                self.terminal = Some("blocked".into());
                kind = EventKind::ModelChanged;
                data = json!({"code":"unsupported_profile","reason":"provider changed the selected model"});
            }
            _ => {}
        }
        Ok(SessionEvent::new(kind, method, thread, turn, data))
    }
}
