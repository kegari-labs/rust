//! Observation-only declared protocol channels. This is not execution authority,
//! complete network/output monitoring, hidden-reasoning capture or a scorer.
use super::*;
use uuid::Uuid;

/// Borrowed adapter observations; recording failure must not authorize, retry or
/// change a provider request. Normal brokers default to ignoring these events.
#[derive(Clone, Copy)]
pub enum ProtocolObservation<'a> {
    ThreadEstablished {
        state: &'a SessionState,
    },
    UnboundNotification {
        method: &'a str,
        params: &'a Value,
    },
    MatchedResponse {
        state: Option<&'a SessionState>,
        rpc_id: &'a Value,
        method: &'a str,
        request_thread: Option<&'a str>,
        inline_review: bool,
        response: &'a Value,
    },
    TransportEnd {
        state: &'a SessionState,
        end: &'a ProtocolTransportEnd,
    },
    Issue {
        state: Option<&'a SessionState>,
        issue: ProtocolCaptureIssue,
    },
    ServerRequest {
        state: Option<&'a SessionState>,
        rpc_id: &'a Value,
        sequence: u64,
        method: &'a str,
        params: &'a Value,
    },
    ToolResult {
        state: Option<&'a SessionState>,
        rpc_id: &'a Value,
        error_code: Option<&'a str>,
    },
    AdapterInventory {
        state: &'a SessionState,
        processed_requests: Option<u64>,
    },
    CompletedItem {
        state: &'a SessionState,
        params: &'a Value,
        accepted: bool,
    },
    AgentMessageDelta {
        state: &'a SessionState,
        params: &'a Value,
        accepted: bool,
    },
    TerminalTurn {
        state: &'a SessionState,
        params: &'a Value,
        accepted: bool,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProtocolCaptureState {
    NotDeclared,
    Unavailable,
    Incomplete,
    Overflow,
    Conflict,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ProtocolCaptureIssue {
    ScopeNotDeclared,
    DeclarationUnavailable,
    MissingRequestInventory,
    UnobservedTransportTail,
    CoverageNotQualified,
    CollectionDeadline,
    UnestablishedThread,
    UnconfirmedResponseTurn,
    LateUnservedRequest,
    UnsupportedTailFrame,
    MissingSession,
    BindingChanged,
    ForeignOrUnknownTurn,
    UnsupportedRequestChannel,
    UnsupportedOutputChannel,
    MalformedMessage,
    MissingFullTurn,
    RejectedTerminal,
    RejectedItem,
    ConflictingDuplicate,
    UnfinishedRequest,
    IncompleteTurnInventory,
    CaptureBound,
    ProtectedStorage,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtocolRecordReference {
    pub sequence: u32,
    pub journal_leaf: String,
    pub digest: String,
    pub bytes: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProtocolTransportEndDiagnostic {
    CleanEof {
        frames: u64,
        decoded_bytes: u64,
        wire_bytes: u64,
    },
    Incomplete {
        frames: u64,
        decoded_bytes: u64,
        wire_bytes: u64,
        error: Error,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeclaredProtocolObservation {
    pub schema_version: u32,
    pub operation: Uuid,
    pub runtime: Uuid,
    pub workspace: Uuid,
    pub trial_digest: Option<String>,
    pub suite_digest: String,
    pub fixture_digest: String,
    pub contract_scope_digest: String,
    pub instruction_digest: String,
    pub session_binding_digest: Option<String>,
    pub thread_id: Option<String>,
    /// Only item/tool/call parameters and terminal agentMessage.text. Tool
    /// results, other provider output, reasoning and external traffic are not
    /// asserted complete. Raw text exists only in the protected journal.
    pub state: ProtocolCaptureState,
    pub issues: BTreeSet<ProtocolCaptureIssue>,
    pub tool_request_attempts: u32,
    pub observed_server_requests: u64,
    pub processed_server_requests: Option<u64>,
    pub terminal_turns: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport_end: Option<ProtocolTransportEndDiagnostic>,
    pub records: Vec<ProtocolRecordReference>,
}
impl DeclaredProtocolObservation {
    /// Data shape only; this cannot reconstruct a worker or grant permission.
    pub fn validate(&self) -> Result<()> {
        let invalid = || {
            Error::new(
                "protocol_observation",
                "invalid declared protocol observation",
            )
        };
        if self.schema_version != 1
            || self.operation.is_nil()
            || self.runtime.is_nil()
            || self.workspace.is_nil()
            || self.records.len() > MAX_RECORDS
        {
            return Err(invalid());
        }
        for digest in [
            &self.contract_scope_digest,
            &self.instruction_digest,
            &self.suite_digest,
            &self.fixture_digest,
        ] {
            validate_digest(digest)?;
        }
        if let Some(digest) = &self.trial_digest {
            validate_digest(digest)?;
        }
        if let Some(digest) = &self.session_binding_digest {
            validate_digest(digest)?;
        }
        if self.thread_id.as_deref().is_some_and(|s| !identifier(s)) {
            return Err(invalid());
        }
        let mut total = 0_u64;
        let mut leaves = BTreeSet::new();
        for (index, record) in self.records.iter().enumerate() {
            let leaf = &record.journal_leaf;
            if record.sequence as usize != index
                || leaf.len() != 19
                || !leaf.starts_with("event-")
                || !leaf.ends_with(".json")
                || !leaf.as_bytes()[6..14].iter().all(u8::is_ascii_digit)
                || !leaves.insert(leaf)
                || record.bytes > MAX_RECORD_BYTES as u64
            {
                return Err(invalid());
            }
            validate_digest(&record.digest)?;
            total = total.checked_add(record.bytes).ok_or_else(invalid)?;
        }
        if total > MAX_CAPTURE_BYTES as u64 {
            return Err(invalid());
        }
        Ok(())
    }
}

const MAX_RECORDS: usize = 4096;
// Each encoded payload remains below the existing 256 KiB retained-journal
// reader, including its envelope. Overflow is explicit, never truncation.
const MAX_RECORD_BYTES: usize = 128 * 1024;
const MAX_CAPTURE_BYTES: usize = 8 * 1024 * 1024;
fn identifier(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256
}
fn id<'a>(value: &'a Value, field: &str) -> Result<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|s| identifier(s))
        .ok_or_else(|| Error::new("protocol_observation", "missing bounded protocol identity"))
}
struct RequestCapture {
    digest: String,
    open: u32,
}
/// Callback-driven live collector. Its input is adapter data; authority is supplied by
/// the actual ScopedWorker separately, never by constructing this data helper.
pub struct ProtocolOperationBinding {
    pub operation: Uuid,
    pub runtime: Uuid,
    pub workspace: Uuid,
}
pub struct ProtocolCapture {
    report: DeclaredProtocolObservation,
    requests: BTreeMap<String, RequestCapture>,
    calls: BTreeMap<String, String>,
    messages: BTreeMap<(String, String, String), String>,
    delta_items: BTreeSet<(String, String, String)>,
    turns: BTreeMap<String, (String, String, Option<String>)>,
    total_bytes: usize,
    last_request_sequence: u64,
    enabled: bool,
    established: bool,
}
impl ProtocolCapture {
    pub fn new(
        binding: ProtocolOperationBinding,
        scope: String,
        instruction: String,
        trial_digest: Option<String>,
        suite_digest: String,
        fixture_digest: String,
    ) -> Self {
        Self {
            report: DeclaredProtocolObservation {
                schema_version: 1,
                operation: binding.operation,
                runtime: binding.runtime,
                workspace: binding.workspace,
                trial_digest,
                suite_digest,
                fixture_digest,
                contract_scope_digest: scope,
                instruction_digest: instruction,
                session_binding_digest: None,
                thread_id: None,
                state: ProtocolCaptureState::Unavailable,
                issues: BTreeSet::new(),
                tool_request_attempts: 0,
                observed_server_requests: 0,
                processed_server_requests: None,
                terminal_turns: 0,
                transport_end: None,
                records: vec![],
            },
            requests: BTreeMap::new(),
            calls: BTreeMap::new(),
            messages: BTreeMap::new(),
            delta_items: BTreeSet::new(),
            turns: BTreeMap::new(),
            total_bytes: 0,
            last_request_sequence: 0,
            enabled: true,
            established: false,
        }
    }
    pub fn disable(&mut self, issue: ProtocolCaptureIssue) {
        self.enabled = false;
        self.issue(issue);
    }
    pub fn issue(&mut self, issue: ProtocolCaptureIssue) {
        self.report.issues.insert(issue);
    }
    fn bind<'a>(&mut self, state: Option<&'a SessionState>) -> Option<&'a SessionState> {
        if !self.established {
            self.issue(ProtocolCaptureIssue::UnestablishedThread);
            return None;
        }
        let Some(state) = state else {
            self.issue(ProtocolCaptureIssue::MissingSession);
            return None;
        };
        let Ok(digest) = canonical_digest(1, &state.binding) else {
            self.issue(ProtocolCaptureIssue::MalformedMessage);
            return None;
        };
        let Some(thread) = state.thread_id.as_deref().filter(|s| identifier(s)) else {
            self.issue(ProtocolCaptureIssue::MissingSession);
            return None;
        };
        if self
            .report
            .session_binding_digest
            .as_ref()
            .is_some_and(|d| d != &digest)
            || self
                .report
                .thread_id
                .as_deref()
                .is_some_and(|s| s != thread)
        {
            self.issue(ProtocolCaptureIssue::BindingChanged);
            return None;
        }
        self.report.session_binding_digest = Some(digest);
        self.report.thread_id = Some(thread.into());
        Some(state)
    }
    fn record(&mut self, value: Value, write: &mut dyn FnMut(&Value) -> Result<String>) {
        let Ok(bytes) = serde_json::to_vec(&value) else {
            self.issue(ProtocolCaptureIssue::MalformedMessage);
            return;
        };
        if self.report.records.len() >= MAX_RECORDS
            || bytes.len() > MAX_RECORD_BYTES
            || self.total_bytes.saturating_add(bytes.len()) > MAX_CAPTURE_BYTES
        {
            self.issue(ProtocolCaptureIssue::CaptureBound);
            return;
        }
        let result =
            canonical_digest(1, &value).and_then(|digest| write(&value).map(|leaf| (digest, leaf)));
        match result {
            Ok((digest, journal_leaf)) => {
                self.total_bytes += bytes.len();
                self.report.records.push(ProtocolRecordReference {
                    sequence: self.report.records.len() as u32,
                    journal_leaf,
                    digest,
                    bytes: bytes.len() as u64,
                });
            }
            Err(error) if error.code == "observation_limit" => {
                self.issue(ProtocolCaptureIssue::CaptureBound)
            }
            Err(_) => self.issue(ProtocolCaptureIssue::ProtectedStorage),
        }
    }
    fn observe_message(
        &mut self,
        thread: &str,
        turn: &str,
        message: &Value,
        write: &mut dyn FnMut(&Value) -> Result<String>,
    ) {
        let Ok(item) = id(message, "id") else {
            self.issue(ProtocolCaptureIssue::MalformedMessage);
            return;
        };
        let key = (thread.to_owned(), turn.to_owned(), item.to_owned());
        let Ok(digest) = canonical_digest(1, message) else {
            self.issue(ProtocolCaptureIssue::MalformedMessage);
            return;
        };
        if let Some(old) = self.messages.get(&key) {
            if old == &digest {
                return;
            }
            self.issue(ProtocolCaptureIssue::ConflictingDuplicate);
        } else {
            if self.messages.len() >= MAX_RECORDS {
                self.issue(ProtocolCaptureIssue::CaptureBound);
                return;
            }
            self.messages.insert(key, digest);
        }
        self.record(
            json!({"channel":"agent_message","thread":thread,"turn":turn,"message":message}),
            write,
        );
    }
    fn agent_message(&mut self, item: &Value) -> Option<Value> {
        let (Ok(item_id), Ok(kind)) = (id(item, "id"), id(item, "type")) else {
            self.issue(ProtocolCaptureIssue::MalformedMessage);
            return None;
        };
        match kind {
            "agentMessage" => {
                let Some(text) = item.get("text").and_then(Value::as_str) else {
                    self.issue(ProtocolCaptureIssue::MalformedMessage);
                    return None;
                };
                if item
                    .get("questions")
                    .is_some_and(|v| !v.is_null() && v.as_array().is_none_or(|a| !a.is_empty()))
                    || item.get("memoryCitation").is_some_and(|v| !v.is_null())
                    || item.as_object().is_some_and(|o| {
                        o.keys().any(|k| {
                            ![
                                "id",
                                "type",
                                "text",
                                "phase",
                                "delivery",
                                "questions",
                                "memoryCitation",
                            ]
                            .contains(&k.as_str())
                        })
                    })
                {
                    self.issue(ProtocolCaptureIssue::UnsupportedOutputChannel);
                }
                Some(json!({"id":item_id,"text":text}))
            }
            // Input and hidden reasoning are deliberately not copied.
            "userMessage" | "hookPrompt" | "reasoning" => None,
            _ => {
                self.issue(ProtocolCaptureIssue::UnsupportedOutputChannel);
                None
            }
        }
    }
    pub fn observe(
        &mut self,
        event: ProtocolObservation<'_>,
        write: &mut dyn FnMut(&Value) -> Result<String>,
    ) {
        if !self.enabled {
            return;
        }
        match event {
            ProtocolObservation::ThreadEstablished { state } => {
                self.established = true;
                if self.bind(Some(state)).is_none() {
                    self.established = false;
                } else {
                    self.record(json!({"channel":"thread_established","thread":state.thread_id,"binding":self.report.session_binding_digest}), write);
                }
            }
            ProtocolObservation::Issue { issue, .. } => self.issue(issue),
            ProtocolObservation::TransportEnd { state, end } => {
                self.bind(Some(state));
                self.report.transport_end = Some(end.diagnostic().clone());
                self.record(
                    json!({"channel":"transport_end","observation":end.diagnostic()}),
                    write,
                );
            }
            ProtocolObservation::UnboundNotification { method, params } => {
                self.issue(ProtocolCaptureIssue::UnestablishedThread);
                match method {
                    "item/completed" => {
                        if let Some(message) =
                            params.get("item").and_then(|item| self.agent_message(item))
                        {
                            self.record(
                                json!({"channel":"unbound_agent_message","message":message}),
                                write,
                            );
                        }
                    }
                    "turn/completed" => self.observe_unbound_turn(params.get("turn"), write),
                    _ => self.issue(ProtocolCaptureIssue::UnsupportedOutputChannel),
                }
            }
            ProtocolObservation::MatchedResponse {
                method, response, ..
            } => {
                if matches!(method, "turn/start" | "review/start") {
                    self.observe_response(event, write);
                } else if method == "thread/start"
                    && let Some(turns) = response
                        .pointer("/thread/turns")
                        .and_then(Value::as_array)
                        .filter(|turns| !turns.is_empty())
                {
                    self.issue(ProtocolCaptureIssue::UnestablishedThread);
                    if turns.len() > MAX_RECORDS {
                        self.issue(ProtocolCaptureIssue::CaptureBound);
                    } else {
                        for turn in turns {
                            self.observe_unbound_turn(Some(turn), write);
                        }
                    }
                }
            }
            ProtocolObservation::ServerRequest {
                state,
                rpc_id,
                sequence,
                method,
                params,
            } => {
                self.report.observed_server_requests =
                    self.report.observed_server_requests.saturating_add(1);
                if self.last_request_sequence.checked_add(1) != Some(sequence) {
                    self.issue(ProtocolCaptureIssue::MissingRequestInventory);
                }
                self.last_request_sequence = sequence;
                if method == "item/tool/call" {
                    self.report.tool_request_attempts =
                        self.report.tool_request_attempts.saturating_add(1);
                    if self.report.tool_request_attempts as usize > MAX_RECORDS {
                        self.issue(ProtocolCaptureIssue::CaptureBound);
                        return;
                    }
                }
                if !self.established && method == "item/tool/call" {
                    self.issue(ProtocolCaptureIssue::UnestablishedThread);
                    self.record(json!({"channel":"unbound_tool_request","server_sequence":sequence,"parameters":params}),write);
                }
                let Some(state) = self.bind(state) else {
                    return;
                };
                if method != "item/tool/call" {
                    self.issue(ProtocolCaptureIssue::UnsupportedRequestChannel);
                    return;
                }
                let parsed = (|| {
                    let thread = id(params, "threadId")?;
                    let turn = id(params, "turnId")?;
                    let call = id(params, "callId")?;
                    id(params, "tool")?;
                    if params.get("arguments").is_none() {
                        return Err(Error::invalid("missing arguments"));
                    }
                    Ok::<_, Error>((thread, turn, call))
                })();
                let Ok((thread, turn, call)) = parsed else {
                    self.issue(ProtocolCaptureIssue::MalformedMessage);
                    return;
                };
                if state.thread_id.as_deref() != Some(thread)
                    || !(state.turn_id.as_deref() == Some(turn)
                        || state.unresolved_turn.as_deref() == Some(turn)
                        || state.completed_turns.contains_key(turn))
                {
                    self.issue(ProtocolCaptureIssue::ForeignOrUnknownTurn);
                    return;
                }
                let Ok(request_key) = canonical_digest(1, rpc_id) else {
                    self.issue(ProtocolCaptureIssue::MalformedMessage);
                    return;
                };
                let Ok(digest) = canonical_digest(1, params) else {
                    self.issue(ProtocolCaptureIssue::MalformedMessage);
                    return;
                };
                let Ok(call_key) = canonical_digest(1, &(thread, turn, call)) else {
                    self.issue(ProtocolCaptureIssue::MalformedMessage);
                    return;
                };
                if self.calls.get(&call_key).is_some_and(|d| d != &digest)
                    || self
                        .requests
                        .get(&request_key)
                        .is_some_and(|r| r.digest != digest)
                {
                    self.issue(ProtocolCaptureIssue::ConflictingDuplicate);
                }
                self.calls.entry(call_key).or_insert_with(|| digest.clone());
                let entry = self
                    .requests
                    .entry(request_key.clone())
                    .or_insert(RequestCapture { digest, open: 0 });
                entry.open = entry.open.saturating_add(1);
                self.record(json!({"channel":"tool_request","server_sequence":sequence,"request":request_key,"parameters":params,
                    "intervention_blocked":state.intervention.is_some(),"input_required":state.input_required,
                    "active_turn":state.turn_id.as_deref()==Some(turn)}), write);
            }
            ProtocolObservation::ToolResult {
                state,
                rpc_id,
                error_code,
            } => {
                if self.bind(state).is_none() {
                    return;
                }
                let Ok(key) = canonical_digest(1, rpc_id) else {
                    self.issue(ProtocolCaptureIssue::MalformedMessage);
                    return;
                };
                let Some(entry) = self.requests.get_mut(&key) else {
                    self.issue(ProtocolCaptureIssue::UnfinishedRequest);
                    return;
                };
                if entry.open == 0 {
                    self.issue(ProtocolCaptureIssue::ConflictingDuplicate);
                    return;
                }
                entry.open -= 1;
                if error_code.is_some_and(|c| !identifier(c)) {
                    self.issue(ProtocolCaptureIssue::MalformedMessage);
                    return;
                }
                self.record(
                    json!({"channel":"tool_result","request":key,"error_code":error_code}),
                    write,
                );
            }
            ProtocolObservation::AdapterInventory {
                state,
                processed_requests,
            } => {
                if self.bind(Some(state)).is_none() {
                    return;
                }
                if self.report.processed_server_requests.is_some()
                    && self.report.processed_server_requests != processed_requests
                {
                    self.issue(ProtocolCaptureIssue::ConflictingDuplicate);
                }
                self.report.processed_server_requests = processed_requests;
                self.record(json!({"channel":"adapter_request_inventory","processed_requests":processed_requests,"observed_requests":self.report.observed_server_requests,"transport_tail":"count_does_not_prove_tail"}),write);
            }
            ProtocolObservation::CompletedItem {
                state,
                params,
                accepted,
            }
            | ProtocolObservation::AgentMessageDelta {
                state,
                params,
                accepted,
            } => {
                let delta = matches!(event, ProtocolObservation::AgentMessageDelta { .. });
                if !self.established
                    && self.report.session_binding_digest.is_none()
                    && !self
                        .report
                        .issues
                        .contains(&ProtocolCaptureIssue::BindingChanged)
                {
                    self.issue(ProtocolCaptureIssue::UnestablishedThread);
                    if !accepted {
                        self.issue(ProtocolCaptureIssue::RejectedItem);
                    }
                    if delta {
                        self.issue(ProtocolCaptureIssue::UnsupportedOutputChannel);
                    } else if let Some(message) =
                        params.get("item").and_then(|item| self.agent_message(item))
                    {
                        self.record(
                            json!({"channel":"unbound_agent_message","adapter_accepted":accepted,"message":message}),
                            write,
                        );
                    }
                    return;
                }
                if self.bind(Some(state)).is_none() {
                    return;
                }
                if !accepted {
                    self.issue(ProtocolCaptureIssue::RejectedItem);
                }
                let (Ok(thread), Ok(turn)) = (id(params, "threadId"), id(params, "turnId")) else {
                    self.issue(ProtocolCaptureIssue::MalformedMessage);
                    return;
                };
                if state.thread_id.as_deref() != Some(thread)
                    || !(state.turn_id.as_deref() == Some(turn)
                        || state.unresolved_turn.as_deref() == Some(turn)
                        || state.completed_turns.contains_key(turn))
                {
                    self.issue(ProtocolCaptureIssue::ForeignOrUnknownTurn);
                    return;
                }
                if delta {
                    let Ok(item) = id(params, "itemId") else {
                        self.issue(ProtocolCaptureIssue::MalformedMessage);
                        return;
                    };
                    let key = (thread.into(), turn.into(), item.into());
                    if self.delta_items.len() >= MAX_RECORDS && !self.delta_items.contains(&key) {
                        self.issue(ProtocolCaptureIssue::CaptureBound);
                        return;
                    }
                    if state.completed_turns.contains_key(turn) {
                        self.issue(ProtocolCaptureIssue::RejectedItem);
                    }
                    // No delta text is retained or mistaken for complete output.
                    self.delta_items.insert(key);
                } else {
                    let Some(item) = params.get("item") else {
                        self.issue(ProtocolCaptureIssue::MalformedMessage);
                        return;
                    };
                    if let Some(message) = self.agent_message(item) {
                        self.observe_message(thread, turn, &message, write);
                    }
                }
            }
            ProtocolObservation::TerminalTurn {
                state,
                params,
                accepted,
            } => {
                if !self.established
                    && self.report.session_binding_digest.is_none()
                    && !self
                        .report
                        .issues
                        .contains(&ProtocolCaptureIssue::BindingChanged)
                {
                    self.issue(ProtocolCaptureIssue::UnestablishedThread);
                    if !accepted {
                        self.issue(ProtocolCaptureIssue::RejectedTerminal);
                    }
                    self.record(
                        json!({"channel":"unbound_terminal_observation","adapter_accepted":accepted}),
                        write,
                    );
                    self.observe_unbound_turn(params.get("turn"), write);
                    return;
                }
                if self.bind(Some(state)).is_none() {
                    return;
                }
                let Some(turn) = params.get("turn") else {
                    self.issue(ProtocolCaptureIssue::MalformedMessage);
                    return;
                };
                let Ok(thread) = id(params, "threadId") else {
                    self.issue(ProtocolCaptureIssue::MalformedMessage);
                    return;
                };
                let Ok(turn_id) = id(turn, "id") else {
                    self.issue(ProtocolCaptureIssue::MalformedMessage);
                    return;
                };
                if state.thread_id.as_deref() != Some(thread)
                    || !(state.completed_turns.contains_key(turn_id)
                        || state.turn_id.as_deref() == Some(turn_id)
                        || state.unresolved_turn.as_deref() == Some(turn_id))
                {
                    self.issue(ProtocolCaptureIssue::ForeignOrUnknownTurn);
                    return;
                }
                if !accepted {
                    self.issue(ProtocolCaptureIssue::RejectedTerminal);
                }
                if !matches!(
                    turn.get("itemsView").and_then(Value::as_str),
                    None | Some("full")
                ) || turn.get("itemsView").is_some_and(|v| !v.is_string())
                {
                    self.issue(ProtocolCaptureIssue::MissingFullTurn);
                    return;
                }
                let Some(items) = turn.get("items").and_then(Value::as_array) else {
                    self.issue(ProtocolCaptureIssue::MissingFullTurn);
                    return;
                };
                if items.len() > MAX_RECORDS
                    || self.turns.len() >= MAX_RECORDS && !self.turns.contains_key(turn_id)
                {
                    self.issue(ProtocolCaptureIssue::CaptureBound);
                    return;
                }
                let mut seen = BTreeSet::new();
                let mut messages = vec![];
                for item in items {
                    let (Ok(item_id), Ok(kind)) = (id(item, "id"), id(item, "type")) else {
                        self.issue(ProtocolCaptureIssue::MalformedMessage);
                        continue;
                    };
                    if !seen.insert(item_id) {
                        self.issue(ProtocolCaptureIssue::ConflictingDuplicate);
                    }
                    let _ = kind;
                    if let Some(message) = self.agent_message(item) {
                        messages.push(message);
                    }
                }
                let Ok(status) = id(turn, "status") else {
                    self.issue(ProtocolCaptureIssue::MalformedMessage);
                    return;
                };
                let terminal_messages: BTreeMap<_, _> = messages
                    .iter()
                    .filter_map(|message| {
                        Some((
                            id(message, "id").ok()?.to_owned(),
                            canonical_digest(1, message).ok()?,
                        ))
                    })
                    .collect();
                let Ok(digest) = canonical_digest(1, &(status, &terminal_messages)) else {
                    self.issue(ProtocolCaptureIssue::MalformedMessage);
                    return;
                };
                let duplicate = if let Some((old_status, old_digest, _)) = self.turns.get(turn_id) {
                    if old_status == status && old_digest == &digest {
                        return;
                    }
                    self.issue(ProtocolCaptureIssue::ConflictingDuplicate);
                    true
                } else {
                    false
                };
                let history_digest = state
                    .provider_history
                    .as_ref()
                    .and_then(|h| h.observed_turn(turn_id))
                    .filter(|(s, _)| *s == status)
                    .and_then(|(_, d)| d)
                    .map(str::to_owned);
                if history_digest.is_none() {
                    self.issue(ProtocolCaptureIssue::MissingFullTurn);
                }
                if !duplicate {
                    self.turns.insert(
                        turn_id.into(),
                        (status.into(), digest.clone(), history_digest.clone()),
                    );
                }
                self.report.terminal_turns = self.turns.len() as u32;

                if self.messages.iter().any(|((t, u, item), digest)| {
                    t == thread && u == turn_id && terminal_messages.get(item) != Some(digest)
                }) || self.delta_items.iter().any(|(t, u, item)| {
                    t == thread && u == turn_id && !terminal_messages.contains_key(item)
                }) {
                    self.issue(ProtocolCaptureIssue::ConflictingDuplicate);
                }
                for message in messages {
                    self.observe_message(thread, turn_id, &message, write);
                }
                self.record(json!({"channel":"terminal_turn","thread":thread,"turn":turn_id,"status":status,"declared_message_digest":digest,"terminal_history_digest":history_digest,"adapter_accepted":accepted}),write);
            }
        }
    }
    fn observe_unbound_turn(
        &mut self,
        turn: Option<&Value>,
        write: &mut dyn FnMut(&Value) -> Result<String>,
    ) {
        let Some(turn) = turn else {
            self.issue(ProtocolCaptureIssue::MalformedMessage);
            return;
        };
        if !matches!(
            turn.get("itemsView").and_then(Value::as_str),
            None | Some("full")
        ) || turn
            .get("itemsView")
            .is_some_and(|value| !value.is_string())
        {
            self.issue(ProtocolCaptureIssue::MissingFullTurn);
            return;
        }
        let Some(items) = turn.get("items").and_then(Value::as_array) else {
            self.issue(ProtocolCaptureIssue::MissingFullTurn);
            return;
        };
        if items.len() > MAX_RECORDS {
            self.issue(ProtocolCaptureIssue::CaptureBound);
            return;
        }
        for item in items {
            if let Some(message) = self.agent_message(item) {
                self.record(
                    json!({"channel":"unbound_agent_message","message":message}),
                    write,
                );
            }
        }
    }
    fn observe_response(
        &mut self,
        event: ProtocolObservation<'_>,
        write: &mut dyn FnMut(&Value) -> Result<String>,
    ) {
        let ProtocolObservation::MatchedResponse {
            state,
            rpc_id,
            method,
            request_thread,
            inline_review,
            response,
        } = event
        else {
            return;
        };
        let Some(state) = self.bind(state) else {
            return;
        };
        let Some(thread) = state.thread_id.as_deref() else {
            self.issue(ProtocolCaptureIssue::MissingSession);
            return;
        };
        if request_thread != Some(thread)
            || (method == "review/start"
                && (!inline_review
                    || response.get("reviewThreadId").and_then(Value::as_str) != Some(thread)))
        {
            self.issue(ProtocolCaptureIssue::ForeignOrUnknownTurn);
            return;
        }
        let Some(turn) = response.get("turn") else {
            self.issue(ProtocolCaptureIssue::MalformedMessage);
            return;
        };
        let Ok(turn_id) = id(turn, "id") else {
            self.issue(ProtocolCaptureIssue::MalformedMessage);
            return;
        };
        if state.turn_id.as_deref() != Some(turn_id) && !state.completed_turns.contains_key(turn_id)
        {
            self.issue(ProtocolCaptureIssue::UnconfirmedResponseTurn);
        }
        if !matches!(
            turn.get("itemsView").and_then(Value::as_str),
            None | Some("full")
        ) || turn
            .get("itemsView")
            .is_some_and(|value| !value.is_string())
        {
            self.issue(ProtocolCaptureIssue::MissingFullTurn);
            return;
        }
        let Some(items) = turn.get("items").and_then(Value::as_array) else {
            self.issue(ProtocolCaptureIssue::MissingFullTurn);
            return;
        };
        if items.len() > MAX_RECORDS {
            self.issue(ProtocolCaptureIssue::CaptureBound);
            return;
        }
        let mut seen = BTreeSet::new();
        for item in items {
            if let Ok(item_id) = id(item, "id")
                && !seen.insert(item_id)
            {
                self.issue(ProtocolCaptureIssue::ConflictingDuplicate);
            }
            if let Some(message) = self.agent_message(item) {
                self.observe_message(thread, turn_id, &message, write);
            }
        }
        self.record(json!({"channel":"matched_turn_response","request":canonical_digest(1,rpc_id).ok(),"method":method,"thread":thread,"turn":turn_id}),write);
    }
    pub fn finish(
        &self,
        state: Option<&SessionState>,
        pending: Option<&str>,
        read: &dyn Fn(&str) -> Result<Value>,
    ) -> DeclaredProtocolObservation {
        let mut report = self.report.clone();
        if !self.enabled {
            report.state = if report
                .issues
                .contains(&ProtocolCaptureIssue::BindingChanged)
                || report
                    .issues
                    .contains(&ProtocolCaptureIssue::ConflictingDuplicate)
            {
                ProtocolCaptureState::Conflict
            } else if report
                .issues
                .contains(&ProtocolCaptureIssue::ScopeNotDeclared)
            {
                ProtocolCaptureState::NotDeclared
            } else {
                ProtocolCaptureState::Unavailable
            };
            return report;
        }
        report
            .issues
            .insert(ProtocolCaptureIssue::CoverageNotQualified);
        if !matches!(
            report.transport_end,
            Some(ProtocolTransportEndDiagnostic::CleanEof { .. })
        ) {
            report
                .issues
                .insert(ProtocolCaptureIssue::UnobservedTransportTail);
        }
        if report.processed_server_requests != Some(report.observed_server_requests)
            || report.processed_server_requests != Some(self.last_request_sequence)
        {
            report
                .issues
                .insert(ProtocolCaptureIssue::MissingRequestInventory);
        }
        let complete = state.is_some_and(|s| {
            s.thread_id == report.thread_id
                && canonical_digest(1, &s.binding).ok() == report.session_binding_digest
                && s.turn_id.is_none()
                && s.unresolved_turn.is_none()
                && pending.is_none()
                && !s.completed_turns.is_empty()
                && s.turns_started as usize == s.completed_turns.len()
                && s.completed_turns.len() == self.turns.len()
                && s.completed_turns.iter().all(|(id, status)| {
                    self.turns.get(id).is_some_and(|(old, declared, digest)| {
                        let messages: BTreeMap<_, _> = self
                            .messages
                            .iter()
                            .filter(|((thread, turn, _), _)| {
                                Some(thread.as_str()) == s.thread_id.as_deref() && turn == id
                            })
                            .map(|((_, _, item), digest)| (item.clone(), digest.clone()))
                            .collect();
                        old == status
                            && canonical_digest(1, &(status, &messages)).ok().as_ref()
                                == Some(declared)
                            && digest.is_some()
                            && s.provider_history
                                .as_ref()
                                .and_then(|h| h.observed_turn(id))
                                .is_some_and(|(actual, d)| {
                                    actual == status && d == digest.as_deref()
                                })
                    })
                })
        });
        if !complete
            || self
                .delta_items
                .iter()
                .any(|key| !self.messages.contains_key(key))
        {
            report
                .issues
                .insert(ProtocolCaptureIssue::IncompleteTurnInventory);
        }
        if self.requests.values().any(|r| r.open != 0) {
            report
                .issues
                .insert(ProtocolCaptureIssue::UnfinishedRequest);
        }
        for record in &report.records {
            let valid = read(&record.journal_leaf).ok().is_some_and(|v| {
                v.get("kind").and_then(Value::as_str) == Some("protocol_observation")
                    && v.get("record").is_some_and(|value| {
                        canonical_digest(1, value).ok().as_ref() == Some(&record.digest)
                            && serde_json::to_vec(value)
                                .is_ok_and(|b| b.len() as u64 == record.bytes)
                    })
            });
            if !valid {
                report.issues.insert(ProtocolCaptureIssue::ProtectedStorage);
            }
        }
        use ProtocolCaptureIssue as I;
        report.state = if report.issues.iter().any(|i| {
            matches!(
                i,
                I::BindingChanged
                    | I::ForeignOrUnknownTurn
                    | I::ConflictingDuplicate
                    | I::RejectedTerminal
                    | I::RejectedItem
            )
        }) {
            ProtocolCaptureState::Conflict
        } else if report.issues.contains(&I::CaptureBound) {
            ProtocolCaptureState::Overflow
        } else if report.session_binding_digest.is_none()
            || report.issues.contains(&I::ProtectedStorage)
            || report.issues.contains(&I::MissingSession)
        {
            ProtocolCaptureState::Unavailable
        } else {
            ProtocolCaptureState::Incomplete
        };
        report
    }
}

#[cfg(test)]
#[path = "observation_tests.rs"]
mod tests;
