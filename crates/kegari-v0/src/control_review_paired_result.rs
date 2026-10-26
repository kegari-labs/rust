//! Durable data for one actual completed resumed repair. It cannot reconstruct
//! the admitted provider bridge, source, checks or joined execution.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PairedResumeResultReference {
    pub(crate) schema_version: u32,
    pub(crate) round_reference_digest: String,
    pub(crate) round_plan_digest: String,
    pub(crate) origin: ReviewReceiptReference,
    pub(crate) origin_preparation: ReviewReceiptReference,
    pub(crate) terminal: ReviewReceiptReference,
    pub(crate) resume_reference_digest: String,
    pub(crate) history_digest: String,
}
impl PairedResumeResultReference {
    pub(crate) fn validate(&self) -> Result<()> {
        encoded_bound(self)?;
        if self.schema_version != 1
            || self.origin.operation == self.terminal.operation
            || self.origin_preparation.operation != preparation_operation(self.origin.operation)?
        {
            return Err(recovery_error("invalid completed paired resume identity"));
        }
        for r in [&self.origin, &self.origin_preparation, &self.terminal] {
            validate_receipt_reference(r)?;
        }
        for digest in [
            &self.round_reference_digest,
            &self.round_plan_digest,
            &self.resume_reference_digest,
            &self.history_digest,
        ] {
            crate::evidence::validate_digest(digest)?;
        }
        Ok(())
    }
}
