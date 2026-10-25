//! Same-process session recovery retains the actual joined completion. Metadata
//! and filesystem proofs do not bypass ordinary authority, capacity or budgets.
use super::*;
use crate::{housekeeping::VerifiedDirectoryReuse, worker_run::VerifiedSessionResume};

impl RuntimeCoordinator {
    pub(crate) fn register_session_resume(
        &self,
        permit: &Permit,
        resume: &crate::worker_run::AdmittedSessionResume,
    ) -> Result<()> {
        self.control.validate_permit(permit, || {
            let completed = self
                .retained_completion(resume.bridge().predecessor())?
                .ok_or_else(|| {
                    Error::new(
                        "reconciliation_required",
                        "resumed registration lost its actual joined predecessor",
                    )
                })?;
            resume.bridge().verify_completion(&completed)?;
            let mut state = self.state.lock().map_err(|_| {
                Error::new("runtime_unavailable", "resume registration lock failed")
            })?;
            let registered = state.runs.get(&permit.operation_id).ok_or_else(|| {
                Error::new(
                    "ownership_conflict",
                    "resume has no registered actual worker",
                )
            })?;
            resume.bridge().verify_contract(&registered.contract)?;
            let digest = canonical_digest(1, &resume.prior_record())?;
            if state
                .resume_proofs
                .get(&permit.operation_id)
                .is_some_and(|old| old != &digest)
            {
                return Err(Error::new(
                    "ownership_conflict",
                    "worker already has another resume origin",
                ));
            }
            state.resume_proofs.insert(permit.operation_id, digest);
            Ok(())
        })
    }
    pub fn next_attempt_for_session_resume(
        &self,
        contract: &WorkerContract,
        proof: &VerifiedSessionResume,
        directory: &VerifiedDirectoryReuse,
    ) -> Result<crate::runtime_schedule::RunSchedule> {
        self.plan_session_resume(contract, proof, directory, &self.control.snapshot()?)
    }

    pub fn admit_resumed_worker(
        &self,
        lease: &Lease,
        input: crate::control::RunAdmission,
        contract: &WorkerContract,
        proof: &VerifiedSessionResume,
        directory: &VerifiedDirectoryReuse,
    ) -> Result<Permit> {
        self.admit_worker_inner(lease, input, contract, None, Some((proof, directory)))
    }

    pub(super) fn session_resume_reference(
        &self,
        proof: &VerifiedSessionResume,
        directory: &VerifiedDirectoryReuse,
    ) -> Result<crate::control::RunResumeReference> {
        let old = proof.recovery().observation().retained();
        let mut rounds = 0;
        for receipt in self.control.operations()? {
            if crate::control::RunReference::from_receipt(&receipt)?
                .is_some_and(|run| run.phase == old.reference().phase)
            {
                if let Some(previous) = crate::control::RunResumeReference::from_receipt(&receipt)?
                {
                    rounds = rounds.max(previous.review_rounds_consumed);
                }
                if let Some(previous) =
                    crate::control::RunContinuationReference::from_receipt(&receipt)?
                {
                    rounds = rounds.max(previous.review_rounds_consumed);
                }
                if let Some(previous) =
                    crate::control::SourceAdvancedReference::from_receipt(&receipt)?
                {
                    rounds = rounds.max(previous.review_rounds_consumed);
                }
            }
        }
        Ok(crate::control::RunResumeReference {
            predecessor: proof.predecessor(),
            predecessor_receipt_digest: old.admission_digest().into(),
            recovery_digest: proof.recovery().digest().into(),
            instruction_compatibility_digest: proof.instructions().digest().into(),
            directory_reuse_digest: directory.digest().into(),
            review_rounds_consumed: rounds,
        })
    }

    pub(super) fn plan_session_resume(
        &self,
        contract: &WorkerContract,
        proof: &VerifiedSessionResume,
        directory: &VerifiedDirectoryReuse,
        snapshot: &crate::control::ControlSnapshot,
    ) -> Result<crate::runtime_schedule::RunSchedule> {
        proof.verify_contract(contract)?;
        directory.revalidate()?;
        let completed = self
            .retained_completion(proof.predecessor())?
            .ok_or_else(|| {
                Error::new(
                    "reconciliation_required",
                    "session resume lost its actual joined worker",
                )
            })?;
        proof.verify_completion(&completed)?;
        let source = completed.execution.as_ref().ok_or_else(|| {
            Error::new("resume_incompatible", "joined worker has no owned source")
        })?;
        let old = proof.recovery().observation().retained();
        let original = old.reference();
        if directory.predecessor() != proof.predecessor()
            || directory.binding() != &source.binding
            || original.phase.work != contract.work_key()?
            || original.phase.phase != contract.instructions().manifest.phase
            || original.workspace != contract.instructions().manifest.workspace_id
            || original.account != contract.account_id()
            || original.profile_digest
                != canonical_digest(1, &contract.instructions().manifest.profile)?
            || snapshot
                .active_operations
                .iter()
                .any(|permit| permit.work_key == original.phase.work)
            || snapshot
                .unresolved_operations
                .values()
                .any(|targets| targets.contains(&original.phase.work))
        {
            return Err(Error::new(
                "resume_incompatible",
                "session resume changed its exact source, account, phase or ownership",
            ));
        }
        self.require_provider_session_clear(&original.account, &original.phase.work)?;
        self.current_admission_authority(contract.authority())?;
        let config = self.configuration.snapshot()?;
        let workspace = config
            .workspaces
            .values()
            .find(|workspace| workspace.id == original.workspace)
            .ok_or_else(|| Error::new("scope_violation", "resume workspace is not configured"))?;
        if config.connections[&workspace.codex_connection].account_id != original.account {
            return Err(Error::new(
                "identity_mismatch",
                "resume cannot replace its provider account",
            ));
        }
        if let Some(error) = self
            .state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "retry proof lock failed"))?
            .retry_errors
            .get(&original.phase.work)
        {
            return Err(error.clone());
        }
        let receipts = self.control.operations()?;
        old.verify_prior_receipts(&receipts)?;
        crate::runtime_schedule::require_provider_clear(
            self.runtime_id,
            &original.account,
            &original.phase.work,
            &receipts,
        )?;
        let preparation = self
            .control
            .operation(proof.recovery().preparation().operation_id)?;
        if canonical_digest(1, &preparation)?
            != canonical_digest(1, proof.recovery().preparation())?
            || preparation.phase != crate::control::ReceiptPhase::Reconciled
        {
            return Err(Error::new(
                "stale_result",
                "session metadata readback is no longer the exact completed preparation",
            ));
        }
        let mut history = BTreeMap::new();
        let mut caps = (
            config.runtime.max_attempts_per_phase,
            config.runtime.max_turns,
            config.runtime.max_review_rounds,
        );
        for receipt in &receipts {
            if let Some(run) = RunReference::from_receipt(receipt)?
                && run.phase == original.phase
            {
                if run.account != original.account
                    || run.workspace != original.workspace
                    || run.profile_digest != original.profile_digest
                    || receipt.runtime_id != self.runtime_id
                    || history.insert(run.attempt, receipt).is_some()
                {
                    return Err(Error::new(
                        "record_corrupt",
                        "resume phase history changed account, profile or identity",
                    ));
                }
                caps.0 = caps.0.min(run.max_attempts);
                caps.1 = caps.1.min(run.max_turns);
                caps.2 = caps.2.min(run.max_review_rounds);
            }
        }
        let (last_attempt, latest) = history.last_key_value().ok_or_else(|| {
            Error::new(
                "no_work_trigger",
                "session resume has no actual prior admission",
            )
        })?;
        if history.keys().copied().ne(1..=history.len() as u32)
            || latest.operation_id != proof.predecessor()
            || canonical_digest(1, latest)? != old.admission_digest()
            || *last_attempt >= caps.0
        {
            return Err(Error::new(
                "attempts_exhausted",
                "session resume requires exact contiguous original budget history with an attempt remaining",
            ));
        }
        let failure_index = history
            .values()
            .rev()
            .take_while(|receipt| {
                receipt
                    .observed_result
                    .as_ref()
                    .is_some_and(|result| result["observation"]["outcome"] == "failed")
            })
            .count()
            .max(1) as u32;
        let not_before = crate::scheduler::retry_at(
            latest.updated_at,
            failure_index,
            config.runtime.retry_cap_ms,
            latest
                .observed_result
                .as_ref()
                .and_then(|value| value.pointer("/observation/error/details/retry_after_ms"))
                .and_then(Value::as_u64),
        )?
        .max(latest.updated_at + Duration::seconds(1));
        if Utc::now() < not_before {
            return Err(Error::new(
                "retry_wait",
                "session resume waits for its bounded failure backoff",
            )
            .detail(json!({"not_before":not_before})));
        }
        Ok(crate::runtime_schedule::RunSchedule {
            attempt: last_attempt + 1,
            max_attempts: caps.0,
            max_turns: caps.1,
            max_review_rounds: caps.2,
            trigger: crate::scheduler::Trigger::ScheduledRetry,
            not_before,
        })
    }
}
