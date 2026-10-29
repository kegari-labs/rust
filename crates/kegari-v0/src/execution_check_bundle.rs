//! Legacy service/check records retained for historical evidence decoding.
//! Codex lifecycle observations replace the deleted custom service runner.
use super::*;
use kegari_execution::journal::CheckMemberKind;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckBundleMemberResult {
    pub ordinal: u32,
    pub process_id: Uuid,
    pub kind: CheckMemberKind,
    pub service: Option<String>,
    pub outcome: Option<CommandOutcome>,
    pub error: Option<Error>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceExecutionRecord {
    pub name: String,
    pub service_process: Uuid,
    pub readiness_process: Uuid,
    pub readiness: CommandOutcome,
    pub terminal: CommandOutcome,
    pub stopped: kegari_execution::ServiceObservation,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckBundleExecutionRecord {
    pub schema_version: u32,
    pub effect_id: Uuid,
    pub plan_digest: String,
    pub services: Vec<ServiceExecutionRecord>,
}
impl CheckBundleExecutionRecord {
    // Descriptive consistency only; the private FailedCheckExecution producer
    // still supplies the actual confined command/unchanged-source authority.
    pub(super) fn services_completed_cleanly(&self) -> bool {
        self.schema_version == 1
            && !self.services.is_empty()
            && self.services.iter().all(|service| {
                [&service.readiness, &service.terminal]
                    .into_iter()
                    .all(|outcome| {
                        outcome.exit_code == Some(0)
                            && outcome.termination_signal.is_none()
                            && !outcome.timed_out
                            && !outcome.output_truncated
                    })
                    && !service.stopped.leader_running
                    && service.stopped.stop_requested
                    && !service.stopped.kill_escalated
            })
    }
}
