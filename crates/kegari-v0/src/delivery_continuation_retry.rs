//! A published operator retry opens a distinct bounded budget. The old completed
//! run remains completed; only its genuine retained obligation supplies source.
use super::*;
use crate::decisions::VerifiedRetryGrant;

#[derive(Clone)]
pub struct RetriedUnfinishedWork {
    original: Arc<UnfinishedChildWork>,
    grant: VerifiedRetryGrant,
    phase: PhaseKey,
    context: Value,
    digest: String,
}
impl RetriedUnfinishedWork {
    pub fn original(&self) -> &UnfinishedChildWork {
        &self.original
    }
    pub fn grant(&self) -> &VerifiedRetryGrant {
        &self.grant
    }
    pub fn phase(&self) -> &PhaseKey {
        &self.phase
    }
    pub fn authority(&self) -> &FreshGate {
        self.original.authority()
    }
    pub fn source(&self) -> &DeliverySource {
        self.original.source()
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
    pub fn progress_digest(&self) -> &str {
        self.original.progress_digest()
    }
    pub fn remaining_task(&self) -> &Value {
        &self.context
    }
    pub fn verify_current(
        &self,
        receipts: &[Receipt],
        grants: &[VerifiedRetryGrant],
    ) -> Result<()> {
        self.original.verify_retry_basis(receipts)?;
        self.original.verify_current(receipts, grants)?;
        crate::runtime_workflow::require_current_grant(&self.grant, grants)?;
        let successors: Vec<_> = grants
            .iter()
            .filter(|g| g.phase() == self.original.phase())
            .collect();
        if successors.len() != 1
            || successors[0].digest() != self.grant.digest()
            || self.grant.work() != self.authority().work()
            || self.grant.record().workspace_id != self.authority().workspace_id()
            || self.grant.record().runtime_id != self.original.predecessor_receipt().runtime_id
            || self.grant.phase() != self.original.phase()
            || self.phase != self.grant.phase_key()
        {
            return Err(Error::new(
                "stale_result",
                "retry grant does not uniquely replace this unfinished phase",
            ));
        }
        let mut prior = BTreeMap::new();
        for receipt in receipts {
            if let Some(run) = RunReference::from_receipt(receipt)? {
                if run.phase == *self.original.phase() {
                    prior.insert(receipt.operation_id, receipt.payload_digest.clone());
                }
                // This capability supplies the first session of the granted
                // budget. Later retryable failures use their own actual run.
                if run.phase == self.phase {
                    return Err(Error::new(
                        "stale_result",
                        "explicit retry budget already has an admission",
                    ));
                }
            }
        }
        if prior != self.grant.record().prior_admissions {
            return Err(Error::new(
                "stale_result",
                "operator retry no longer covers the complete original budget",
            ));
        }
        self.source().revalidate_filesystem()
    }
    pub fn verify_assignment(&self, assignment: &DeliveryWorkerAssignment) -> Result<()> {
        let expected = assignment::retried_repair_worker_assignment(self)?;
        if assignment.digest()? != expected.digest()? {
            return Err(Error::new(
                "worker_binding",
                "explicit retry assignment differs from its granted obligation",
            ));
        }
        Ok(())
    }
    pub fn verify_contract(&self, contract: &crate::worker::WorkerContract) -> Result<()> {
        self.original
            .verify_contract_with_task(&self.context, contract)
    }
}
impl UnfinishedChildWork {
    pub(crate) fn verify_retry_basis(&self, receipts: &[Receipt]) -> Result<()> {
        self.verify_failure_inventory(receipts)?;
        // Implementation budgets remain separate across explicit grants. A
        // native grant is checked by the selector/admission gate; an older
        // budget's mere presence must not exhaust the new budget. Check/review
        // histories still use the complete conservative failure inventory.
        for receipt in receipts {
            if let Some(run) = RunReference::from_receipt(receipt)?
                && same_phase_group(
                    self.predecessor_receipt().runtime_id,
                    self.reference(),
                    receipt,
                    &run,
                )
                && run.phase == *self.phase()
                && run.attempt > self.reference().attempt
            {
                return Err(Error::new(
                    "stale_result",
                    "unfinished implementation has a later admission in its exact budget",
                ));
            }
        }
        let predecessor = receipts
            .iter()
            .find(|r| r.operation_id == self.predecessor())
            .ok_or_else(|| {
                Error::new(
                    "reconciliation_required",
                    "unfinished predecessor receipt missing",
                )
            })?;
        let failure = receipts
            .iter()
            .find(|r| r.operation_id == self.failure_operation())
            .ok_or_else(|| {
                Error::new(
                    "reconciliation_required",
                    "unfinished failure receipt missing",
                )
            })?;
        if canonical_digest(1, predecessor)? != self.predecessor_receipt_digest()
            || canonical_digest(1, failure)? != self.failure_receipt_digest()
        {
            return Err(Error::new(
                "stale_result",
                "unfinished retry basis differs from actual terminal receipts",
            ));
        }
        self.source().revalidate_filesystem()
    }
    pub(crate) fn retry_blocker(
        &self,
        limits: &crate::config::RuntimeConfig,
        receipts: &[Receipt],
    ) -> Result<bool> {
        self.verify_retry_basis(receipts)?;
        let trigger = crate::control::RunContinuationReference {
            predecessor: self.predecessor(),
            predecessor_receipt_digest: self.predecessor_receipt_digest().into(),
            trigger_digest: self.digest().into(),
            progress_digest: self.progress_digest().into(),
            failure_operation: self.failure_operation(),
            review_rounds_consumed: self.review_rounds_consumed(),
        };
        match crate::runtime_schedule::plan_unfinished(
            limits,
            self.phase(),
            self.authority().workspace_id(),
            receipts,
            &trigger,
            Utc::now(),
        ) {
            Err(error)
                if matches!(
                    error.code.as_str(),
                    "attempts_exhausted" | "review_rounds_exhausted" | "no_progress"
                ) =>
            {
                Ok(true)
            }
            Ok(_) => Ok(false),
            Err(error) => Err(error),
        }
    }
}
impl DeliveryCoordinator {
    pub fn retry_unfinished(
        &self,
        original: Arc<UnfinishedChildWork>,
        grant: VerifiedRetryGrant,
        current: &[VerifiedRetryGrant],
    ) -> Result<RetriedUnfinishedWork> {
        (self.current_gate)(original.authority())?;
        let context = json!({"purpose":"complete the verified remaining obligation under an explicit operator retry budget",
            "remaining_task":original.remaining_task(),"retry":{"grant":grant.record().operation_id,"grant_digest":grant.digest(),
            "reason":grant.record().reason,"budget":grant.new_budget()},"prior_trigger":original.digest()});
        let digest = canonical_digest(
            1,
            &json!({"purpose":"retried_unfinished_child_work","original":original.digest(),"grant":grant.digest(),"context":context}),
        )?;
        let result = RetriedUnfinishedWork {
            phase: grant.phase_key(),
            original,
            grant,
            context,
            digest,
        };
        result.verify_current(&self.host.operations()?, current)?;
        Ok(result)
    }
}
