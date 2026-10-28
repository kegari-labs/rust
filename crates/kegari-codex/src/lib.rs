//! Pinned Codex protocol client, session observations and stream framing.
//! Protocol data never establishes native execution authority.
use kegari_digest::canonical_digest;
use kegari_types::error::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::time::{Instant, timeout_at};

mod client;
mod history;
mod observation;
mod resume;
mod session;
mod transport_observation;
pub use client::{DenyTools, RequestGate, RpcClient, SessionScope, ToolBroker};
use client::{instruction_sources, validate_thread_response};
pub use history::RecordedThreadHistory;
pub use observation::{
    DeclaredProtocolObservation, ProtocolCapture, ProtocolCaptureIssue, ProtocolCaptureState,
    ProtocolObservation, ProtocolOperationBinding, ProtocolRecordReference,
    ProtocolTransportEndDiagnostic,
};
pub use resume::{
    ResumeCandidate, ResumeLineageEntry, ResumeRequest, SessionResumeOrigin, SubmissionHistory,
    TransientTurnFailure, VerifiedThreadRecovery,
};
pub use session::SessionState;
use transport_observation::ObservationOnlyDeny;
pub use transport_observation::ProtocolTransportEnd;
const MAX_PENDING_EVENTS: usize = 4096;

pub const SUPPORTED_CODEX_VERSION: &str = "codex-cli 0.164.0-alpha.11.1";
pub const PROTOCOL_PROFILE: &str = "kegari-codex-0.164.0-alpha.11.1-v1";
pub const MAX_FRAME_BYTES: usize = 10 * 1024 * 1024;

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
    pub fn new(
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
    /// Cumulative increases without a qualified new-session boundary cannot be
    /// charged to the new operation. Keys have the account/thread binding.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub prior_unattributed_usage: BTreeMap<String, PriorUnattributedUsage>,
}
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PriorUnattributedUsage {
    /// Sum of observed increases only; never attributed to either operation.
    pub known_increase: u64,
    /// The first pre-turn counter had no retained baseline. Its preceding
    /// amount remains unknown even when subsequent increases are measurable.
    pub retained_baseline_missing: bool,
    /// An unknown-turn update arrived after new work was submitted. Its
    /// cumulative increase cannot be assigned to either operation by timing.
    #[serde(default)]
    pub timing_unknown: bool,
    /// First monotone counter carrying an actual new-session turn identity.
    /// Its increase is unqualified: retained history does not authenticate the
    /// old operation's final token total. Only subsequent deltas are charged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_new_turn_boundary: Option<u64>,
}
impl UsageLedger {
    /// Start accounting for a new admitted session of the same provider thread.
    /// Previous totals/elapsed stay with their original operation; only the
    /// cumulative observation baseline crosses the session boundary.
    pub fn resumed_session(&self, account: &str, thread: &str) -> Self {
        let key = format!("{account}\0{thread}");
        let mut next = Self::default();
        if let Some(value) = self.cumulative_baselines.get(&key) {
            next.cumulative_baselines.insert(key.clone(), *value);
        }
        if !self.cumulative_baselines.contains_key(&key)
            || self.earlier_usage_unknown.contains(&key)
            || self
                .prior_unattributed_usage
                .get(&key)
                .is_some_and(|usage| {
                    usage.known_increase > 0
                        || usage.retained_baseline_missing
                        || usage.timing_unknown
                })
        {
            next.earlier_usage_unknown.insert(key);
        }
        next
    }
    pub fn observe_resume_baseline(
        &mut self,
        account: &str,
        thread: &str,
        total: u64,
        timing_unknown: bool,
    ) {
        let key = format!("{account}\0{thread}");
        let previous = self.cumulative_baselines.get(&key).copied();
        if timing_unknown {
            self.earlier_usage_unknown.insert(key.clone());
            self.prior_unattributed_usage
                .entry(key.clone())
                .or_default()
                .timing_unknown = true;
        }
        match previous {
            None => {
                self.earlier_usage_unknown.insert(key.clone());
                self.prior_unattributed_usage
                    .entry(key.clone())
                    .or_default()
                    .retained_baseline_missing = true;
            }
            Some(old) if total > old => {
                let prior = self
                    .prior_unattributed_usage
                    .entry(key.clone())
                    .or_default();
                prior.known_increase = prior.known_increase.saturating_add(total - old);
            }
            Some(_) => (),
        }
        if previous.is_none_or(|old| total >= old) {
            self.cumulative_baselines.insert(key, total);
        }
    }
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

#[derive(Clone, Debug)]
pub struct ToolCall {
    pub thread_id: String,
    pub turn_id: String,
    pub call_id: String,
    pub name: String,
    pub arguments: Value,
}

pub fn app_server_argv(permissions: &PermissionConfiguration) -> Result<Vec<String>> {
    let mut args = vec![
        "app-server".into(),
        "--listen".into(),
        "stdio://".into(),
        "--strict-config".into(),
    ];
    for (key, value) in permissions
        .settings
        .as_object()
        .ok_or_else(|| Error::invalid("permission settings must be an object"))?
    {
        args.push("-c".into());
        args.push(format!("{key}={}", toml_inline(value)?));
    }
    Ok(args)
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

/// Metadata-only connection evidence. This never creates a thread or turn and
/// does not attest live worker sandbox qualification or successful model work.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodexConnectionMetadata {
    pub supported_version: String,
    pub protocol_profile: String,
    pub account_id: String,
    pub auth_home: PathBuf,
    pub permission_digest: String,
    pub argument_digest: Option<String>,
    pub model_catalog: Vec<Value>,
    pub rate_limits: Value,
    pub observed_at: String,
}

/// A stream result, not proof of process exit or successful work.
pub enum TransportRead {
    Message(RpcMessage),
    CleanEof,
}

/// Framing over caller-supplied streams. Authorization, deadlines and owned
/// process cleanup remain the caller's responsibility.
pub struct RpcTransport {
    reader: Pin<Box<dyn AsyncRead + Send>>,
    writer: Pin<Box<dyn AsyncWrite + Send>>,
    decoder: JsonRpcDecoder,
    messages: VecDeque<RpcMessage>,
    clean_eof_observed: bool,
    wire_bytes_read: u64,
    last_valid_output: Instant,
}
impl RpcTransport {
    pub fn new<R, W>(reader: R, writer: W) -> Self
    where
        R: AsyncRead + Send + 'static,
        W: AsyncWrite + Send + 'static,
    {
        Self {
            reader: Box::pin(reader),
            writer: Box::pin(writer),
            decoder: JsonRpcDecoder::default(),
            messages: VecDeque::new(),
            clean_eof_observed: false,
            wire_bytes_read: 0,
            last_valid_output: Instant::now(),
        }
    }
    pub async fn send(&mut self, value: &Value) -> Result<()> {
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
    pub async fn read(&mut self, new_byte_bound: Option<u64>) -> Result<TransportRead> {
        let before = self.wire_bytes_read;
        loop {
            if let Some(message) = self.messages.pop_front() {
                return Ok(TransportRead::Message(message));
            }
            if self.clean_eof_observed {
                return Ok(TransportRead::CleanEof);
            }
            let mut bytes = [0; 8192];
            let read_limit = new_byte_bound
                .map(|bound| {
                    bound
                        .saturating_sub(self.wire_bytes_read.saturating_sub(before))
                        .saturating_add(1)
                        .min(bytes.len() as u64) as usize
                })
                .unwrap_or(bytes.len());
            let n = self
                .reader
                .read(&mut bytes[..read_limit])
                .await
                .map_err(|e| Error::io("read app-server protocol", e))?;
            self.wire_bytes_read = self
                .wire_bytes_read
                .checked_add(n as u64)
                .ok_or_else(|| Error::new("bounded_input", "transport byte count overflow"))?;
            if new_byte_bound.is_some_and(|bound| self.wire_bytes_read - before > bound) {
                // The just-consumed byte cannot be silently skipped on a later
                // read. This observation stream is irreversibly incomplete.
                self.decoder.poisoned = true;
                self.decoder.pending.clear();
                return Err(Error::new(
                    "bounded_input",
                    "declared tail wire byte bound exceeded",
                ));
            }
            if n == 0 {
                self.decoder.finish()?;
                self.clean_eof_observed = true;
                return Ok(TransportRead::CleanEof);
            }
            let messages = self.decoder.push(&bytes[..n])?;
            if !messages.is_empty() {
                self.last_valid_output = Instant::now();
            }
            self.messages.extend(messages);
        }
    }
    pub fn pending_bytes(&self) -> usize {
        self.decoder.pending.len()
    }
    pub fn wire_bytes_read(&self) -> u64 {
        self.wire_bytes_read
    }
    pub fn last_valid_output(&self) -> Instant {
        self.last_valid_output
    }
    pub fn has_queued_messages(&self) -> bool {
        !self.messages.is_empty()
    }
    pub fn clean_eof_observed(&self) -> bool {
        self.clean_eof_observed
    }
}

#[cfg(test)]
mod tests;

fn required_str<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Error::new("protocol_error", format!("missing or invalid {key}")))
}

fn validate_digest(value: &str) -> Result<()> {
    let suffix = value
        .strip_prefix("sha256:v1:")
        .or_else(|| value.strip_prefix("sha256:"));
    if !suffix.is_some_and(|s| {
        s.len() == 64
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }) {
        return Err(Error::invalid(
            "evidence reference requires an explicit SHA-256 digest/schema",
        ));
    }
    Ok(())
}
