//! Protected retained-session boundary and actual Control receipt lineage.
use crate::control::{Receipt, RunResumeReference, run_reference_from_receipt};
use kegari_codex::{ResumeCandidate, SessionState};
use kegari_digest::canonical_digest;
use kegari_types::{
    error::{Error, Result},
    run::RunReference,
};
use uuid::Uuid;

/// Retained session obtained through the backend's protected journal checks.
/// Consistent protocol data alone cannot construct this native recovery input.
#[derive(Clone, Debug)]
pub struct RetainedResumeSession {
    candidate: ResumeCandidate,
    validation: Option<super::ColdValidationCheckpoint>,
}
impl RetainedResumeSession {
    pub(crate) fn from_journal(
        state: SessionState,
        admission_digest: String,
        reference: RunReference,
        journal_digest: String,
    ) -> Result<Self> {
        Ok(Self {
            validation: None,
            candidate: ResumeCandidate::from_retained_parts(
                state,
                admission_digest,
                reference,
                journal_digest,
            )?,
        })
    }
    pub(crate) fn with_validation(
        mut self,
        checkpoint: Option<super::ColdValidationCheckpoint>,
    ) -> Self {
        self.validation = checkpoint;
        self
    }
    pub(crate) fn validation(&self) -> Option<&super::ColdValidationCheckpoint> {
        self.validation.as_ref()
    }
    pub(crate) fn protocol_candidate(&self) -> &ResumeCandidate {
        &self.candidate
    }
    pub fn state(&self) -> &SessionState {
        self.candidate.state()
    }
    pub fn admission_digest(&self) -> &str {
        self.candidate.admission_digest()
    }
    pub fn reference(&self) -> &RunReference {
        self.candidate.reference()
    }
    pub fn journal_digest(&self) -> &str {
        self.candidate.journal_digest()
    }
    pub fn digest(&self) -> &str {
        self.candidate.digest()
    }
    pub(crate) fn verify_prior_receipts(&self, receipts: &[Receipt]) -> Result<()> {
        let operation: Uuid = self
            .state()
            .binding
            .operation_id
            .parse()
            .map_err(|_| resume_history_error("candidate operation is invalid"))?;
        let current = receipts
            .iter()
            .find(|r| r.operation_id == operation)
            .ok_or_else(|| resume_history_error("candidate actual admission disappeared"))?;
        if canonical_digest(1, current)? != self.admission_digest() {
            return Err(resume_history_error("candidate actual receipt changed"));
        }
        for segment in self.candidate.prior_lineage() {
            let predecessor: Uuid = segment
                .operation_id()
                .parse()
                .map_err(|_| resume_history_error("historical operation is invalid"))?;
            let prior = receipts
                .iter()
                .find(|r| r.operation_id == predecessor)
                .ok_or_else(|| resume_history_error("historical admission disappeared"))?;
            let run = run_reference_from_receipt(prior)?
                .ok_or_else(|| resume_history_error("historical run reference disappeared"))?;
            let next = receipts
                .iter()
                .find(|r| r.operation_id == segment.successor())
                .ok_or_else(|| {
                    resume_history_error("historical successor admission disappeared")
                })?;
            let resume = RunResumeReference::from_receipt(next)?.ok_or_else(|| {
                resume_history_error("historical successor lost its typed resume admission")
            })?;
            if prior.runtime_id != current.runtime_id
                || next.runtime_id != current.runtime_id
                || canonical_digest(1, prior)? != segment.admission_digest()
                || canonical_digest(1, &run)? != canonical_digest(1, segment.reference())?
                || resume.predecessor() != predecessor
                || resume.predecessor_receipt_digest != segment.admission_digest()
                || resume.recovery_digest() != segment.recovery_digest()
                || resume.instruction_compatibility_digest
                    != segment.instruction_compatibility_digest()
            {
                return Err(resume_history_error(
                    "historical submitted-input chain differs from actual Control lineage",
                ));
            }
        }
        Ok(())
    }
}
fn resume_history_error(message: &str) -> Error {
    Error::new("resume_incompatible", message)
}

#[cfg(test)]
#[path = "worker_recovery_retained_tests.rs"]
mod tests;
