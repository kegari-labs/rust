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
    /// Earlier source/admission histories, kept as original immutable segments.
    /// Only this record's current attempts can satisfy its current obligations.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prior_history: Vec<ValidationEvidence>,
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
        self.validate_history_kind(work, revision, kind, true)
    }

    pub(crate) fn validate_observed_history(&self) -> Result<()> {
        let current = self.current()?;
        let revision = RevisionBinding {
            repository: current.binding.repository.clone(),
            head: current.binding.revision.commit.clone(),
            base: current.binding.revision.commit.clone(),
            tested_commit: current.binding.revision.commit.clone(),
            tested_tree: current.binding.revision.tree.clone(),
        };
        self.validate_history_kind(
            &current.binding.work,
            &revision,
            current.kind.clone(),
            false,
        )
    }

    pub(crate) fn all_attempts(&self) -> impl Iterator<Item = &CheckExecutionRecord> {
        self.prior_history
            .iter()
            .flat_map(|prior| prior.attempts.iter())
            .chain(&self.attempts)
    }
    pub(crate) fn history_count(&self) -> usize {
        self.history.len()
            + self
                .prior_history
                .iter()
                .map(|prior| prior.history.len())
                .sum::<usize>()
    }

    fn validate_history_kind(
        &self,
        work: &WorkIdentity,
        revision: &RevisionBinding,
        kind: CheckKind,
        require_pass: bool,
    ) -> Result<()> {
        let schema = if kind == CheckKind::Local { 3 } else { 4 }
            + if self.prior_history.is_empty() { 0 } else { 2 };
        if self.schema_version != schema
            || !self.original_cwd.is_absolute()
            || self.history.is_empty()
            || self.history_count() > MAX_VALIDATION_HISTORY
            || require_pass && self.attempts.is_empty()
            || self.all_attempts().count() > 2048
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
        let mut previous_operations = BTreeSet::new();
        let mut previous_time = None;
        for prior in &self.prior_history {
            if !prior.prior_history.is_empty() {
                return Err(Error::new(
                    "validation_gate",
                    "retained validation history must be a flat bounded prefix",
                ));
            }
            prior.validate_observed_history()?;
            let old = prior.current()?;
            if old.kind != kind
                || old.binding.work != *work
                || old.binding.runtime_id != current.binding.runtime_id
                || old.binding.repository != current.binding.repository
                || old.workflow_digest != current.workflow_digest
                // Prior integrated results remain historical across a freshly
                // admitted parent vector. They never enter current selection;
                // all current attempts still bind the exact new vector below.
                || kind != CheckKind::Integrated
                    && old.binding.candidate_vector_digest != current.binding.candidate_vector_digest
                || old.binding.operation_id == current.binding.operation_id
                || old.created_at > current.created_at
                || previous_time.is_some_and(|time| old.created_at < time)
                || !previous_operations.insert(old.binding.operation_id)
                || old.requirements.iter().any(|requirement| {
                    !current
                        .requirements
                        .iter()
                        .any(|new| new.id == requirement.id)
                })
            {
                return Err(Error::new(
                    "validation_gate",
                    "continued validation changed ownership, removed an obligation or relabelled a prior source result",
                ));
            }
            previous_time = Some(old.created_at);
        }
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
            let provenance = attempt.codex.as_ref().ok_or_else(|| {
                Error::new(
                    "legacy_execution_evidence",
                    "validation needs native Codex provenance",
                )
            })?;
            let retained_plan = provenance.plan.as_ref().ok_or_else(|| {
                Error::new(
                    "validation_gate",
                    "native result has no discovered obligation",
                )
            })?;
            let digest = canonical_digest(1, retained_plan)?;
            let plan = plans.get(&digest).ok_or_else(|| {
                Error::new(
                    "validation_gate",
                    "attempt is outside the retained plan history",
                )
            })?;
            if attempt.evidence_class != ExecutionEvidenceClass::CodexExecution
                || canonical_digest(1, *plan)? != digest
                || attempt.binding != plan.binding
                || attempt.started_at < plan.created_at
                || attempt.kind != kind
                || !invocations.insert((
                    attempt.binding.operation_id,
                    &provenance.thread_id,
                    &provenance.turn_id,
                    &provenance.item_id,
                    &attempt.check_name,
                ))
            {
                return Err(Error::new(
                    "validation_gate",
                    "attempt has substituted ownership, chronology or invocation identity",
                ));
            }
        }
        if require_pass {
            self.selected()?;
        }
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
                        && r.codex
                            .as_ref()
                            .and_then(|p| p.plan.as_ref())
                            .is_some_and(|p| {
                                canonical_digest(1, p).ok().as_deref() == Some(&self.plan_digest)
                            })
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
                    kind: r.kind.clone(),
                    operation_id: r.binding.operation_id,
                    work_digest: r.binding.work.digest()?,
                    revision: revision.clone(),
                    workflow_id: None,
                    run_id: None,
                    attempt: 1,
                    runner: Runner::Codex,
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
        let records = self.all_attempts().map(|record| {
            let digest = canonical_digest(1, record)?;
            let provenance = record.codex.as_ref().ok_or_else(|| Error::new("validation_gate", "review attempt lacks native provenance"))?;
            let plan = provenance.plan.as_ref().ok_or_else(|| Error::new("validation_gate", "review attempt lacks discovered enrollment"))?;
            Ok(serde_json::json!({"record_digest":digest,"obligation_id":record.check_name,
                "plan_digest":canonical_digest(1, plan)?,"operation_id":record.binding.operation_id,
                "thread_id":provenance.thread_id,"turn_id":provenance.turn_id,"item_id":provenance.item_id,
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
            if !configured.result_artifact.is_empty() {
                let declared = std::path::Path::new(&configured.result_artifact);
                let expected = if declared.is_absolute() {
                    declared.to_path_buf()
                } else {
                    self.original_cwd.join(declared)
                };
                let enrolled = requirement.artifacts.iter().any(|path| {
                    let path = std::path::Path::new(path);
                    let path = if path.is_absolute() {
                        path.to_path_buf()
                    } else {
                        actual.command.cwd.join(path)
                    };
                    path == expected
                });
                if !enrolled
                    || !actual
                        .artifacts
                        .iter()
                        .any(|artifact| artifact.path == expected)
                    || !actual.codex.as_ref().is_some_and(|codex| {
                        codex.schema_version == 2
                            && codex
                                .artifact_freshness
                                .iter()
                                .any(|fresh| fresh.path == expected)
                    })
                {
                    return Err(Error::new(
                        "validation_gate",
                        "configured result artifact lacks its enrolled fresh native output",
                    ));
                }
            }
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
            .all_attempts()
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
