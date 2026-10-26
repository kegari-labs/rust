//! Read-only completed resume closure. The marker is data, while the required
//! native callback separately retains the real admitted bridges and joined Arcs.
//! The worklist is flat; reading nested repaired sources does not recurse.
use super::*;

pub(in crate::control) fn verify_completed_paired_result(
    snapshot: &ControlSnapshot,
    expected: &ReviewRoundReference,
    reference: &PairedResumeResultReference,
    receipts: &[Receipt],
) -> Result<()> {
    let mut pending = vec![reference.clone()];
    let mut seen = BTreeMap::new();
    while let Some(reference) = pending.pop() {
        reference.validate()?;
        let digest = canonical_digest(1, &reference)?;
        if let Some(prior) = seen.insert(reference.terminal.operation, digest.clone()) {
            if prior != digest {
                return Err(round_error(
                    "resumed source has conflicting historical closure",
                ));
            }
            continue;
        }
        if seen.len() > MAX_ROUND_RECEIPTS || receipts.len() > MAX_ROUND_RECEIPTS {
            return Err(round_error(
                "resumed source exceeds protected receipt bound",
            ));
        }
        let terminal = exact_receipt(receipts, &reference.terminal)?;
        let round = ReviewRoundReference::from_receipt(terminal)?
            .ok_or_else(|| round_error("completed resume lost its exact round"))?;
        if !round.is_repair()
            || round.runtime != expected.runtime
            || round.workspace != expected.workspace
            || round.account != expected.account
            || round.subject_phase().work != expected.subject_phase().work
            || round.subject_phase().generation != expected.subject_phase().generation
            || canonical_digest(1, &round)? != reference.round_reference_digest
        {
            return Err(round_error(
                "completed resume changes exact work or original round",
            ));
        }
        verify_passed_validation(snapshot, &round, receipts)?;
        let mut current = terminal;
        let mut chain = BTreeMap::new();
        let mut visited = BTreeSet::new();
        loop {
            if !visited.insert(current.operation_id) || visited.len() > MAX_ROUND_RECEIPTS {
                return Err(round_error(
                    "completed resume ancestry cycles or exceeds bound",
                ));
            }
            let is_terminal = current.operation_id == terminal.operation_id;
            let run = closed_worker(current, if is_terminal { "completed" } else { "failed" })?;
            require_not_held(snapshot, current)?;
            if current.runtime_id != round.runtime
                || run.workspace != round.workspace
                || run.account != round.account
                || run.phase != *round.subject_phase()
                || ReviewRoundReference::from_receipt(current)?.as_ref() != Some(&round)
            {
                return Err(round_error(
                    "completed resume changes original implementation namespace",
                ));
            }
            chain.insert(current.operation_id, canonical_digest(1, current)?);
            if current.operation_id == reference.origin.operation {
                break;
            }
            let link = ReviewRoundResumeReference::from_model_receipt(current)?
                .ok_or_else(|| round_error("completed resume skips a typed admitted bridge"))?;
            if link.recovery.origin != reference.origin
                || link.recovery.origin_preparation != reference.origin_preparation
                || link.recovery.round_plan_digest != reference.round_plan_digest
                || link.recovery.round_reference_digest != reference.round_reference_digest
                || (is_terminal && canonical_digest(1, &link)? != reference.resume_reference_digest)
            {
                return Err(round_error(
                    "completed resume replaced original protected lineage",
                ));
            }
            verify_closed_metadata(snapshot, receipts, &round, &link, None)?;
            let metadata = exact_receipt(receipts, &link.metadata)?;
            let previous = exact_receipt(receipts, &link.recovery.predecessor)?;
            verify_transient(previous, &link.recovery)?;
            let old = closed_worker(previous, "failed")?;
            let previous_link = ReviewRoundResumeReference::from_model_receipt(previous)?;
            if Some(run.attempt) != old.attempt.checked_add(1)
                || run.phase != old.phase
                || run.workspace != old.workspace
                || run.account != old.account
                || run.profile_digest != old.profile_digest
                || run.max_attempts > old.max_attempts
                || run.max_turns > old.max_turns
                || run.max_review_rounds > old.max_review_rounds
                || metadata.actor != current.actor
                || metadata.owner_generation != current.owner_generation
                || previous.actor != current.actor
                || previous.owner_generation != current.owner_generation
                || link.recovery.previous_resume_digest
                    != previous_link
                        .as_ref()
                        .map(|r| canonical_digest(1, r))
                        .transpose()?
            {
                return Err(round_error(
                    "completed resume resets bounds, owner or previous bridge",
                ));
            }
            let resume = RunResumeReference::from_receipt(current)?
                .ok_or_else(|| round_error("completed resume lost session marker"))?;
            if resume.review_rounds_consumed != retained_round_floor(previous)?.max(round.ordinal) {
                return Err(round_error(
                    "completed resume changed retained implementation round floor",
                ));
            }
            chain.insert(metadata.operation_id, canonical_digest(1, metadata)?);
            current = previous;
        }
        reference.origin.matches_observation(current)?;
        if ReviewRoundResumeReference::from_model_receipt(current)?.is_some() {
            return Err(round_error(
                "completed resume origin is itself a resumed model",
            ));
        }
        if ReviewReceiptReference::preparation_for(current)? != reference.origin_preparation {
            return Err(round_error(
                "completed resume changed actual original preparation",
            ));
        }
        let origin_run = closed_worker(current, "failed")?;
        let dispatch = exact_receipt(receipts, &round.dispatch_source().implementation)?;
        let dispatch_run = closed_worker(dispatch, "completed")?;
        if dispatch_run.phase != origin_run.phase
            || dispatch_run.workspace != origin_run.workspace
            || dispatch_run.account != origin_run.account
            || dispatch_run.profile_digest != origin_run.profile_digest
            || dispatch_run.attempt.checked_add(1) != Some(origin_run.attempt)
            || origin_run.max_attempts > dispatch_run.max_attempts
            || origin_run.max_turns > dispatch_run.max_turns
            || origin_run.max_review_rounds > dispatch_run.max_review_rounds
        {
            return Err(round_error(
                "completed resume origin skips its exact source or increases its caps",
            ));
        }
        verify_persisted_preparation(receipts, current, &round)?;
        let prep = exact_receipt(receipts, &reference.origin_preparation)?;
        chain.insert(prep.operation_id, canonical_digest(1, prep)?);
        if canonical_digest(1, &chain)? != reference.history_digest {
            return Err(round_error(
                "completed resume model/metadata closure changed",
            ));
        }
        let findings = pair_chain::findings(&round)
            .ok_or_else(|| round_error("completed resumed repair has no original Findings"))?;
        let review = exact_receipt(receipts, &findings.review)?;
        let review_round = ReviewRoundReference::from_receipt(review)?
            .ok_or_else(|| round_error("original Findings lost its round"))?;
        verify_findings(snapshot, &round, findings, review, &review_round)?;
        verify_passed_validation(snapshot, &review_round, receipts)?;
        verify_persisted_preparation(receipts, review, &review_round)?;
        verify_prior_repair_sources(snapshot, &round, current, receipts, &mut pending)?;
        for source in [&round.source, round.dispatch_source(), &review_round.source] {
            let source_run = closed_worker(
                exact_receipt(receipts, &source.implementation)?,
                "completed",
            )?;
            if source_run.phase != origin_run.phase || source_run.attempt >= origin_run.attempt {
                return Err(round_error(
                    "resumed source history cycles or crosses implementation namespace",
                ));
            }
            verify_source_reference_fields(snapshot, &round, source, receipts, false)?;
            if let Some(previous) = &source.paired_resume {
                pending.push(previous.clone());
            }
        }
    }
    Ok(())
}

pub(super) fn retained_round_floor(receipt: &Receipt) -> Result<u32> {
    let mut value = 0;
    if let Some(r) = RunContinuationReference::from_receipt(receipt)? {
        value = value.max(r.review_rounds_consumed);
    }
    if let Some(r) = SourceAdvancedReference::from_receipt(receipt)? {
        value = value.max(r.review_rounds_consumed);
    }
    if let Some(r) = RunResumeReference::from_receipt(receipt)? {
        value = value.max(r.review_rounds_consumed);
    }
    Ok(value)
}

/// Validate the original admitted continuation's full prior pair/validation
/// closure. Resumed sources are queued into the outer worklist, never decoded
/// recursively. Ordinary prior pair sources decrease the implementation attempt.
pub(in crate::control) fn verify_prior_repair_sources(
    snapshot: &ControlSnapshot,
    expected: &ReviewRoundReference,
    origin: &Receipt,
    receipts: &[Receipt],
    pending: &mut Vec<PairedResumeResultReference>,
) -> Result<()> {
    let mut round = expected.clone();
    let mut current = origin;
    let mut seen = BTreeSet::new();
    loop {
        if !seen.insert(current.operation_id) || seen.len() > MAX_ROUND_RECEIPTS {
            return Err(round_error(
                "completed resumed pair source prefix cycles or exceeds bound",
            ));
        }
        verify_passed_validation(snapshot, &round, receipts)?;
        let run = RunReference::from_receipt(current)?
            .ok_or_else(|| round_error("paired source prefix lost its actual run"))?;
        let source = round.dispatch_source();
        let source_receipt = exact_receipt(receipts, &source.implementation)?;
        let source_run = closed_worker(source_receipt, "completed")?;
        verify_source_reference_fields(snapshot, &round, source, receipts, false)?;
        if source_run.phase != run.phase
            || source_run.workspace != run.workspace
            || source_run.account != run.account
            || source_run.profile_digest != run.profile_digest
            || source_run.attempt.checked_add(1) != Some(run.attempt)
            || run.max_attempts > source_run.max_attempts
            || run.max_turns > source_run.max_turns
            || run.max_review_rounds > source_run.max_review_rounds
        {
            return Err(round_error(
                "completed pair source prefix skips an attempt or resets its caps",
            ));
        }
        let ReviewRoundCause::RepairContinuation {
            previous_round_reference_digest,
            current_source,
            validation,
            ..
        } = &round.cause
        else {
            if !matches!(round.cause, ReviewRoundCause::PairedRepair { .. }) {
                return Err(round_error(
                    "completed pair source prefix is not a Findings repair",
                ));
            }
            return Ok(());
        };
        let prior = ReviewRoundReference::from_receipt(source_receipt)?
            .ok_or_else(|| round_error("continued pair lost its actual previous round marker"))?;
        if !prior.is_repair()
            || prior.runtime != round.runtime
            || prior.workspace != round.workspace
            || prior.account != round.account
            || prior.review_phase != round.review_phase
            || prior.subject_phase() != round.subject_phase()
            || prior.source != round.source
            || prior.round_id != round.round_id
            || prior.ordinal != round.ordinal
            || pair_chain::findings(&prior) != pair_chain::findings(&round)
            || round.round_limit > prior.round_limit
            || canonical_digest(1, &prior)? != *previous_round_reference_digest
        {
            return Err(round_error(
                "continued completed pair substituted its prior round or Findings",
            ));
        }
        verify_passed_validation(snapshot, &prior, receipts)?;
        pair_chain::verify_validation(
            snapshot,
            &round,
            current_source,
            validation,
            receipts,
            false,
        )?;
        match (
            &current_source.paired_resume,
            ReviewRoundResumeReference::from_model_receipt(source_receipt)?,
        ) {
            (Some(reference), Some(_)) => {
                reference.terminal.matches_observation(source_receipt)?;
                pending.push(reference.clone());
                if pending.len() > MAX_ROUND_RECEIPTS {
                    return Err(round_error("completed pair source worklist exceeds bound"));
                }
                return Ok(());
            }
            (None, None) => verify_persisted_preparation(receipts, source_receipt, &prior)?,
            _ => {
                return Err(round_error(
                    "continued pair source omitted or invented resumed lineage",
                ));
            }
        }
        current = source_receipt;
        round = prior;
    }
}
