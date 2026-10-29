//! Original completed validation histories carried across an authorized source
//! advancement. Public descriptions cannot construct this retained owner.
use super::*;
use crate::evidence::ValidationEvidence;
use crate::execution::{CheckExecutionRecord, RetainedArtifact};
use crate::worker_run::RunCompletion;
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub(crate) struct ValidationPrefix {
    history: Vec<ValidationEvidence>,
    artifacts: BTreeMap<String, RetainedArtifact>,
    digest: String,
    archives: Vec<crate::validation_archive::ResolvedValidationArchive>,
}
impl ValidationPrefix {
    pub(crate) fn from_completion(completion: &RunCompletion) -> Result<Option<Self>> {
        Self::from_settled_completion(completion, false)
    }
    pub(crate) fn from_cold_completion(completion: &RunCompletion) -> Result<Option<Self>> {
        Self::from_settled_completion(completion, true)
    }
    fn from_settled_completion(completion: &RunCompletion, cold: bool) -> Result<Option<Self>> {
        let Some(plan) = completion.validation_plan.as_ref() else {
            if completion.validation_error.is_some() || !completion.validation_attempts.is_empty() {
                return Err(Error::new(
                    "validation_history_uncertain",
                    "prior validation has no complete original enrollment",
                ));
            }
            return Ok(None);
        };
        let settled_provider_failure =
            matches!(completion.outcome, crate::worker_run::RunOutcome::Failed)
                && completion.state.terminal.as_deref() == Some("failed")
                && completion.state.turn_id.is_none()
                && completion.state.unresolved_turn.is_none()
                && completion.error.as_ref().is_some_and(|error| {
                    !matches!(
                        error.code.as_str(),
                        "operation_uncertain"
                            | "provider_intervention"
                            | "input_required"
                            | "interrupted"
                    )
                });
        if completion.validation_error.is_some()
            || !completion.local_exit_confirmed
            || completion.error.is_some()
                && !settled_provider_failure
                && !(cold
                    && completion
                        .error
                        .as_ref()
                        .is_some_and(|error| error.code == "turn_incomplete")
                    && completion.state.turn_id.is_none()
                    && completion.state.unresolved_turn.is_none())
            || completion.hook_error.is_some()
            || completion.state.intervention.is_some()
            || completion.state.input_required
        {
            return Err(Error::new(
                "validation_history_uncertain",
                "prior validation is not a joined settled result",
            ));
        }
        let contract = completion.contract.as_ref().ok_or_else(|| {
            Error::new(
                "validation_history_binding",
                "prior validation contract is missing",
            )
        })?;
        let execution = completion.execution.as_ref().ok_or_else(|| {
            Error::new(
                "validation_history_binding",
                "prior validation source owner is missing",
            )
        })?;
        let current = plan.record();
        if current.original_binding != execution.binding
            || current.binding.operation_id != completion.operation_id
            || current.instruction_digest != contract.instructions().manifest_digest
            || current.tool_contract_digest != contract.tools().digest()
            || current.workflow_digest != contract.workflow().digest
            || current.original_binding.isolation_digest != contract.isolation().digest()
            || !matches!(current.kind, CheckKind::Local | CheckKind::Integrated)
        {
            return Err(Error::new(
                "validation_history_binding",
                "prior validation differs from its original admitted owner",
            ));
        }
        // Verifies both complete attempt membership and original protected-byte
        // inventory. Copies below preserve capabilities, not decoded records.
        let reader = completion.validation_artifact_reader()?;
        for check in &completion.checks {
            let record = check.production_evidence()?;
            verify_attempt(
                plan,
                check.validation_plan(),
                &completion.validation_attempts,
                record,
            )?;
        }
        for check in &completion.failed_checks {
            verify_attempt(
                plan,
                check.validation_plan(),
                &completion.validation_attempts,
                check.record(),
            )?;
        }
        for proof in completion
            .validation_attempts
            .iter()
            .flat_map(|attempt| &attempt.artifacts)
        {
            reader.read(proof)?;
        }
        let segment = ValidationEvidence {
            schema_version: if current.kind == CheckKind::Local {
                3
            } else {
                4
            },
            plan_digest: plan.digest().into(),
            original_cwd: contract.isolation().plan().cwd.clone(),
            history: plan.history().to_vec(),
            attempts: completion.validation_attempts.clone(),
            prior_history: Vec::new(),
        };
        segment.validate_observed_history()?;
        let mut history = contract
            .validation_prefix
            .as_ref()
            .map(|prefix| prefix.history.clone())
            .unwrap_or_default();
        let mut artifacts = contract
            .validation_prefix
            .as_ref()
            .map(|prefix| prefix.artifacts.clone())
            .unwrap_or_default();
        if history.iter().any(|prior| {
            prior
                .current()
                .is_ok_and(|prior| prior.binding.operation_id == completion.operation_id)
        }) {
            return Err(Error::new(
                "validation_history_binding",
                "validation continuation repeats an earlier admission",
            ));
        }
        history.push(segment);
        if history
            .iter()
            .map(|segment| segment.history.len())
            .sum::<usize>()
            >= MAX_VALIDATION_HISTORY
            || history
                .iter()
                .map(|segment| segment.attempts.len())
                .sum::<usize>()
                >= 2048
            || serde_json::to_vec(&history)
                .map_err(|_| Error::invalid("encode validation prefix"))?
                .len()
                > crate::evidence::validation::MAX_VALIDATION_EVIDENCE_BYTES
        {
            return Err(Error::new(
                "validation_history_limit",
                "retained validation prefix leaves no bounded room for a new admission",
            ));
        }
        for artifact in &completion.validation_artifacts {
            artifacts
                .entry(artifact.proof_digest()?)
                .or_insert_with(|| artifact.clone());
        }
        let digest = canonical_digest(1, &history)?;
        Ok(Some(Self {
            history,
            artifacts,
            digest,
            archives: contract
                .validation_prefix
                .as_ref()
                .map(|prefix| prefix.archives.clone())
                .unwrap_or_default(),
        }))
    }
    pub(crate) fn from_archive(
        archive: crate::validation_archive::ResolvedValidationArchive,
    ) -> Result<Self> {
        archive.evidence().validate_observed_history()?;
        let mut segment = archive.evidence().clone();
        let mut history = std::mem::take(&mut segment.prior_history);
        segment.schema_version = if segment.current()?.kind == CheckKind::Local {
            3
        } else {
            4
        };
        history.push(segment);
        let digest = canonical_digest(1, &history)?;
        Ok(Self {
            history,
            artifacts: BTreeMap::new(),
            digest,
            archives: vec![archive],
        })
    }
    pub(crate) fn archive_evidence(&self) -> Result<ValidationEvidence> {
        let (last, prior) = self
            .history
            .split_last()
            .ok_or_else(|| Error::invalid("empty validation prefix"))?;
        let mut evidence = last.clone();
        evidence.prior_history = prior.to_vec();
        evidence.schema_version = if evidence.current()?.kind == CheckKind::Local {
            3
        } else {
            4
        } + if prior.is_empty() { 0 } else { 2 };
        evidence.validate_observed_history()?;
        Ok(evidence)
    }
    pub(crate) fn extends(&self, prior: &Self) -> bool {
        self.history.len() == prior.history.len() + 1
            && self.history[..prior.history.len()] == prior.history
    }
    pub(crate) fn history(&self) -> &[ValidationEvidence] {
        &self.history
    }
    pub(crate) fn digest(&self) -> &str {
        &self.digest
    }
    pub(crate) fn descriptor(&self) -> Result<serde_json::Value> {
        let current = self
            .history
            .last()
            .ok_or_else(|| Error::invalid("empty validation prefix"))?
            .current()?;
        Ok(
            serde_json::json!({"schema_version":1,"history_digest":self.digest,
            "prior_operation":current.binding.operation_id,"prior_source":current.binding.revision,
            "prior_candidate_vector":current.binding.candidate_vector_digest,
            "prior_plan_digest":self.history.last().unwrap().plan_digest,
            "requirements":current.requirements,
            "history_count":self.history.len(),
            "attempts":self.history.iter().flat_map(|segment|segment.attempts.iter()).map(|record|Ok(serde_json::json!({"record_digest":canonical_digest(1,record)?,"obligation":record.check_name,"successful":record.successful,"source":record.binding.revision}))).collect::<Result<Vec<_>>>()?,
            "requirement":"Preserve every retained obligation ID. Discover a fresh plan with actual reads for this new source and record only its actual Codex results. Prior failures and results remain historical and will accompany final independent review; they cannot satisfy new-source checks. Explain command changes without dropping or weakening required coverage."}),
        )
    }
    pub(crate) fn require_obligations(&self, proposal: &ValidationProposal) -> Result<()> {
        let ids = proposal
            .requirements
            .iter()
            .map(|requirement| requirement.id.as_str())
            .collect::<BTreeSet<_>>();
        for segment in &self.history {
            if segment
                .current()?
                .requirements
                .iter()
                .any(|requirement| !ids.contains(requirement.id.as_str()))
            {
                return Err(Error::new(
                    "validation_obligations_pending",
                    "continuation omitted an original required obligation",
                ));
            }
        }
        Ok(())
    }
    pub(crate) fn read_artifact(
        &self,
        proof: &kegari_types::artifact::ArtifactProof,
    ) -> Result<Vec<u8>> {
        let digest = canonical_digest(1, proof)?;
        if let Some(artifact) = self.artifacts.get(&digest) {
            return artifact.read_verified(proof);
        }
        for archive in &self.archives {
            if archive
                .artifact_index()
                .iter()
                .any(|(key, original)| *key == digest && *original == proof)
            {
                return archive.read_artifact(proof);
            }
        }
        Err(Error::new(
            "artifact_conflict",
            "artifact is outside the original validation prefix",
        ))
    }
}

fn verify_attempt(
    plan: &ValidationPlan,
    original: Option<&ValidationPlan>,
    attempts: &[CheckExecutionRecord],
    record: &CheckExecutionRecord,
) -> Result<()> {
    let original = original.ok_or_else(|| {
        Error::new(
            "validation_history_binding",
            "prior attempt lost original plan ownership",
        )
    })?;
    let stored = record
        .codex
        .as_ref()
        .and_then(|provenance| provenance.plan.as_ref())
        .ok_or_else(|| {
            Error::new(
                "legacy_execution_evidence",
                "continuation requires native execution provenance",
            )
        })?;
    let digest = canonical_digest(1, record)?;
    if canonical_digest(1, stored)? != original.digest()
        || !plan
            .history()
            .iter()
            .any(|entry| canonical_digest(1, entry).ok().as_deref() == Some(original.digest()))
        || attempts
            .iter()
            .filter(|attempt| {
                canonical_digest(1, *attempt).ok().as_deref() == Some(digest.as_str())
            })
            .count()
            != 1
    {
        return Err(Error::new(
            "validation_history_binding",
            "prior attempt changed its complete original membership",
        ));
    }
    Ok(())
}

impl crate::worker::WorkerContract {
    pub(crate) fn attach_validation_prefix(&mut self, prefix: ValidationPrefix) -> Result<()> {
        let current = prefix
            .history
            .last()
            .ok_or_else(|| Error::invalid("empty validation prefix"))?
            .current()?;
        if self.instructions().manifest.role != kegari_context::instructions::AgentRole::Validation
            || self.validation_prefix.is_some()
            || self
                .context()
                .pointer("/evidence/delivery_assignment/validation_assignment/continuation")
                != Some(&prefix.descriptor()?)
            || self.authority().work() != Some(&current.binding.work)
            || self.workflow().digest != current.workflow_digest
        {
            return Err(Error::new(
                "validation_history_binding",
                "continued validation owner differs from the admitted assignment",
            ));
        }
        self.validation_prefix = Some(prefix);
        Ok(())
    }
}

impl crate::worker::WorkerContract {
    pub(crate) fn attach_recovered_validation(
        &mut self,
        checkpoint: &crate::worker_run::ColdValidationCheckpoint,
    ) -> Result<()> {
        let prefix = checkpoint.prefix()?;
        let current = prefix
            .history()
            .last()
            .ok_or_else(|| Error::invalid("empty recovered history"))?
            .current()?;
        if self.validation_prefix.is_some()
            || !matches!(
                self.instructions().manifest.role,
                kegari_context::instructions::AgentRole::Implementation
                    | kegari_context::instructions::AgentRole::Documentation
                    | kegari_context::instructions::AgentRole::Testing
            )
            || self.authority().work() != Some(&current.binding.work)
            || self.workflow().digest != current.workflow_digest
            || self.instructions().manifest.operation_id == current.binding.operation_id
        {
            return Err(Error::new(
                "validation_history_binding",
                "recovered historical obligations differ from the new admitted child",
            ));
        }
        // This is historical input, not an enrolled plan or accepted result. The
        // existing validation evidence tool exposes it; discovery must preserve
        // its obligations and create a fresh source-bound plan.
        self.validation_prefix = Some(prefix);
        Ok(())
    }
}
