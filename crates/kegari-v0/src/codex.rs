//! Pinned stdio app-server protocol. Provider messages are data, never authority.
use crate::digest::canonical_digest;
use crate::error::{Error, Result};
use crate::isolation::{ExecutionRole, ValidatedIsolation};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::process::{Child, Command};
use tokio::time::{Instant, timeout_at};

pub const SUPPORTED_CODEX_VERSION: &str = "codex-cli 0.164.0-alpha.11.1";
pub const PROTOCOL_PROFILE: &str = "kegari-codex-0.164.0-alpha.11.1-v1";
pub const MAX_FRAME_BYTES: usize = 10 * 1024 * 1024;
const MAX_PENDING_EVENTS: usize = 4096;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CodexProfile {
    pub model: String,
    pub reasoning: String,
    pub speed: String,
}
impl CodexProfile {
    pub fn service_tier(&self) -> Result<&str> {
        match self.speed.as_str() {
            "standard" => Ok("default"),
            "fast" => Ok("priority"),
            _ => Err(Error::new(
                "unsupported_profile",
                "only explicit standard or fast speed is supported",
            )),
        }
    }
    pub fn validate_catalog(&self, models: &[Value]) -> Result<()> {
        if self.model.is_empty() || self.reasoning.is_empty() {
            return Err(Error::new(
                "unsupported_profile",
                "model and reasoning are required",
            ));
        }
        let matches: Vec<_> = models
            .iter()
            .filter(|m| m.get("model").and_then(Value::as_str) == Some(&self.model))
            .collect();
        if matches.len() != 1 {
            return Err(Error::new(
                "unsupported_profile",
                "requested model is absent or ambiguous in the authenticated catalog",
            ));
        }
        let model = matches[0];
        if !model
            .get("supportedReasoningEfforts")
            .and_then(Value::as_array)
            .is_some_and(|a| {
                a.iter().any(|r| {
                    r.get("reasoningEffort").and_then(Value::as_str) == Some(&self.reasoning)
                })
            })
        {
            return Err(Error::new(
                "unsupported_profile",
                "requested reasoning effort is unavailable",
            ));
        }
        let tier = self.service_tier()?;
        // Standard is explicitly documented by this schema; accelerated tiers
        // require positive account/catalog entitlement, never a fallback.
        if tier != "default"
            && !model
                .get("serviceTiers")
                .and_then(Value::as_array)
                .is_some_and(|a| {
                    a.iter()
                        .any(|r| r.get("id").and_then(Value::as_str) == Some(tier))
                })
        {
            return Err(Error::new(
                "unsupported_profile",
                "requested speed is not advertised for this account/model",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SessionBinding {
    pub schema_version: u32,
    pub account_id: String,
    pub work_identity: String,
    pub operation_id: String,
    pub generation: String,
    pub instruction_digest: String,
    pub isolation_digest: String,
    pub profile: CodexProfile,
    pub cwd: PathBuf,
}
impl SessionBinding {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1
            || [
                &self.account_id,
                &self.work_identity,
                &self.operation_id,
                &self.generation,
                &self.instruction_digest,
                &self.isolation_digest,
            ]
            .iter()
            .any(|v| v.is_empty())
            || !self.cwd.is_absolute()
        {
            return Err(Error::invalid("incomplete session binding"));
        }
        self.profile.service_tier()?;
        Ok(())
    }
    pub fn compatible_resume(&self, previous: &Self) -> Result<()> {
        self.validate()?;
        // A resumed operation may have a new operation ID; all effective context
        // and account/scope identities still have to be identical.
        let mut current = self.clone();
        current.operation_id = previous.operation_id.clone();
        if &current != previous {
            return Err(Error::new(
                "resume_incompatible",
                "account, work, instructions, scope or profile changed; reconcile before a fresh admission",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionConfiguration {
    pub profile_id: String,
    pub settings: Value,
    pub isolation_digest: String,
}
impl PermissionConfiguration {
    pub fn for_isolation(isolation: &ValidatedIsolation, codex_home: &Path) -> Result<Self> {
        isolation.revalidate()?;
        let p = isolation.plan();
        let home = codex_home
            .canonicalize()
            .map_err(|e| Error::io("resolve isolated Codex home", e))?;
        if p.repository.starts_with(&home)
            || home.starts_with(&p.repository)
            || home.starts_with(&p.scratch)
            || p.read_roots
                .iter()
                .chain(p.source_writes.iter())
                .any(|g| g.path.starts_with(&home) || home.starts_with(&g.path))
        {
            return Err(Error::new(
                "scope_violation",
                "Codex authentication home must be outside worker access",
            ));
        }
        let profile_id = format!(
            "kegari_{}",
            isolation.digest().rsplit(':').next().unwrap_or_default()
        );
        let mut fs = BTreeMap::<String, Value>::from([
            (":root".into(), json!("deny")),
            (":minimal".into(), json!("read")),
            (":slash_tmp".into(), json!("deny")),
            (":tmpdir".into(), json!("deny")),
        ]);
        fs.insert(p.repository.to_string_lossy().into_owned(), json!("read"));
        fs.insert(p.scratch.to_string_lossy().into_owned(), json!("write"));
        for grant in &p.read_roots {
            fs.insert(grant.path.to_string_lossy().into_owned(), json!("read"));
        }
        for grant in &p.source_writes {
            fs.insert(grant.path.to_string_lossy().into_owned(), json!("write"));
        }
        // Scope grants cannot reopen privileged roots because IsolationPlan rejects overlap.
        for forbidden in p.forbidden_roots.iter().chain(std::iter::once(&home)) {
            fs.insert(forbidden.to_string_lossy().into_owned(), json!("deny"));
        }
        for folder in [".git", ".codex", ".agents"] {
            fs.insert(
                p.repository.join(folder).to_string_lossy().into_owned(),
                json!("read"),
            );
        }
        fs.insert(
            p.repository.join(".aws").to_string_lossy().into_owned(),
            json!("deny"),
        );
        let mut permissions = BTreeMap::new();
        permissions.insert(
            profile_id.clone(),
            json!({"filesystem":fs,"network":{"enabled":false}}),
        );
        let no_exec = matches!(
            p.role,
            ExecutionRole::Coordination | ExecutionRole::IndependentReview
        );
        Ok(Self {
            profile_id: profile_id.clone(),
            isolation_digest: isolation.digest().into(),
            settings: json!({
                "default_permissions":profile_id, "permissions":permissions,
                "approval_policy":"never", "approvals_reviewer":"user", "web_search":"disabled", "forced_login_method":"chatgpt",
                "apps":{"_default":{"enabled":false,"default_tools_enabled":false}},
                "features":{"apps":false,"plugins":false,"browser_use":false,"browser_use_external":false,
                    "computer_use":false,"in_app_browser":false,"image_generation":false,"multi_agent":false,
                    "shell_tool":!no_exec,"code_mode":false,"memories":false,"hooks":false,
                    "skill_mcp_dependency_install":false,"request_permissions_tool":false},
                "shell_environment_policy":{"inherit":"none","include_only":[],"set":{"HOME":p.scratch,"TMPDIR":p.scratch,"PATH":"/usr/bin:/bin:/usr/sbin:/sbin"}}
            }),
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum RpcMessage {
    Response {
        id: Value,
        result: Value,
    },
    Failure {
        id: Value,
        error: Value,
    },
    Notification {
        method: String,
        params: Value,
    },
    Request {
        id: Value,
        method: String,
        params: Value,
    },
}

fn valid_id(v: &Value) -> bool {
    v.as_str().is_some_and(|s| !s.is_empty()) || v.as_i64().is_some() || v.as_u64().is_some()
}
fn parse_message(v: Value) -> Result<RpcMessage> {
    let object = v
        .as_object()
        .ok_or_else(|| Error::new("protocol_error", "JSON-RPC frame is not an object"))?;
    if let Some(version) = object.get("jsonrpc")
        && version != "2.0"
    {
        return Err(Error::new("protocol_error", "unsupported JSON-RPC version"));
    }
    let id = object.get("id");
    if id.is_some_and(|id| !valid_id(id)) {
        return Err(Error::new("protocol_error", "invalid JSON-RPC ID"));
    }
    if let Some(method) = object.get("method") {
        if object.contains_key("result") || object.contains_key("error") {
            return Err(Error::new(
                "protocol_error",
                "mixed JSON-RPC request and response",
            ));
        }
        let method = method
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::new("protocol_error", "invalid JSON-RPC method"))?
            .to_owned();
        let params = object.get("params").cloned().unwrap_or(Value::Null);
        return Ok(match id {
            Some(id) => RpcMessage::Request {
                id: id.clone(),
                method,
                params,
            },
            None => RpcMessage::Notification { method, params },
        });
    }
    let id = id
        .ok_or_else(|| Error::new("protocol_error", "response without request ID"))?
        .clone();
    match (object.get("result"), object.get("error")) {
        (Some(result), None) => Ok(RpcMessage::Response {
            id,
            result: result.clone(),
        }),
        (None, Some(error))
            if error.get("code").and_then(Value::as_i64).is_some()
                && error.get("message").and_then(Value::as_str).is_some() =>
        {
            Ok(RpcMessage::Failure {
                id,
                error: error.clone(),
            })
        }
        _ => Err(Error::new(
            "protocol_error",
            "response must have exactly one valid result or error",
        )),
    }
}

#[derive(Default)]
pub struct JsonRpcDecoder {
    pending: Vec<u8>,
    poisoned: bool,
}
impl JsonRpcDecoder {
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<RpcMessage>> {
        if self.poisoned {
            return Err(Error::new(
                "protocol_error",
                "decoder is closed after a malformed frame",
            ));
        }
        let result = self.decode(bytes);
        if result.is_err() {
            self.poisoned = true;
            self.pending.clear();
        }
        result
    }
    fn decode(&mut self, bytes: &[u8]) -> Result<Vec<RpcMessage>> {
        let mut messages = Vec::new();
        for chunk in bytes.split_inclusive(|b| *b == b'\n') {
            let newline = chunk.last() == Some(&b'\n');
            let content = if newline {
                &chunk[..chunk.len() - 1]
            } else {
                chunk
            };
            if self.pending.len().saturating_add(content.len()) > MAX_FRAME_BYTES {
                return Err(Error::new(
                    "protocol_error",
                    "app-server frame exceeds 10 MiB",
                ));
            }
            self.pending.extend_from_slice(content);
            if newline {
                if self.pending.last() == Some(&b'\r') {
                    self.pending.pop();
                }
                let value = serde_json::from_slice(&self.pending)
                    .map_err(|_| Error::new("protocol_error", "malformed app-server JSON frame"))?;
                messages.push(parse_message(value)?);
                self.pending.clear();
            }
        }
        Ok(messages)
    }
    pub fn finish(&mut self) -> Result<()> {
        if self.pending.is_empty() && !self.poisoned {
            Ok(())
        } else {
            self.poisoned = true;
            Err(Error::new(
                "protocol_error",
                "unterminated or invalid app-server frame at exit",
            ))
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    SessionStarted,
    StartupFailure,
    TurnStarted,
    TurnCompleted,
    TurnFailed,
    TurnCancelled,
    InputRequired,
    ProviderIntervention,
    ToolResult,
    UnsupportedTool,
    Notification,
    ProtocolError,
    ProcessExit,
    ModelChanged,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionEvent {
    pub kind: EventKind,
    pub method: String,
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub observed_at: String,
    pub data: Value,
}
impl SessionEvent {
    fn new(
        kind: EventKind,
        method: &str,
        thread: Option<String>,
        turn: Option<String>,
        data: Value,
    ) -> Self {
        Self {
            kind,
            method: method.into(),
            thread_id: thread,
            turn_id: turn,
            observed_at: chrono::Utc::now().to_rfc3339(),
            data,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderIntervention {
    pub schema_version: u32,
    pub account_id: String,
    pub thread_id: String,
    pub turn_id: String,
    pub source_method: String,
    pub reason_code: String,
    pub required_resolution: String,
    pub observed_at: String,
    pub source_digest: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct UsageLedger {
    /// Key includes verified account identity and provider thread identity.
    pub cumulative_baselines: BTreeMap<String, u64>,
    pub observed_tokens: BTreeMap<String, u64>,
    pub earlier_usage_unknown: BTreeSet<String>,
    pub ended_operations: BTreeMap<String, u64>,
}
impl UsageLedger {
    pub fn observe(&mut self, account: &str, thread: &str, total: u64, fresh_thread: bool) -> u64 {
        let key = format!("{account}\0{thread}");
        let previous = self.cumulative_baselines.get(&key).copied();
        let delta = match previous {
            Some(old) => total.saturating_sub(old),
            None if fresh_thread => total,
            None => {
                self.earlier_usage_unknown.insert(key.clone());
                0
            }
        };
        if previous.is_none_or(|old| total >= old) {
            self.cumulative_baselines.insert(key, total);
        }
        let observed = self.observed_tokens.entry(account.into()).or_default();
        *observed = observed.saturating_add(delta);
        delta
    }
    pub fn finish_elapsed(&mut self, operation: &str, elapsed_ms: u64) {
        self.ended_operations
            .entry(operation.into())
            .or_insert(elapsed_ms);
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionState {
    pub schema_version: u32,
    pub binding: SessionBinding,
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub turns_started: u32,
    pub max_turns: u32,
    pub correction_required: u64,
    pub correction_consumed: u64,
    pub intervention: Option<ProviderIntervention>,
    pub input_required: bool,
    pub terminal: Option<String>,
    pub completed_turns: BTreeMap<String, String>,
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
            turns_started: 0,
            max_turns,
            correction_required: 0,
            correction_consumed: 0,
            intervention: None,
            input_required: false,
            terminal: None,
            completed_turns: BTreeMap::new(),
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
            || state.completed_turns.len() > state.max_turns as usize
        {
            return Err(Error::new(
                "corrupt_receipt",
                "inconsistent provider recovery state",
            ));
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
            && self.correction_consumed == self.correction_required
            && self.intervention.is_none()
            && !self.input_required
    }
    pub fn interruption(&mut self, reason: &str) -> SessionEvent {
        if self.intervention.is_none() {
            self.terminal = Some("interrupted".into());
        }
        SessionEvent::new(
            if self.intervention.is_some() {
                EventKind::ProviderIntervention
            } else {
                EventKind::ProcessExit
            },
            reason,
            self.thread_id.clone(),
            self.turn_id.take(),
            json!({"outcome":if self.intervention.is_some(){"blocked"}else{"interrupted"}}),
        )
    }
    fn block_provider(&mut self, method: &str, params: &Value, reason: &str) -> Result<()> {
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
                self.turn_id = Some(id.into());
                kind = EventKind::TurnStarted;
            }
            "turn/completed" => {
                let t = params
                    .get("turn")
                    .ok_or_else(|| Error::new("protocol_error", "completion lacks turn"))?;
                let id = required_str(t, "id")?;
                let status = required_str(t, "status")?;
                if let Some(previous) = self.completed_turns.get(id) {
                    if previous != status {
                        return Err(Error::new(
                            "protocol_error",
                            "duplicate terminal event changed its outcome",
                        ));
                    }
                    return Ok(SessionEvent::new(
                        EventKind::Notification,
                        method,
                        thread,
                        turn,
                        json!({"duplicate":true}),
                    ));
                }
                if self.turn_id.as_deref() != Some(id) {
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
                if let Some(error) = t.get("error").filter(|v| !v.is_null()) {
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
                        let p = json!({"threadId":thread,"turnId":id,"error":error});
                        self.block_provider(method, &p, code)?;
                    }
                }
                if self.intervention.is_none() {
                    self.terminal = Some(status.into());
                }
                self.turn_id = None;
                self.completed_turns.insert(id.into(), status.into());
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
                required_str(params, "turnId")?;
                let total = params
                    .pointer("/tokenUsage/total/totalTokens")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| {
                        Error::new("protocol_error", "invalid cumulative token counter")
                    })?;
                let delta = self
                    .usage
                    .observe(&self.binding.account_id, t, total, false);
                data = json!({"observed_delta":delta,"cumulative_total":total});
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

fn required_str<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Error::new("protocol_error", format!("missing or invalid {key}")))
}

#[derive(Clone, Debug)]
pub struct ToolCall {
    pub thread_id: String,
    pub turn_id: String,
    pub call_id: String,
    pub name: String,
    pub arguments: Value,
}
pub trait ToolBroker: Send {
    fn call<'a>(
        &'a mut self,
        call: &'a ToolCall,
    ) -> Pin<Box<dyn Future<Output = Result<Value>> + Send + 'a>>;
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

/// The transport accepts injected duplex pipes for deterministic protocol tests.
/// Production construction is `spawn`, which pins the executable/profile.
pub struct RpcClient {
    reader: Pin<Box<dyn AsyncRead + Send>>,
    writer: Pin<Box<dyn AsyncWrite + Send>>,
    decoder: JsonRpcDecoder,
    messages: VecDeque<RpcMessage>,
    events: VecDeque<SessionEvent>,
    next_id: u64,
    request_timeout: Duration,
    child: Option<Child>,
    diagnostic_task: Option<tokio::task::JoinHandle<()>>,
    expected_home: Option<PathBuf>,
    initialized: bool,
    uncertain_request: Option<u64>,
    allowed_tools: BTreeSet<String>,
    account_id: Option<String>,
    models: Option<Vec<Value>>,
    verified_permissions: Option<String>,
    pending_reconciliation: Option<String>,
    pub last_valid_output: Instant,
    pub last_semantic_event: Instant,
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
            reader: Box::pin(reader),
            writer: Box::pin(writer),
            decoder: JsonRpcDecoder::default(),
            messages: VecDeque::new(),
            events: VecDeque::new(),
            next_id: 1,
            request_timeout,
            child: None,
            diagnostic_task: None,
            expected_home: None,
            initialized: false,
            uncertain_request: None,
            allowed_tools: BTreeSet::new(),
            account_id: None,
            models: None,
            verified_permissions: None,
            pending_reconciliation: None,
            last_valid_output: Instant::now(),
            last_semantic_event: Instant::now(),
        })
    }
    pub async fn verify_version(executable: &Path, deadline: Duration) -> Result<()> {
        if !executable.is_absolute() {
            return Err(Error::invalid(
                "Codex executable must be an absolute verified path",
            ));
        }
        let mut command = Command::new(executable);
        command
            .arg("--version")
            .stdin(Stdio::null())
            .kill_on_drop(true);
        let output = tokio::time::timeout(deadline, command.output())
            .await
            .map_err(|_| Error::new("timeout", "Codex version probe timed out"))?
            .map_err(|e| Error::io("Codex version probe", e))?;
        if !output.status.success()
            || String::from_utf8_lossy(&output.stdout).trim() != SUPPORTED_CODEX_VERSION
        {
            return Err(Error::new(
                "unsupported_codex_version",
                format!(
                    "required {SUPPORTED_CODEX_VERSION}; install and qualify a matching executable"
                ),
            ));
        }
        Ok(())
    }
    pub async fn spawn(
        executable: &Path,
        codex_home: &Path,
        permissions: &PermissionConfiguration,
        request_timeout: Duration,
    ) -> Result<Self> {
        Self::verify_version(executable, request_timeout).await?;
        let home = codex_home
            .canonicalize()
            .map_err(|e| Error::io("Codex auth home", e))?;
        if !home.is_dir() {
            return Err(Error::invalid(
                "Codex auth home must be an existing isolated directory",
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            let m = home
                .metadata()
                .map_err(|e| Error::io("Codex auth home metadata", e))?;
            if m.permissions().mode() & 0o077 != 0 || m.uid() != unsafe { libc::geteuid() } {
                return Err(Error::new(
                    "unsafe_permissions",
                    "Codex auth home must be owner-only and owned by the execution user",
                ));
            }
        }
        let mut command = Command::new(executable);
        command.args(["app-server", "--listen", "stdio://", "--strict-config"]);
        for (key, value) in permissions
            .settings
            .as_object()
            .ok_or_else(|| Error::invalid("permission settings must be an object"))?
        {
            command
                .arg("-c")
                .arg(format!("{key}={}", toml_inline(value)?));
        }
        command
            .env_clear()
            .env("CODEX_HOME", &home)
            .env("HOME", &home)
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .env("LANG", "C.UTF-8")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        {
            command.process_group(0);
        }
        let mut child = command
            .spawn()
            .map_err(|e| Error::io("start Codex app-server", e))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::new("process_error", "app-server stdout unavailable"))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| Error::new("process_error", "app-server stdin unavailable"))?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| Error::new("process_error", "app-server stderr unavailable"))?;
        // Diagnostics are drained separately. Arbitrary provider text is not a
        // protocol frame and cannot leak into status/log output.
        let diagnostic_task = tokio::spawn(async move {
            let mut buffer = [0u8; 4096];
            while matches!(stderr.read(&mut buffer).await,Ok(n) if n>0) {}
        });
        let mut rpc = Self::from_transport(stdout, stdin, request_timeout)?;
        rpc.child = Some(child);
        rpc.diagnostic_task = Some(diagnostic_task);
        rpc.expected_home = Some(home);
        Ok(rpc)
    }
    async fn send(&mut self, value: &Value) -> Result<()> {
        let mut bytes = serde_json::to_vec(value)
            .map_err(|_| Error::new("protocol_error", "cannot encode request"))?;
        if bytes.len() > MAX_FRAME_BYTES {
            return Err(Error::new(
                "protocol_error",
                "outgoing frame exceeds 10 MiB",
            ));
        }
        bytes.push(b'\n');
        self.writer
            .write_all(&bytes)
            .await
            .map_err(|e| Error::io("write app-server protocol", e))?;
        self.writer
            .flush()
            .await
            .map_err(|e| Error::io("flush app-server protocol", e))
    }
    async fn read_message(&mut self) -> Result<RpcMessage> {
        loop {
            if let Some(message) = self.messages.pop_front() {
                return Ok(message);
            }
            let mut bytes = [0; 8192];
            let n = self
                .reader
                .read(&mut bytes)
                .await
                .map_err(|e| Error::io("read app-server protocol", e))?;
            if n == 0 {
                self.decoder.finish()?;
                return Err(Error::new(
                    "process_exit",
                    "app-server exited; no terminal work result is implied",
                ));
            }
            let messages = self.decoder.push(&bytes[..n])?;
            if !messages.is_empty() {
                self.last_valid_output = Instant::now();
            }
            self.messages.extend(messages);
        }
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
                    broker.call(&call).await
                } else {
                    Err(Error::new(
                        "unsupported_tool",
                        "tool is unavailable for this bound role, phase or turn",
                    ))
                };
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
        mut state: Option<&mut SessionState>,
        broker: &mut dyn ToolBroker,
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
        let deadline = Instant::now() + self.request_timeout;
        let result = timeout_at(deadline, async {
            self.send(&json!({"id":id,"method":method,"params":params}))
                .await?;
            loop {
                match self.read_message().await? {
                    RpcMessage::Response {
                        id: response,
                        result,
                    } if response == id => return Ok(result),
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
                            s.notification(&method, &params)?
                        } else {
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
        match result {
            Ok(result) => {
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
    pub async fn initialize(&mut self) -> Result<Value> {
        if self.initialized {
            return Err(Error::new(
                "protocol_error",
                "initialize may run once per transport",
            ));
        }
        let response=self.request("initialize",json!({"clientInfo":{"name":"kegari-v0","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}}),None,&mut DenyTools).await?;
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
        let response = self
            .request(
                "account/read",
                json!({"refreshToken":false}),
                None,
                &mut DenyTools,
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
                    &mut DenyTools,
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
        let account = self.account_id.clone().ok_or_else(|| {
            Error::new(
                "authentication_required",
                "verify account before reading allowance",
            )
        })?;
        let response = self
            .request("account/rateLimits/read", json!({}), None, &mut DenyTools)
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
        let response = self
            .request(
                "config/read",
                json!({"includeLayers":true,"cwd":cwd}),
                None,
                &mut DenyTools,
            )
            .await?;
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
    fn validate_session(
        &self,
        state: &SessionState,
        isolation: &ValidatedIsolation,
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
        if state.binding.cwd != isolation.plan().cwd
            || state.binding.isolation_digest != isolation.digest()
            || permissions.isolation_digest != isolation.digest()
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
        isolation: &ValidatedIsolation,
        permissions: &PermissionConfiguration,
        instructions: &str,
        tools: Vec<Value>,
    ) -> Result<()> {
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
        let response=self.request("thread/start",json!({"cwd":state.binding.cwd,"model":state.binding.profile.model,"modelProvider":"openai","serviceTier":state.binding.profile.service_tier()?,"approvalPolicy":"never","approvalsReviewer":"user","permissions":permissions.profile_id,"developerInstructions":instructions,"config":config,"dynamicTools":tools,"runtimeWorkspaceRoots":[state.binding.cwd],"allowProviderModelFallback":false,"historyMode":"paginated"}),Some(state),&mut DenyTools).await?;
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
        self.emit(SessionEvent::new(
            EventKind::SessionStarted,
            "thread/start",
            state.thread_id.clone(),
            None,
            json!({"profile":PROTOCOL_PROFILE}),
        ))?;
        Ok(())
    }
    pub async fn resume_thread(
        &mut self,
        state: &mut SessionState,
        previous: &SessionBinding,
        isolation: &ValidatedIsolation,
        permissions: &PermissionConfiguration,
        reconciliation: &str,
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
        let response=self.request("thread/resume",json!({"threadId":thread,"cwd":state.binding.cwd,"model":state.binding.profile.model,"modelProvider":"openai","serviceTier":state.binding.profile.service_tier()?,"permissions":permissions.profile_id,"approvalPolicy":"never","approvalsReviewer":"user","config":permissions.settings,"excludeTurns":true}),Some(state),&mut DenyTools).await?;
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
        self.pending_reconciliation = Some(reconciliation.into());
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
            Err(e) if e.code == "provider_response" => Ok(false),
            Err(e) => Err(e),
        }
        // Accepted delivery is not proof the worker processed the watermark.
    }
    pub async fn interrupt(&mut self, state: &mut SessionState) -> Result<()> {
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
            &mut DenyTools,
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
        if silence.is_zero() || stall.is_zero() {
            return Err(Error::invalid("watchdog intervals must be positive"));
        }
        let deadline = (self.last_valid_output + silence).min(self.last_semantic_event + stall);
        let incoming = timeout_at(deadline, self.read_message()).await;
        match incoming {
            Err(_) => {
                let _ = self.interrupt(state).await;
                Ok(state.interruption("watchdog_expired"))
            }
            Ok(Err(e)) if e.code == "process_exit" => Ok(state.interruption("process_exit")),
            Ok(Err(e)) => Err(e),
            Ok(Ok(RpcMessage::Notification { method, params })) => {
                let event = state.notification(&method, &params)?;
                self.emit(event.clone())?;
                Ok(event)
            }
            Ok(Ok(RpcMessage::Request { id, method, params })) => {
                self.service_request(id, &method, params, Some(state), broker)
                    .await?;
                self.events
                    .back()
                    .cloned()
                    .ok_or_else(|| Error::new("protocol_error", "missing tool event"))
            }
            Ok(Ok(_)) => Err(Error::new(
                "protocol_error",
                "unsolicited response without an outstanding request",
            )),
        }
    }
    pub async fn terminate(&mut self) -> Result<()> {
        if let Some(child) = &mut self.child
            && child
                .try_wait()
                .map_err(|e| Error::io("check owned provider process", e))?
                .is_none()
        {
            #[cfg(unix)]
            if let Some(id) = child.id() {
                unsafe {
                    libc::kill(-(id as i32), libc::SIGTERM);
                }
            }
            if tokio::time::timeout(Duration::from_secs(5), child.wait())
                .await
                .is_err()
            {
                #[cfg(unix)]
                if let Some(id) = child.id() {
                    unsafe {
                        libc::kill(-(id as i32), libc::SIGKILL);
                    }
                }
                child
                    .kill()
                    .await
                    .map_err(|e| Error::io("kill owned provider", e))?;
                child
                    .wait()
                    .await
                    .map_err(|e| Error::io("reap owned provider", e))?;
            }
        }
        if let Some(task) = self.diagnostic_task.take() {
            task.abort();
        }
        Ok(())
    }
}
impl Drop for RpcClient {
    fn drop(&mut self) {
        if let Some(task) = self.diagnostic_task.take() {
            task.abort();
        }
    }
}
fn instruction_sources(response: &Value) -> Result<Vec<String>> {
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
fn validate_thread_response(
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
fn toml_inline(value: &Value) -> Result<String> {
    match value {
        Value::Null => Err(Error::invalid("null cannot be a Codex config override")),
        Value::Bool(v) => Ok(v.to_string()),
        Value::Number(v) => Ok(v.to_string()),
        Value::String(s) => {
            serde_json::to_string(s).map_err(|_| Error::invalid("invalid config string"))
        }
        Value::Array(a) => Ok(format!(
            "[{}]",
            a.iter()
                .map(toml_inline)
                .collect::<Result<Vec<_>>>()?
                .join(",")
        )),
        Value::Object(o) => Ok(format!(
            "{{{}}}",
            o.iter()
                .map(|(k, v)| Ok(format!(
                    "{}={}",
                    serde_json::to_string(k).map_err(|_| Error::invalid("invalid config key"))?,
                    toml_inline(v)?
                )))
                .collect::<Result<Vec<_>>>()?
                .join(",")
        )),
    }
}
