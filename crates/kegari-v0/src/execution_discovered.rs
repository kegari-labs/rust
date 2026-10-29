//! Historical custom-executor plan records remain decodable; they cannot be
//! promoted into new native Codex evidence. No command or resource runner remains.
use super::*;
use crate::validation::{ValidationPlan, ValidationPlanRecord};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveredServiceObservation {
    pub name: String,
    pub start: Value,
    pub readiness: Value,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveredCheckProvenance {
    pub schema_version: u32,
    pub plan: ValidationPlanRecord,
    pub plan_digest: String,
    pub invocation_id: Uuid,
    pub observation: Value,
    pub service_observations: Vec<DiscoveredServiceObservation>,
    pub prepared_at: DateTime<Utc>,
    pub deadline_at: DateTime<Utc>,
    pub command_timeout_ms: u64,
    pub output_bytes: usize,
    pub transcript: PathBuf,
    pub artifacts: Vec<PathBuf>,
}
pub(super) fn verify_capability(
    record: &CheckExecutionRecord,
    plan: Option<&ValidationPlan>,
) -> Result<()> {
    if let Some(codex) = &record.codex {
        return match (&codex.plan, plan) {
            (None, None) => Ok(()),
            (Some(retained), Some(actual))
                if canonical_digest(1, retained)? == actual.digest()
                    && canonical_digest(1, actual.record())? == actual.digest() =>
            {
                Ok(())
            }
            _ => Err(Error::new(
                "validation_plan_binding",
                "Codex check lacks its original private validation enrollment",
            )),
        };
    }
    match (&record.discovered, plan) {
        (None, None) => Ok(()),
        _ => Err(Error::new(
            "legacy_execution_evidence",
            "legacy custom-executor records cannot establish Codex validation",
        )),
    }
}
