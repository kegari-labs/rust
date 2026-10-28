//! Exact admitted successor observations, not execution or completion authority.
//! Only the retained opaque passed capability and current round/native gate may
//! use these data to move from latest-validation checks to immutable evidence.
use super::*;
use crate::control::PassedValidationReference;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PassedValidationSuccessor {
    operation: Uuid,
    round: ReviewRoundReference,
    preparation: ReviewReceiptReference,
    request_digest: String,
}
impl PassedValidationSuccessor {
    pub(crate) fn operation(&self) -> Uuid {
        self.operation
    }
    pub(crate) fn round(&self) -> &ReviewRoundReference {
        &self.round
    }
    pub(crate) fn preparation(&self) -> &ReviewReceiptReference {
        &self.preparation
    }
    pub(crate) fn request_digest(&self) -> &str {
        &self.request_digest
    }
}

/// Structural protected-admission read only: this cannot prove current native
/// permission, a process join or model completion, and cannot mint a capability.
pub(crate) fn verify_validation_successor_admission(
    receipts: &[Receipt],
    worker_operation: Uuid,
    passed: &PassedValidationReference,
) -> Result<PassedValidationSuccessor> {
    if receipts.len() > MAX_RECEIPTS {
        return Err(history_limit());
    }
    let model = actual_receipt(receipts, worker_operation)?;
    let round = ReviewRoundReference::from_receipt(model)?
        .ok_or_else(|| blocked("passed validation successor has no exact round"))?;
    if round.is_repair() || round.passed_validation.as_ref() != Some(passed) {
        return Err(blocked(
            "review successor changed its passed validation reference",
        ));
    }
    passed.validate_for_round(&round)?;
    let run = crate::control::run_reference_from_receipt(model)?
        .ok_or_else(|| blocked("review successor is not model admission"))?;
    let preparation = ReviewReceiptReference::preparation_for(model)?;
    let prep = actual_receipt(receipts, preparation.operation)?;
    preparation.matches_observation(prep)?;
    let expected = PreparationReference {
        worker_operation,
        workspace: run.workspace,
        account: run.account.clone(),
        phase: run.phase.clone(),
    };
    let actual = PreparationReference::from_receipt(prep)?
        .ok_or_else(|| blocked("review successor lacks typed preparation"))?;
    let result = prep
        .observed_result
        .as_ref()
        .ok_or_else(|| blocked("review successor preparation lacks exit"))?;
    if actual != expected
        || prep.operation_id != crate::control::preparation_operation(worker_operation)?
        || model.runtime_id != prep.runtime_id
        || model.owner_generation != prep.owner_generation
        || model.actor != prep.actor
        || ReviewRoundReference::from_receipt(prep)?.as_ref() != Some(&round)
        || prep.phase != ReceiptPhase::Reconciled
        || result["local_execution"] != "exited"
        || result["unresolved_effects"]
            .as_array()
            .is_none_or(|items| !items.is_empty())
        || result["observation"]["phase"] != "worker_preparation"
        || result["observation"]["result"] != "prepared"
    {
        return Err(blocked(
            "review successor changed exact prepared owner or round",
        ));
    }
    let request_digest = canonical_digest(
        1,
        &json!({
            "runtime":model.runtime_id,"operation":model.operation_id,"request":model.request_id,
            "owner_generation":model.owner_generation,"actor":model.actor,"kind":model.kind,
            "targets":model.canonical_targets,"payload_digest":model.payload_digest,"expected":model.expected
        }),
    )?;
    Ok(PassedValidationSuccessor {
        operation: worker_operation,
        round,
        preparation,
        request_digest,
    })
}
impl VerifiedReviewRoundPlan {
    /// Current native caller must additionally retain/check actual owner, phase,
    /// permission/fences and joined handles. The only history projection removes
    /// the exact structurally verified successor and its immutable preparation;
    /// no caller selects arbitrary omitted receipts.
    pub(crate) fn verify_validation_successor_current(
        &self,
        receipts: &[Receipt],
        grants: &[VerifiedRetryGrant],
        successor: &PassedValidationSuccessor,
    ) -> Result<()> {
        let passed = self
            .reference
            .passed_validation
            .as_ref()
            .ok_or_else(|| blocked("round omitted actual passed validation"))?;
        let actual = verify_validation_successor_admission(receipts, successor.operation, passed)?;
        if &actual != successor || actual.round != self.reference {
            return Err(blocked(
                "passed validation successor immutable request changed",
            ));
        }
        // The immutable request must also be the actual next attempt/cause of
        // this opaque plan. Structural marker equality alone cannot qualify a
        // live admission with a changed ordinal or raised retained caps.
        let model = actual_receipt(receipts, actual.operation)?;
        let run = crate::control::run_reference_from_receipt(model)?
            .ok_or_else(|| blocked("passed validation successor is not a model run"))?;
        self.verify_run_cause(model)?;
        self.verify_run_bounds(&run)?;
        let before: Vec<_> = receipts
            .iter()
            .filter(|r| {
                r.operation_id != actual.operation && r.operation_id != actual.preparation.operation
            })
            .cloned()
            .collect();
        self.verify_current(&before, grants)
    }
    /// Bind an actual closed successor after validating its immutable request.
    /// This remains data and cannot substitute for the actual joined completion.
    pub(crate) fn validation_successor_terminal(
        &self,
        receipts: &[Receipt],
        successor: &PassedValidationSuccessor,
        outcome: &str,
    ) -> Result<ReviewReceiptReference> {
        let passed = self
            .reference
            .passed_validation
            .as_ref()
            .ok_or_else(|| blocked("terminal successor omitted validation"))?;
        let actual = verify_validation_successor_admission(receipts, successor.operation, passed)?;
        if &actual != successor || actual.round != self.reference {
            return Err(blocked("terminal successor request changed"));
        }
        let receipt = actual_receipt(receipts, actual.operation)?;
        self.verify_terminal_header(receipts, receipt, outcome)?;
        receipt_reference(receipt)
    }
}

impl VerifiedReviewRoundPlan {
    pub(super) fn verify_passed_current(
        &self,
        receipts: &[Receipt],
        grants: &[VerifiedRetryGrant],
    ) -> Result<()> {
        if self.reference.passed_validation.as_ref()
            != self.passed_validation.as_ref().map(|p| p.reference())
        {
            return Err(blocked(
                "round omitted or substituted actual passed validation capability",
            ));
        }
        let Some(passed) = &self.passed_validation else {
            if let RoundBasis::ExplicitGrant(proof) = &self.basis {
                let prior = actual_receipt(receipts, proof.predecessor())?;
                if ReviewRoundReference::from_receipt(prior)?
                    .is_some_and(|p| p.passed_validation.is_some())
                {
                    return Err(blocked(
                        "granted review discarded original passed validation",
                    ));
                }
            }
            return Ok(());
        };
        if passed.source().digest()? != self.reference.source.source_digest {
            return Err(blocked("passed validation source differs from review"));
        }
        match &self.basis {
            RoundBasis::Initial | RoundBasis::AfterRepair { .. } => {
                passed.verify_current(receipts, grants)
            }
            RoundBasis::ExplicitGrant(proof) => {
                verify_validation_successor_admission(
                    receipts,
                    proof.predecessor(),
                    passed.reference(),
                )?;
                passed.verify_retained(receipts, grants)
            }
            RoundBasis::Transient { receipt, .. } => {
                verify_validation_successor_admission(
                    receipts,
                    receipt.operation_id,
                    passed.reference(),
                )?;
                passed.verify_retained(receipts, grants)
            }
            RoundBasis::PairedRepair { findings, .. } => {
                verify_validation_successor_admission(
                    receipts,
                    findings.receipt.operation_id,
                    passed.reference(),
                )?;
                passed.verify_retained(receipts, grants)
            }
            RoundBasis::RepairContinuation { previous, .. } => {
                let findings = previous.completed_findings()?;
                verify_validation_successor_admission(
                    receipts,
                    findings.receipt.operation_id,
                    passed.reference(),
                )?;
                passed.verify_retained(receipts, grants)
            }
        }
    }
    /// Metadata-only handoff for the explicit retry state. It recognizes one
    /// exact new native grant for this closed failed successor; it does not admit
    /// another model or waive the retry producer's actual joined/source checks.
    /// The caller supplies `failed` from its actual retained runtime reader and
    /// rechecks Arc identity inside its final serialized gate.
    pub(crate) fn verify_validation_successor_retry(
        &self,
        receipts: &[Receipt],
        grants: &[VerifiedRetryGrant],
        successor: &PassedValidationSuccessor,
        grant: &VerifiedRetryGrant,
        failed: &Arc<RunCompletion>,
    ) -> Result<()> {
        self.validation_successor_terminal(receipts, successor, "failed")?;
        let actual = actual_receipt(receipts, successor.operation)?;
        let run = closed_run(actual, "failed")?;
        require_recorded_session(actual, failed)?;
        if failed.operation_id != actual.operation_id
            || !failed.local_exit_confirmed
            || !matches!(failed.outcome, RunOutcome::Failed)
            || failed.hook_error.is_some()
            || failed.state.intervention.is_some()
            || failed.state.input_required
            || !failed.checks.is_empty()
            || !failed.failed_checks.is_empty()
            || failed.error.as_ref().is_none_or(|e| {
                matches!(
                    e.code.as_str(),
                    "operation_uncertain"
                        | "provider_intervention"
                        | "input_required"
                        | "interrupted"
                )
            })
        {
            return Err(blocked(
                "review retry handoff lacks actual resolved failed successor",
            ));
        }
        crate::runtime_workflow::require_current_grant(grant, grants)?;
        if grant.phase() != &run.phase
            || grant.record().runtime_id != actual.runtime_id
            || grant.record().workspace_id != run.workspace
            || grants.iter().filter(|g| g.phase() == grant.phase()).count() != 1
        {
            return Err(blocked(
                "review retry handoff has no unique exact native grant",
            ));
        }
        let mut prior = BTreeMap::new();
        let mut ordinals = BTreeMap::new();
        for receipt in receipts {
            if let Some(reference) = crate::control::run_reference_from_receipt(receipt)?
                && reference.phase == run.phase
            {
                let outcome = receipt
                    .observed_result
                    .as_ref()
                    .and_then(|v| v.pointer("/observation/outcome"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| blocked("review grant prefix is not terminal"))?;
                if !matches!(outcome, "failed" | "completed") {
                    return Err(blocked("review grant prefix is unresolved"));
                }
                closed_run(receipt, outcome)?;
                if receipt.runtime_id != actual.runtime_id
                    || reference.workspace != run.workspace
                    || reference.account != run.account
                    || ordinals
                        .insert(reference.attempt, receipt.operation_id)
                        .is_some()
                    || prior
                        .insert(receipt.operation_id, receipt.payload_digest.clone())
                        .is_some()
                {
                    return Err(blocked("review grant prefix changes worker namespace"));
                }
            }
        }
        if prior != grant.record().prior_admissions
            || ordinals.keys().copied().ne(1..=ordinals.len() as u32)
            || ordinals.last_key_value().map(|(_, id)| *id) != Some(actual.operation_id)
        {
            return Err(blocked(
                "review grant does not cover exact latest failed phase history",
            ));
        }
        let before: Vec<_> = receipts
            .iter()
            .filter(|r| {
                r.operation_id != actual.operation_id
                    && r.operation_id != successor.preparation.operation
            })
            .cloned()
            .collect();
        if inventory(&before, &self.reference)? != self.inventory_digest
            || implementation_inventory(&before, &self.reference)? != self.source_inventory_digest
        {
            return Err(blocked(
                "review retry successor history changed before new grant",
            ));
        }
        require_source_receipts(&self.source, receipts)?;
        require_latest_source(receipts, &self.reference, grants)?;
        for (id, hash) in &self.retained {
            if canonical_digest(1, actual_receipt(receipts, *id)?)? != *hash {
                return Err(blocked("review retry immutable ancestry changed"));
            }
        }
        for original in self.grant.iter().chain(self.validation_grants.values()) {
            crate::runtime_workflow::require_current_grant(original, grants)?;
            if grants
                .iter()
                .filter(|g| g.phase() == original.phase())
                .count()
                != 1
            {
                return Err(blocked("original consumed grant namespace is ambiguous"));
            }
        }
        self.passed_validation
            .as_ref()
            .ok_or_else(|| blocked("retry lost original passed validation"))?
            .verify_retained(receipts, grants)?;
        self.source.source.revalidate_filesystem()
    }
}

#[cfg(test)]
#[path = "delivery_review_passed_tests.rs"]
mod tests;
