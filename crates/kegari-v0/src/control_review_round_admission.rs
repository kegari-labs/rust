//! Atomic receipt provenance and review-round accounting. Data references
//! never replace the required opaque/current-authority callback. No provider I/O
//! or public permit reconstruction occurs while Control is locked.
use super::*;
#[path = "control_review_round_recovery_admission.rs"]
mod recovery;
use crate::scheduler::PhaseKey;
pub(super) use recovery::{RoundRecoveryContext, RoundResumeCheck};
pub(crate) use recovery::{RoundRecoveryProvenance, RoundResumeProvenance};

const MAX_ROUND_RECEIPTS: usize = 65_536;
const PREPARATION_LINK: &str = "review_round_preparation";

#[path = "control_review_pair_chain.rs"]
mod pair_chain;

#[derive(Clone)]
pub(crate) enum ReviewRoundRunCause {
    Review,
    PairedRepair(RunContinuationReference),
    AfterRepair(SourceAdvancedReference),
}
#[derive(Clone)]
pub(crate) struct ReviewRoundRunProvenance {
    pub round: ReviewRoundReference,
    pub preparation: ReviewReceiptReference,
    pub cause: ReviewRoundRunCause,
}
impl ReviewRoundRunProvenance {
    pub(super) fn persist(&self, expected: &mut BTreeMap<String, String>) -> Result<()> {
        expected.insert("review_round".into(), encode(&self.round)?);
        expected.insert(PREPARATION_LINK.into(), encode(&self.preparation)?);
        match &self.cause {
            ReviewRoundRunCause::Review => (),
            ReviewRoundRunCause::PairedRepair(value) => {
                expected.insert("continuation".into(), encode(value)?);
            }
            ReviewRoundRunCause::AfterRepair(value) => {
                expected.insert("source_advanced".into(), encode(value)?);
            }
        }
        Ok(())
    }
}
pub(super) struct RoundPreparationCheck<'a> {
    pub snapshot: &'a ControlSnapshot,
    pub operation: Uuid,
    pub prep: &'a PreparationReference,
    pub round: &'a ReviewRoundReference,
    pub receipts: &'a [Receipt],
}
pub(super) struct RoundRunCheck<'a> {
    pub snapshot: &'a ControlSnapshot,
    pub lease: &'a Lease,
    pub operation: Uuid,
    pub run: &'a RunReference,
    pub provenance: &'a ReviewRoundRunProvenance,
    pub history: &'a BTreeMap<u32, Receipt>,
    pub receipts: &'a [Receipt],
}

impl ControlHost {
    /// Preparation records a proposal; it does not allocate a model attempt or
    /// review round. The callback verifies the opaque plan and current grant.
    pub(crate) fn admit_round_preparation_observed(
        &self,
        lease: &Lease,
        input: PreparationAdmission,
        round: ReviewRoundReference,
        fresh: impl FnOnce(&ControlSnapshot) -> Result<()>,
    ) -> Result<Permit> {
        input.reference.validate()?;
        crate::evidence::validate_digest(&input.scope_digest)?;
        if input.request_id != input.reference.operation_id()?
            || input.work_key != input.reference.phase.work
        {
            return Err(round_error(
                "round preparation changes its actual worker identity",
            ));
        }
        self.admit_inner(
            lease,
            input.request_id,
            input.work_key,
            input.scope_digest,
            Some(AdmissionReference::RoundPreparation(
                input.reference,
                Box::new(round),
            )),
            fresh,
        )
    }
    /// Only the real native consumer calls this after assembling its contract.
    /// Its predicate must revalidate actual joined capabilities, native grants,
    /// source, exact schedule/current settings, capacity and permissions.
    pub(crate) fn admit_round_run_observed(
        &self,
        lease: &Lease,
        input: RunAdmission,
        provenance: ReviewRoundRunProvenance,
        fresh: impl FnOnce(&ControlSnapshot) -> Result<()>,
    ) -> Result<Permit> {
        input.reference.validate()?;
        crate::evidence::validate_digest(&input.scope_digest)?;
        if input.work_key != input.reference.phase.work {
            return Err(round_error("round run changes canonical work"));
        }
        self.admit_inner(
            lease,
            input.request_id,
            input.work_key,
            input.scope_digest,
            Some(AdmissionReference::RoundRun(
                input.reference,
                Box::new(provenance),
            )),
            fresh,
        )
    }
    pub(super) fn review_round_receipts(&self) -> Result<Vec<Receipt>> {
        self.receipts
            .directory
            .records_bounded(MAX_ROUND_RECEIPTS)?
            .into_iter()
            .map(|leaf| self.receipts.directory.read(&leaf))
            .collect()
    }
    pub(super) fn verify_round_preparation(&self, input: RoundPreparationCheck<'_>) -> Result<()> {
        let RoundPreparationCheck {
            snapshot,
            operation,
            prep,
            round,
            receipts,
        } = input;
        verify_subject(snapshot, round, prep.workspace, &prep.account, &prep.phase)?;
        if operation != prep.operation_id()?
            || prep.worker_operation == round.source.implementation.operation
            || prep.worker_operation == round.dispatch_source().implementation.operation
            || receipts
                .iter()
                .any(|r| r.operation_id == prep.worker_operation)
        {
            return Err(round_error(
                "preparation worker identity is already admitted or replaces its source",
            ));
        }
        verify_candidate(snapshot, round, receipts)?;
        Ok(())
    }
    pub(super) fn verify_round_run(&self, input: RoundRunCheck<'_>) -> Result<()> {
        let RoundRunCheck {
            snapshot,
            lease,
            operation,
            run,
            provenance,
            history,
            receipts,
        } = input;
        let round = &provenance.round;
        verify_subject(snapshot, round, run.workspace, &run.account, &run.phase)?;
        let prep = exact_receipt(receipts, &provenance.preparation)?;
        let actual_prep = PreparationReference::from_receipt(prep)?
            .ok_or_else(|| round_error("model run lacks its actual typed preparation"))?;
        let expected_prep = PreparationReference {
            worker_operation: operation,
            workspace: run.workspace,
            account: run.account.clone(),
            phase: run.phase.clone(),
        };
        let result = prep
            .observed_result
            .as_ref()
            .ok_or_else(|| round_error("preparation has no exit observation"))?;
        if actual_prep != expected_prep
            || prep.operation_id != preparation_operation(operation)?
            || prep.runtime_id != snapshot.intent.runtime_id
            || prep.actor != format!("owner:{}", lease.owner_id)
            || prep.owner_generation != lease.owner_generation
            || prep.expected.contains_key(PREPARATION_LINK)
            || ReviewRoundReference::from_receipt(prep)?.as_ref() != Some(round)
            || prep.phase != ReceiptPhase::Reconciled
            || result["local_execution"] != "exited"
            || result["unresolved_effects"]
                .as_array()
                .is_none_or(|v| !v.is_empty())
            || result["observation"]["phase"] != "worker_preparation"
            || result["observation"]["result"] != "prepared"
        {
            return Err(round_error(
                "preparation did not close for this exact owner, worker and round proposal",
            ));
        }
        require_not_held(snapshot, prep)?;
        verify_candidate(snapshot, round, receipts)?;
        match (&round.cause, &provenance.cause) {
            (
                ReviewRoundCause::Initial {} | ReviewRoundCause::TransientRetry { .. },
                ReviewRoundRunCause::Review,
            ) => {
                verify_review_session_caps(run, history)?;
            }
            (
                ReviewRoundCause::PairedRepair { findings },
                ReviewRoundRunCause::PairedRepair(link),
            ) => {
                link.validate()?;
                if link.predecessor != round.source.implementation.operation
                    || link.predecessor_receipt_digest != round.source.implementation.receipt_digest
                    || link.failure_operation != findings.review.operation
                {
                    return Err(round_error(
                        "paired repair changes the actual Findings/source cause",
                    ));
                }
                self.verify_run_continuation_accounting(
                    snapshot,
                    run,
                    link,
                    history,
                    Some(round),
                    false,
                )?;
            }
            (
                ReviewRoundCause::RepairContinuation {
                    current_source,
                    validation,
                    ..
                },
                ReviewRoundRunCause::PairedRepair(link),
            ) => {
                link.validate()?;
                if link.predecessor != current_source.implementation.operation
                    || link.predecessor_receipt_digest
                        != current_source.implementation.receipt_digest
                    || link.failure_operation != validation.validation.operation
                {
                    return Err(round_error(
                        "continued repair changes latest source or actual validation cause",
                    ));
                }
                self.verify_run_continuation_accounting(
                    snapshot,
                    run,
                    link,
                    history,
                    Some(round),
                    false,
                )?;
            }
            (
                ReviewRoundCause::AfterRepair {
                    findings, repair, ..
                },
                ReviewRoundRunCause::AfterRepair(link),
            ) => {
                link.validate()?;
                if link.predecessor != findings.review.operation
                    || link.predecessor_receipt_digest != findings.review.receipt_digest
                    || link.source_operation != repair.operation
                    || link.source_receipt_digest != repair.receipt_digest
                {
                    return Err(round_error(
                        "after-repair review changes its exact paired cause",
                    ));
                }
                self.verify_source_advanced_accounting(snapshot, run, link, history, Some(round))?;
            }
            _ => {
                return Err(round_error(
                    "round cause differs from actual worker admission cause",
                ));
            }
        }
        if !round.is_repair() && round.round_limit > run.max_review_rounds {
            return Err(round_error(
                "review marker exceeds actual captured review-session cap",
            ));
        }
        // Existing continuation verification retains original implementation
        // attempts/turns/settings. The namespace-specific round bound is separate.
        Ok(())
    }
}

fn encode(value: &impl Serialize) -> Result<String> {
    serde_json::to_string(value).map_err(|_| Error::invalid("review round provenance encoding"))
}
fn round_error(message: &str) -> Error {
    Error::new("review_round_lineage", message)
}
fn exact_receipt<'a>(
    receipts: &'a [Receipt],
    reference: &ReviewReceiptReference,
) -> Result<&'a Receipt> {
    let mut matches = receipts
        .iter()
        .filter(|r| r.operation_id == reference.operation);
    let value = matches
        .next()
        .ok_or_else(|| round_error("referenced round receipt is missing"))?;
    if matches.next().is_some() {
        return Err(round_error("duplicate round receipt identity"));
    }
    reference.matches_observation(value)?;
    Ok(value)
}
fn require_not_held(snapshot: &ControlSnapshot, receipt: &Receipt) -> Result<()> {
    if snapshot
        .active_operations
        .iter()
        .any(|p| p.operation_id == receipt.operation_id)
        || snapshot
            .pending_local_processes
            .contains(&receipt.operation_id)
        || snapshot
            .pending_external_effects
            .contains(&receipt.operation_id)
        || snapshot.unresolved_operations.iter().any(|(id, targets)| {
            *id == receipt.operation_id
                || targets
                    .iter()
                    .any(|target| receipt.canonical_targets.contains(target))
        })
    {
        return Err(Error::new(
            "reconciliation_required",
            "round reference still has active or uncertain ownership",
        ));
    }
    Ok(())
}
fn closed_worker(receipt: &Receipt, outcome: &str) -> Result<RunReference> {
    let run = RunReference::from_receipt(receipt)?
        .ok_or_else(|| round_error("round reference is not a worker admission"))?;
    let observed = receipt
        .observed_result
        .as_ref()
        .ok_or_else(|| round_error("round worker has no terminal observation"))?;
    if receipt.phase != ReceiptPhase::Reconciled
        || observed["local_execution"] != "exited"
        || observed["unresolved_effects"]
            .as_array()
            .is_none_or(|v| !v.is_empty())
        || observed["observation"]["outcome"] != outcome
        || observed["observation"]["local_exit_confirmed"] != true
        || observed["observation"]["session"]["terminal"] != outcome
        || !observed["observation"]["session"]["turn_id"].is_null()
        || !observed["observation"]["session"]["unresolved_turn"].is_null()
        || !observed["observation"]["hook_error"].is_null()
        || !observed["observation"]["session"]["intervention"].is_null()
        || observed["observation"]["session"]["input_required"] != false
    {
        return Err(Error::new(
            "reconciliation_required",
            "round worker lacks exact terminal exit and effect resolution",
        ));
    }
    Ok(run)
}
fn verify_subject(
    snapshot: &ControlSnapshot,
    round: &ReviewRoundReference,
    workspace: Uuid,
    account: &str,
    phase: &PhaseKey,
) -> Result<()> {
    round.validate()?;
    if round.runtime != snapshot.intent.runtime_id
        || workspace != round.workspace
        || account != round.account
        || phase != round.subject_phase()
    {
        return Err(round_error(
            "round reference belongs to a different runtime/workspace/account/phase",
        ));
    }
    Ok(())
}
/// Read only the exact passed-check reference closure. This does not recurse
/// through source/round readers or mint a check, grant or execution capability.
fn verify_passed_validation(
    snapshot: &ControlSnapshot,
    round: &ReviewRoundReference,
    receipts: &[Receipt],
) -> Result<()> {
    if let Some(passed) = &round.passed_validation {
        passed.validate_for_round(round)?;
        if passed.retained_receipts.len() > MAX_ROUND_RECEIPTS {
            return Err(round_error(
                "passed validation inventory exceeds protected bound",
            ));
        }
        for (operation, digest) in &passed.retained_receipts {
            let actual = exact_receipt(
                receipts,
                &ReviewReceiptReference {
                    operation: *operation,
                    receipt_digest: digest.clone(),
                },
            )?;
            require_not_held(snapshot, actual)?;
        }
        let actual = exact_receipt(receipts, &passed.validation)?;
        let run = closed_worker(actual, "completed")?;
        if actual.runtime_id != round.runtime
            || run.workspace != round.workspace
            || run.account != round.account
            || run.phase.phase != "child_local_validation"
            || run.phase.work != round.review_phase.work
            || run.phase.generation != round.review_phase.generation
        {
            return Err(round_error(
                "passed validation belongs to another exact worker source",
            ));
        }
    }
    Ok(())
}
fn verify_source(
    snapshot: &ControlSnapshot,
    round: &ReviewRoundReference,
    receipts: &[Receipt],
    current: bool,
) -> Result<()> {
    verify_passed_validation(snapshot, round, receipts)?;
    // Original reviewed source remains authenticated historical provenance.
    // Only the exact latest repaired source can justify another implementation.
    if round.dispatch_source() != &round.source {
        verify_source_reference(snapshot, round, &round.source, receipts, false)?;
    }
    verify_source_reference(snapshot, round, round.dispatch_source(), receipts, current)
}
fn verify_source_reference(
    snapshot: &ControlSnapshot,
    round: &ReviewRoundReference,
    reference: &ReviewedSourceReference,
    receipts: &[Receipt],
    current: bool,
) -> Result<()> {
    verify_source_reference_fields(snapshot, round, reference, receipts, current)?;
    let actual = exact_receipt(receipts, &reference.implementation)?;
    match (
        &reference.paired_resume,
        ReviewRoundResumeReference::from_model_receipt(actual)?,
    ) {
        (Some(proof), Some(_)) => {
            recovery::verify_completed_paired_result(snapshot, round, proof, receipts)
        }
        (None, None) => Ok(()),
        _ => Err(round_error(
            "source omits or invents its exact admitted paired resume lineage",
        )),
    }
}
fn verify_source_reference_fields(
    snapshot: &ControlSnapshot,
    round: &ReviewRoundReference,
    reference: &ReviewedSourceReference,
    receipts: &[Receipt],
    current: bool,
) -> Result<()> {
    let source = exact_receipt(receipts, &reference.implementation)?;
    let run = closed_worker(source, "completed")?;
    if source.runtime_id != round.runtime
        || run.workspace != round.workspace
        || run.account != round.account
        || run.phase != reference.implementation_phase
        || source.canonical_targets != [round.review_phase.work.clone()]
    {
        return Err(round_error(
            "reviewed implementation differs from exact source namespace",
        ));
    }
    let seal = exact_receipt(receipts, &reference.seal)?;
    let observed = seal
        .observed_result
        .as_ref()
        .ok_or_else(|| round_error("source seal has no observed revision"))?;
    if seal.runtime_id != round.runtime
        || seal.kind != "child_source_sealing"
        || seal.canonical_targets != [reference.repository_id.clone()]
        || seal.expected.get("admitted_work") != Some(&round.review_phase.work)
        || !matches!(
            seal.phase,
            ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
        )
        || observed["revision"]["repository"]["hostname"] != reference.repository_hostname
        || observed["revision"]["repository"]["id"] != reference.repository_id
        || observed["revision"]["head"] != reference.commit
        || observed["revision"]["tested_commit"] != reference.commit
        || observed["revision"]["tested_tree"] != reference.tree
    {
        return Err(round_error(
            "source seal differs from exact observed repository/commit/tree",
        ));
    }
    require_not_held(snapshot, source)?;
    require_not_held(snapshot, seal)?;
    if current {
        // Actual later implementations survive deletion of their native grant.
        // Mixed namespaces require an additional opaque ordered ancestry slice;
        // this bounded increment refuses them rather than guessing from time.
        for receipt in receipts {
            if let Some(other) = RunReference::from_receipt(receipt)?
                && receipt.runtime_id == round.runtime
                && other.workspace == round.workspace
                && other.phase.work == round.review_phase.work
                && other.phase.generation == round.review_phase.generation
                && other.phase.phase == "implementation"
                && (other.phase != run.phase || other.attempt >= run.attempt)
                && receipt.operation_id != source.operation_id
            {
                return Err(round_error(
                    "newer or unaccounted implementation grant history supersedes source",
                ));
            }
        }
    }
    Ok(())
}

fn review_history<'a>(
    round: &ReviewRoundReference,
    receipts: &'a [Receipt],
) -> Result<BTreeMap<u32, (&'a Receipt, RunReference, ReviewRoundReference)>> {
    if receipts.len() > MAX_ROUND_RECEIPTS {
        return Err(Error::new(
            "review_round_history_limit",
            "protected round history exceeds finite receipt inventory",
        ));
    }
    let mut history = BTreeMap::new();
    for receipt in receipts {
        if let Some(run) = RunReference::from_receipt(receipt)?
            && run.phase == round.review_phase
        {
            if receipt.runtime_id != round.runtime
                || run.workspace != round.workspace
                || run.account != round.account
            {
                return Err(round_error(
                    "review namespace has conflicting runtime/workspace/account",
                ));
            }
            let marker = ReviewRoundReference::from_receipt(receipt)?.ok_or_else(|| {
                Error::new(
                    "review_round_lineage_unavailable",
                    "legacy review history cannot be reinterpreted as exact rounds",
                )
            })?;
            if marker.review_phase != round.review_phase
                || marker.runtime != round.runtime
                || marker.workspace != round.workspace
                || marker.account != round.account
                || history
                    .insert(run.attempt, (receipt, run, marker))
                    .is_some()
            {
                return Err(round_error(
                    "duplicate or mismatched review attempt history",
                ));
            }
        }
    }
    if history.keys().copied().ne(1..=history.len() as u32) {
        return Err(round_error("review attempt history contains a gap"));
    }
    Ok(history)
}
fn verify_candidate(
    snapshot: &ControlSnapshot,
    round: &ReviewRoundReference,
    receipts: &[Receipt],
) -> Result<()> {
    verify_source(snapshot, round, receipts, true)?;
    let history = review_history(round, receipts)?;
    let mut previous: Option<(&Receipt, &RunReference, &ReviewRoundReference)> = None;
    let mut allocated = BTreeSet::new();
    for (receipt, run, marker) in history.values() {
        verify_persisted_preparation(receipts, receipt, marker)?;
        verify_source(snapshot, marker, receipts, false)?;
        require_not_held(snapshot, receipt)?;
        verify_review_transition(snapshot, marker, previous, receipts)?;
        if !matches!(marker.cause, ReviewRoundCause::TransientRetry { .. })
            && !allocated.insert(marker.round_id.clone())
        {
            return Err(round_error("review round identity was allocated twice"));
        }
        if let Some((_, prior_run, prior_marker)) = previous
            && (run.max_attempts > prior_run.max_attempts
                || run.max_turns > prior_run.max_turns
                || run.max_review_rounds > prior_run.max_review_rounds
                || marker.round_limit > prior_marker.round_limit
                || run.profile_digest != prior_run.profile_digest)
        {
            return Err(round_error(
                "review history resets retained bounds or profile",
            ));
        }
        previous = Some((receipt, run, marker));
    }
    match &round.cause {
        ReviewRoundCause::PairedRepair { findings } => {
            let (latest, _, latest_marker) =
                previous.ok_or_else(|| round_error("paired repair has no completed review"))?;
            verify_findings(snapshot, round, findings, latest, latest_marker)?;
            if round.round_id != latest_marker.round_id
                || round.ordinal != latest_marker.ordinal
                || round.round_limit > latest_marker.round_limit
                || round.source != latest_marker.source
                || round.passed_validation != latest_marker.passed_validation
            {
                return Err(round_error(
                    "paired repair changes its exact reviewed source/round",
                ));
            }
            for receipt in receipts {
                if RunReference::from_receipt(receipt)?.is_some()
                    && let Some(marker) = ReviewRoundReference::from_receipt(receipt)?
                    && let Some(prior) = pair_chain::findings(&marker)
                    && marker.runtime == round.runtime
                    && marker.workspace == round.workspace
                    && prior.review.operation == findings.review.operation
                {
                    return Err(Error::new(
                        "no_progress",
                        "Findings already consumed its paired implementation session",
                    ));
                }
            }
        }
        ReviewRoundCause::RepairContinuation {
            findings,
            current_source,
            validation,
            ..
        } => {
            let (latest, _, latest_marker) = previous.ok_or_else(|| {
                round_error("continued repair has no original completed Findings")
            })?;
            verify_findings(snapshot, round, findings, latest, latest_marker)?;
            if round.round_id != latest_marker.round_id
                || round.ordinal != latest_marker.ordinal
                || round.source != latest_marker.source
                || round.passed_validation != latest_marker.passed_validation
                || round.round_limit > latest_marker.round_limit
            {
                return Err(round_error(
                    "continued repair changes original reviewed round",
                ));
            }
            pair_chain::verify_repair_chain(
                snapshot,
                round,
                &current_source.implementation,
                receipts,
                current_source.paired_resume.as_ref(),
            )?;
            pair_chain::verify_validation(
                snapshot,
                round,
                current_source,
                validation,
                receipts,
                true,
            )?;
        }
        _ => verify_review_transition(snapshot, round, previous, receipts)?,
    }
    Ok(())
}
fn verify_review_transition(
    snapshot: &ControlSnapshot,
    next: &ReviewRoundReference,
    previous: Option<(&Receipt, &RunReference, &ReviewRoundReference)>,
    receipts: &[Receipt],
) -> Result<()> {
    match (&next.cause, previous) {
        (ReviewRoundCause::Initial {}, None) if next.ordinal == 1 => Ok(()),
        (
            ReviewRoundCause::TransientRetry {
                predecessor,
                prior_round_reference_digest,
                terminal_state_digest,
                classified_failure_digest,
                classification,
            },
            Some((receipt, _, prior)),
        ) => {
            predecessor.matches_observation(receipt)?;
            closed_worker(receipt, "failed")?;
            require_not_held(snapshot, receipt)?;
            require_spacing(receipt)?;
            let raw = receipt
                .observed_result
                .as_ref()
                .and_then(|v| v.pointer("/observation/session"))
                .ok_or_else(|| {
                    round_error("transient predecessor lacks actual terminal session")
                })?;
            let state: crate::codex::SessionState = serde_json::from_value(raw.clone())
                .map_err(|_| round_error("transient terminal session is malformed"))?;
            let actual = match state.terminal_transient_failure()? {
                Some(crate::codex::TransientTurnFailure::ServerOverloaded) => {
                    TransientFailureCode::ServerOverloaded
                }
                Some(crate::codex::TransientTurnFailure::InternalServerError) => {
                    TransientFailureCode::InternalServerError
                }
                None => {
                    return Err(Error::new(
                        "persistent_blocker",
                        "review failure lacks supported actual transient provenance",
                    ));
                }
            };
            if next.round_id != prior.round_id
                || next.passed_validation != prior.passed_validation
                || next.ordinal != prior.ordinal
                || next.source != prior.source
                || next.round_limit > prior.round_limit
                || *prior_round_reference_digest != canonical_digest(1, prior)?
                || *terminal_state_digest != canonical_digest(1, &state)?
                || &actual != classification
                || *classified_failure_digest
                    != canonical_digest(
                        1,
                        &serde_json::json!({
                    "state":terminal_state_digest,"classification":classification}),
                    )?
            {
                return Err(round_error(
                    "transient review changes round/source or terminal provenance",
                ));
            }
            Ok(())
        }
        (
            ReviewRoundCause::AfterRepair {
                findings,
                repair,
                paired_round_reference_digest,
            },
            Some((receipt, _, prior)),
        ) => {
            verify_findings(snapshot, next, findings, receipt, prior)?;
            let repair_receipt = exact_receipt(receipts, repair)?;
            let repair_run = closed_worker(repair_receipt, "completed")?;
            require_not_held(snapshot, repair_receipt)?;
            require_spacing(repair_receipt)?;
            let paired = ReviewRoundReference::from_receipt(repair_receipt)?
                .ok_or_else(|| round_error("repair lacks exact paired round marker"))?;
            if next.source.paired_resume.is_none() {
                verify_persisted_preparation(receipts, repair_receipt, &paired)?;
            }
            pair_chain::verify_repair_chain(
                snapshot,
                &paired,
                repair,
                receipts,
                next.source.paired_resume.as_ref(),
            )?;
            if pair_chain::findings(&paired) != Some(findings)
                || paired.runtime != next.runtime
                || paired.workspace != next.workspace
                || paired.account != next.account
                || paired.review_phase != next.review_phase
                || paired.source != prior.source
                || paired.round_id != prior.round_id
                || *paired_round_reference_digest != canonical_digest(1, &paired)?
                || next.ordinal
                    != prior
                        .ordinal
                        .checked_add(1)
                        .ok_or_else(|| round_error("review ordinal overflow"))?
                || next.round_limit > prior.round_limit
                || next.round_limit > paired.round_limit
                || next.source.implementation.operation != repair_receipt.operation_id
                || next.source.implementation.receipt_digest != repair.receipt_digest
                || repair_run.phase != next.source.implementation_phase
                || next.source.tree == prior.source.tree
            {
                return Err(round_error(
                    "new review lacks exact paired repair and changed source",
                ));
            }
            Ok(())
        }
        _ => Err(round_error(
            "review cannot reset, skip, or reorder its actual round ancestry",
        )),
    }
}
fn verify_findings(
    snapshot: &ControlSnapshot,
    next: &ReviewRoundReference,
    findings: &FindingsReference,
    receipt: &Receipt,
    previous: &ReviewRoundReference,
) -> Result<()> {
    findings.review.matches_observation(receipt)?;
    let run = closed_worker(receipt, "completed")?;
    require_not_held(snapshot, receipt)?;
    require_spacing(receipt)?;
    if run.phase != next.review_phase
        || previous.review_phase != next.review_phase
        || findings.round_id != previous.round_id
        || findings.round_ordinal != previous.ordinal
        || findings.round_reference_digest != canonical_digest(1, previous)?
        || findings.reviewed_source_digest != previous.source.source_digest
    {
        return Err(round_error(
            "Findings reference differs from actual latest reviewed source and round",
        ));
    }
    // The protected typed Findings claim and actual joined completion are
    // independently mandatory in the final opaque producer callback.
    Ok(())
}
fn verify_persisted_preparation(
    receipts: &[Receipt],
    run: &Receipt,
    round: &ReviewRoundReference,
) -> Result<()> {
    let reference = ReviewReceiptReference::preparation_for(run)?;
    let prep = exact_receipt(receipts, &reference)?;
    let actual = PreparationReference::from_receipt(prep)?
        .ok_or_else(|| round_error("referenced preparation lacks worker identity"))?;
    let result = prep
        .observed_result
        .as_ref()
        .ok_or_else(|| round_error("preparation has no exit"))?;
    if prep.runtime_id != run.runtime_id
        || prep.actor != run.actor
        || prep.owner_generation != run.owner_generation
        || actual.worker_operation != run.operation_id
        || prep.operation_id != preparation_operation(run.operation_id)?
        || prep.phase != ReceiptPhase::Reconciled
        || result["local_execution"] != "exited"
        || result["unresolved_effects"]
            .as_array()
            .is_none_or(|v| !v.is_empty())
        || result["observation"]["phase"] != "worker_preparation"
        || result["observation"]["result"] != "prepared"
        || ReviewRoundReference::from_receipt(prep)?.as_ref() != Some(round)
    {
        return Err(round_error(
            "persisted model/preparation round lineage differs",
        ));
    }
    Ok(())
}
fn verify_review_session_caps(run: &RunReference, history: &BTreeMap<u32, Receipt>) -> Result<()> {
    for receipt in history.values() {
        let old = RunReference::from_receipt(receipt)?
            .ok_or_else(|| round_error("review run history missing"))?;
        if old.workspace != run.workspace
            || old.account != run.account
            || old.profile_digest != run.profile_digest
            || run.max_attempts > old.max_attempts
            || run.max_turns > old.max_turns
            || run.max_review_rounds > old.max_review_rounds
        {
            return Err(Error::new(
                "attempts_exhausted",
                "review admission would reset retained session bounds",
            ));
        }
    }
    Ok(())
}
fn require_spacing(receipt: &Receipt) -> Result<()> {
    if Utc::now().signed_duration_since(receipt.updated_at) < chrono::Duration::seconds(1) {
        return Err(Error::new(
            "continuation_wait",
            "round transition requires one second after exact reconciliation",
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "control_review_round_admission_tests.rs"]
mod tests;
