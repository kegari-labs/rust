//! Original integrated execution and separate read-only adequacy. The prepared
//! type deliberately cannot enter readiness/publication as IntegratedEvidence.
use super::*;
use crate::instructions::AgentRole;
use crate::runtime::VerifiedAuthorizedOutcome;
use crate::worker_run::RunCompletion;

#[derive(Clone, Debug)]
pub struct PreparedIntegratedValidation {
    signed: Signed<IntegratedRecord>,
    archive: crate::validation_archive::ResolvedValidationArchive,
}
#[derive(Clone, Debug)]
pub struct IntegratedReviewAssignment {
    prepared: PreparedIntegratedValidation,
    context: Value,
}
impl IntegratedReviewAssignment {
    pub fn context(&self) -> &Value {
        &self.context
    }
    pub(crate) fn signed_input(&self) -> &Signed<IntegratedRecord> {
        &self.prepared.signed
    }
    pub(crate) fn verify_assignment(
        &self,
        contract: &ParentContract,
        vector: &ParentCandidateVector,
    ) -> Result<()> {
        let input = &self.prepared.signed.payload;
        if input.schema_version != 3
            || input.archive.as_ref() != Some(self.prepared.archive.reference())
            || input.validation.is_some()
            || input.review.is_some()
            || input.contract_digest != contract.digest()?
            || input.vector_digest != vector.digest()
        {
            return Err(Error::new(
                "validation_adequacy",
                "integrated review input is not the original prepared vector",
            ));
        }
        Ok(())
    }
}
impl DeliveryCoordinator {
    pub async fn prepare_integrated_validation(
        &self,
        contract: &ParentContract,
        e2e: &FreshGate,
        vector: &ParentCandidateVector,
        observer: &ProductionObserver,
        workflow: &Workflow,
        completed: (&VerifiedAuthorizedOutcome, &RunCompletion),
    ) -> Result<PreparedIntegratedValidation> {
        let (outcome, completion) = completed;
        super::super::child::completed_run(outcome, completion)?;
        let worker = outcome.outcome();
        let plan = completion.validation_plan.as_ref().ok_or_else(|| {
            Error::new(
                "validation_obligations_pending",
                "integrated worker has no current discovered obligations",
            )
        })?;
        if worker.validation_history_digest.is_none() {
            return Err(Error::new(
                "validation_history_binding",
                "integrated discovery requires protected original history",
            ));
        }
        if let Some(error) = &completion.validation_error {
            return Err(error.clone());
        }
        let actual = completion.contract.as_ref().ok_or_else(|| {
            Error::new(
                "worker_binding",
                "integrated completion has no original contract",
            )
        })?;
        let execution = completion.execution.as_ref().ok_or_else(|| {
            Error::new(
                "worker_binding",
                "integrated completion has no original execution owner",
            )
        })?;
        let current = plan.record();
        if current.kind != CheckKind::Integrated
            || current.original_binding != execution.binding
            || current.binding.operation_id != completion.operation_id
            || current.binding.work != *e2e.work()
            || current.binding.candidate_vector_digest.as_deref() != Some(vector.digest())
            || current.workflow_digest != workflow.digest
            || workflow.digest != contract.gate.workflow_digest()
            || current.instruction_digest != actual.instructions().manifest_digest
            || current.tool_contract_digest != actual.tools().digest()
            || current.original_binding.isolation_digest != actual.isolation().digest()
            || worker.instructions.role != AgentRole::Validation
            || worker.instruction_digest != actual.instructions().manifest_digest
            || worker.tool_contract_digest != actual.tools().digest()
            || outcome.original_authority().work() != e2e.work()
            || outcome.original_authority().config_revision() != e2e.config_revision()
        {
            return Err(Error::new(
                "validation_plan_binding",
                "integrated plan differs from actual original worker/vector ownership",
            ));
        }
        let candidate = vector
            .candidates()
            .iter()
            .find(|c| c.repository == current.binding.repository)
            .ok_or_else(|| {
                Error::new(
                    "integrated_gate",
                    "integrated primary source is outside vector",
                )
            })?;
        let mut attempts = completion.validation_attempts.clone();
        for record in &attempts {
            record.validate_shape()?;
            if record.discovered.is_none() {
                return Err(Error::new(
                    "validation_lineage_pending",
                    "mixed named/discovered integrated history cannot discard attempts",
                ));
            }
        }
        for check in &completion.checks {
            let record = check.production_evidence()?;
            super::super::discovered_validation::retain_attempt(
                plan,
                check.validation_plan(),
                record,
            )?;
            super::super::discovered_validation::require_recorded_attempt(&attempts, record)?;
        }
        for check in &completion.failed_checks {
            super::super::discovered_validation::retain_attempt(
                plan,
                check.validation_plan(),
                check.record(),
            )?;
            super::super::discovered_validation::require_recorded_attempt(
                &attempts,
                check.record(),
            )?;
        }
        attempts.sort_by_key(|r| {
            (
                r.started_at,
                r.completed_at,
                r.discovered.as_ref().map(|p| p.invocation_id),
            )
        });
        let validation = evidence::ValidationEvidence {
            schema_version: 2,
            plan_digest: plan.digest().into(),
            original_cwd: actual.isolation().plan().cwd.clone(),
            history: plan.history().to_vec(),
            attempts,
        };
        validation.validate_integrated(
            e2e.work(),
            &candidate_binding(candidate)?,
            vector.digest(),
        )?;
        validation.verify_configured(
            &contract
                .integrated_commands
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            workflow,
        )?;
        let mut selected = Vec::new();
        for record in validation.selected()? {
            let digest = canonical_digest(1, record)?;
            let cap = completion.checks.iter().find(|check| check.digest().ok().as_deref() == Some(&digest)
                && check.validation_plan().is_some_and(|actual| actual.digest() == plan.digest()))
                .ok_or_else(|| Error::new("validation_plan_binding", "selected integrated success has no original current-plan execution capability"))?;
            selected.push(cap);
        }
        let artifacts = completion.validation_artifact_reader()?;
        let archive = crate::validation_archive::retain(
            &self.records,
            &self.key,
            crate::validation_archive::ValidationArchiveNamespace {
                workspace_id: e2e.workspace_id(),
                runtime_id: current.binding.runtime_id,
            },
            &validation,
            |artifact| artifacts.read(artifact),
        )?;
        let signed = self
            .retain_integrated_execution(contract, e2e, vector, observer, &selected, Some(&archive))
            .await?;
        Ok(PreparedIntegratedValidation { signed, archive })
    }

    pub async fn prepare_integrated_review(
        &self,
        contract: &ParentContract,
        e2e: &FreshGate,
        vector: &ParentCandidateVector,
        observer: &ProductionObserver,
        prepared: PreparedIntegratedValidation,
        task: Value,
    ) -> Result<IntegratedReviewAssignment> {
        self.key
            .verify("parent_integrated_execution", &prepared.signed)?;
        self.reobserve_parent_vector(contract, observer, vector)
            .await?;
        (self.current_gate)(e2e)?;
        if prepared.signed.payload.e2e_work != *e2e.work() {
            return Err(Error::new(
                "stale_result",
                "integrated review E2E authority changed",
            ));
        }
        let resolved = crate::validation_archive::read(
            &self.records,
            &self.key,
            &crate::validation_archive::ValidationArchiveNamespace {
                workspace_id: e2e.workspace_id(),
                runtime_id: prepared.signed.payload.runtime_id,
            },
            prepared.archive.reference(),
        )?;
        let validation = resolved.evidence();
        let mut context = json!({"task":task,"integrated_validation_review":{
            "schema_version":1,"work":e2e.work(),"parent":contract.work(),"vector_digest":vector.digest(),
            "candidates":vector.candidates(),"baselines":vector.baselines(),"test_resources":contract.test_resources(),
            "validation":validation,"validation_evidence_digest":validation.digest()?,"validation_attempt_references":validation.attempt_references()?,
            "requirement":"Independently assess integrated sufficiency against the complete parent task and every candidate/resource. Address all obligations, previous failures and plan changes. Exit zero alone is not semantic adequacy.",
            "required_result_schema":{"schema_version":2,"verdict":"pass | findings | blocked","unresolved_findings":[],"supporting_links":[],
                "validation":{"evidence_digest":validation.digest()?,"sufficient":false,"rationale":"complete-vector sufficiency and history reasoning",
                "requirements":validation.current()?.requirements.iter().map(|r| (r.id.clone(), json!({"sufficient":false,"rationale":"assessment with actual evidence","evidence_digests":[]}))).collect::<BTreeMap<_,_>>()}}}});
        let artifacts = super::super::discovered_validation::review_artifacts(&resolved, &context)?;
        context["integrated_validation_review"]["validation_artifacts"] = artifacts;
        if serde_json::to_vec(&context)
            .map_err(|_| Error::invalid("encode integrated review context"))?
            .len()
            > 8 * 1024 * 1024
        {
            return Err(Error::new(
                "bounded_input",
                "complete integrated review context exceeds its finite bound",
            ));
        }
        let assignment = IntegratedReviewAssignment { prepared, context };
        assignment.verify_assignment(contract, vector)?;
        Ok(assignment)
    }

    pub async fn complete_integrated_review(
        &self,
        contract: &ParentContract,
        e2e: &FreshGate,
        vector: &ParentCandidateVector,
        observer: &ProductionObserver,
        assignment: &IntegratedReviewAssignment,
        completed: (&VerifiedAuthorizedOutcome, &RunCompletion),
    ) -> Result<IntegratedEvidence> {
        let (outcome, completion) = completed;
        super::super::child::completed_run(outcome, completion)?;
        assignment.verify_assignment(contract, vector)?;
        self.key
            .verify("parent_integrated_execution", assignment.signed_input())?;
        (self.current_gate)(e2e)?;
        self.reobserve_parent_vector(contract, observer, vector)
            .await?;
        let worker = outcome.outcome();
        let actual = completion.contract.as_ref().ok_or_else(|| {
            Error::new(
                "worker_binding",
                "integrated review lacks original assembled contract",
            )
        })?;
        if outcome.original_authority().work() != e2e.work()
            || outcome.original_authority().config_revision() != e2e.config_revision()
            || canonical_digest(1, &worker.instructions.profile)?
                != canonical_digest(1, e2e.profile())?
            || worker.instructions.repository_revision.as_deref()
                != Some(
                    assignment
                        .prepared
                        .archive
                        .evidence()
                        .current()?
                        .binding
                        .revision
                        .commit
                        .as_str(),
                )
            || worker.instructions.role != AgentRole::Review
            || worker.instructions.workflow_digest != contract.gate.workflow_digest()
            || worker.instructions.permissions.repository_code_execution
            || !worker
                .instructions
                .permissions
                .source_write_paths
                .is_empty()
            || !worker
                .instructions
                .permissions
                .resource_write_paths
                .is_empty()
            || worker.instruction_digest != actual.instructions().manifest_digest
            || worker.tool_contract_digest != actual.tools().digest()
        {
            return Err(Error::new(
                "validation_adequacy",
                "integrated adequacy is not an actual separate read-only Review worker",
            ));
        }
        super::super::assignment::verify_assignment_contract(
            actual,
            e2e,
            "parent_integrated_review",
            assignment.context(),
        )?;
        let claim: super::super::ReviewResultClaim = serde_json::from_value(worker.claim.clone())
            .map_err(|_| {
            Error::new(
                "worker_claim",
                "integrated review claim has no supported schema",
            )
        })?;
        if claim.schema_version != 2
            || claim.verdict != evidence::Verdict::Pass
            || !claim.unresolved_findings.is_empty()
            || claim.supporting_links.len() > 256
            || claim
                .supporting_links
                .iter()
                .any(|s| s.trim().is_empty() || s.len() > 65536)
        {
            return Err(Error::new(
                "validation_adequacy",
                "integrated review is insufficient, blocked or has unresolved findings",
            ));
        }
        let assessment = claim.validation.ok_or_else(|| {
            Error::new(
                "validation_adequacy",
                "integrated review omitted adequacy assessment",
            )
        })?;
        let mut record = assignment.prepared.signed.payload.clone();
        let validation = assignment.prepared.archive.evidence();
        validation.verify_assessment(&assessment, true)?;
        record.review = Some(evidence::IntegratedAdequacyReview {
            schema_version: 1,
            operation_id: worker.operation_id,
            session_id: worker.session.thread_id.clone().ok_or_else(|| {
                Error::new(
                    "validation_adequacy",
                    "integrated review lacks original thread",
                )
            })?,
            work: e2e.work().clone(),
            candidate_vector_digest: vector.digest().into(),
            instruction_manifest: worker.instruction_digest.clone(),
            tool_contract_digest: worker.tool_contract_digest.clone(),
            context_digest: worker.instructions.context_digest.clone(),
            completed_at: Utc::now(),
            assessment,
        });
        let signed = self.key.sign("parent_integrated_execution", &record)?;
        integrated_record_bound(&signed)?;
        let evidence = self.resolve_integrated_record(signed, e2e.workspace_id())?;
        self.reverify_integrated_record(contract, vector, e2e.work(), &evidence)?;
        self.records.write(
            &format!("parent-integrated-{}.json", &evidence.digest()?[7..]),
            &evidence.signed,
        )?;
        self.reobserve_parent_vector(contract, observer, vector)
            .await?;
        (self.current_gate)(e2e)?;
        Ok(evidence)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_signed_integrated_history_is_readable_but_not_new_publication_authority() {
        let digest = crate::digest::bytes_digest(b"historical descriptive identity");
        let record = IntegratedRecord {
            schema_version: 1,
            runtime_id: Uuid::from_u128(1),
            contract_digest: digest.clone(),
            vector_digest: digest.clone(),
            e2e_work: WorkIdentity {
                linear_endpoint: "https://linear.invalid/graphql".into(),
                linear_workspace: Uuid::from_u128(2),
                parent: Uuid::from_u128(3),
                child: Some(Uuid::from_u128(4)),
                generation: crate::evidence::Generation {
                    authorization: digest.clone(),
                    definition: digest.clone(),
                    correction: digest.clone(),
                    scope: digest,
                },
            },
            records: vec![],
            outputs: vec![],
            runs: vec![],
            validation: None,
            review: None,
            archive: None,
            selected: vec![],
            test_resources: BTreeSet::new(),
        };
        let key =
            crate::evidence::AttestationKey::new("historical-test".into(), vec![9; 32]).unwrap();
        let signed = key.sign("parent_integrated_execution", &record).unwrap();
        let bytes = serde_json::to_vec(&signed).unwrap();
        let restored: Signed<IntegratedRecord> = serde_json::from_slice(&bytes).unwrap();
        key.verify("parent_integrated_execution", &restored)
            .unwrap();
        let fields = serde_json::to_value(&restored.payload).unwrap();
        assert!(fields.get("validation").is_none());
        assert!(fields.get("review").is_none());
        assert_eq!(serde_json::to_vec(&restored).unwrap(), bytes);
        let mut invoked = false;
        let result = (|| {
            require_new_integrated_publication(&restored.payload)?;
            invoked = true;
            Ok::<(), Error>(())
        })();
        assert_eq!(result.unwrap_err().code, "validation_adequacy");
        assert!(!invoked);
        let mut renamed = restored.payload;
        renamed.schema_version = 2;
        assert_eq!(
            require_new_integrated_publication(&renamed)
                .unwrap_err()
                .code,
            "validation_adequacy"
        );
    }
}
