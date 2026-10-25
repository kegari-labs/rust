//! Durable identity of the preparation that precedes one exact model admission.
//! This record is provenance only; current authority remains a required callback.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparationReference {
    pub worker_operation: Uuid,
    pub workspace: Uuid,
    pub account: String,
    pub phase: crate::scheduler::PhaseKey,
}

impl PreparationReference {
    /// The same stable identity used by launch-intent recovery. It never changes
    /// when the phase, scope or account is edited under an existing worker ID.
    pub fn operation_id(&self) -> Result<Uuid> {
        preparation_operation(self.worker_operation)
    }

    pub fn from_receipt(receipt: &Receipt) -> Result<Option<Self>> {
        let Some(encoded) = receipt.expected.get("worker_preparation") else {
            return Ok(None);
        };
        let value: Self = serde_json::from_str(encoded)
            .map_err(|_| Error::new("record_corrupt", "invalid worker preparation reference"))?;
        value.validate()?;
        if receipt.kind != "admission"
            || receipt.operation_id != value.operation_id()?
            || receipt.request_id != receipt.operation_id
            || receipt.canonical_targets != [value.phase.work.clone()]
            || [
                "run",
                "metadata",
                "continuation",
                "source_advanced",
                "release_resume",
                "session_resume",
                "session_recovery_preparation",
            ]
            .iter()
            .any(|key| receipt.expected.contains_key(*key))
        {
            return Err(Error::new(
                "record_corrupt",
                "worker preparation differs from its exact admission identity",
            ));
        }
        crate::evidence::validate_digest(receipt.expected.get("scope").ok_or_else(|| {
            Error::new("record_corrupt", "worker preparation has no admitted scope")
        })?)?;
        Ok(Some(value))
    }

    pub(super) fn validate(&self) -> Result<()> {
        if self.worker_operation.is_nil()
            || self.workspace.is_nil()
            || self.account.trim().is_empty()
            || self.account.len() > 512
            || self.phase.work.is_empty()
            || self.phase.work.len() > 512
            || self.phase.phase.is_empty()
            || self.phase.phase.len() > 64
            || !self.phase.phase.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
            })
            || self.phase.retry_grant.is_some_and(|id| id.is_nil())
        {
            return Err(Error::new(
                "record_corrupt",
                "invalid worker preparation identity",
            ));
        }
        crate::evidence::validate_digest(&self.phase.generation)
    }
}

pub struct PreparationAdmission {
    pub request_id: Uuid,
    pub work_key: String,
    pub scope_digest: String,
    pub reference: PreparationReference,
}

pub(crate) fn preparation_operation(worker: Uuid) -> Result<Uuid> {
    if worker.is_nil() {
        return Err(Error::invalid(
            "worker preparation requires an operation identity",
        ));
    }
    let digest = canonical_digest(
        1,
        &serde_json::json!({"purpose":"worker_preparation","worker":worker}),
    )?;
    let bytes = hex::decode(digest.rsplit(':').next().unwrap_or_default())
        .map_err(|_| Error::invalid("preparation identity digest is invalid"))?;
    let mut id: [u8; 16] = bytes
        .get(..16)
        .ok_or_else(|| Error::invalid("short preparation identity"))?
        .try_into()
        .map_err(|_| Error::invalid("preparation identity length"))?;
    id[6] = (id[6] & 0x0f) | 0x80;
    id[8] = (id[8] & 0x3f) | 0x80;
    Ok(Uuid::from_bytes(id))
}

impl ControlHost {
    pub fn admit_preparation(
        &self,
        lease: &Lease,
        input: PreparationAdmission,
        fresh_gate: impl FnOnce() -> Result<()>,
    ) -> Result<Permit> {
        self.admit_preparation_observed(lease, input, |_| fresh_gate())
    }

    pub(crate) fn admit_preparation_observed(
        &self,
        lease: &Lease,
        input: PreparationAdmission,
        fresh_gate: impl FnOnce(&ControlSnapshot) -> Result<()>,
    ) -> Result<Permit> {
        input.reference.validate()?;
        crate::evidence::validate_digest(&input.scope_digest)?;
        if input.request_id != input.reference.operation_id()?
            || input.work_key != input.reference.phase.work
        {
            return Err(Error::new(
                "scope_violation",
                "preparation must bind its exact worker operation and canonical work",
            ));
        }
        self.admit_inner(
            lease,
            input.request_id,
            input.work_key,
            input.scope_digest,
            Some(AdmissionReference::Preparation(input.reference)),
            fresh_gate,
        )
    }
}
