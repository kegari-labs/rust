//! Correction provenance changes effective evidence identity while retaining the
//! original directory, isolation and operation ownership. Freshness remains an
//! enclosing host gate at both process start and effect completion.
use super::*;
use crate::corrections::VerifiedCorrectionTransition;
use std::sync::Arc;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorrectionExecutionProvenance {
    pub original_binding_digest: String,
    pub transition_digest: String,
    pub watermark: u64,
    pub input_digest: String,
}

/// Only a native verified correction transition can produce this capability.
/// It is not reconstructed from a serialized binding or journal record.
#[derive(Clone, Debug)]
pub struct EffectiveExecutionBinding {
    original: ExecutionBinding,
    effective: ExecutionBinding,
    transition: Arc<VerifiedCorrectionTransition>,
}
impl EffectiveExecutionBinding {
    pub fn new(
        original: &ExecutionBinding,
        ownership: &OwnedDirectory,
        isolation: &ValidatedIsolation,
        transition: Arc<VerifiedCorrectionTransition>,
    ) -> Result<Self> {
        original.validate(isolation)?;
        ownership.verify(original, isolation)?;
        if original.work != *transition.original_work()
            || original.work.child.is_none()
            || isolation.plan().role != ExecutionRole::Child
            || original.candidate_vector_digest.is_some()
            || transition
                .original_authority()
                .child_scope()
                .is_none_or(|scope| {
                    scope.write_repository.as_deref() != Some(original.repository.id.as_str())
                })
        {
            return Err(Error::new(
                "correction_execution_binding",
                "correction is not an in-scope child transition for the original owned execution",
            ));
        }
        let mut effective = original.clone();
        effective.work = transition.effective_work().clone();
        effective.validate(isolation)?;
        Ok(Self {
            original: original.clone(),
            effective,
            transition,
        })
    }
    pub fn original(&self) -> &ExecutionBinding {
        &self.original
    }
    pub fn binding(&self) -> &ExecutionBinding {
        &self.effective
    }
    pub fn transition(&self) -> &Arc<VerifiedCorrectionTransition> {
        &self.transition
    }
    pub fn provenance(&self) -> Result<CorrectionExecutionProvenance> {
        Ok(CorrectionExecutionProvenance {
            original_binding_digest: canonical_digest(1, &self.original)?,
            transition_digest: self.transition.digest().into(),
            watermark: self.transition.watermark(),
            input_digest: self.transition.input_digest(),
        })
    }
}
