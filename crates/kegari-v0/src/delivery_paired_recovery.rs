//! Paired-session recovery uses the existing opaque round/history verifier
//! as its only lineage basis.
//! Actual admitted bridges extend the original failed paired session. Every
//! bridge and joined result is retained by identity; raw receipts only describe
//! their immutable lineage. This module cannot launch a model or mint a source.
use super::*;
use crate::{
    codex::RetainedResumeSession,
    control::{
        ControlHost, PairedResumeResultReference, ReviewRoundRecoveryReference,
        ReviewRoundResumeReference,
    },
    housekeeping::VerifiedDirectoryReuse,
    worker_run::{AdmittedSessionResume, VerifiedProviderRecovery, VerifiedSessionResume},
};

pub(crate) struct PairedRecoveryInput<'a> {
    pub plan: Arc<VerifiedReviewRoundPlan>,
    pub failed: Arc<RunCompletion>,
    pub protected: RetainedResumeSession,
    pub current: &'a FreshGate,
    pub config: &'a Config,
    pub grants: &'a [VerifiedRetryGrant],
}

/// Native lookups must come from the same actual RuntimeCoordinator. The
/// callbacks are cache-only when invoked under Control's serialized final gate.
pub(crate) struct PairedRecoveryReaders<'a> {
    pub current_gate: &'a dyn Fn(&FreshGate) -> Result<()>,
    pub joined: &'a dyn Fn(Uuid) -> Result<Option<Arc<RunCompletion>>>,
    pub resumed: &'a dyn Fn(Uuid) -> Result<Option<Arc<AdmittedSessionResume>>>,
}

/// Same-process candidate only. No Deserialize/public constructor and no raw
/// receipt/summary can create this type. The native runtime must supply its
/// genuine joined reader at creation AND every metadata/source/model final gate.
#[derive(Clone)]
pub(crate) struct VerifiedPairedRecoveryCandidate {
    plan: Arc<VerifiedReviewRoundPlan>,
    failed: Arc<RunCompletion>,
    protected: RetainedResumeSession,
    receipt: Receipt,
    chain: PairedResumeChain,
    provenance: ReviewRoundRecoveryReference,
    config_revision: String,
    review_inventory: String,
    implementation_inventory: String,
    digest: String,
}
impl VerifiedPairedRecoveryCandidate {
    pub(crate) fn plan(&self) -> &Arc<VerifiedReviewRoundPlan> {
        &self.plan
    }
    pub(crate) fn failed(&self) -> &Arc<RunCompletion> {
        &self.failed
    }
    pub(crate) fn provenance(&self) -> &ReviewRoundRecoveryReference {
        &self.provenance
    }
    pub(crate) fn digest(&self) -> &str {
        &self.digest
    }
    pub(crate) fn phase(&self) -> &PhaseKey {
        &self.protected.reference().phase
    }

    pub(crate) fn verify_joined(&self, readers: &PairedRecoveryReaders<'_>) -> Result<()> {
        self.plan.verify_joined(&|id| (readers.joined)(id))?;
        self.plan.verify_resumed_bridges(readers.resumed)?;
        self.chain.verify_joined(readers)?;
        require_joined(&self.failed, &|id| (readers.joined)(id))
    }

    /// Cache-only final-gate checks. Actual fresh authority creation, absence of
    /// live/unknown effects, model/metadata attempt accounting, Retry-After and
    /// capacity stay in the existing runtime + Control recovery planners.
    pub(crate) fn verify_current(
        &self,
        receipts: &[Receipt],
        grants: &[VerifiedRetryGrant],
        config: &Config,
        current: &FreshGate,
        readers: &PairedRecoveryReaders<'_>,
    ) -> Result<()> {
        (readers.current_gate)(current)?;
        self.verify_joined(readers)?;
        self.provenance.validate()?;
        if config.revision()? != self.config_revision
            || current.config_revision() != self.config_revision
            || !self.plan.reference().is_repair()
            || self.plan.reference().ordinal > config.runtime.max_review_rounds
            || inventory(receipts, self.plan.reference())? != self.review_inventory
            || implementation_inventory(receipts, self.plan.reference())?
                != self.implementation_inventory
        {
            return Err(recovery_blocked(
                "paired recovery configuration, round or inventory changed",
            ));
        }
        self.plan.authority().compare_admitted_authority(current)?;
        self.chain
            .verify_current(&self.plan, receipts, grants, "failed", readers)?;
        self.protected.verify_prior_receipts(receipts)?;
        verify_failed_candidate(
            &self.plan,
            &self.failed,
            &self.protected,
            &self.receipt,
            self.chain.is_resumed(),
        )?;
        let reference = self.protected.reference();
        if reference.attempt
            >= reference
                .max_attempts
                .min(config.runtime.max_attempts_per_phase)
        {
            return Err(Error::new(
                "attempts_exhausted",
                "original implementation budget has no resumed attempt",
            ));
        }
        // Review grant can bound the paired namespace but never renew the
        // implementation phase. Any successor implementation grant supersedes it.
        if grants.iter().any(|grant| grant.phase() == &reference.phase) {
            return Err(recovery_blocked(
                "paired implementation budget was superseded",
            ));
        }
        if let Some(id) = reference.phase.retry_grant {
            let grant = grants
                .iter()
                .find(|grant| {
                    grant.record().operation_id == id && grant.phase_key() == reference.phase
                })
                .ok_or_else(|| recovery_blocked("original implementation grant disappeared"))?;
            crate::runtime_workflow::require_current_grant(grant, grants)?;
            if reference.max_attempts > grant.new_budget().max_attempts
                || reference.max_turns > grant.new_budget().max_turns
            {
                return Err(recovery_blocked(
                    "paired recovery exceeds original implementation grant",
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn verify_provider(
        &self,
        provider: &VerifiedProviderRecovery,
        receipts: &[Receipt],
    ) -> Result<()> {
        let preparation = actual_receipt(receipts, provider.preparation().operation_id)?;
        let decoded = ReviewRoundRecoveryReference::from_metadata_receipt(preparation)?
            .ok_or_else(|| recovery_blocked("readback lacks actual paired metadata admission"))?;
        if decoded != self.provenance
            || canonical_digest(1, preparation)? != canonical_digest(1, provider.preparation())?
            || preparation.phase != ReceiptPhase::Reconciled
            || preparation.observed_result.as_ref().is_none_or(|observed| {
                observed["local_execution"] != "exited"
                    || observed["unresolved_effects"]
                        .as_array()
                        .is_none_or(|effects| !effects.is_empty())
                    || observed["observation"]["metadata_recovery"] != "verified"
                    || observed["observation"]["local_exit_confirmed"] != true
                    || observed["observation"]["proof_digest"] != provider.observation().digest()
            })
            || provider.observation().retained().digest() != self.protected.digest()
            || canonical_digest(1, provider.observation().retained().state())?
                != self.provenance.terminal_state_digest
            || classification(provider.observation().transient_failure())
                != self.provenance.classification
        {
            return Err(recovery_blocked(
                "metadata recovery changed exact candidate or observed terminal history",
            ));
        }
        Ok(())
    }

    /// Produces data for the eventual round-qualified Control admission only.
    /// This does not return AdmittedSessionResume, enroll a directory, admit a
    /// model or substitute for the native consumer's actual joined callback.
    pub(crate) fn resume_reference(
        &self,
        bridge: &VerifiedSessionResume,
        directory: &VerifiedDirectoryReuse,
        receipts: &[Receipt],
    ) -> Result<ReviewRoundResumeReference> {
        bridge.verify_completion(&self.failed)?;
        self.verify_provider(bridge.recovery(), receipts)?;
        directory.revalidate()?;
        let execution = self.failed.execution.as_ref().ok_or_else(|| {
            recovery_blocked("failed implementation lost original owned execution")
        })?;
        if directory.predecessor() != self.failed.operation_id
            || directory.binding() != &execution.binding
        {
            return Err(recovery_blocked(
                "paired recovery changed actual owned source or directory family",
            ));
        }
        let reference = ReviewRoundResumeReference {
            schema_version: 1,
            recovery: self.provenance.clone(),
            metadata: receipt_reference(bridge.recovery().preparation())?,
            provider_recovery_digest: bridge.recovery().digest().into(),
            instruction_compatibility_digest: bridge.instructions().digest().into(),
            directory_reuse_digest: directory.digest().into(),
        };
        reference.validate()?;
        Ok(reference)
    }
}
impl VerifiedPairedRecoveryCandidate {
    pub(crate) fn observe(
        control: &ControlHost,
        input: PairedRecoveryInput<'_>,
        readers: &PairedRecoveryReaders<'_>,
    ) -> Result<Self> {
        (readers.current_gate)(input.current)?;
        input.plan.verify_joined(&|id| (readers.joined)(id))?;
        require_joined(&input.failed, &|id| (readers.joined)(id))?;
        let receipts = control.operations()?;
        let receipt = actual_receipt(&receipts, input.failed.operation_id)?.clone();
        let chain = PairedResumeChain::observe(
            &input.plan,
            &receipts,
            input.grants,
            &receipt,
            "failed",
            readers,
        )?;
        verify_failed_candidate(
            &input.plan,
            &input.failed,
            &input.protected,
            &receipt,
            chain.is_resumed(),
        )?;
        let failure = input
            .failed
            .state
            .terminal_transient_failure()?
            .ok_or_else(|| {
                recovery_blocked("failed turn has no supported actual transient observation")
            })?;
        let previous = ReviewRoundResumeReference::from_model_receipt(&receipt)?;
        let provenance = ReviewRoundRecoveryReference {
            schema_version: 1,
            round_reference_digest: canonical_digest(1, input.plan.reference())?,
            round_plan_digest: input.plan.digest().into(),
            origin: receipt_reference(&chain.origin)?,
            origin_preparation: receipt_reference(&chain.preparation)?,
            predecessor: receipt_reference(&receipt)?,
            protected_candidate_digest: input.protected.digest().into(),
            terminal_state_digest: canonical_digest(1, &input.failed.state)?,
            classification: classification(&failure),
            previous_resume_digest: previous
                .as_ref()
                .map(|v| canonical_digest(1, v))
                .transpose()?,
        };
        let review_inventory = inventory(&receipts, input.plan.reference())?;
        let implementation_inventory = implementation_inventory(&receipts, input.plan.reference())?;
        let config_revision = input.config.revision()?;
        let digest = canonical_digest(
            1,
            &json!({"purpose":"paired_recovery_candidate",
            "provenance":provenance,"config":config_revision,
            "reviews":review_inventory,"implementations":implementation_inventory,
            "chain":chain.digest()?}),
        )?;
        let candidate = Self {
            plan: input.plan,
            failed: input.failed,
            protected: input.protected,
            receipt,
            chain,
            provenance,
            config_revision,
            review_inventory,
            implementation_inventory,
            digest,
        };
        candidate.verify_current(
            &receipts,
            input.grants,
            input.config,
            input.current,
            readers,
        )?;
        Ok(candidate)
    }
}
fn classification(failure: &crate::codex::TransientTurnFailure) -> TransientFailureCode {
    match failure {
        crate::codex::TransientTurnFailure::ServerOverloaded => {
            TransientFailureCode::ServerOverloaded
        }
        crate::codex::TransientTurnFailure::InternalServerError => {
            TransientFailureCode::InternalServerError
        }
    }
}
fn verify_failed_candidate(
    plan: &VerifiedReviewRoundPlan,
    failed: &RunCompletion,
    protected: &RetainedResumeSession,
    receipt: &Receipt,
    resumed: bool,
) -> Result<()> {
    let run = closed_run(receipt, "failed")?;
    require_recorded_session(receipt, failed)?;
    let contract = failed
        .contract
        .as_ref()
        .ok_or_else(|| recovery_blocked("failed paired implementation has no actual contract"))?;
    let execution = failed
        .execution
        .as_ref()
        .ok_or_else(|| recovery_blocked("failed paired implementation has no actual execution"))?;
    if !plan.reference().is_repair()
        || plan.subject_phase().phase != "implementation"
        || !matches!(failed.outcome, RunOutcome::Failed)
        || !failed.local_exit_confirmed
        || failed.hook_error.is_some()
        || failed
            .error
            .as_ref()
            .is_none_or(|e| e.code != "turn_incomplete")
        || !failed.checks.is_empty()
        || !failed.failed_checks.is_empty()
        || failed.state.terminal_transient_failure()?.is_none()
        || failed.state.resume_origin.is_some() != resumed
        || !matches!(
            contract.instructions().manifest.role,
            AgentRole::Implementation | AgentRole::Testing | AgentRole::Documentation
        )
        || contract.workflow().front_matter.hooks.before_run.is_some()
        || contract.workflow().front_matter.hooks.after_run.is_some()
        || execution.binding.operation_id != failed.operation_id
        || contract.instructions().manifest.operation_id != failed.operation_id
        || run.phase != *plan.subject_phase()
        || canonical_digest(1, &run)? != canonical_digest(1, protected.reference())?
        || canonical_digest(1, receipt)? != protected.admission_digest()
        || canonical_digest(1, &failed.state)? != canonical_digest(1, protected.state())?
    {
        return Err(recovery_blocked(
            "paired recovery requires an exact terminal transient session with its actual admitted ancestry and no checks or hooks",
        ));
    }
    if !resumed {
        plan.verify_contract(contract)?;
    }
    execution
        .ownership
        .verify(&execution.binding, contract.isolation())
}
#[path = "delivery_paired_completion.rs"]
mod completion;
use completion::PairedResumeChain;
pub(crate) use completion::{PairedCompletionEvidence, VerifiedCompletedPairedResume};

fn recovery_blocked(message: &str) -> Error {
    Error::new("paired_recovery_lineage", message)
}
