//! Descriptive discovered-validation evidence. These records do not construct
//! execution or enrollment capabilities; delivery supplies original owners.
use super::*;
use crate::execution::{CheckExecutionRecord, ExecutionEvidenceClass};
use crate::validation::{MAX_VALIDATION_HISTORY, ValidationPlanRecord};

pub const MAX_VALIDATION_EVIDENCE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationEvidence {
    pub schema_version: u32,
    pub plan_digest: String,
    pub original_cwd: std::path::PathBuf,
    pub history: Vec<ValidationPlanRecord>,
    /// All completed attempts retained by the original validation worker,
    /// including superseded-plan and failed attempts. Never a success-only list.
    pub attempts: Vec<CheckExecutionRecord>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationRequirementAssessment {
    pub sufficient: bool,
    pub rationale: String,
    pub evidence_digests: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationAssessment {
    pub evidence_digest: String,
    pub sufficient: bool,
    /// Addresses discovery, earlier failures and changes as well as the final
    /// results. The host binds the assessment; it does not infer its meaning.
    pub rationale: String,
    pub requirements: BTreeMap<String, ValidationRequirementAssessment>,
}

impl ValidationEvidence {
    pub fn digest(&self) -> Result<String> {
        canonical_digest(1, self)
    }

    pub(crate) fn current(&self) -> Result<&ValidationPlanRecord> {
        self.history
            .last()
            .ok_or_else(|| Error::new("validation_gate", "validation history is empty"))
    }

    pub(crate) fn requirements(&self) -> Result<Vec<CheckRequirement>> {
        let kind = self.current()?.kind.clone();
        Ok(self
            .current()?
            .requirements
            .iter()
            .map(|r| CheckRequirement {
                id: r.id.clone(),
                kind: kind.clone(),
                workflow_id: None,
                command: r.argv.clone(),
            })
            .collect())
    }

    /// The same predicate is used before review and when signed child history is
    /// read again. It validates a description, never grants dispatch authority.
    pub(crate) fn validate(&self, work: &WorkIdentity, revision: &RevisionBinding) -> Result<()> {
        self.validate_kind(work, revision, CheckKind::Local)
    }

    pub(crate) fn validate_integrated(
        &self,
        work: &WorkIdentity,
        revision: &RevisionBinding,
        vector: &str,
    ) -> Result<()> {
        self.validate_kind(work, revision, CheckKind::Integrated)?;
        validate_digest(vector)?;
        if self
            .history
            .iter()
            .any(|plan| plan.binding.candidate_vector_digest.as_deref() != Some(vector))
        {
            return Err(Error::new(
                "validation_gate",
                "integrated history differs from the entire candidate vector",
            ));
        }
        Ok(())
    }

    fn validate_kind(
        &self,
        work: &WorkIdentity,
        revision: &RevisionBinding,
        kind: CheckKind,
    ) -> Result<()> {
        if self.schema_version != if kind == CheckKind::Local { 1 } else { 2 }
            || !self.original_cwd.is_absolute()
            || self.history.is_empty()
            || self.history.len() > MAX_VALIDATION_HISTORY
            || self.attempts.is_empty()
            || self.attempts.len() > 2048
            || serde_json::to_vec(self)
                .map_err(|_| Error::invalid("encode validation evidence"))?
                .len()
                > MAX_VALIDATION_EVIDENCE_BYTES
        {
            return Err(Error::new(
                "validation_gate",
                "validation evidence is incomplete or exceeds its bound",
            ));
        }
        let mut plans = BTreeMap::new();
        let mut previous: Option<&ValidationPlanRecord> = None;
        for (index, plan) in self.history.iter().enumerate() {
            plan.validate_shape()?;
            if plan.kind != kind {
                return Err(Error::new(
                    "validation_gate",
                    "validation history changed its admitted kind",
                ));
            }
            let digest = canonical_digest(1, plan)?;
            if plan.ordinal as usize != index + 1
                || plan.predecessor != previous.map(|p| canonical_digest(1, p)).transpose()?
                || previous.is_some_and(|p| {
                    p.original_binding != plan.original_binding
                        || p.workflow_digest != plan.workflow_digest
                        || p.instruction_digest != plan.instruction_digest
                        || p.tool_contract_digest != plan.tool_contract_digest
                        || p.created_at > plan.created_at
                        || p.requirements
                            .iter()
                            .any(|r| !plan.requirements.iter().any(|next| next.id == r.id))
                })
                || plans.insert(digest, plan).is_some()
            {
                return Err(Error::new(
                    "validation_gate",
                    "validation history changed ownership, removed obligations or broke its predecessor chain",
                ));
            }
            previous = Some(plan);
        }
        let current = self.current()?;
        if canonical_digest(1, current)? != self.plan_digest
            || &current.binding.work != work
            || current.binding.repository != revision.repository
            || current.binding.revision.commit != revision.tested_commit
            || current.binding.revision.tree != revision.tested_tree
        {
            return Err(Error::new(
                "validation_gate",
                "current plan is not the exact admitted source/work",
            ));
        }
        let mut invocations = BTreeSet::new();
        for attempt in &self.attempts {
            attempt.validate_shape()?;
            let provenance = attempt.discovered.as_ref().ok_or_else(|| {
                Error::new(
                    "validation_gate",
                    "named execution cannot stand in for a discovered attempt",
                )
            })?;
            let plan = plans.get(&provenance.plan_digest).ok_or_else(|| {
                Error::new(
                    "validation_gate",
                    "attempt is outside the retained plan history",
                )
            })?;
            if attempt.evidence_class != ExecutionEvidenceClass::ConfinedExecution
                || canonical_digest(1, *plan)? != canonical_digest(1, &provenance.plan)?
                || attempt.binding != plan.binding
                || attempt.started_at < plan.created_at
                || attempt.kind != kind
                || !invocations.insert(provenance.invocation_id)
            {
                return Err(Error::new(
                    "validation_gate",
                    "attempt has substituted ownership, chronology or invocation identity",
                ));
            }
        }
        self.selected()?;
        Ok(())
    }

    pub(crate) fn selected_digests(&self) -> Result<Vec<String>> {
        self.selected()?
            .into_iter()
            .map(|record| canonical_digest(1, record))
            .collect()
    }
    pub(crate) fn selected(&self) -> Result<Vec<&CheckExecutionRecord>> {
        let mut result = Vec::new();
        for requirement in &self.current()?.requirements {
            let mut matching: Vec<_> = self
                .attempts
                .iter()
                .filter(|r| {
                    r.check_name == requirement.id
                        && r.discovered
                            .as_ref()
                            .is_some_and(|p| p.plan_digest == self.plan_digest)
                })
                .collect();
            matching.sort_by_key(|r| r.completed_at);
            let last = matching.last().copied().ok_or_else(|| {
                Error::new(
                    "validation_obligations_pending",
                    "current obligation has no current-plan execution",
                )
            })?;
            if matching
                .windows(2)
                .any(|pair| pair[0].completed_at >= pair[1].started_at)
            {
                return Err(Error::new(
                    "validation_gate",
                    "overlapping obligation attempts have no unambiguous latest result",
                ));
            }
            if !last.successful || last.blocker.is_some() || last.before != last.after {
                return Err(Error::new(
                    "validation_failed",
                    "latest current obligation attempt is failed or changed its source",
                ));
            }
            result.push(last);
        }
        Ok(result)
    }

    pub(crate) fn runs(&self, revision: &RevisionBinding) -> Result<Vec<CheckRun>> {
        self.selected()?
            .into_iter()
            .map(|r| {
                Ok(CheckRun {
                    id: r.check_name.clone(),
                    kind: CheckKind::Local,
                    operation_id: r.binding.operation_id,
                    work_digest: r.binding.work.digest()?,
                    revision: revision.clone(),
                    workflow_id: None,
                    run_id: None,
                    attempt: 1,
                    runner: Runner::LocalSandbox,
                    command: r.command.argv.clone(),
                    environment_identity: r.command.isolation_digest.clone(),
                    started_at: Some(r.started_at),
                    completed_at: Some(r.completed_at),
                    conclusion: Conclusion::Success,
                    artifacts: r
                        .artifacts
                        .iter()
                        .map(crate::execution::artifact_reference)
                        .collect(),
                })
            })
            .collect()
    }

    /// Host-computed identifiers for the read-only reviewer. It need not run a
    /// hashing command to reference the exact retained attempt records.
    pub(crate) fn attempt_references(&self) -> Result<serde_json::Value> {
        let selected = self
            .selected()?
            .into_iter()
            .map(|record| canonical_digest(1, record))
            .collect::<Result<BTreeSet<_>>>()?;
        let records = self.attempts.iter().map(|record| {
            let digest = canonical_digest(1, record)?;
            let provenance = record.discovered.as_ref().ok_or_else(|| Error::new("validation_gate", "review attempt lacks discovered provenance"))?;
            Ok(serde_json::json!({"record_digest":digest,"obligation_id":record.check_name,
                "plan_digest":provenance.plan_digest,"invocation_id":provenance.invocation_id,
                "successful":record.successful,"started_at":record.started_at,"completed_at":record.completed_at,
                "selected":selected.contains(&digest)}))
        }).collect::<Result<Vec<_>>>()?;
        Ok(serde_json::Value::Array(records))
    }

    pub(crate) fn verify_configured(
        &self,
        names: &[String],
        workflow: &kegari_context::workflow::Workflow,
    ) -> Result<()> {
        let selected = self.selected()?;
        let commands = match &self.current()?.kind {
            CheckKind::Local => &workflow.front_matter.checks.local,
            CheckKind::Integrated => &workflow.front_matter.checks.integrated,
            _ => {
                return Err(Error::new(
                    "validation_gate",
                    "unsupported configured validation kind",
                ));
            }
        };
        for name in names {
            let configured = commands.get(name).ok_or_else(|| {
                Error::new(
                    "incompletely_prepared",
                    "configured local obligation is absent from workflow",
                )
            })?;
            let requirement = self
                .current()?
                .requirements
                .iter()
                .find(|r| &r.id == name)
                .ok_or_else(|| {
                    Error::new(
                        "validation_obligations_pending",
                        "discovery omitted a configured required local check",
                    )
                })?;
            let actual = selected
                .iter()
                .find(|r| &r.check_name == name)
                .ok_or_else(|| {
                    Error::new(
                        "validation_obligations_pending",
                        "configured required check has not run",
                    )
                })?;
            if requirement.argv != configured.argv
                || requirement.services != configured.services
                || actual.command.cwd != self.original_cwd
            {
                return Err(Error::new(
                    "validation_gate",
                    "discovery changed a configured required command, service or execution cwd",
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn verify_assessment(
        &self,
        assessment: &ValidationAssessment,
        require_sufficient: bool,
    ) -> Result<()> {
        let ids: BTreeSet<_> = self.current()?.requirements.iter().map(|r| &r.id).collect();
        if assessment.evidence_digest != self.digest()?
            || assessment.requirements.keys().collect::<BTreeSet<_>>() != ids
            || !bounded_reason(&assessment.rationale)
            || (require_sufficient && !assessment.sufficient)
        {
            return Err(Error::new(
                "validation_adequacy",
                "review does not assess the exact complete validation history",
            ));
        }
        let all = self
            .attempts
            .iter()
            .map(|r| canonical_digest(1, r))
            .collect::<Result<BTreeSet<_>>>()?;
        for selected in self.selected()? {
            let item = &assessment.requirements[&selected.check_name];
            let references: BTreeSet<_> = item.evidence_digests.iter().collect();
            if !bounded_reason(&item.rationale)
                || item.evidence_digests.is_empty()
                || item.evidence_digests.len() > 2048
                || references.len() != item.evidence_digests.len()
                || item
                    .evidence_digests
                    .iter()
                    .any(|digest| !all.contains(digest))
                || !item
                    .evidence_digests
                    .contains(&canonical_digest(1, selected)?)
                || (require_sufficient && !item.sufficient)
                || (assessment.sufficient && !item.sufficient)
            {
                return Err(Error::new(
                    "validation_adequacy",
                    "review omits current execution evidence or contains an insufficient obligation",
                ));
            }
        }
        Ok(())
    }
}

/// Host-signed historical description of an actual separate read-only review.
/// Neither this DTO nor its assessment constructs a worker/execution capability.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntegratedAdequacyReview {
    pub schema_version: u32,
    pub operation_id: Uuid,
    pub session_id: String,
    pub work: WorkIdentity,
    pub candidate_vector_digest: String,
    pub instruction_manifest: String,
    pub tool_contract_digest: String,
    pub context_digest: String,
    pub completed_at: chrono::DateTime<chrono::Utc>,
    pub assessment: ValidationAssessment,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedIntegratedValidation {
    pub validation: ValidationEvidence,
    pub review: IntegratedAdequacyReview,
}
impl ReviewedIntegratedValidation {
    pub(crate) fn verify(
        &self,
        parent: &WorkIdentity,
        e2e: Uuid,
        vector: &str,
        candidates: &[kegari_git::CandidateManifest],
    ) -> Result<()> {
        let current = self.validation.current()?;
        let mut expected = parent.clone();
        expected.child = Some(e2e);
        let candidate = candidates
            .iter()
            .find(|c| c.repository == current.binding.repository)
            .ok_or_else(|| {
                Error::new(
                    "integrated_gate",
                    "integrated validation repository is outside candidate vector",
                )
            })?;
        let source = candidate
            .result
            .as_ref()
            .ok_or_else(|| Error::new("integrated_gate", "integrated candidate has no result"))?;
        let revision = RevisionBinding {
            repository: candidate.repository.clone(),
            head: source.commit.clone(),
            base: candidate.production.commit.clone(),
            tested_commit: source.commit.clone(),
            tested_tree: source.tree.clone(),
        };
        self.validation
            .validate_integrated(&expected, &revision, vector)?;
        if self.review.schema_version != 1
            || self.review.operation_id.is_nil()
            || self.review.operation_id == current.binding.operation_id
            || self.review.session_id.trim().is_empty()
            || self.review.session_id.len() > 4096
            || self.review.work != expected
            || self.review.candidate_vector_digest != vector
            || self.review.completed_at > chrono::Utc::now()
            || self
                .validation
                .attempts
                .iter()
                .any(|a| a.completed_at > self.review.completed_at)
        {
            return Err(Error::new(
                "validation_adequacy",
                "integrated adequacy is not a separate exact-vector completed review",
            ));
        }
        for value in [
            &self.review.instruction_manifest,
            &self.review.tool_contract_digest,
            &self.review.context_digest,
        ] {
            validate_digest(value)?;
        }
        self.validation
            .verify_assessment(&self.review.assessment, true)
    }
}

fn bounded_reason(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 65_536
}

#[cfg(test)]
#[path = "evidence_validation_tests.rs"]
mod tests;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchivedIntegratedValidation {
    pub archive: ValidationArchiveReference,
    pub selected: Vec<String>,
    pub test_resources: BTreeSet<String>,
    pub review: IntegratedAdequacyReview,
}
impl ArchivedIntegratedValidation {
    pub(crate) fn verify_reference(&self, resolved: &ResolvedValidationArchive) -> Result<()> {
        if resolved.reference() != &self.archive
            || resolved.evidence().selected_digests()? != self.selected
        {
            return Err(Error::new(
                "validation_archive",
                "compact integrated references differ from complete authenticated history",
            ));
        }
        Ok(())
    }
    pub(crate) fn runs(&self, resolved: &ResolvedValidationArchive) -> Result<Vec<IntegratedRun>> {
        self.verify_reference(resolved)?;
        integrated_archive_runs(resolved, &self.test_resources)
    }
}
pub(crate) fn integrated_archive_runs(
    resolved: &ResolvedValidationArchive,
    resources: &BTreeSet<String>,
) -> Result<Vec<IntegratedRun>> {
    resolved
        .evidence()
        .selected()?
        .into_iter()
        .map(|record| {
            Ok(IntegratedRun {
                id: record.check_name.clone(),
                operation_id: record.binding.operation_id,
                e2e_child: record
                    .binding
                    .work
                    .child
                    .ok_or_else(|| Error::new("validation_archive", "integrated child absent"))?,
                candidate_vector_digest: record
                    .binding
                    .candidate_vector_digest
                    .clone()
                    .ok_or_else(|| Error::new("validation_archive", "integrated vector absent"))?,
                command: record.command.argv.clone(),
                test_resources: resources.clone(),
                started_at: record.started_at,
                completed_at: record.completed_at,
                conclusion: Conclusion::Success,
                after_staging_integration: true,
                artifacts: record
                    .artifacts
                    .iter()
                    .map(|a| ArtifactReference {
                        reference: format!(
                            "retained-validation:{}:{}",
                            resolved.reference().manifest_digest,
                            a.digest
                        ),
                        digest: a.digest.clone(),
                    })
                    .collect(),
            })
        })
        .collect()
}
