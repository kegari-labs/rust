//! Stream RPC client and protocol policy; native ownership stays with the host.
use super::*;

/// Trusted-host revalidation callback, not a serialized grant or native proof.
/// Production host admission continues to require its concrete validated scope.
pub trait SessionScope: Sync {
    fn revalidate(&self) -> Result<()>;
    fn cwd(&self) -> &Path;
    fn isolation_digest(&self) -> &str;
}

pub trait ToolBroker: Send {
    /// Native repository tools execute inside Codex. Record their structured
    /// observations before subsequent workflow-tool calls; this is not a launch
    /// callback or a per-command authorization gate.
    fn observe_execution(
        &mut self,
        _state: &SessionState,
        _event: &NativeExecutionObservation,
    ) -> Result<()> {
        Ok(())
    }
    /// Diagnostic callback before filtering or after terminal validation. No
    /// error return: capture failure cannot change dispatch or erase its hold.
    fn observe_protocol(&mut self, _event: ProtocolObservation<'_>) {}
    /// Adapter-owned current state before a bound tool request, including tools
    /// interleaved before a synchronous start/steer response arrives.
    fn observe_session(&mut self, _state: &SessionState) -> Result<()> {
        Ok(())
    }
    fn call<'a>(
        &'a mut self,
        call: &'a ToolCall,
    ) -> Pin<Box<dyn Future<Output = Result<Value>> + Send + 'a>>;
}
pub(super) fn checkpoint_notification(
    state: &mut SessionState,
    broker: &mut dyn ToolBroker,
    method: &str,
    params: &Value,
) -> Result<SessionEvent> {
    let result = state.notification(method, params);
    match method {
        "turn/completed" => broker.observe_protocol(ProtocolObservation::TerminalTurn {
            state,
            params,
            accepted: result.is_ok(),
        }),
        "item/completed" => broker.observe_protocol(ProtocolObservation::CompletedItem {
            state,
            params,
            accepted: result.is_ok(),
        }),
        "item/agentMessage/delta" => {
            broker.observe_protocol(ProtocolObservation::AgentMessageDelta {
                state,
                params,
                accepted: result.is_ok(),
            })
        }
        _ => (),
    }
    // A protocol inconsistency cannot hide a policy signal already observed
    // from the bound provider thread. Persist that hold before returning either
    // the ordinary event or its validation error.
    if result.is_ok() || state.intervention.is_some() {
        broker.observe_session(state)?;
    }
    if result.is_ok()
        && let Some(event) = NativeExecutionObservation::from_notification(state, method, params)?
    {
        broker.observe_execution(state, &event)?;
    }
    result
}
fn checkpoint_response_policy(
    state: &mut SessionState,
    broker: &mut dyn ToolBroker,
    method: &str,
    request_thread: Option<&str>,
    inline_review: bool,
    response: &Value,
) -> Result<()> {
    if !matches!(method, "turn/start" | "review/start") {
        return Ok(());
    }
    let thread = state
        .thread_id
        .as_deref()
        .ok_or_else(|| Error::new("protocol_error", "turn response has no bound thread"))?;
    if request_thread != Some(thread) {
        return Err(Error::new(
            "identity_mismatch",
            "turn response request belongs to another thread",
        ));
    }
    if method == "review/start"
        && (!inline_review
            || response.get("reviewThreadId").and_then(Value::as_str) != Some(thread))
    {
        return Err(Error::new(
            "identity_mismatch",
            "review response does not identify the bound inline thread",
        ));
    }
    let turn = response
        .get("turn")
        .ok_or_else(|| Error::new("protocol_error", "turn response lacks its turn identity"))?;
    required_str(turn, "id")?;
    // The matched RPC and exact thread/turn identity are sufficient for a
    // policy observation. Malformed ordinary Turn fields or later ownership
    // bookkeeping must not erase an authenticated provider hold.
    let result = state.observe_turn_policy(method, turn);
    if state.intervention.is_some() {
        broker.observe_session(state)?;
    }
    result
}
pub struct DenyTools;
impl ToolBroker for DenyTools {
    fn call<'a>(
        &'a mut self,
        _: &'a ToolCall,
    ) -> Pin<Box<dyn Future<Output = Result<Value>> + Send + 'a>> {
        Box::pin(async {
            Err(Error::new(
                "unsupported_tool",
                "tool is not advertised for this phase",
            ))
        })
    }
}

/// Production worker request boundary. Only the bounded pipe write is performed
/// inside dispatch; provider responses and dynamic tools are handled afterward.
/// Read-only metadata and owned cancellation do not enter this mutation hook.
pub trait RequestGate: Send + Sync {
    fn dispatch(
        &self,
        id: u64,
        method: &str,
        frame: &Value,
        transmit: &mut dyn FnMut() -> Result<()>,
    ) -> Result<()>;
    fn observed(&self, id: u64, method: &str, result: &Result<Value>) -> Result<()>;
    fn uncertain(&self, id: u64, method: &str) -> Result<()>;
}
fn changes_session(method: &str) -> bool {
    matches!(
        method,
        "thread/start"
            | "thread/resume"
            | "turn/start"
            | "turn/steer"
            | "review/start"
            | "command/exec"
    )
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandExecResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}
pub struct RpcClient {
    pub(super) transport: RpcTransport,
    pub(super) events: VecDeque<SessionEvent>,
    pub(super) next_id: u64,
    pub(super) server_request_count: u64,
    pub(super) tail_observation_started: bool,
    pub(super) request_timeout: Duration,
    pub(super) expected_home: Option<PathBuf>,
    pub(super) initialized: bool,
    pub(super) startup_error: Option<Error>,
    pub(super) uncertain_request: Option<u64>,
    pub(super) allowed_tools: BTreeSet<String>,
    pub(super) account_id: Option<String>,
    pub(super) models: Option<Vec<Value>>,
    pub(super) verified_permissions: Option<String>,
    pub(super) pending_reconciliation: Option<String>,
    pub(super) request_gate: Option<std::sync::Arc<dyn RequestGate>>,
    last_semantic_event: Instant,
    active_command: Option<String>,
}
impl RpcClient {
    pub fn from_transport<R, W>(reader: R, writer: W, request_timeout: Duration) -> Result<Self>
    where
        R: AsyncRead + Send + 'static,
        W: AsyncWrite + Send + 'static,
    {
        if request_timeout.is_zero() {
            return Err(Error::invalid("request timeout must be positive"));
        }
        Ok(Self {
            transport: RpcTransport::new(reader, writer),
            events: VecDeque::new(),
            next_id: 1,
            server_request_count: 0,
            tail_observation_started: false,
            request_timeout,
            expected_home: None,
            initialized: false,
            startup_error: None,
            uncertain_request: None,
            allowed_tools: BTreeSet::new(),
            account_id: None,
            models: None,
            verified_permissions: None,
            active_command: None,
            pending_reconciliation: None,
            request_gate: None,
            last_semantic_event: Instant::now(),
        })
    }
    /// Attach plain expected-home/startup context to caller-supplied streams.
    /// This constructor performs no native launch, adoption or authorization.
    pub fn from_host_transport<R, W>(
        reader: R,
        writer: W,
        request_timeout: Duration,
        expected_home: PathBuf,
        startup_error: Option<Error>,
    ) -> Result<Self>
    where
        R: AsyncRead + Send + 'static,
        W: AsyncWrite + Send + 'static,
    {
        let mut client = Self::from_transport(reader, writer, request_timeout)?;
        client.expected_home = Some(expected_home);
        client.startup_error = startup_error;
        Ok(client)
    }
    pub fn bind_request_gate(&mut self, gate: std::sync::Arc<dyn RequestGate>) -> Result<()> {
        if self.request_gate.is_some() || !self.allowed_tools.is_empty() {
            return Err(Error::new(
                "ownership_conflict",
                "provider mutation gate must be bound once before a worker thread",
            ));
        }
        self.request_gate = Some(gate);
        Ok(())
    }
    async fn dispatch_request(&mut self, id: u64, method: &str, params: Value) -> Result<()> {
        let frame = json!({"id":id,"method":method,"params":params});
        if let Some(gate) = self
            .request_gate
            .clone()
            .filter(|_| changes_session(method))
        {
            let handle = tokio::runtime::Handle::current();
            if handle.runtime_flavor() != tokio::runtime::RuntimeFlavor::MultiThread {
                return Err(Error::new(
                    "prerequisite",
                    "guarded provider dispatch requires the multithread host runtime",
                ));
            }
            let bound = self.request_timeout.min(Duration::from_secs(5));
            tokio::task::block_in_place(|| {
                gate.dispatch(id, method, &frame, &mut || {
                    handle.block_on(async {
                        tokio::time::timeout(bound, self.send(&frame))
                            .await
                            .map_err(|_| {
                                Error::new("timeout", "provider pipe dispatch exceeded its bound")
                            })?
                    })
                })
            })
        } else {
            self.send(&frame).await
        }
    }

    pub(super) async fn send(&mut self, value: &Value) -> Result<()> {
        if let Some(error) = &self.startup_error {
            return Err(error.clone());
        }
        self.transport.send(value).await
    }
    /// Last complete valid protocol output, retained across cancelled reads.
    pub fn last_valid_output(&self) -> Instant {
        self.transport.last_valid_output()
    }
    /// Last semantic session progress, independently of valid transport output.
    pub fn last_semantic_event(&self) -> Instant {
        self.last_semantic_event
    }
    pub(super) async fn read_message(&mut self) -> Result<RpcMessage> {
        match self.read_transport().await? {
            TransportRead::Message(message) => Ok(message),
            TransportRead::CleanEof => Err(Error::new(
                "process_exit",
                "app-server exited; no terminal work result is implied",
            )),
        }
    }
    async fn read_transport(&mut self) -> Result<TransportRead> {
        self.read_transport_bounded(None).await
    }
    pub(super) async fn read_transport_bounded(
        &mut self,
        new_byte_bound: Option<u64>,
    ) -> Result<TransportRead> {
        self.transport.read(new_byte_bound).await
    }
    fn emit(&mut self, event: SessionEvent) -> Result<()> {
        if self.events.len() >= MAX_PENDING_EVENTS {
            return Err(Error::new(
                "event_backpressure",
                "consumer did not drain bounded provider events",
            ));
        }
        if matches!(
            event.kind,
            EventKind::TurnStarted
                | EventKind::TurnCompleted
                | EventKind::TurnFailed
                | EventKind::TurnCancelled
                | EventKind::ToolResult
                | EventKind::ProviderIntervention
                | EventKind::InputRequired
        ) {
            self.last_semantic_event = Instant::now();
        }
        self.events.push_back(event);
        Ok(())
    }
    /// Count of processed server requests only; not stdout EOF/tail evidence.
    pub fn observed_server_request_count(&self) -> u64 {
        self.server_request_count
    }
    pub fn drain_events(&mut self) -> Vec<SessionEvent> {
        self.events.drain(..).collect()
    }
    async fn service_request(
        &mut self,
        id: Value,
        method: &str,
        params: Value,
        mut state: Option<&mut SessionState>,
        broker: &mut dyn ToolBroker,
    ) -> Result<()> {
        self.server_request_count = self
            .server_request_count
            .checked_add(1)
            .ok_or_else(|| Error::new("protocol_error", "server request count overflow"))?;
        broker.observe_protocol(ProtocolObservation::ServerRequest {
            state: state.as_deref(),
            rpc_id: &id,
            sequence: self.server_request_count,
            method,
            params: &params,
        });
        let thread = params
            .get("threadId")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let turn = params
            .get("turnId")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let event_kind;
        let result = match method {
            "item/tool/call" => {
                let call = ToolCall {
                    thread_id: required_str(&params, "threadId")?.into(),
                    turn_id: required_str(&params, "turnId")?.into(),
                    call_id: required_str(&params, "callId")?.into(),
                    name: required_str(&params, "tool")?.into(),
                    arguments: params.get("arguments").cloned().ok_or_else(|| {
                        Error::new("protocol_error", "tool call has no arguments")
                    })?,
                };
                let bound = state.as_ref().is_some_and(|s| {
                    s.thread_id.as_deref() == Some(&call.thread_id)
                        && s.turn_id.as_deref() == Some(&call.turn_id)
                        && s.intervention.is_none()
                        && !s.input_required
                });
                let tool_result = if bound && self.allowed_tools.contains(&call.name) {
                    broker.observe_session(state.as_deref().expect("checked bound session"))?;
                    broker.call(&call).await
                } else {
                    Err(Error::new(
                        "unsupported_tool",
                        "tool is unavailable for this bound role, phase or turn",
                    ))
                };
                broker.observe_protocol(ProtocolObservation::ToolResult {
                    state: state.as_deref(),
                    rpc_id: &id,
                    error_code: tool_result.as_ref().err().map(|e| e.code.as_str()),
                });
                event_kind = if tool_result.is_ok() {
                    EventKind::ToolResult
                } else {
                    EventKind::UnsupportedTool
                };
                let success = tool_result.is_ok();
                let value = match tool_result {
                    Ok(v) => v,
                    Err(e) => json!({"error":e}),
                };
                json!({"contentItems":[{"type":"inputText","text":serde_json::to_string(&value).map_err(|_|Error::new("protocol_error","cannot encode tool result"))?}],"success":success})
            }
            "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
                if let Some(s) = state.as_mut() {
                    s.input_required = true;
                }
                event_kind = EventKind::InputRequired;
                json!({"decision":"cancel"})
            }
            "execCommandApproval" | "applyPatchApproval" => {
                if let Some(s) = state.as_mut() {
                    s.input_required = true;
                }
                event_kind = EventKind::InputRequired;
                json!({"decision":"abort"})
            }
            "item/permissions/requestApproval" => {
                if let Some(s) = state.as_mut() {
                    s.input_required = true;
                }
                event_kind = EventKind::InputRequired;
                json!({"permissions":{},"scope":"turn"})
            }
            "item/tool/requestUserInput" => {
                if let Some(s) = state.as_mut() {
                    s.input_required = true;
                }
                event_kind = EventKind::InputRequired;
                json!({"answers":{}})
            }
            "mcpServer/elicitation/request" => {
                if let Some(s) = state.as_mut() {
                    s.input_required = true;
                }
                event_kind = EventKind::InputRequired;
                json!({"action":"cancel"})
            }
            _ => {
                self.send(&json!({"id":id,"error":{"code":-32601,"message":"unsupported server request"}})).await?;
                self.emit(SessionEvent::new(
                    EventKind::UnsupportedTool,
                    method,
                    thread,
                    turn,
                    Value::Null,
                ))?;
                return Ok(());
            }
        };
        if let Some(current) = state.as_deref() {
            broker.observe_session(current)?;
        }
        self.send(&json!({"id":id,"result":result})).await?;
        self.emit(SessionEvent::new(
            event_kind,
            method,
            thread,
            turn,
            json!({"request_id":id}),
        ))
    }
    pub async fn request(
        &mut self,
        method: &str,
        params: Value,
        state: Option<&mut SessionState>,
        broker: &mut dyn ToolBroker,
    ) -> Result<Value> {
        self.request_bounded(method, params, state, broker, self.request_timeout)
            .await
    }
    async fn request_bounded(
        &mut self,
        method: &str,
        params: Value,
        mut state: Option<&mut SessionState>,
        broker: &mut dyn ToolBroker,
        timeout: Duration,
    ) -> Result<Value> {
        if let Some(id) = self.uncertain_request {
            return Err(Error::new("operation_uncertain", "transport has an unanswered request; reconnect and reconcile before further requests").detail(json!({"request_id":id})));
        }
        if !self.initialized && method != "initialize" {
            return Err(Error::new(
                "protocol_error",
                "initialize handshake required",
            ));
        }
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| Error::new("protocol_error", "request ID exhausted"))?;
        let deadline = Instant::now() + timeout;
        let request_thread = params
            .get("threadId")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let inline_review = params.get("delivery").and_then(Value::as_str) == Some("inline");
        let submitted = if let Some(current) = state.as_deref_mut()
            && matches!(method, "turn/start" | "turn/steer" | "review/start")
            && let Some(submissions) = &mut current.submissions
        {
            submissions.prepare(id, method, &params)?;
            // Persist the exact input/frame provenance before any pipe write.
            broker.observe_session(current)?;
            true
        } else {
            false
        };
        let result = timeout_at(deadline, async {
            self.dispatch_request(id, method, params).await?;
            loop {
                match self.read_message().await? {
                    RpcMessage::Response {
                        id: response,
                        result,
                    } if response == id => {
                        broker.observe_protocol(ProtocolObservation::MatchedResponse {
                            state: state.as_deref(),
                            rpc_id: &response,
                            method,
                            request_thread: request_thread.as_deref(),
                            inline_review,
                            response: &result,
                        });
                        return Ok(result);
                    }
                    RpcMessage::Failure {
                        id: response,
                        error,
                    } if response == id => {
                        return Err(
                            Error::new("provider_response", "app-server rejected request").detail(
                                json!({"request_id":id,"method":method,"rpc_code":error["code"]}),
                            ),
                        );
                    }
                    RpcMessage::Response { .. } | RpcMessage::Failure { .. } => {
                        return Err(Error::new(
                            "protocol_error",
                            "response ID does not match outstanding request",
                        ));
                    }
                    RpcMessage::Notification { method, params } => {
                        let event = if let Some(s) = state.as_deref_mut() {
                            checkpoint_notification(s, broker, &method, &params)?
                        } else {
                            broker.observe_protocol(ProtocolObservation::UnboundNotification {
                                method: &method,
                                params: &params,
                            });
                            SessionEvent::new(
                                EventKind::Notification,
                                &method,
                                None,
                                None,
                                Value::Null,
                            )
                        };
                        self.emit(event)?;
                    }
                    RpcMessage::Request { id, method, params } => {
                        self.service_request(id, &method, params, state.as_deref_mut(), broker)
                            .await?
                    }
                }
            }
        })
        .await;
        let response_policy = match (&result, state.as_deref_mut()) {
            (Ok(Ok(response)), Some(current)) => checkpoint_response_policy(
                current,
                broker,
                method,
                request_thread.as_deref(),
                inline_review,
                response,
            ),
            _ => Ok(()),
        };
        let gate = self
            .request_gate
            .clone()
            .filter(|_| changes_session(method));
        if let Some(gate) = gate {
            match &result {
                Ok(observed)
                    if observed.is_ok()
                        || observed
                            .as_ref()
                            .is_err_and(|e| e.code == "provider_response") =>
                {
                    gate.observed(id, method, observed)?
                }
                _ => gate.uncertain(id, method)?,
            }
        }
        if let Err(error) = response_policy {
            self.uncertain_request = Some(id);
            return Err(error);
        }
        match result {
            Ok(result) => {
                if submitted && let Some(current) = state {
                    if let Ok(response) = &result {
                        let acknowledged = if method == "turn/steer" {
                            response.get("turnId").and_then(Value::as_str)
                        } else {
                            response.pointer("/turn/id").and_then(Value::as_str)
                        };
                        if let Some(turn) = acknowledged
                            && let Err(error) = current.require_new_turn_identity(turn)
                        {
                            self.uncertain_request = Some(id);
                            broker.observe_session(current)?;
                            return Err(error);
                        }
                    }
                    current
                        .submissions
                        .as_mut()
                        .ok_or_else(|| {
                            Error::new("protocol_error", "submission inventory disappeared")
                        })?
                        .observe_response(id, &result)?;
                    broker.observe_session(current)?;
                }
                if result
                    .as_ref()
                    .is_err_and(|e| e.code != "provider_response")
                {
                    self.uncertain_request = Some(id);
                }
                result
            }
            Err(_) => {
                self.uncertain_request = Some(id);
                Err(Error::new("timeout", "app-server request outcome may be unknown; reconcile using the request/session identity").detail(json!({"request_id":id,"method":method,"outcome":"uncertain"})))
            }
        }
    }
    /// Execute an explicit lifecycle command through the pinned server sandbox.
    /// This has no process authority; hosts retain admission and lifecycle policy.
    pub async fn execute_command(
        &mut self,
        scope: &dyn SessionScope,
        permissions: &PermissionConfiguration,
        command: &[String],
        process_id: &str,
        timeout: Duration,
        output_limit: usize,
    ) -> Result<CommandExecResult> {
        scope.revalidate()?;
        if self.verified_permissions.as_deref() != Some(&permissions.isolation_digest)
            || scope.isolation_digest() != permissions.isolation_digest
            || permissions.settings.pointer("/features/shell_tool") != Some(&json!(true))
            || self.account_id.is_none()
        {
            return Err(Error::new(
                "isolation_unverified",
                "lifecycle command requires authenticated, verified executable permissions",
            ));
        }
        if command.is_empty()
            || command.iter().any(|arg| arg.contains('\0'))
            || process_id.is_empty()
            || process_id.len() > 256
            || process_id.chars().any(char::is_control)
            || timeout.is_zero()
            || timeout > Duration::from_secs(86_400)
            || output_limit == 0
            || output_limit > 1024 * 1024
            || self.active_command.is_some()
        {
            return Err(Error::invalid(
                "lifecycle command needs an unused identity and finite command bounds",
            ));
        }
        let mut environment = permissions
            .settings
            .pointer("/shell_environment_policy/set")
            .and_then(Value::as_object)
            .cloned()
            .ok_or_else(|| {
                Error::new(
                    "isolation_unverified",
                    "lifecycle environment was not explicitly verified",
                )
            })?;
        if environment.values().any(|value| !value.is_string()) {
            return Err(Error::new(
                "isolation_unverified",
                "invalid verified lifecycle environment",
            ));
        }
        // The app-server's own auth home is not repository command authority.
        // Supported null overrides explicitly remove its provider-only variable.
        environment.insert("CODEX_HOME".into(), Value::Null);
        self.active_command = Some(process_id.into());
        let response = self
            .request_bounded(
                "command/exec",
                json!({"command":command,"processId":process_id,"cwd":scope.cwd(),
                "permissionProfile":permissions.profile_id,"timeoutMs":timeout.as_millis() as u64,
                "outputBytesCap":output_limit,"disableTimeout":false,"disableOutputCap":false,
                "tty":false,"streamStdin":false,"streamStdoutStderr":false,"env":environment}),
                None,
                &mut DenyTools,
                timeout + Duration::from_secs(5),
            )
            .await;
        match response {
            Ok(value) => {
                let parsed = serde_json::from_value::<CommandExecResult>(value).map_err(|_| {
                    Error::new(
                        "provider_response",
                        "command/exec omitted its actual exit/output result",
                    )
                })?;
                if parsed.stdout.len() > output_limit || parsed.stderr.len() > output_limit {
                    return Err(Error::new(
                        "response_limit",
                        "command/exec output exceeds its declared bound",
                    ));
                }
                self.active_command = None;
                Ok(parsed)
            }
            Err(error) => {
                // This cleanup cannot turn an unknown command into success or
                // clear the existing request uncertainty/admission fence.
                let _ = self.terminate_command().await;
                Err(error)
            }
        }
    }
    /// Best-effort supported cleanup on this connection. The acknowledgement is
    /// not evidence that a hook succeeded or that a native process has exited.
    pub async fn terminate_command(&mut self) -> Result<()> {
        let Some(process_id) = self.active_command.clone() else {
            return Ok(());
        };
        let id = self.next_id;
        self.next_id = id
            .checked_add(1)
            .ok_or_else(|| Error::new("protocol_error", "request ID exhausted"))?;
        timeout_at(Instant::now() + Duration::from_secs(2), async {
            self.send(&json!({"id":id,"method":"command/exec/terminate","params":{"processId":process_id}})).await?;
            loop {
                match self.read_message().await? {
                    RpcMessage::Response { id: response, .. } if response == id => return Ok(()),
                    RpcMessage::Failure { id: response, .. } if response == id => return Err(Error::new("provider_response", "Codex refused lifecycle termination")),
                    // The original command can finish concurrently. Its late
                    // response is not adopted as a successful hook observation.
                    RpcMessage::Response { .. } | RpcMessage::Failure { .. } | RpcMessage::Notification { .. } => {},
                    RpcMessage::Request { .. } => return Err(Error::new("protocol_error", "unexpected request during lifecycle cleanup")),
                }
            }
        }).await.map_err(|_| Error::new("timeout", "Codex lifecycle cleanup acknowledgement timed out"))?
    }
    pub async fn initialize(&mut self) -> Result<Value> {
        self.initialize_observed(&mut DenyTools).await
    }
    pub async fn initialize_observed(&mut self, observer: &mut dyn ToolBroker) -> Result<Value> {
        let mut denied = ObservationOnlyDeny::new(observer);
        if self.initialized {
            return Err(Error::new(
                "protocol_error",
                "initialize may run once per transport",
            ));
        }
        let response=self.request("initialize",json!({"clientInfo":{"name":"kegari-v0","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}}),None,&mut denied).await?;
        required_str(&response, "userAgent")?;
        required_str(&response, "platformFamily")?;
        required_str(&response, "platformOs")?;
        let home = required_str(&response, "codexHome")?;
        if self
            .expected_home
            .as_ref()
            .is_some_and(|expected| Path::new(home) != expected)
        {
            return Err(Error::new(
                "identity_mismatch",
                "app-server used another Codex auth home",
            ));
        }
        self.send(&json!({"method":"initialized"})).await?;
        self.initialized = true;
        Ok(response)
    }
    pub async fn verify_account(&mut self, expected: &str) -> Result<Value> {
        self.verify_account_observed(expected, &mut DenyTools).await
    }
    pub async fn verify_account_observed(
        &mut self,
        expected: &str,
        observer: &mut dyn ToolBroker,
    ) -> Result<Value> {
        let mut denied = ObservationOnlyDeny::new(observer);
        let response = self
            .request(
                "account/read",
                json!({"refreshToken":false}),
                None,
                &mut denied,
            )
            .await?;
        if response.pointer("/account/type").and_then(Value::as_str) != Some("chatgpt") {
            return Err(Error::new(
                "authentication_required",
                "a supported ChatGPT account login is required; paid API fallback is disabled",
            ));
        }
        let id = response
            .pointer("/workspaceRouting/chatgptAccountId")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                Error::new(
                    "identity_unavailable",
                    "provider did not expose a stable account identity",
                )
            })?;
        if id != expected {
            return Err(Error::new(
                "identity_mismatch",
                "authenticated Codex account differs from configured verified identity",
            ));
        }
        self.account_id = Some(id.into());
        Ok(response)
    }
    pub async fn discover_models(&mut self) -> Result<Vec<Value>> {
        self.discover_models_observed(&mut DenyTools).await
    }
    pub async fn discover_models_observed(
        &mut self,
        observer: &mut dyn ToolBroker,
    ) -> Result<Vec<Value>> {
        let mut denied = ObservationOnlyDeny::new(observer);
        if self.account_id.is_none() {
            return Err(Error::new(
                "authentication_required",
                "verify account before reading its model catalog",
            ));
        }
        let mut models = Vec::new();
        let mut cursor = Value::Null;
        let mut seen = BTreeSet::new();
        loop {
            let response = self
                .request(
                    "model/list",
                    json!({"cursor":cursor,"limit":100,"includeHidden":false}),
                    None,
                    &mut denied,
                )
                .await?;
            let data = response
                .get("data")
                .and_then(Value::as_array)
                .ok_or_else(|| Error::new("provider_response", "model page lacks data"))?;
            for model in data {
                required_str(model, "model")?;
                models.push(model.clone());
            }
            match response.get("nextCursor") {
                None | Some(Value::Null) => break,
                Some(Value::String(next)) if !next.is_empty() && seen.insert(next.clone()) => {
                    cursor = json!(next)
                }
                _ => {
                    return Err(Error::new(
                        "provider_pagination",
                        "invalid or looping model cursor",
                    ));
                }
            }
            if seen.len() > 1000 {
                return Err(Error::new(
                    "provider_pagination",
                    "model listing exceeds bounded page limit",
                ));
            }
        }
        self.models = Some(models.clone());
        Ok(models)
    }
    pub async fn rate_limits(&mut self) -> Result<Value> {
        self.rate_limits_observed(&mut DenyTools).await
    }
    pub async fn rate_limits_observed(&mut self, observer: &mut dyn ToolBroker) -> Result<Value> {
        let mut denied = ObservationOnlyDeny::new(observer);
        let account = self.account_id.clone().ok_or_else(|| {
            Error::new(
                "authentication_required",
                "verify account before reading allowance",
            )
        })?;
        let response = self
            .request("account/rateLimits/read", json!({}), None, &mut denied)
            .await?;
        Ok(
            json!({"account_id":account,"observed_at":chrono::Utc::now().to_rfc3339(),"snapshot":response}),
        )
    }
    /// Called after initialize and before thread creation. Thread-local overrides
    /// alone cannot establish that other configured provider tools are disabled.
    pub async fn verify_execution_configuration(
        &mut self,
        permissions: &PermissionConfiguration,
        cwd: &Path,
    ) -> Result<()> {
        self.verify_execution_configuration_observed(permissions, cwd, &mut DenyTools)
            .await
    }
    pub async fn verify_execution_configuration_observed(
        &mut self,
        permissions: &PermissionConfiguration,
        cwd: &Path,
        observer: &mut dyn ToolBroker,
    ) -> Result<()> {
        let mut denied = ObservationOnlyDeny::new(observer);
        let response = self
            .request(
                "config/read",
                json!({"includeLayers":true,"cwd":cwd}),
                None,
                &mut denied,
            )
            .await?;
        self.validate_execution_configuration(permissions, &response)
    }
    pub(super) fn validate_execution_configuration(
        &mut self,
        permissions: &PermissionConfiguration,
        response: &Value,
    ) -> Result<()> {
        let config = response.get("config").ok_or_else(|| {
            Error::new(
                "provider_response",
                "config/read omitted effective configuration",
            )
        })?;
        if config.get("sandbox_mode").is_some_and(|v| !v.is_null())
            || config
                .get("sandbox_workspace_write")
                .is_some_and(|v| !v.is_null())
        {
            return Err(Error::new(
                "isolation_unverified",
                "legacy sandbox configuration competes with named permissions",
            ));
        }
        if config.get("default_permissions") != permissions.settings.get("default_permissions")
            || config.pointer(&format!("/permissions/{}", permissions.profile_id))
                != permissions
                    .settings
                    .pointer(&format!("/permissions/{}", permissions.profile_id))
        {
            return Err(Error::new(
                "isolation_unverified",
                "effective named permission rules differ from the admitted boundary",
            ));
        }
        for key in [
            "approval_policy",
            "approvals_reviewer",
            "web_search",
            "forced_login_method",
        ] {
            if config.get(key) != permissions.settings.get(key) {
                return Err(Error::new(
                    "isolation_unverified",
                    format!("effective {key} differs from the confined profile"),
                ));
            }
        }
        if let Some(expected) = permissions.settings.get("allow_login_shell")
            && config.get("allow_login_shell") != Some(expected)
        {
            return Err(Error::new(
                "isolation_unverified",
                "effective login-shell policy changed",
            ));
        }
        if let Some(expected) = permissions.settings.get("shell_environment_policy") {
            let actual = config.get("shell_environment_policy").ok_or_else(|| {
                Error::new(
                    "isolation_unverified",
                    "effective shell environment was not reported",
                )
            })?;
            for key in [
                "inherit",
                "ignore_default_excludes",
                "include_only",
                "exclude",
                "set",
            ] {
                if let Some(value) = expected.get(key)
                    && actual.get(key) != Some(value)
                {
                    return Err(Error::new(
                        "isolation_unverified",
                        format!("effective shell environment {key} changed"),
                    ));
                }
            }
        }
        if config
            .get("mcp_servers")
            .and_then(Value::as_object)
            .is_some_and(|servers| {
                servers
                    .values()
                    .any(|s| s.get("enabled") != Some(&json!(false)))
            })
        {
            return Err(Error::new(
                "isolation_unverified",
                "external MCP tools remain enabled in the Codex auth home",
            ));
        }
        for (key, expected) in permissions.settings["features"]
            .as_object()
            .ok_or_else(|| Error::invalid("missing confined features"))?
        {
            if config.get("features").and_then(|f| f.get(key)) != Some(expected) {
                return Err(Error::new(
                    "isolation_unverified",
                    format!("effective feature {key} differs from the confined profile"),
                ));
            }
        }
        self.verified_permissions = Some(permissions.isolation_digest.clone());
        Ok(())
    }
    pub fn validate_session(
        &self,
        state: &SessionState,
        isolation: &dyn SessionScope,
        permissions: &PermissionConfiguration,
    ) -> Result<()> {
        state.binding.validate()?;
        isolation.revalidate()?;
        if self.account_id.as_deref() != Some(&state.binding.account_id) {
            return Err(Error::new(
                "identity_mismatch",
                "session is not bound to the authenticated account",
            ));
        }
        state
            .binding
            .profile
            .validate_catalog(self.models.as_deref().ok_or_else(|| {
                Error::new(
                    "unsupported_profile",
                    "authenticated model catalog has not been read",
                )
            })?)?;
        if self.verified_permissions.as_deref() != Some(&permissions.isolation_digest) {
            return Err(Error::new(
                "isolation_unverified",
                "effective provider configuration must be checked before thread admission",
            ));
        }
        if state.binding.cwd.as_path() != isolation.cwd()
            || state.binding.isolation_digest != isolation.isolation_digest()
            || permissions.isolation_digest != isolation.isolation_digest()
        {
            return Err(Error::new(
                "scope_violation",
                "session and isolation snapshots disagree",
            ));
        }
        Ok(())
    }
    pub async fn start_thread(
        &mut self,
        state: &mut SessionState,
        isolation: &dyn SessionScope,
        permissions: &PermissionConfiguration,
        instructions: &str,
        tools: Vec<Value>,
    ) -> Result<()> {
        self.start_thread_observed(
            state,
            isolation,
            permissions,
            instructions,
            tools,
            &mut DenyTools,
        )
        .await
    }
    pub async fn start_thread_observed(
        &mut self,
        state: &mut SessionState,
        isolation: &dyn SessionScope,
        permissions: &PermissionConfiguration,
        instructions: &str,
        tools: Vec<Value>,
        observer: &mut dyn ToolBroker,
    ) -> Result<()> {
        let mut denied = ObservationOnlyDeny::new(observer);
        self.validate_session(state, isolation, permissions)?;
        state.next_turn_allowed()?;
        if state.thread_id.is_some() {
            return Err(Error::new(
                "request_conflict",
                "bound thread already exists",
            ));
        }
        if instructions.trim().is_empty() {
            return Err(Error::invalid("assembled instructions cannot be empty"));
        }
        let mut names = BTreeSet::new();
        for tool in &tools {
            if tool.get("type").and_then(Value::as_str) != Some("function")
                || tool.get("inputSchema").is_none()
                || !names.insert(required_str(tool, "name")?.to_owned())
            {
                return Err(Error::invalid(
                    "dynamic tools must be unique implemented function definitions",
                ));
            }
        }
        let mut config = permissions.settings.clone();
        config["model_reasoning_effort"] = json!(state.binding.profile.reasoning);
        config["review_model"] = json!(state.binding.profile.model);
        let response=self.request("thread/start",json!({"cwd":state.binding.cwd,"model":state.binding.profile.model,"modelProvider":"openai","serviceTier":state.binding.profile.service_tier()?,"approvalPolicy":"never","approvalsReviewer":"user","permissions":permissions.profile_id,"developerInstructions":instructions,"config":config,"dynamicTools":tools,"runtimeWorkspaceRoots":[state.binding.cwd],"allowProviderModelFallback":false,"historyMode":"paginated"}),Some(state),&mut denied).await?;
        validate_thread_response(&response, state, permissions)?;
        let id = required_str(
            response
                .get("thread")
                .ok_or_else(|| Error::new("protocol_error", "thread start lacks thread"))?,
            "id",
        )?
        .to_owned();
        state
            .usage
            .cumulative_baselines
            .insert(format!("{}\0{}", state.binding.account_id, id), 0);
        state.thread_id = Some(id);
        state.provider_instruction_sources = instruction_sources(&response)?;
        state.advertised_tools = names.clone();
        self.allowed_tools = names;
        denied.observe_protocol(ProtocolObservation::ThreadEstablished { state });
        self.emit(SessionEvent::new(
            EventKind::SessionStarted,
            "thread/start",
            state.thread_id.clone(),
            None,
            json!({"profile":PROTOCOL_PROFILE}),
        ))?;
        Ok(())
    }
    /// Reobserve only the exact already-owned thread through the authenticated
    /// account. This is read-only reconciliation, never an adoption mechanism.
    /// Unknown, active, divergent or provider-blocked histories cannot resume.
    pub async fn observe_owned_thread(
        &mut self,
        state: &mut SessionState,
        broker: &mut dyn ToolBroker,
    ) -> Result<String> {
        if self.account_id.as_deref() != Some(&state.binding.account_id)
            || state.thread_id.is_none()
        {
            return Err(Error::new(
                "identity_mismatch",
                "thread observation requires the actual bound account and recorded thread",
            ));
        }
        if state.intervention.is_some() || state.input_required {
            return Err(Error::new(
                "provider_intervention",
                "recorded provider intervention/input requirement cannot be cleared by reading a thread",
            ));
        }
        if state.turn_id.is_some()
            || state.unresolved_turn.is_some()
            || state.completed_turns.is_empty()
            || state.last_completed_turn.is_none()
        {
            return Err(Error::new(
                "reconciliation_required",
                "prior active or unobserved turn needs separate exact outcome reconciliation",
            ));
        }
        let response = self
            .request(
                "thread/read",
                json!({"threadId":state.thread_id,"includeTurns":true}),
                Some(state),
                broker,
            )
            .await?;
        let thread = response
            .get("thread")
            .ok_or_else(|| Error::new("provider_response", "owned thread read lacks thread"))?;
        if thread.get("id").and_then(Value::as_str) != state.thread_id.as_deref()
            || thread.get("cwd").and_then(Value::as_str) != state.binding.cwd.to_str()
            || thread.get("modelProvider").and_then(Value::as_str) != Some("openai")
        {
            return Err(Error::new(
                "identity_mismatch",
                "provider thread differs from recorded identity, cwd or provider",
            ));
        }
        let turns = thread
            .get("turns")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                Error::new(
                    "provider_response",
                    "thread read lacks bounded complete turn history",
                )
            })?;
        if turns.len() > 4096 {
            return Err(Error::new(
                "reconciliation_required",
                "provider thread history differs from retained completed turns",
            ));
        }
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
                    let source = json!({"threadId":state.thread_id,"turnId":required_str(turn,"id")?,"error":error});
                    state.block_provider("thread/read", &source, code)?;
                    broker.observe_session(state)?;
                    return Err(Error::new(
                        "provider_intervention",
                        "actual retained provider history requires intervention",
                    ));
                }
            }
        }
        let mut observed = BTreeMap::new();
        let mut last = None;
        for turn in turns {
            let id = required_str(turn, "id")?;
            let status = required_str(turn, "status")?;
            if turn.get("error").is_some_and(|error| !error.is_null()) {
                return Err(Error::new(
                    "reconciliation_required",
                    "provider history contains an unresolved turn failure",
                ));
            }
            if status != "completed" || observed.insert(id.to_owned(), status.to_owned()).is_some()
            {
                return Err(Error::new(
                    "reconciliation_required",
                    "provider history has active, failed or duplicate turns",
                ));
            }
            last = Some(id);
        }
        if state.turn_id.is_some()
            || observed != state.completed_turns
            || last != state.last_completed_turn.as_deref()
            || !matches!(
                thread.pointer("/status/type").and_then(Value::as_str),
                Some("idle" | "notLoaded")
            )
        {
            return Err(Error::new(
                "reconciliation_required",
                "provider thread is active or no longer matches the exact completed history",
            ));
        }
        if state.intervention.is_some() || state.input_required {
            return Err(Error::new(
                "provider_intervention",
                "thread observation received a new provider blocker",
            ));
        }
        state
            .provider_history
            .as_ref()
            .ok_or_else(|| {
                Error::new(
                    "reconciliation_required",
                    "legacy provider journal lacks full observed history provenance",
                )
            })?
            .verify_readback(turns)?;
        canonical_digest(
            1,
            &json!({"binding":state.binding,"thread":state.thread_id,"turns":observed,"last_turn":last,"status":thread["status"],"response_digest":canonical_digest(1,&response)?}),
        )
    }
    pub async fn resume_thread(
        &mut self,
        state: &mut SessionState,
        previous: &SessionBinding,
        isolation: &dyn SessionScope,
        permissions: &PermissionConfiguration,
        reconciliation: &str,
        broker: &mut dyn ToolBroker,
    ) -> Result<()> {
        self.validate_session(state, isolation, permissions)?;
        state.binding.compatible_resume(previous)?;
        if state.intervention.is_some() {
            return Err(Error::new(
                "provider_intervention",
                "resume is blocked until the provider intervention is resolved",
            ));
        }
        if reconciliation.trim().is_empty() {
            return Err(Error::invalid(
                "resume requires actual reconciliation findings",
            ));
        }
        let thread = state
            .thread_id
            .clone()
            .ok_or_else(|| Error::new("resume_incompatible", "no owned thread is recorded"))?;
        let observed_thread = self.observe_owned_thread(state, broker).await?;
        let reconciliation =
            format!("{reconciliation}\nVerified owned provider history: {observed_thread}");
        let response=self.request("thread/resume",json!({"threadId":thread,"cwd":state.binding.cwd,"model":state.binding.profile.model,"modelProvider":"openai","serviceTier":state.binding.profile.service_tier()?,"permissions":permissions.profile_id,"approvalPolicy":"never","approvalsReviewer":"user","config":permissions.settings,"excludeTurns":true}),Some(state),broker).await?;
        validate_thread_response(&response, state, permissions)?;
        if response.pointer("/thread/id").and_then(Value::as_str) != Some(&thread) {
            return Err(Error::new(
                "identity_mismatch",
                "provider resumed an unrelated thread",
            ));
        }
        state.provider_instruction_sources = instruction_sources(&response)?;
        // Reconciliation is supplied by the next explicitly admitted turn; resume
        // itself must not launch work or pretend remembered completion is current.
        state.terminal = None;
        state.reconciliation_digest = Some(canonical_digest(1, &reconciliation)?);
        self.pending_reconciliation = Some(reconciliation);
        self.allowed_tools = state.advertised_tools.clone();
        Ok(())
    }
    pub async fn start_turn(
        &mut self,
        state: &mut SessionState,
        permissions: &PermissionConfiguration,
        input: &str,
        broker: &mut dyn ToolBroker,
    ) -> Result<String> {
        state.next_turn_allowed()?;
        let thread = state
            .thread_id
            .clone()
            .ok_or_else(|| Error::new("protocol_error", "thread must start before a turn"))?;
        if input.trim().is_empty() {
            return Err(Error::invalid("turn input is empty"));
        }
        let input = match self.pending_reconciliation.take() {
            Some(findings) => format!(
                "Host reconciliation findings (current provider and local facts):\n{findings}\n\nCurrent continuation:\n{input}"
            ),
            None => input.to_owned(),
        };
        // Count before transmission: a lost response is not a free attempt.
        state.turns_started += 1;
        state.terminal = None;
        let response=self.request("turn/start",json!({"threadId":thread,"cwd":state.binding.cwd,"model":state.binding.profile.model,"effort":state.binding.profile.reasoning,"serviceTierForTurn":state.binding.profile.service_tier()?,"permissions":permissions.profile_id,"approvalPolicy":"never","approvalsReviewer":"user","input":[{"type":"text","text":input}]}),Some(state),broker).await?;
        let turn = required_str(
            response
                .get("turn")
                .ok_or_else(|| Error::new("protocol_error", "turn/start response lacks turn"))?,
            "id",
        )?
        .to_owned();
        state.require_new_turn_identity(&turn)?;
        if state.turn_id.as_ref().is_some_and(|id| id != &turn) {
            return Err(Error::new(
                "protocol_error",
                "turn/start disagrees with notification",
            ));
        }
        if !state.completed_turns.contains_key(&turn) {
            state.turn_id = Some(turn.clone());
        }
        self.last_semantic_event = Instant::now();
        Ok(turn)
    }
    pub async fn steer(
        &mut self,
        state: &mut SessionState,
        correction: u64,
        input: &str,
        broker: &mut dyn ToolBroker,
    ) -> Result<bool> {
        state.observe_correction(correction);
        if state.intervention.is_some() {
            return Err(Error::new(
                "provider_intervention",
                "steering cannot bypass a provider hold",
            ));
        }
        let Some(turn) = state.turn_id.clone() else {
            return Ok(false);
        };
        let thread = state
            .thread_id
            .clone()
            .ok_or_else(|| Error::new("protocol_error", "active turn without thread"))?;
        let result=self.request("turn/steer",json!({"threadId":thread,"expectedTurnId":turn,"input":[{"type":"text","text":input}]}),Some(state),broker).await;
        match result {
            Ok(value) if value.get("turnId").and_then(Value::as_str) == Some(&turn) => Ok(true),
            Ok(_) => Err(Error::new(
                "protocol_error",
                "steer response has another turn ID",
            )),
            Err(e)
                if e.code == "provider_response"
                    && state.turn_id.is_none()
                    && state.completed_turns.contains_key(&turn) =>
            {
                Ok(false)
            }
            Err(e) => Err(e),
        }
        // Accepted delivery is not proof the worker processed the watermark.
    }
    pub async fn interrupt(&mut self, state: &mut SessionState) -> Result<()> {
        self.interrupt_observed(state, &mut DenyTools).await
    }
    pub(crate) async fn interrupt_observed(
        &mut self,
        state: &mut SessionState,
        observer: &mut dyn ToolBroker,
    ) -> Result<()> {
        let mut denied = ObservationOnlyDeny::new(observer);
        let Some(turn) = state.turn_id.clone() else {
            return Ok(());
        };
        let thread = state
            .thread_id
            .clone()
            .ok_or_else(|| Error::new("protocol_error", "active turn lacks thread"))?;
        self.request(
            "turn/interrupt",
            json!({"threadId":thread,"turnId":turn}),
            Some(state),
            &mut denied,
        )
        .await?;
        Ok(())
    }
    pub async fn review(
        &mut self,
        state: &mut SessionState,
        review_instructions: &str,
        broker: &mut dyn ToolBroker,
    ) -> Result<Value> {
        state.next_turn_allowed()?;
        let thread = state
            .thread_id
            .clone()
            .ok_or_else(|| Error::new("protocol_error", "review requires a bound thread"))?;
        if review_instructions.trim().is_empty() {
            return Err(Error::invalid(
                "native review requires delivered guidance and exact revision context",
            ));
        }
        state.turns_started += 1;
        state.terminal = None;
        let response = self.request("review/start",json!({"threadId":thread,"delivery":"inline","target":{"type":"custom","instructions":review_instructions}}),Some(state),broker).await?;
        if response.get("reviewThreadId").and_then(Value::as_str) != Some(&thread) {
            return Err(Error::new(
                "identity_mismatch",
                "native review escaped its bound thread",
            ));
        }
        let turn = required_str(
            response
                .get("turn")
                .ok_or_else(|| Error::new("protocol_error", "native review lacks turn"))?,
            "id",
        )?
        .to_owned();
        if state.turn_id.as_ref().is_some_and(|id| id != &turn) {
            return Err(Error::new(
                "protocol_error",
                "native review turn differs from notification",
            ));
        }
        if !state.completed_turns.contains_key(&turn) {
            state.turn_id = Some(turn);
        }
        self.last_semantic_event = Instant::now();
        Ok(response)
    }
    pub async fn pump(
        &mut self,
        state: &mut SessionState,
        broker: &mut dyn ToolBroker,
        silence: Duration,
        stall: Duration,
    ) -> Result<SessionEvent> {
        self.pump_or_wake(state, broker, silence, stall, None)
            .await?
            .ok_or_else(|| {
                Error::new(
                    "protocol_error",
                    "unexpected host wake without a wake source",
                )
            })
    }
    pub async fn pump_or_wake(
        &mut self,
        state: &mut SessionState,
        broker: &mut dyn ToolBroker,
        silence: Duration,
        stall: Duration,
        wake: Option<&tokio::sync::Notify>,
    ) -> Result<Option<SessionEvent>> {
        if silence.is_zero() || stall.is_zero() {
            return Err(Error::invalid("watchdog intervals must be positive"));
        }
        let deadline = (self.last_valid_output() + silence).min(self.last_semantic_event + stall);
        self.pump_until(state, broker, deadline, wake).await
    }
    /// The provider has completed its turn while the host is classifying new
    /// native input. Keep reading intervention/tool messages without applying an
    /// agent-progress watchdog to host work; the original operation deadline is
    /// still absolute and cannot be extended by this wait.
    pub async fn pump_waiting(
        &mut self,
        state: &mut SessionState,
        broker: &mut dyn ToolBroker,
        deadline: tokio::time::Instant,
        wake: Option<&tokio::sync::Notify>,
    ) -> Result<Option<SessionEvent>> {
        if state.turn_id.is_some() {
            return Err(Error::invalid(
                "host classification wait requires no active provider turn",
            ));
        }
        self.pump_until(state, broker, deadline, wake).await
    }
    pub(super) async fn pump_until(
        &mut self,
        state: &mut SessionState,
        broker: &mut dyn ToolBroker,
        deadline: tokio::time::Instant,
        wake: Option<&tokio::sync::Notify>,
    ) -> Result<Option<SessionEvent>> {
        let incoming = match wake {
            Some(wake) => {
                tokio::select! { value=timeout_at(deadline,self.read_message())=>value, _=wake.notified()=>return Ok(None) }
            }
            None => timeout_at(deadline, self.read_message()).await,
        };
        match incoming {
            Err(_) => {
                let _ = self.interrupt_observed(state, broker).await;
                Ok(Some(state.interruption("watchdog_expired")))
            }
            Ok(Err(e)) if e.code == "process_exit" => Ok(Some(state.interruption("process_exit"))),
            Ok(Err(e)) => Err(e),
            Ok(Ok(RpcMessage::Notification { method, params })) => {
                let event = checkpoint_notification(state, broker, &method, &params)?;
                self.emit(event.clone())?;
                Ok(Some(event))
            }
            Ok(Ok(RpcMessage::Request { id, method, params })) => {
                self.service_request(id, &method, params, Some(state), broker)
                    .await?;
                self.events
                    .back()
                    .cloned()
                    .map(Some)
                    .ok_or_else(|| Error::new("protocol_error", "missing tool event"))
            }
            Ok(Ok(_)) => Err(Error::new(
                "protocol_error",
                "unsolicited response without an outstanding request",
            )),
        }
    }
}

pub(super) fn instruction_sources(response: &Value) -> Result<Vec<String>> {
    let values = response
        .get("instructionSources")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            Error::new(
                "instruction_discovery_unavailable",
                "provider did not confirm instruction discovery",
            )
        })?;
    values
        .iter()
        .map(|v| {
            v.as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .ok_or_else(|| Error::new("protocol_error", "invalid provider instruction source"))
        })
        .collect()
}
pub(super) fn validate_thread_response(
    response: &Value,
    state: &SessionState,
    permissions: &PermissionConfiguration,
) -> Result<()> {
    if response.get("cwd").and_then(Value::as_str) != state.binding.cwd.to_str()
        || response.get("model").and_then(Value::as_str) != Some(&state.binding.profile.model)
        || response.get("modelProvider").and_then(Value::as_str) != Some("openai")
        || response.get("reasoningEffort").and_then(Value::as_str)
            != Some(&state.binding.profile.reasoning)
        || response.get("approvalPolicy") != Some(&json!("never"))
        || response
            .pointer("/activePermissionProfile/id")
            .and_then(Value::as_str)
            != Some(&permissions.profile_id)
    {
        return Err(Error::new(
            "unsupported_profile",
            "provider did not preserve the requested cwd, model, reasoning, approvals or permission profile",
        ));
    }
    let expected = state.binding.profile.service_tier()?;
    let actual = response
        .get("serviceTier")
        .and_then(Value::as_str)
        .unwrap_or("default");
    if actual != expected {
        return Err(Error::new(
            "unsupported_profile",
            "provider changed requested speed",
        ));
    }
    Ok(())
}
impl RpcClient {
    /// A caller must already own/authorize this transport and its private home.
    /// In tests, injected finite responses exercise this same metadata protocol.
    pub async fn connection_metadata(
        &mut self,
        expected_account: &str,
        permissions: &PermissionConfiguration,
        cwd: &Path,
    ) -> Result<CodexConnectionMetadata> {
        let initialized = self.initialize().await?;
        self.verify_account(expected_account).await?;
        let model_catalog = self.discover_models().await?;
        let rate_limits = self.rate_limits().await?;
        self.verify_execution_configuration(permissions, cwd)
            .await?;
        Ok(CodexConnectionMetadata {
            supported_version: SUPPORTED_CODEX_VERSION.into(),
            protocol_profile: PROTOCOL_PROFILE.into(),
            account_id: expected_account.into(),
            auth_home: PathBuf::from(required_str(&initialized, "codexHome")?),
            permission_digest: canonical_digest(1, &permissions.settings)?,
            argument_digest: None,
            model_catalog,
            rate_limits,
            observed_at: chrono::Utc::now().to_rfc3339(),
        })
    }
}

#[cfg(test)]
#[path = "client_boundary_tests.rs"]
mod boundary_tests;
