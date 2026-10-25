//! Immutable SourceAdvanced provenance for exact-source validation retries.
//! This is a private reader over actual opaque causes and whole Control records.
//! It never places SourceAdvanced metadata on the retried admission itself.
use super::*;
use crate::control::SourceAdvancedReference;

#[derive(Clone)]
pub(super) struct ValidationSourceLineage {
    advanced: Arc<VerifiedSourceAdvanced>,
    origin: Receipt,
    origin_run: RunReference,
    origin_completion: Arc<RunCompletion>,
    source_receipt: Receipt,
    prefix: BTreeMap<Uuid, String>,
    digest: String,
}
impl ValidationSourceLineage {
    pub(super) fn advanced(&self) -> &Arc<VerifiedSourceAdvanced> {
        &self.advanced
    }
    pub(super) fn origin(&self) -> &Receipt {
        &self.origin
    }
    pub(super) fn digest(&self) -> &str {
        &self.digest
    }
    pub(super) fn from_original(
        assignment: &DeliveryWorkerAssignment,
        completion: &Arc<RunCompletion>,
        receipt: &Receipt,
        run: &RunReference,
        advanced: Option<Arc<VerifiedSourceAdvanced>>,
        receipts: &[Receipt],
        grants: &[VerifiedRetryGrant],
    ) -> Result<Option<Self>> {
        let marker = SourceAdvancedReference::from_receipt(receipt)?;
        let Some(advanced) = advanced else {
            if marker.is_some() || run.phase.retry_grant.is_some() {
                return Err(Error::new(
                    "worker_binding",
                    "validation retry omitted its actual source or prior retry lineage",
                ));
            }
            require_plain_retry(receipt)?;
            for item in receipts {
                if let Some(reference) = RunReference::from_receipt(item)?
                    && same_group(receipt.runtime_id, run, item, &reference)
                    && SourceAdvancedReference::from_receipt(item)?.is_some()
                {
                    return Err(Error::new(
                        "worker_binding",
                        "validation retry cannot discard an earlier source advancement",
                    ));
                }
            }
            return Ok(None);
        };
        if marker.as_ref() != Some(&source_reference(&advanced))
            || advanced.phase().phase != "child_local_validation"
            || advanced.requires_review_round()
        {
            return Err(Error::new(
                "worker_binding",
                "failed validation was not admitted by this exact source advancement",
            ));
        }
        advanced.verify_assignment(assignment)?;
        advanced.verify_contract(completion.contract.as_ref().ok_or_else(|| {
            Error::new("worker_binding", "original validation contract is absent")
        })?)?;
        let source_receipt = unique_receipt(receipts, advanced.source_operation())?.clone();
        verify_original_header(&advanced, receipt, run, source_receipt.runtime_id)?;
        closed_failed_validation(receipt)?;
        bound_completion(assignment, completion, receipt, run)?;
        let mut prefix = BTreeMap::new();
        for item in receipts {
            if let Some(reference) = RunReference::from_receipt(item)?
                && same_group(receipt.runtime_id, run, item, &reference)
                && item.operation_id != receipt.operation_id
            {
                if prefix.len() >= MAX_VALIDATION_HISTORY_RECEIPTS {
                    return Err(Error::new(
                        "bounded_input",
                        "validation source prefix is oversized",
                    ));
                }
                if prefix
                    .insert(item.operation_id, canonical_digest(1, item)?)
                    .is_some()
                {
                    return Err(Error::new(
                        "record_corrupt",
                        "duplicate source prefix receipt",
                    ));
                }
            }
        }
        let digest = canonical_digest(
            1,
            &json!({
                "purpose":"validation_retry_source_lineage",
                "source_advanced":advanced.digest(),
                "origin":canonical_digest(1, receipt)?,
                "source":canonical_digest(1, &source_receipt)?,
                "prefix":prefix,
            }),
        )?;
        let lineage = Self {
            advanced,
            origin: receipt.clone(),
            origin_run: run.clone(),
            origin_completion: completion.clone(),
            source_receipt,
            prefix,
            digest,
        };
        // At first mint only the actual original terminal is removed. A later
        // independent validation remains in the prefix and fails VSA's exact
        // pre-admission inventory. No caller chooses this projection.
        let prior: Vec<_> = receipts
            .iter()
            .filter(|r| r.operation_id != receipt.operation_id)
            .cloned()
            .collect();
        lineage
            .advanced
            .verify_consumed_validation_prefix(&prior, grants, receipt)?;
        lineage.verify_prefix(receipts, grants)?;
        Ok(Some(lineage))
    }
    /// The retry verifier must first validate its whole current phase inventory
    /// and all current native grants. Only then can the exact recorded original
    /// prefix be passed to VSA's immutable lineage verifier. Nothing is hidden
    /// from the retry verifier; this removes its authenticated successors only.
    pub(super) fn verify_prefix(
        &self,
        receipts: &[Receipt],
        grants: &[VerifiedRetryGrant],
    ) -> Result<()> {
        let origin = unique_receipt(receipts, self.origin.operation_id)?;
        if canonical_digest(1, origin)? != canonical_digest(1, &self.origin)? {
            return Err(Error::new(
                "stale_result",
                "original source validation receipt changed",
            ));
        }
        let source = unique_receipt(receipts, self.advanced.source_operation())?;
        if canonical_digest(1, source)? != self.advanced.source_receipt_digest()
            || canonical_digest(1, source)? != canonical_digest(1, &self.source_receipt)?
        {
            return Err(Error::new(
                "stale_result",
                "original repaired implementation receipt changed",
            ));
        }
        let predecessor = unique_receipt(receipts, self.advanced.predecessor())?;
        if canonical_digest(1, predecessor)? != self.advanced.predecessor_receipt_digest()
            || predecessor
                .observed_result
                .as_ref()
                .and_then(|v| v.get("observation"))
                != Some(&self.advanced.previous_completion().observation())
        {
            return Err(Error::new(
                "stale_result",
                "source advancement predecessor changed",
            ));
        }
        if SourceAdvancedReference::from_receipt(origin)?.as_ref()
            != Some(&source_reference(&self.advanced))
        {
            return Err(Error::new(
                "worker_binding",
                "original validation source marker changed",
            ));
        }
        verify_original_header(
            &self.advanced,
            origin,
            &self.origin_run,
            self.source_receipt.runtime_id,
        )?;
        closed_failed_validation(origin)?;
        bound_completion(
            self.advanced.assignment(),
            &self.origin_completion,
            origin,
            &self.origin_run,
        )?;
        let mut prefix = Vec::new();
        let mut found = BTreeSet::new();
        for receipt in receipts {
            let group = RunReference::from_receipt(receipt)?.is_some_and(|run| {
                same_group(self.origin.runtime_id, &self.origin_run, receipt, &run)
            });
            if !group {
                prefix.push(receipt.clone());
            } else if let Some(expected) = self.prefix.get(&receipt.operation_id) {
                if !found.insert(receipt.operation_id) || canonical_digest(1, receipt)? != *expected
                {
                    return Err(Error::new(
                        "stale_result",
                        "validation source prefix changed",
                    ));
                }
                prefix.push(receipt.clone());
            } else if receipt.operation_id != self.origin.operation_id {
                // Every tail entry is separately in the whole current retry
                // inventory and exact native grant ancestry. It cannot be a new
                // SourceAdvanced/continuation/resume cause hidden by projection.
                require_plain_retry(receipt)?;
                if RunReference::from_receipt(receipt)?
                    .is_none_or(|run| run.phase.retry_grant.is_none())
                {
                    return Err(Error::new(
                        "stale_result",
                        "ungranted validation appeared after source origin",
                    ));
                }
                closed_failed_validation(receipt)?;
            }
        }
        if found.len() != self.prefix.len() {
            return Err(Error::new(
                "reconciliation_required",
                "original validation prefix is incomplete",
            ));
        }
        self.advanced
            .verify_consumed_validation_prefix(&prefix, grants, origin)
    }
    pub(super) fn verify_joined(
        &self,
        retained: &impl Fn(Uuid) -> Result<Option<Arc<RunCompletion>>>,
    ) -> Result<()> {
        require_joined(&self.origin_completion, retained)?;
        self.advanced.verify_joined(retained)?;
        let source = retained(self.advanced.source_operation())?.ok_or_else(|| {
            Error::new(
                "reconciliation_required",
                "original repaired source completion is unavailable",
            )
        })?;
        if !source.local_exit_confirmed
            || !matches!(source.outcome, RunOutcome::Completed)
            || source.operation_id != self.source_receipt.operation_id
            || self
                .source_receipt
                .observed_result
                .as_ref()
                .and_then(|v| v.get("observation"))
                != Some(&source.observation())
        {
            return Err(Error::new(
                "worker_binding",
                "retained repaired source differs from actual completed receipt",
            ));
        }
        Ok(())
    }
}
fn source_reference(advanced: &VerifiedSourceAdvanced) -> SourceAdvancedReference {
    SourceAdvancedReference {
        predecessor: advanced.predecessor(),
        predecessor_receipt_digest: advanced.predecessor_receipt_digest().into(),
        source_operation: advanced.source_operation(),
        source_receipt_digest: advanced.source_receipt_digest().into(),
        trigger_digest: advanced.digest().into(),
        progress_digest: advanced.progress_digest().into(),
        review_rounds_consumed: advanced.review_rounds_consumed(),
    }
}
fn verify_original_header(
    advanced: &VerifiedSourceAdvanced,
    receipt: &Receipt,
    run: &RunReference,
    runtime: Uuid,
) -> Result<()> {
    let old = advanced.reference();
    if run.phase != *advanced.phase()
        || run.workspace != old.workspace
        || run.account != old.account
        || run.profile_digest != old.profile_digest
        || run.attempt
            != old.attempt.checked_add(1).ok_or_else(|| {
                Error::new("attempts_exhausted", "original validation ordinal overflow")
            })?
        || run.attempt > run.max_attempts
        || [run.max_attempts, run.max_turns, run.max_review_rounds].contains(&0)
        || run.max_attempts > old.max_attempts
        || run.max_turns > old.max_turns
        || run.max_review_rounds > old.max_review_rounds
        || receipt.runtime_id != runtime
    {
        return Err(Error::new(
            "worker_binding",
            "source validation differs from actual bounded next admission",
        ));
    }
    Ok(())
}
pub(super) fn require_plain_retry(receipt: &Receipt) -> Result<()> {
    // Typed parsers validate actual marker syntax; malformed metadata is not
    // treated as an absent marker. A new grant never adopts a source/round cause.
    if crate::control::SourceAdvancedReference::from_receipt(receipt)?.is_some()
        || crate::control::RunContinuationReference::from_receipt(receipt)?.is_some()
        || crate::control::RunResumeReference::from_receipt(receipt)?.is_some()
        || crate::control::ReviewRoundReference::from_receipt(receipt)?.is_some()
    {
        return Err(Error::new(
            "worker_binding",
            "granted validation cannot reinterpret another admission cause",
        ));
    }
    Ok(())
}
pub(super) fn require_joined(
    expected: &Arc<RunCompletion>,
    retained: &impl Fn(Uuid) -> Result<Option<Arc<RunCompletion>>>,
) -> Result<()> {
    let actual = retained(expected.operation_id)?.ok_or_else(|| {
        Error::new(
            "reconciliation_required",
            "source validation joined completion is missing",
        )
    })?;
    if !Arc::ptr_eq(&actual, expected) || !actual.local_exit_confirmed {
        return Err(Error::new(
            "worker_binding",
            "source validation lost its actual joined completion",
        ));
    }
    Ok(())
}
