use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

pub type Result<T> = std::result::Result<T, Error>;

/// Safe, stable public errors. Never put raw credential-bearing responses here.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Error {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    pub details: Value,
    pub next_action: Option<String>,
}

impl Error {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable: false,
            details: Value::Null,
            next_action: None,
        }
    }
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid_config", message)
    }
    pub fn io(context: impl AsRef<str>, error: std::io::Error) -> Self {
        Self::new(
            "io_error",
            format!("{}: {}", context.as_ref(), error.kind()),
        )
    }
    pub fn next(mut self, action: impl Into<String>) -> Self {
        self.next_action = Some(action.into());
        self
    }
    pub fn detail(mut self, details: Value) -> Self {
        self.details = details;
        self
    }
    pub fn exit_code(&self) -> i32 {
        match self.code.as_str() {
            "invalid_input" | "invalid_command" => 2,
            "authentication_required" | "access_denied" | "missing_secret" => 5,
            "stale_result"
            | "stale_config"
            | "stale_control"
            | "request_conflict"
            | "ownership_conflict"
            | "identity_mismatch"
            | "unsupported_codex_version"
            | "incompatible_version"
            | "snapshot_expired" => 6,
            "provider_transport" | "target_unavailable" | "timeout" => 4,
            "operation_uncertain" | "operation_failed" | "partial_release" => 7,
            _ => 3,
        }
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for Error {}
