//! Flat actual receipt validation for multiple repair sessions in
//! one Findings round. This is an internal Control data verifier, not a source,
//! failure, completion or permit constructor. Call only with protected inventory
//! under the existing serialized admission and its required opaque callback.
use super::*;
use crate::control::RepairValidationReference;

pub(super) fn findings(round: &ReviewRoundReference) -> Option<&FindingsReference> {
    match &round.cause {
        ReviewRoundCause::PairedRepair { findings }
        | ReviewRoundCause::RepairContinuation { findings, .. } => Some(findings),
        _ => None,
    }
}

/// Verify actual same-phase repair attempts in increasing attempt order. No
/// recursive parsing, guessed timestamp order or second Findings consumption.
pub(super) fn verify_repair_chain(
    snapshot: &ControlSnapshot,
    expected: &ReviewRoundReference,
    last: &ReviewReceiptReference,
    receipts: &[Receipt],
) -> Result<()> {
    if receipts.len() > MAX_ROUND_RECEIPTS {
        return Err(Error::new(
            "review_round_history_limit",
            "pair history exceeds protected inventory bound",
        ));
    }
    let expected_findings =
        findings(expected).ok_or_else(|| round_error("pair chain requires a repair marker"))?;
    let review = exact_receipt(receipts, &expected_findings.review)?;
    let review_marker = ReviewRoundReference::from_receipt(review)?
        .ok_or_else(|| round_error("original Findings lost its review marker"))?;
    verify_findings(
        snapshot,
        expected,
        expected_findings,
        review,
        &review_marker,
    )?;
    let original = exact_receipt(receipts, &expected.source.implementation)?;
    let original_run = closed_worker(original, "completed")?;
    let terminal = exact_receipt(receipts, last)?;
    let terminal_run = closed_worker(terminal, "completed")?;
    if terminal_run.phase != original_run.phase || terminal_run.attempt <= original_run.attempt {
        return Err(round_error(
            "pair chain has no later implementation in the original budget",
        ));
    }
    let mut selected = BTreeMap::new();
    for receipt in receipts {
        let Some(run) = RunReference::from_receipt(receipt)? else {
            continue;
        };
        if receipt.runtime_id == expected.runtime
            && run.workspace == expected.workspace
            && run.phase == original_run.phase
            && run.attempt > original_run.attempt
            && run.attempt <= terminal_run.attempt
            && selected.insert(run.attempt, (receipt, run)).is_some()
        {
            return Err(round_error(
                "pair chain has conflicting implementation ordinals",
            ));
        }
    }
    let mut previous = original;
    let mut previous_run = original_run;
    let mut previous_marker: Option<ReviewRoundReference> = None;
    for (receipt, run) in selected.values() {
        let marker = ReviewRoundReference::from_receipt(receipt)?
            .ok_or_else(|| round_error("pair chain loses typed round ancestry"))?;
        let actual_run = closed_worker(receipt, "completed")?;
        require_not_held(snapshot, receipt)?;
        verify_persisted_preparation(receipts, receipt, &marker)?;
        if canonical_digest(1, &actual_run)? != canonical_digest(1, run)?
            || receipt.runtime_id != expected.runtime
            || run.workspace != expected.workspace
            || run.account != expected.account
            || run.phase != expected.source.implementation_phase
            || Some(run.attempt) != previous_run.attempt.checked_add(1)
            || run.max_attempts > previous_run.max_attempts
            || run.max_turns > previous_run.max_turns
            || run.max_review_rounds > previous_run.max_review_rounds
            || run.profile_digest != previous_run.profile_digest
            || marker.review_phase != expected.review_phase
            || marker.runtime != expected.runtime
            || marker.workspace != expected.workspace
            || marker.account != expected.account
            || marker.source != expected.source
            || marker.round_id != expected.round_id
            || marker.ordinal != expected.ordinal
            || findings(&marker) != Some(expected_findings)
        {
            return Err(round_error(
                "pair chain changes ordinal, source, findings or original implementation caps",
            ));
        }
        match (&marker.cause, &previous_marker) {
            (ReviewRoundCause::PairedRepair { .. }, None) => {
                if previous.operation_id != expected.source.implementation.operation
                    || marker.round_limit > review_marker.round_limit
                {
                    return Err(round_error(
                        "initial pair does not follow reviewed implementation",
                    ));
                }
            }
            (
                ReviewRoundCause::RepairContinuation {
                    previous_round_reference_digest,
                    current_source,
                    validation,
                    ..
                },
                Some(prior),
            ) => {
                if *previous_round_reference_digest != canonical_digest(1, prior)?
                    || current_source.implementation.operation != previous.operation_id
                    || current_source.implementation.receipt_digest
                        != canonical_digest(1, previous)?
                    || marker.round_limit > prior.round_limit
                {
                    return Err(round_error(
                        "continued pair skips or substitutes its actual previous repair",
                    ));
                }
                verify_source_reference(snapshot, &marker, current_source, receipts, false)?;
                verify_validation(
                    snapshot,
                    &marker,
                    current_source,
                    validation,
                    receipts,
                    false,
                )?;
            }
            _ => {
                return Err(round_error(
                    "Findings pair resets or omits a repair-chain link",
                ));
            }
        }
        previous = receipt;
        previous_run = run.clone();
        previous_marker = Some(marker);
    }
    last.matches_observation(previous)?;
    let prior = previous_marker.ok_or_else(|| round_error("pair chain is empty"))?;
    // For a pending continuation, the latest closed run is its predecessor.
    // For an AfterRepair source check, expected is that closed run's own marker.
    if let ReviewRoundCause::RepairContinuation {
        current_source,
        previous_round_reference_digest,
        ..
    } = &expected.cause
        && current_source.implementation == *last
        && (*previous_round_reference_digest != canonical_digest(1, &prior)?
            || expected.round_limit > prior.round_limit)
    {
        return Err(round_error(
            "next repair changes latest pair marker or raises its cap",
        ));
    }
    require_spacing(previous)
}

pub(super) fn verify_validation(
    snapshot: &ControlSnapshot,
    round: &ReviewRoundReference,
    source: &ReviewedSourceReference,
    validation: &RepairValidationReference,
    receipts: &[Receipt],
    current: bool,
) -> Result<()> {
    let receipt = exact_receipt(receipts, &validation.validation)?;
    let run = closed_worker(receipt, "completed")?;
    require_not_held(snapshot, receipt)?;
    require_spacing(receipt)?;
    let link = SourceAdvancedReference::from_receipt(receipt)?
        .ok_or_else(|| round_error("failed local validation lost exact source advancement"))?;
    let prior = exact_receipt(
        receipts,
        &ReviewReceiptReference {
            operation: link.predecessor,
            receipt_digest: link.predecessor_receipt_digest.clone(),
        },
    )?;
    let prior_run = closed_worker(prior, "completed")?;
    require_not_held(snapshot, prior)?;
    if receipt.runtime_id != round.runtime
        || run.workspace != round.workspace
        || run.account != round.account
        || run.phase != validation.phase
        || run.phase.phase != "child_local_validation"
        || run.phase.work != round.review_phase.work
        || run.phase.generation != round.review_phase.generation
        || link != validation.source_advanced
        || link.source_operation != source.implementation.operation
        || link.source_receipt_digest != source.implementation.receipt_digest
        || prior_run.phase != run.phase
        || prior_run.workspace != run.workspace
        || prior_run.account != run.account
        || Some(run.attempt) != prior_run.attempt.checked_add(1)
        || run.max_attempts > prior_run.max_attempts
        || run.max_turns > prior_run.max_turns
        || run.max_review_rounds > prior_run.max_review_rounds
        || run.profile_digest != prior_run.profile_digest
    {
        return Err(round_error(
            "local validation checks another source, phase or predecessor",
        ));
    }
    let mut inventory = BTreeMap::new();
    for observed in receipts {
        let Some(candidate) = RunReference::from_receipt(observed)? else {
            continue;
        };
        if observed.runtime_id == round.runtime
            && candidate.workspace == round.workspace
            && candidate.phase.work == run.phase.work
            && candidate.phase.generation == run.phase.generation
            && candidate.phase.phase == run.phase.phase
        {
            if current && (candidate.phase != run.phase || candidate.attempt > run.attempt) {
                return Err(round_error(
                    "later or mixed-grant validation supersedes repair obligation",
                ));
            }
            // Historical link validates its exact original prefix. Later
            // completed validation is not retroactive revocation of that fact.
            if (current || candidate.phase == run.phase && candidate.attempt <= run.attempt)
                && inventory.insert(observed.operation_id, observed).is_some()
            {
                return Err(round_error("duplicate validation receipt inventory"));
            }
        }
    }
    if canonical_digest(1, &inventory)? != validation.failure_inventory_digest {
        return Err(round_error(
            "failed validation inventory changed or disappeared",
        ));
    }
    // The actual protected outcome claim and configured failed-check execution
    // capabilities remain mandatory in the opaque producer/final native gate.
    Ok(())
}
