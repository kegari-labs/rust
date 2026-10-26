//! Atomic paired metadata and resumed implementation admission. This is a
//! protected-record verifier, not a provider/source/joined-completion factory.
//! All methods run under the existing Control gate; the mandatory final opaque
//! callback supplies current native authority without network I/O or reentry.
use super::*;
#[path = "control_review_paired_result_admission.rs"]
mod completed;
use completed::retained_round_floor;
pub(super) use completed::verify_completed_paired_result;
#[cfg(test)]
pub(super) use completed::verify_prior_repair_sources;

#[derive(Clone)]
pub(crate) struct RoundRecoveryProvenance {
    pub(crate) round: ReviewRoundReference,
    pub(crate) recovery: ReviewRoundRecoveryReference,
}
#[derive(Clone)]
pub(crate) struct RoundResumeProvenance {
    pub(crate) round: ReviewRoundReference,
    pub(crate) resume: ReviewRoundResumeReference,
}
impl RoundRecoveryProvenance {
    pub(in crate::control) fn persist(
        &self,
        expected: &mut BTreeMap<String, String>,
    ) -> Result<()> {
        expected.insert("review_round".into(), encode(&self.round)?);
        expected.insert("review_round_recovery".into(), encode(&self.recovery)?);
        Ok(())
    }
}
impl RoundResumeProvenance {
    pub(in crate::control) fn persist(
        &self,
        expected: &mut BTreeMap<String, String>,
    ) -> Result<()> {
        expected.insert("review_round".into(), encode(&self.round)?);
        expected.insert("review_round_resume".into(), encode(&self.resume)?);
        Ok(())
    }
}
/// Only the actual protected lineage reader below can construct this context.
/// It is never exposed as a caller-controlled omission of generic round guards.
pub(in crate::control) struct RoundRecoveryContext {
    round: ReviewRoundReference,
    recovery: ReviewRoundRecoveryReference,
}
impl RoundRecoveryContext {
    pub(in crate::control) fn verify_metadata(&self, receipt: &Receipt) -> Result<()> {
        if ReviewRoundRecoveryReference::from_metadata_receipt(receipt)?.as_ref()
            != Some(&self.recovery)
            || ReviewRoundReference::from_receipt(receipt)?.as_ref() != Some(&self.round)
        {
            return Err(round_error(
                "paired metadata history changed its exact original candidate",
            ));
        }
        Ok(())
    }
}
pub(in crate::control) struct RoundResumeCheck<'a> {
    pub snapshot: &'a ControlSnapshot,
    pub lease: &'a Lease,
    pub run: &'a RunReference,
    pub resume: &'a RunResumeReference,
    pub provenance: &'a RoundResumeProvenance,
    pub history: &'a BTreeMap<u32, Receipt>,
    pub receipts: &'a [Receipt],
}
impl ControlHost {
    pub(crate) fn admit_round_session_recovery_preparation(
        &self,
        lease: &Lease,
        reference: SessionRecoveryPreparationReference,
        provenance: RoundRecoveryProvenance,
        scope_digest: String,
        fresh: impl FnOnce(&ControlSnapshot) -> Result<()>,
    ) -> Result<Permit> {
        reference.validate()?;
        provenance.round.validate()?;
        provenance.recovery.validate()?;
        crate::evidence::validate_digest(&scope_digest)?;
        self.admit_inner(
            lease,
            reference.operation_id()?,
            reference.phase.work.clone(),
            scope_digest,
            Some(AdmissionReference::RoundSessionRecovery(
                reference,
                Box::new(provenance),
            )),
            fresh,
        )
    }
    pub(crate) fn admit_round_run_resumed_observed(
        &self,
        lease: &Lease,
        input: RunAdmission,
        resume: RunResumeReference,
        provenance: RoundResumeProvenance,
        fresh: impl FnOnce(&ControlSnapshot) -> Result<()>,
    ) -> Result<Permit> {
        input.reference.validate()?;
        resume.validate()?;
        provenance.round.validate()?;
        provenance.resume.validate()?;
        crate::evidence::validate_digest(&input.scope_digest)?;
        if input.request_id == resume.predecessor || input.work_key != input.reference.phase.work {
            return Err(Error::new(
                "scope_violation",
                "paired resume needs a new exact-work operation",
            ));
        }
        self.admit_inner(
            lease,
            input.request_id,
            input.work_key,
            input.scope_digest,
            Some(AdmissionReference::RoundResumedRun(
                input.reference,
                resume,
                Box::new(provenance),
            )),
            fresh,
        )
    }
    pub(in crate::control) fn verify_round_session_recovery(
        &self,
        snapshot: &ControlSnapshot,
        lease: &Lease,
        reference: &SessionRecoveryPreparationReference,
        provenance: &RoundRecoveryProvenance,
        receipts: &[Receipt],
    ) -> Result<()> {
        let context = verify_origin(
            snapshot,
            lease,
            &provenance.round,
            &provenance.recovery,
            receipts,
        )?;
        if reference.predecessor != context.recovery.predecessor.operation
            || reference.predecessor_receipt_digest != context.recovery.predecessor.receipt_digest
            || reference.candidate_digest != context.recovery.protected_candidate_digest
            || reference.phase != *context.round.subject_phase()
            || reference.workspace != context.round.workspace
            || reference.account != context.round.account
        {
            return Err(round_error(
                "paired metadata changes exact recovery subject",
            ));
        }
        self.verify_session_recovery_preparation_accounting(snapshot, reference, Some(&context))
    }
    pub(in crate::control) fn verify_round_run_resume(
        &self,
        input: RoundResumeCheck<'_>,
    ) -> Result<()> {
        let RoundResumeCheck {
            snapshot,
            lease,
            run,
            resume,
            provenance,
            history,
            receipts,
        } = input;
        let marker = &provenance.resume;
        let context = verify_origin(
            snapshot,
            lease,
            &provenance.round,
            &marker.recovery,
            receipts,
        )?;
        verify_subject(
            snapshot,
            &context.round,
            run.workspace,
            &run.account,
            &run.phase,
        )?;
        if resume.predecessor != marker.recovery.predecessor.operation
            || resume.predecessor_receipt_digest != marker.recovery.predecessor.receipt_digest
            || resume.recovery_digest != marker.provider_recovery_digest
            || resume.instruction_compatibility_digest != marker.instruction_compatibility_digest
            || resume.directory_reuse_digest != marker.directory_reuse_digest
            || resume.review_rounds_consumed
                != retained_round_floor(exact_receipt(receipts, &context.recovery.predecessor)?)?
                    .max(context.round.ordinal)
        {
            return Err(round_error(
                "paired model changed original round or opaque recovery bindings",
            ));
        }
        verify_closed_metadata(snapshot, receipts, &context.round, marker, Some(lease))?;
        let metadata = exact_receipt(receipts, &marker.metadata)?;
        let captured = SessionRecoveryPreparationReference::from_receipt(metadata)?
            .ok_or_else(|| round_error("paired metadata lost captured bounds"))?;
        if run.max_attempts > captured.max_attempts {
            return Err(Error::new(
                "attempts_exhausted",
                "paired resume raises metadata-captured attempt bound",
            ));
        }
        self.verify_run_resume_accounting(snapshot, run, resume, history, Some(&context))
    }
}

fn verify_origin(
    snapshot: &ControlSnapshot,
    lease: &Lease,
    round: &ReviewRoundReference,
    recovery: &ReviewRoundRecoveryReference,
    receipts: &[Receipt],
) -> Result<RoundRecoveryContext> {
    round.validate()?;
    recovery.validate()?;
    if receipts.len() > MAX_ROUND_RECEIPTS {
        return Err(Error::new(
            "review_round_history_limit",
            "paired recovery inventory exceeds protected bound",
        ));
    }
    if !round.is_repair()
        || round.runtime != snapshot.intent.runtime_id
        || canonical_digest(1, round)? != recovery.round_reference_digest
    {
        return Err(round_error(
            "paired recovery changed original round identity",
        ));
    }
    let original = exact_receipt(receipts, &recovery.origin)?;
    let original_run = closed_worker(original, "failed")?;
    verify_subject(
        snapshot,
        round,
        original_run.workspace,
        &original_run.account,
        &original_run.phase,
    )?;
    if ReviewRoundReference::from_receipt(original)?.as_ref() != Some(round)
        || ReviewRoundResumeReference::from_model_receipt(original)?.is_some()
        || ReviewReceiptReference::preparation_for(original)? != recovery.origin_preparation
    {
        return Err(round_error(
            "paired recovery origin is not the original prepared repair",
        ));
    }
    verify_persisted_preparation(receipts, original, round)?;
    require_not_held(snapshot, original)?;
    verify_original_sources(snapshot, round, receipts)?;
    let mut runs = BTreeMap::new();
    for receipt in receipts {
        let Some(run) = RunReference::from_receipt(receipt)? else {
            continue;
        };
        if run.workspace == round.workspace
            && run.phase.work == round.review_phase.work
            && run.phase.generation == round.review_phase.generation
            && run.phase.phase == "implementation"
            && (receipt.runtime_id != round.runtime
                || run.account != round.account
                || run.phase != *round.subject_phase()
                || runs.insert(run.attempt, (receipt, run)).is_some())
        {
            return Err(round_error(
                "paired recovery has unaccounted implementation namespace or ordinal",
            ));
        }
    }
    if runs.keys().copied().ne(1..=runs.len() as u32) {
        return Err(round_error(
            "paired recovery implementation inventory has a gap",
        ));
    }
    let (_, (latest, latest_run)) = runs
        .last_key_value()
        .ok_or_else(|| round_error("paired recovery history is empty"))?;
    recovery.predecessor.matches_observation(latest)?;
    if latest_run.attempt < original_run.attempt
        || latest.actor != format!("owner:{}", lease.owner_id)
        || latest.owner_generation != lease.owner_generation
    {
        return Err(round_error(
            "paired recovery lost exact current process owner or original attempt",
        ));
    }
    let source = exact_receipt(receipts, &round.dispatch_source().implementation)?;
    let source_run = closed_worker(source, "completed")?;
    if source_run.attempt.checked_add(1) != Some(original_run.attempt) {
        return Err(round_error(
            "original paired repair skipped its actual source attempt",
        ));
    }
    let mut prior: Option<(&Receipt, &RunReference)> = None;
    for (receipt, run) in runs.values() {
        if let Some((_, previous)) = prior
            && (run.max_attempts > previous.max_attempts
                || run.max_turns > previous.max_turns
                || run.max_review_rounds > previous.max_review_rounds
                || run.profile_digest != previous.profile_digest)
        {
            return Err(Error::new(
                "attempts_exhausted",
                "paired recovery history resets implementation bounds",
            ));
        }
        if run.attempt >= original_run.attempt {
            closed_worker(receipt, "failed")?;
            require_not_held(snapshot, receipt)?;
            if ReviewRoundReference::from_receipt(receipt)?.as_ref() != Some(round) {
                return Err(round_error(
                    "paired recovery crossed another round or source",
                ));
            }
            if run.attempt > original_run.attempt {
                let previous = prior
                    .ok_or_else(|| round_error("paired resume has no prior attempt"))?
                    .0;
                let link = ReviewRoundResumeReference::from_model_receipt(receipt)?
                    .ok_or_else(|| round_error("paired resumed failure lost its typed bridge"))?;
                if link.recovery.origin != recovery.origin
                    || link.recovery.origin_preparation != recovery.origin_preparation
                    || link.recovery.round_plan_digest != recovery.round_plan_digest
                    || link.recovery.predecessor.operation != previous.operation_id
                    || link.recovery.predecessor.receipt_digest != canonical_digest(1, previous)?
                {
                    return Err(round_error(
                        "paired resume chain omitted or replaced an actual predecessor",
                    ));
                }
                let previous_link = ReviewRoundResumeReference::from_model_receipt(previous)?;
                if link.recovery.previous_resume_digest
                    != previous_link
                        .as_ref()
                        .map(|v| canonical_digest(1, v))
                        .transpose()?
                {
                    return Err(round_error(
                        "paired resume chain changed the previous admitted bridge",
                    ));
                }
                verify_transient(previous, &link.recovery)?;
                verify_closed_metadata(snapshot, receipts, round, &link, None)?;
                let metadata = exact_receipt(receipts, &link.metadata)?;
                if metadata.actor != receipt.actor
                    || metadata.owner_generation != receipt.owner_generation
                {
                    return Err(round_error(
                        "resumed model changed its actual metadata process owner",
                    ));
                }
            }
        }
        prior = Some((receipt, run));
    }
    let previous_link = ReviewRoundResumeReference::from_model_receipt(latest)?;
    if recovery.previous_resume_digest
        != previous_link
            .as_ref()
            .map(|v| canonical_digest(1, v))
            .transpose()?
    {
        return Err(round_error(
            "current paired recovery lost the latest admitted bridge",
        ));
    }
    verify_transient(latest, recovery)?;
    Ok(RoundRecoveryContext {
        round: round.clone(),
        recovery: recovery.clone(),
    })
}
/// The original paired admission already consumed Findings. Recheck its actual
/// source/review/preparation closure without treating the closed failed run as a
/// new Findings consumer or pretending the original source is latest again.
fn verify_original_sources(
    snapshot: &ControlSnapshot,
    round: &ReviewRoundReference,
    receipts: &[Receipt],
) -> Result<()> {
    verify_source(snapshot, round, receipts, false)?;
    let history = review_history(round, receipts)?;
    let mut previous = None;
    let mut allocated = BTreeSet::new();
    for (receipt, run, marker) in history.values() {
        verify_persisted_preparation(receipts, receipt, marker)?;
        verify_source(snapshot, marker, receipts, false)?;
        require_not_held(snapshot, receipt)?;
        verify_review_transition(snapshot, marker, previous, receipts)?;
        if !matches!(marker.cause, ReviewRoundCause::TransientRetry { .. })
            && !allocated.insert(marker.round_id.clone())
        {
            return Err(round_error(
                "paired recovery review round was allocated twice",
            ));
        }
        if let Some((_, prior_run, prior_marker)) = previous
            && (run.max_attempts > prior_run.max_attempts
                || run.max_turns > prior_run.max_turns
                || run.max_review_rounds > prior_run.max_review_rounds
                || marker.round_limit > prior_marker.round_limit
                || run.profile_digest != prior_run.profile_digest)
        {
            return Err(round_error("paired recovery review history resets bounds"));
        }
        previous = Some((receipt, run, marker));
    }
    let findings =
        pair_chain::findings(round).ok_or_else(|| round_error("paired recovery lacks Findings"))?;
    let (review, _, marker) =
        previous.ok_or_else(|| round_error("paired recovery has no completed review"))?;
    verify_findings(snapshot, round, findings, review, marker)?;
    if marker.round_id != round.round_id
        || marker.ordinal != round.ordinal
        || marker.source != round.source
        || marker.passed_validation != round.passed_validation
        || round.round_limit > marker.round_limit
    {
        return Err(round_error(
            "paired recovery changed its original reviewed subject",
        ));
    }
    if let ReviewRoundCause::RepairContinuation {
        current_source,
        validation,
        ..
    } = &round.cause
    {
        pair_chain::verify_repair_chain(
            snapshot,
            round,
            &current_source.implementation,
            receipts,
            current_source.paired_resume.as_ref(),
        )?;
        pair_chain::verify_validation(snapshot, round, current_source, validation, receipts, true)?;
    }
    Ok(())
}
fn verify_transient(receipt: &Receipt, reference: &ReviewRoundRecoveryReference) -> Result<()> {
    closed_worker(receipt, "failed")?;
    let raw = receipt
        .observed_result
        .as_ref()
        .and_then(|v| v.pointer("/observation/session"))
        .ok_or_else(|| round_error("paired failure has no terminal session"))?;
    let state: crate::codex::SessionState = serde_json::from_value(raw.clone())
        .map_err(|_| round_error("paired failed session is malformed"))?;
    let classification = match state.terminal_transient_failure()? {
        Some(crate::codex::TransientTurnFailure::ServerOverloaded) => {
            TransientFailureCode::ServerOverloaded
        }
        Some(crate::codex::TransientTurnFailure::InternalServerError) => {
            TransientFailureCode::InternalServerError
        }
        None => {
            return Err(Error::new(
                "persistent_blocker",
                "paired failure lacks supported actual transient provenance",
            ));
        }
    };
    if canonical_digest(1, &state)? != reference.terminal_state_digest
        || classification != reference.classification
    {
        return Err(round_error(
            "paired failure changed terminal history or classification",
        ));
    }
    Ok(())
}
fn verify_closed_metadata(
    snapshot: &ControlSnapshot,
    receipts: &[Receipt],
    round: &ReviewRoundReference,
    link: &ReviewRoundResumeReference,
    lease: Option<&Lease>,
) -> Result<()> {
    let metadata = exact_receipt(receipts, &link.metadata)?;
    let preparation = SessionRecoveryPreparationReference::from_receipt(metadata)?
        .ok_or_else(|| round_error("paired resume metadata has no exact preparation"))?;
    let observed = metadata
        .observed_result
        .as_ref()
        .ok_or_else(|| round_error("paired metadata has no terminal observation"))?;
    let proof_digest = observed["observation"]["proof_digest"]
        .as_str()
        .ok_or_else(|| round_error("paired metadata lacks observed provider proof digest"))?;
    crate::evidence::validate_digest(proof_digest)?;
    if metadata.runtime_id != round.runtime
        || ReviewRoundReference::from_receipt(metadata)?.as_ref() != Some(round)
        || ReviewRoundRecoveryReference::from_metadata_receipt(metadata)?.as_ref()
            != Some(&link.recovery)
        || metadata.phase != ReceiptPhase::Reconciled
        || observed["local_execution"] != "exited"
        || observed["unresolved_effects"]
            .as_array()
            .is_none_or(|v| !v.is_empty())
        || observed["observation"]["metadata_recovery"] != "verified"
        || observed["observation"]["local_exit_confirmed"] != true
        || lease.is_some_and(|lease| {
            metadata.actor != format!("owner:{}", lease.owner_id)
                || metadata.owner_generation != lease.owner_generation
        })
    {
        return Err(round_error(
            "paired resume lacks actual closed metadata under its owner",
        ));
    }
    require_not_held(snapshot, metadata)?;
    let mut attempts = BTreeMap::new();
    for receipt in receipts {
        if let Some(other) = SessionRecoveryPreparationReference::from_receipt(receipt)?
            && other.predecessor == preparation.predecessor
            && (other.phase != preparation.phase
                || other.workspace != preparation.workspace
                || other.account != preparation.account
                || receipt.runtime_id != round.runtime
                || ReviewRoundRecoveryReference::from_metadata_receipt(receipt)?.as_ref()
                    != Some(&link.recovery)
                || other.max_attempts < preparation.max_attempts
                || attempts.insert(other.attempt, receipt).is_some())
        {
            return Err(round_error(
                "paired metadata inventory changed identity or bounds",
            ));
        }
    }
    if attempts.keys().copied().ne(1..=preparation.attempt)
        || attempts.last_key_value().map(|(_, r)| r.operation_id) != Some(metadata.operation_id)
    {
        return Err(round_error(
            "paired resume metadata is not the exact latest complete inventory",
        ));
    }
    for receipt in attempts
        .values()
        .filter(|r| r.operation_id != metadata.operation_id)
    {
        let observed = receipt
            .observed_result
            .as_ref()
            .ok_or_else(|| round_error("prior paired metadata remains uncertain"))?;
        if receipt.phase != ReceiptPhase::Reconciled
            || observed["local_execution"] != "exited"
            || observed["unresolved_effects"]
                .as_array()
                .is_none_or(|v| !v.is_empty())
            || observed["observation"]["metadata_recovery"] != "failed_no_effect"
            || observed["observation"]["retryable"] != true
        {
            return Err(Error::new(
                "reconciliation_required",
                "prior paired metadata is not a retryable no-effect failure",
            ));
        }
        require_not_held(snapshot, receipt)?;
    }
    Ok(())
}
