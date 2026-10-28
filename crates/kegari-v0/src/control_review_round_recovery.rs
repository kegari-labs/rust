//! Data-only paired implementation metadata/readback and resume linkage.
//! No decoder creates a permit or opaque recovery/source/ownership capability.
//! Dedicated atomic admission separately validates actual protected records.
use super::*;
use kegari_types::run::RunReference;

const RECOVERY_KEY: &str = "review_round_recovery";
const RESUME_KEY: &str = "review_round_resume";
const MAX_ENCODED: usize = 16 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReviewRoundRecoveryReference {
    pub schema_version: u32,
    pub round_reference_digest: String,
    pub round_plan_digest: String,
    pub origin: ReviewReceiptReference,
    pub origin_preparation: ReviewReceiptReference,
    pub predecessor: ReviewReceiptReference,
    pub protected_candidate_digest: String,
    pub terminal_state_digest: String,
    pub classification: TransientFailureCode,
    /// Required for a later resumed failure; not a substituted first source.
    pub previous_resume_digest: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReviewRoundResumeReference {
    pub schema_version: u32,
    pub recovery: ReviewRoundRecoveryReference,
    pub metadata: ReviewReceiptReference,
    pub provider_recovery_digest: String,
    pub instruction_compatibility_digest: String,
    pub directory_reuse_digest: String,
}

impl ReviewRoundRecoveryReference {
    pub(crate) fn validate(&self) -> Result<()> {
        encoded_bound(self)?;
        if self.schema_version != 1
            || self.origin.operation == self.origin_preparation.operation
            || self.origin_preparation.operation != preparation_operation(self.origin.operation)?
            || (self.origin.operation == self.predecessor.operation
                && self.origin.receipt_digest != self.predecessor.receipt_digest)
            || (self.previous_resume_digest.is_none()
                != (self.predecessor.operation == self.origin.operation))
        {
            return Err(recovery_error("invalid exact paired recovery origin"));
        }
        for reference in [&self.origin, &self.origin_preparation, &self.predecessor] {
            validate_receipt_reference(reference)?;
        }
        for digest in [
            &self.round_reference_digest,
            &self.round_plan_digest,
            &self.protected_candidate_digest,
            &self.terminal_state_digest,
        ] {
            crate::evidence::validate_digest(digest)?;
        }
        if let Some(digest) = &self.previous_resume_digest {
            crate::evidence::validate_digest(digest)?;
        }
        Ok(())
    }
    pub(crate) fn from_metadata_receipt(receipt: &Receipt) -> Result<Option<Self>> {
        let Some(value) = decode::<Self>(receipt, RECOVERY_KEY)? else {
            return Ok(None);
        };
        value.validate()?;
        let round = round_data(receipt, &value.round_reference_digest)?;
        let preparation: SessionRecoveryPreparationReference =
            decode(receipt, "session_recovery_preparation")?.ok_or_else(|| {
                recovery_error("paired metadata lacks actual recovery preparation")
            })?;
        if receipt.operation_id != preparation.operation_id()?
            || preparation.predecessor != value.predecessor.operation
            || preparation.predecessor_receipt_digest != value.predecessor.receipt_digest
            || preparation.candidate_digest != value.protected_candidate_digest
            || preparation.phase != *round.subject_phase()
            || preparation.workspace != round.workspace
            || preparation.account != round.account
            || [
                "run",
                "worker_preparation",
                "metadata",
                "session_resume",
                RESUME_KEY,
                "continuation",
                "source_advanced",
                "release_resume",
                "review_round_preparation",
            ]
            .iter()
            .any(|key| receipt.expected.contains_key(*key))
        {
            return Err(recovery_error(
                "paired metadata changed its exact recovery admission",
            ));
        }
        Ok(Some(value))
    }
}
impl ReviewRoundResumeReference {
    pub(crate) fn validate(&self) -> Result<()> {
        encoded_bound(self)?;
        self.recovery.validate()?;
        validate_receipt_reference(&self.metadata)?;
        if self.schema_version != 1
            || self.metadata.operation == self.recovery.predecessor.operation
            || self.metadata.operation == self.recovery.origin_preparation.operation
        {
            return Err(recovery_error(
                "invalid paired model-resume metadata identity",
            ));
        }
        for digest in [
            &self.provider_recovery_digest,
            &self.instruction_compatibility_digest,
            &self.directory_reuse_digest,
        ] {
            crate::evidence::validate_digest(digest)?;
        }
        Ok(())
    }
    pub(crate) fn from_model_receipt(receipt: &Receipt) -> Result<Option<Self>> {
        let Some(value) = decode::<Self>(receipt, RESUME_KEY)? else {
            return Ok(None);
        };
        value.validate()?;
        let round = round_data(receipt, &value.recovery.round_reference_digest)?;
        let run: RunReference = decode(receipt, "run")?
            .ok_or_else(|| recovery_error("paired resume has no actual run reference"))?;
        crate::control::validate_run_reference(&run)?;
        // Direct bounded data decoding avoids recursion between marker readers.
        // The generic reader delegates this exact shape here; this reader
        // must never call it back.
        let resume: RunResumeReference = decode(receipt, "session_resume")?
            .ok_or_else(|| recovery_error("paired resume lacks provider-session linkage"))?;
        resume.validate()?;
        if run.phase != *round.subject_phase()
            || run.workspace != round.workspace
            || run.account != round.account
            || run.attempt < 2
            || receipt.operation_id == value.recovery.predecessor.operation
            || resume.predecessor != value.recovery.predecessor.operation
            || resume.predecessor_receipt_digest != value.recovery.predecessor.receipt_digest
            || resume.recovery_digest != value.provider_recovery_digest
            || resume.instruction_compatibility_digest != value.instruction_compatibility_digest
            || resume.directory_reuse_digest != value.directory_reuse_digest
            || resume.review_rounds_consumed < round.ordinal
            || [
                "worker_preparation",
                "metadata",
                "session_recovery_preparation",
                RECOVERY_KEY,
                "continuation",
                "source_advanced",
                "release_resume",
                "review_round_preparation",
            ]
            .iter()
            .any(|key| receipt.expected.contains_key(*key))
        {
            return Err(recovery_error(
                "paired resume differs from its exact original phase or proofs",
            ));
        }
        Ok(Some(value))
    }
}
pub(super) fn paired_round(receipt: &Receipt) -> Result<Option<ReviewRoundReference>> {
    if let Some(recovery) = ReviewRoundRecoveryReference::from_metadata_receipt(receipt)? {
        return round_data(receipt, &recovery.round_reference_digest).map(Some);
    }
    if let Some(resume) = ReviewRoundResumeReference::from_model_receipt(receipt)? {
        return round_data(receipt, &resume.recovery.round_reference_digest).map(Some);
    }
    Ok(None)
}
fn encoded_bound(value: &impl Serialize) -> Result<()> {
    if serde_json::to_vec(value)
        .map_err(|_| recovery_error("paired recovery encoding failed"))?
        .len()
        > MAX_ENCODED
    {
        return Err(Error::new(
            "review_round_history_limit",
            "paired recovery exceeds protected encoded-record bound",
        ));
    }
    Ok(())
}
fn validate_receipt_reference(reference: &ReviewReceiptReference) -> Result<()> {
    if reference.operation.is_nil() {
        return Err(recovery_error("nil paired receipt identity"));
    }
    crate::evidence::validate_digest(&reference.receipt_digest)
}
fn decode<T: serde::de::DeserializeOwned>(receipt: &Receipt, key: &str) -> Result<Option<T>> {
    let Some(encoded) = receipt.expected.get(key) else {
        return Ok(None);
    };
    if encoded.len() > MAX_ENCODED {
        return Err(recovery_error("paired recovery reference exceeds bound"));
    }
    serde_json::from_str(encoded)
        .map(Some)
        .map_err(|_| recovery_error("malformed paired recovery reference"))
}
fn round_data(receipt: &Receipt, digest: &str) -> Result<ReviewRoundReference> {
    let round: ReviewRoundReference = decode(receipt, "review_round")?
        .ok_or_else(|| recovery_error("paired recovery has no immutable round reference"))?;
    round.validate()?;
    crate::evidence::validate_digest(
        receipt
            .expected
            .get("scope")
            .ok_or_else(|| recovery_error("paired recovery scope is missing"))?,
    )?;
    if !round.is_repair()
        || round.subject_phase().phase != "implementation"
        || receipt.kind != "admission"
        || receipt.runtime_id != round.runtime
        || receipt.operation_id.is_nil()
        || receipt.request_id != receipt.operation_id
        || receipt.canonical_targets != [round.subject_phase().work.clone()]
        || canonical_digest(1, &round)? != digest
    {
        return Err(recovery_error(
            "paired recovery changed round, runtime or parent work",
        ));
    }
    Ok(round)
}
fn recovery_error(message: &str) -> Error {
    Error::new("paired_recovery_lineage", message)
}

#[cfg(test)]
#[path = "control_review_round_recovery_tests.rs"]
mod tests;

#[path = "control_review_paired_result.rs"]
mod paired_result;
pub(crate) use paired_result::PairedResumeResultReference;
