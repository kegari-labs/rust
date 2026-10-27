//! Child-only bridge from original enrolled plans and execution capabilities to
//! mechanical completion. Adequacy remains a separate independent review gate.
use super::*;
use crate::execution::{CheckExecutionRecord, ExecutionEvidenceClass};
use crate::validation::ValidationPlan;
use crate::worker_run::RunCompletion;

impl DeliveryCoordinator {
    pub(super) fn verify_discovered_local(
        &self,
        admission: &DeliveryAdmission,
        workflow: &Workflow,
        completion: &RunCompletion,
        plan: &ValidationPlan,
    ) -> Result<LocalPassed> {
        self.current(admission, &[DeliveryPhase::LocalValidation])?;
        let seal = completion.validation_history.as_ref().ok_or_else(|| {
            Error::new(
                "validation_history_binding",
                "discovered local verification requires original worker history ownership",
            )
        })?;
        seal.verify(
            completion.operation_id,
            Some(plan),
            &completion.validation_attempts,
            completion.validation_error.as_ref(),
        )?;
        if completion
            .validation_plan
            .as_ref()
            .map(ValidationPlan::digest)
            != Some(plan.digest())
        {
            return Err(Error::new(
                "validation_history_binding",
                "local plan differs from the completion's current enrollment",
            ));
        }
        if let Some(error) = &completion.validation_error {
            return Err(error.clone());
        }
        let current = plan.record();
        let contract = completion.contract.as_ref().ok_or_else(|| {
            Error::new(
                "worker_binding",
                "validation lacks original worker contract",
            )
        })?;
        let execution = completion.execution.as_ref().ok_or_else(|| {
            Error::new(
                "worker_binding",
                "validation lacks original execution owner",
            )
        })?;
        if current.original_binding != execution.binding
            || current.binding.operation_id != completion.operation_id
            || current.workflow_digest != workflow.digest
            || workflow.digest != admission.gate.workflow_digest()
            || current.instruction_digest != contract.instructions().manifest_digest
            || current.tool_contract_digest != contract.tools().digest()
            || current.original_binding.isolation_digest != contract.isolation().digest()
            || current.binding.work != admission.scope.work
        {
            return Err(Error::new(
                "validation_plan_binding",
                "current enrolled validation plan differs from its actual completed worker",
            ));
        }
        // This is the original worker's entire emitted completed-attempt list,
        // including timeouts/missing artifacts which mint no repair capability.
        let mut attempts = completion.validation_attempts.clone();
        for record in &attempts {
            record.validate_shape()?;
            if record.discovered.is_none() {
                return Err(Error::new(
                    "validation_lineage_pending",
                    "mixed named/discovered attempt history requires explicit lineage migration",
                ));
            }
        }
        for check in &completion.checks {
            let record = check.production_evidence()?;
            retain_attempt(plan, check.validation_plan(), record)?;
            require_recorded_attempt(&attempts, record)?;
        }
        for check in &completion.failed_checks {
            let record = check.record();
            record.validate_shape()?;
            retain_attempt(plan, check.validation_plan(), record)?;
            require_recorded_attempt(&attempts, record)?;
        }
        // A deterministic ordering also makes the exact review/publication
        // binding stable. Duplicate invocation IDs are independently refused.
        attempts.sort_by_key(|r| {
            (
                r.started_at,
                r.completed_at,
                r.discovered.as_ref().map(|p| p.invocation_id),
            )
        });
        let validation = evidence::ValidationEvidence {
            schema_version: 1,
            plan_digest: plan.digest().into(),
            original_cwd: contract.isolation().plan().cwd.clone(),
            history: plan.history().to_vec(),
            attempts,
        };
        validation.validate(&admission.scope.work, &admission.scope.revision)?;
        validation.verify_configured(
            &self.contract(admission)?.configuration.local_checks,
            workflow,
        )?;
        for selected in validation.selected()? {
            if !completion.checks.iter().any(|check| {
                check.digest().ok() == canonical_digest(1, selected).ok()
                    && check
                        .validation_plan()
                        .is_some_and(|original| original.digest() == plan.digest())
            }) {
                return Err(Error::new(
                    "validation_plan_binding",
                    "selected success has no original execution capability for the current plan",
                ));
            }
        }
        let checks = validation.runs(&admission.scope.revision)?;
        evidence::verify_checks(
            &validation.requirements()?,
            &admission.scope.work,
            &admission.scope.revision,
            &CheckSnapshot {
                pagination_complete: true,
                observed_at: Utc::now(),
                runs: checks.clone(),
            },
        )?;
        Ok(LocalPassed {
            work: admission.scope.work.clone(),
            revision: admission.scope.revision.clone(),
            workflow_digest: workflow.digest.clone(),
            checks,
            validation: Some(Box::new(validation)),
        })
    }
}

pub(super) fn require_legacy_continuation(completion: &RunCompletion) -> Result<()> {
    if completion.validation_plan.is_some()
        || completion.validation_error.is_some()
        || completion
            .validation_attempts
            .iter()
            .any(|r| r.discovered.is_some())
        || completion
            .checks
            .iter()
            .any(|r| r.validation_plan().is_some())
        || completion
            .failed_checks
            .iter()
            .any(|r| r.validation_plan().is_some())
    {
        return Err(Error::new(
            "validation_lineage_pending",
            "discovered validation continuation requires retained original obligation and attempt history; legacy retry cannot restart it",
        ));
    }
    Ok(())
}

/// Known serialization/storage limits must fail before a native publication.
/// This bounded slice carries the complete evidence inline; it has no implicit
/// large-object retrieval protocol and never truncates the review inventory.
pub(super) fn publication_bounds<T: Serialize>(purpose: &str, signed: &Signed<T>) -> Result<()> {
    let bytes =
        serde_json::to_vec(signed).map_err(|_| Error::invalid("encode discovered publication"))?;
    if bytes.len() > 256 * 1024 || record_body(purpose, signed)?.len() > 60 * 1024 {
        return Err(Error::new(
            "bounded_input",
            "complete discovered validation publication exceeds the retained inline publication limit",
        ));
    }
    Ok(())
}

fn require_recorded_attempt(
    attempts: &[CheckExecutionRecord],
    record: &CheckExecutionRecord,
) -> Result<()> {
    let digest = canonical_digest(1, record)?;
    if attempts
        .iter()
        .filter(|r| canonical_digest(1, *r).ok().as_ref() == Some(&digest))
        .count()
        != 1
    {
        return Err(Error::new(
            "validation_plan_binding",
            "execution capability differs from the original complete attempt inventory",
        ));
    }
    Ok(())
}

fn retain_attempt(
    current: &ValidationPlan,
    original: Option<&ValidationPlan>,
    record: &CheckExecutionRecord,
) -> Result<()> {
    let original = original.ok_or_else(|| {
        Error::new(
            "validation_plan_binding",
            "attempt has no original opaque enrollment",
        )
    })?;
    let provenance = record.discovered.as_ref().ok_or_else(|| {
        Error::new(
            "validation_plan_binding",
            "named checks cannot replace discovered attempts",
        )
    })?;
    let retained = current
        .history()
        .iter()
        .find(|p| canonical_digest(1, *p).ok().as_deref() == Some(original.digest()));
    if record.evidence_class != ExecutionEvidenceClass::ConfinedExecution
        || provenance.plan_digest != original.digest()
        || canonical_digest(1, &provenance.plan)? != canonical_digest(1, original.record())?
        || retained.is_none()
        || original.history().len() > current.history().len()
        || canonical_digest(1, original.history())?
            != canonical_digest(1, &current.history()[..original.history().len()])?
    {
        return Err(Error::new(
            "validation_plan_binding",
            "attempt was not emitted by the retained original plan history",
        ));
    }
    Ok(())
}

impl LocalPassed {
    pub(super) fn verify_review_adequacy(&self, review: &PublishedReview) -> Result<()> {
        match (&self.validation, &review.verified.attestation().review) {
            (
                Some(expected),
                evidence::ReviewKind::AgentValidation {
                    validation,
                    assessment,
                    ..
                },
            ) => {
                if expected.digest()? != validation.digest()? {
                    return Err(Error::new(
                        "validation_adequacy",
                        "review assessed another validation history",
                    ));
                }
                validation.validate(&self.work, &self.revision)?;
                validation.verify_assessment(assessment, true)
            }
            (None, evidence::ReviewKind::AgentValidation { .. }) | (Some(_), _) => Err(Error::new(
                "validation_adequacy",
                "legacy review cannot satisfy discovered validation",
            )),
            (None, _) => Ok(()),
        }
    }
    pub(super) fn child_schema(&self) -> u32 {
        if self.validation.is_some() { 2 } else { 1 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_inline_publication_is_bounded_before_any_provider_action() {
        let key = AttestationKey::new("publication-bound-test".into(), vec![3; 32]).unwrap();
        let small = key
            .sign(
                "codex_review",
                &json!({"retained":"complete small evidence"}),
            )
            .unwrap();
        publication_bounds("codex_review", &small).unwrap();
        let large = key
            .sign("codex_review", &json!({"retained":"x".repeat(60 * 1024)}))
            .unwrap();
        assert_eq!(
            publication_bounds("codex_review", &large).unwrap_err().code,
            "bounded_input"
        );
        assert!(serde_json::to_vec(&large).unwrap().len() < 256 * 1024);
        assert!(record_body("codex_review", &large).unwrap().len() > 60 * 1024);
    }
}
