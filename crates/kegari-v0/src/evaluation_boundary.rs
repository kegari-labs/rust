//! Diagnostic data only. Decoding this does not authorize any operation or
//! qualify native containment, acceptance, source reuse or a successful trial.
use crate::{evidence::validate_digest, worker::ReviewTarget};
use kegari_types::error::{Error, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndependentBoundaryObservation {
    pub schema_version: u32,
    /// Actual collected HEAD; an unavailable diagnostic may differ from admission.
    pub source_revision: String,
    /// Actual final snapshot, not continuous filesystem monitoring.
    pub candidate_digest: String,
    pub check: IndependentBoundaryCheck,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum IndependentBoundaryCheck {
    /// Original registration and retained authority matched at this endpoint.
    /// This says nothing about complete reads/effects/output or native exit.
    MatchedAtCollection {
        identity: Box<IndependentBoundaryIdentity>,
    },
    Unavailable {
        error: Error,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndependentBoundaryIdentity {
    pub operation: Uuid,
    pub runtime: Uuid,
    pub workspace: Uuid,
    pub owner_generation: u64,
    pub owner: Uuid,
    pub checked_at: chrono::DateTime<chrono::Utc>,
    pub permit_digest: String,
    pub admission_request_digest: String,
    pub contract_scope_digest: String,
    pub authority_digest: String,
    pub current_authority_digest: String,
    pub review_identity: String,
    pub target: ReviewTarget,
    pub source_revision: String,
    pub instruction_manifest_digest: String,
    pub context_digest: String,
    pub context_keys: Vec<String>,
    pub tool_contract_digest: String,
    pub isolation_digest: String,
    pub permission_digest: String,
}
impl IndependentBoundaryObservation {
    /// Shape validation only. Valid diagnostic data is not runtime authority.
    pub fn validate(&self) -> Result<()> {
        let invalid = || {
            Error::new(
                "boundary_observation",
                "independent boundary observation is malformed",
            )
        };
        if self.schema_version != 1 || !oid(&self.source_revision) {
            return Err(invalid());
        }
        validate_digest(&self.candidate_digest)?;
        if let IndependentBoundaryCheck::MatchedAtCollection { identity: i } = &self.check {
            if i.operation.is_nil()
                || i.runtime.is_nil()
                || i.workspace.is_nil()
                || i.owner.is_nil()
                || i.owner_generation == 0
                || i.target.pull_request == 0
                || i.target.repository.id == 0
                || i.source_revision != self.source_revision
                || i.target.head != self.source_revision
                || !oid(&i.target.base)
                || i.authority_digest != i.current_authority_digest
                || i.context_keys.windows(2).any(|keys| keys[0] >= keys[1])
                || i.context_keys.iter().any(|key| key.is_empty())
            {
                return Err(invalid());
            }
            for digest in [
                &i.permit_digest,
                &i.admission_request_digest,
                &i.contract_scope_digest,
                &i.authority_digest,
                &i.current_authority_digest,
                &i.review_identity,
                &i.instruction_manifest_digest,
                &i.context_digest,
                &i.tool_contract_digest,
                &i.isolation_digest,
                &i.permission_digest,
            ] {
                validate_digest(digest)?;
            }
        }
        if serde_json::to_vec(self).map_err(|_| invalid())?.len() > 65_536 {
            return Err(Error::new(
                "boundary_observation",
                "independent boundary diagnostic exceeds 64 KiB",
            ));
        }
        Ok(())
    }
}
fn oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
