//! Native identities retained from actual registered round/resume admissions.
//! Receipt data is only checked against these capabilities, never used to create
//! a missing plan, bridge, joined result or directory enrollment.
use super::*;

impl RuntimeCoordinator {
    pub(crate) fn register_review_round(
        &self,
        permit: &Permit,
        contract: &Arc<WorkerContract>,
        round: Arc<crate::delivery::VerifiedReviewRoundPlan>,
    ) -> Result<()> {
        self.control.validate_permit(permit, || {
            self.registration.verify_active_owner(&permit.lease)?;
            round.verify_contract(contract)?;
            round.verify_joined(&|id| self.retained_completion(id))?;
            round.verify_resumed_bridges(&|id| self.retained_session_resume(id))?;
            let current = self.current_admission_authority(contract.authority())?;
            WorkerAuthority::Authorized(round.authority().clone()).compare(&current)?;
            let receipt = self.control.operation(permit.operation_id)?;
            if crate::control::ReviewRoundReference::from_receipt(&receipt)?.as_ref()
                != Some(round.reference())
                || receipt.expected.contains_key("review_round_resume")
            {
                return Err(paired_error(
                    "registered round differs from actual original admission",
                ));
            }
            let preparation = crate::control::ReviewReceiptReference::preparation_for(&receipt)?;
            preparation.matches_observation(&self.control.operation(preparation.operation)?)?;
            let mut state = self.state.lock().map_err(|_| paired_lock())?;
            let registered = state
                .runs
                .get(&permit.operation_id)
                .ok_or_else(|| paired_error("round has no actual registered worker"))?;
            if !Arc::ptr_eq(&registered.contract, contract)
                || registered.permit.operation_id != permit.operation_id
                || registered.permit.scope_digest != permit.scope_digest
                || registered.budget.phase != *round.subject_phase()
                || registered.joined
                || state
                    .admitted_rounds
                    .get(&permit.operation_id)
                    .is_some_and(|old| !Arc::ptr_eq(old, &round))
            {
                return Err(paired_error(
                    "round registration changed actual contract, permit or plan",
                ));
            }
            if state.admitted_rounds.len() >= 65_536
                && !state.admitted_rounds.contains_key(&permit.operation_id)
            {
                return Err(Error::new(
                    "record_limit",
                    "retained round inventory exceeds its bound",
                ));
            }
            state
                .admitted_rounds
                .entry(permit.operation_id)
                .or_insert(round);
            Ok(())
        })
    }

    pub(crate) fn register_paired_resume_round(
        &self,
        permit: &Permit,
        contract: &Arc<WorkerContract>,
        candidate: &crate::delivery::VerifiedPairedRecoveryCandidate,
        admitted: &crate::worker_run::AdmittedSessionResume,
    ) -> Result<()> {
        self.control.validate_permit(permit, || {
            self.registration.verify_active_owner(&permit.lease)?;
            admitted.bridge().verify_contract(contract)?;
            admitted.bridge().verify_completion(candidate.failed())?;
            let receipt = self.control.operation(permit.operation_id)?;
            let resumed = crate::control::ReviewRoundResumeReference::from_model_receipt(&receipt)?
                .ok_or_else(|| {
                    paired_error("paired resume registration has no actual typed admission")
                })?;
            if crate::control::ReviewRoundReference::from_receipt(&receipt)?.as_ref()
                != Some(candidate.plan().reference())
                || &resumed.recovery != candidate.provenance()
                || resumed.provider_recovery_digest != admitted.bridge().recovery().digest()
                || resumed.instruction_compatibility_digest
                    != admitted.bridge().instructions().digest()
            {
                return Err(paired_error(
                    "paired resume registration changed its original plan or actual bridge",
                ));
            }
            let mut state = self.state.lock().map_err(|_| paired_lock())?;
            let run = state
                .runs
                .get(&permit.operation_id)
                .ok_or_else(|| paired_error("paired resume is not registered"))?;
            let bridge = state
                .admitted_resumes
                .get(&permit.operation_id)
                .ok_or_else(|| paired_error("paired resume lost its registered admitted bridge"))?;
            if !Arc::ptr_eq(&run.contract, contract)
                || run.joined
                || run.budget.phase != *candidate.phase()
                || canonical_digest(1, &bridge.prior_record())?
                    != canonical_digest(1, &admitted.prior_record())?
                || state
                    .admitted_rounds
                    .get(&permit.operation_id)
                    .is_some_and(|old| !Arc::ptr_eq(old, candidate.plan()))
            {
                return Err(paired_error(
                    "paired resume differs from actual registered contract or bridge",
                ));
            }
            if state.admitted_rounds.len() >= 65_536
                && !state.admitted_rounds.contains_key(&permit.operation_id)
            {
                return Err(Error::new(
                    "record_limit",
                    "retained round inventory exceeds its bound",
                ));
            }
            state
                .admitted_rounds
                .entry(permit.operation_id)
                .or_insert_with(|| candidate.plan().clone());
            Ok(())
        })
    }

    /// Clone only the capability retained during its actual native admission.
    /// Ancestors remain available after later resumes in the same directory.
    pub(crate) fn retained_session_resume(
        &self,
        operation: Uuid,
    ) -> Result<Option<Arc<crate::worker_run::AdmittedSessionResume>>> {
        Ok(self
            .state
            .lock()
            .map_err(|_| paired_lock())?
            .admitted_resumes
            .get(&operation)
            .cloned())
    }

    pub(crate) fn retained_review_round(
        &self,
        operation: Uuid,
    ) -> Result<Option<Arc<crate::delivery::VerifiedReviewRoundPlan>>> {
        let state = self.state.lock().map_err(|_| paired_lock())?;
        let Some(round) = state.admitted_rounds.get(&operation) else {
            return Ok(None);
        };
        let registered = state
            .runs
            .get(&operation)
            .ok_or_else(|| paired_error("retained round lost registered worker"))?;
        let completed = state
            .completions
            .get(&operation)
            .ok_or_else(|| paired_error("retained round has no actual joined result"))?;
        if !registered.joined
            || completed.operation_id != operation
            || completed
                .contract
                .as_ref()
                .is_none_or(|contract| !Arc::ptr_eq(contract, &registered.contract))
            || registered.budget.phase != *round.subject_phase()
        {
            return Err(paired_error(
                "retained round changed actual joined contract or phase",
            ));
        }
        Ok(Some(round.clone()))
    }
    pub(super) fn verify_paired_recovery(
        &self,
        candidate: &crate::delivery::VerifiedPairedRecoveryCandidate,
        current: &crate::reconcile::FreshGate,
        config: &crate::config::Config,
        receipts: &[crate::control::Receipt],
        grants: &[crate::decisions::VerifiedRetryGrant],
    ) -> Result<()> {
        let current_gate = |gate: &crate::reconcile::FreshGate| {
            let expected = WorkerAuthority::Authorized(gate.clone());
            expected.compare(&self.current_admission_authority(&expected)?)
        };
        let joined = |id| self.retained_completion(id);
        let resumed = |id| self.retained_session_resume(id);
        let readers = crate::delivery::PairedRecoveryReaders {
            current_gate: &current_gate,
            joined: &joined,
            resumed: &resumed,
        };
        candidate.verify_current(receipts, grants, config, current, &readers)
    }

    pub(crate) fn next_attempt_for_paired_session_resume(
        &self,
        contract: &WorkerContract,
        proof: &crate::worker_run::VerifiedSessionResume,
        directory: &crate::housekeeping::VerifiedDirectoryReuse,
        candidate: &crate::delivery::VerifiedPairedRecoveryCandidate,
    ) -> Result<crate::runtime_schedule::RunSchedule> {
        self.plan_paired_session_resume(
            contract,
            proof,
            directory,
            candidate,
            &self.control.snapshot()?,
        )
    }

    pub(super) fn plan_paired_session_resume(
        &self,
        contract: &WorkerContract,
        proof: &crate::worker_run::VerifiedSessionResume,
        directory: &crate::housekeeping::VerifiedDirectoryReuse,
        candidate: &crate::delivery::VerifiedPairedRecoveryCandidate,
        snapshot: &crate::control::ControlSnapshot,
    ) -> Result<crate::runtime_schedule::RunSchedule> {
        let config = self.configuration.snapshot()?;
        let receipts = self.control.operations()?;
        let authority = self.current_admission_authority(contract.authority())?;
        let WorkerAuthority::Authorized(current) = &authority else {
            return Err(paired_error(
                "paired resume requires current child authority",
            ));
        };
        let grants = crate::runtime_workflow::current_retry_grants(
            &config,
            &self.observations.admission_snapshot()?,
            current.work(),
            &receipts,
            &self.services.key,
        )?;
        self.verify_paired_recovery(candidate, current, &config, &receipts, &grants)?;
        candidate.plan().verify_contract(contract)?;
        proof.verify_completion(candidate.failed())?;
        candidate.resume_reference(proof, directory, &receipts)?;
        // This planner retains implementation scalar caps; candidate verification
        // above separately checks the review namespace ordinal and native grant.
        self.plan_session_resume(contract, proof, directory, snapshot)
    }

    pub(crate) fn admit_paired_resumed_worker(
        &self,
        lease: &Lease,
        input: crate::control::RunAdmission,
        contract: &WorkerContract,
        proof: &crate::worker_run::VerifiedSessionResume,
        directory: &crate::housekeeping::VerifiedDirectoryReuse,
        candidate: &crate::delivery::VerifiedPairedRecoveryCandidate,
    ) -> Result<Permit> {
        self.admit_worker_context(
            lease,
            input,
            contract,
            WorkerAdmissionContext {
                planning: WorkerPlanning {
                    readmission: None,
                    round: None,
                },
                session_resume: Some((proof, directory)),
                paired_resume: Some(candidate),
                parent_reassembly: None,
                preparation: None,
            },
        )
    }
}

pub(super) fn paired_error(message: &str) -> Error {
    Error::new("paired_recovery_lineage", message)
}
fn paired_lock() -> Error {
    Error::new(
        "runtime_unavailable",
        "paired recovery capability registry lock failed",
    )
}
