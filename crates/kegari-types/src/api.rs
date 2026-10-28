//! Shared control request and response data.
use crate::error::Error;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommandName {
    Overview,
    Status,
    Show,
    Open,
    Approve,
    Correct,
    Release,
    Setup,
    ConnectionList,
    ConnectionAdd,
    ConnectionReconnect,
    WorkspaceList,
    WorkspaceAdd,
    WorkspaceConfigure,
    ConfigShow,
    ConfigCheck,
    Pause,
    Resume,
    Stop,
    Start,
    /// Local Unix peer only; never exposed as a remote service action.
    ServiceStop,
    /// One explicit local native evaluation; no remote endpoint authorization.
    EvaluationRun,
    OperationShow,
    OperationReconcile,
    Retry,
    ReleaseRecover,
    Cleanup,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandRequest {
    pub api_version: u32,
    pub runtime_id: Uuid,
    pub request_id: Uuid,
    pub command: CommandName,
    pub args: Value,
    pub expected_config_revision: Option<String>,
    pub expected_control_generation: Option<u64>,
}
impl std::fmt::Debug for CommandRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CommandRequest")
            .field("runtime_id", &self.runtime_id)
            .field("request_id", &self.request_id)
            .field("command", &self.command)
            .field("args", &"[OMITTED]")
            .finish()
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    pub api_version: u32,
    pub request_id: Uuid,
    pub runtime_id: Uuid,
    pub config_revision: Option<String>,
    pub observed_at: DateTime<Utc>,
    pub ok: bool,
    pub operation_id: Option<Uuid>,
    pub state: Option<String>,
    pub data: Value,
    pub error: Option<Error>,
}
