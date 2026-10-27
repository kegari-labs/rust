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
        let artifacts = completion.validation_artifact_reader()?;
        let archive = crate::validation_archive::retain(
            &self.records,
            &self.key,
            crate::validation_archive::ValidationArchiveNamespace {
                workspace_id: admission.gate.workspace_id(),
                runtime_id: admission.scope.runtime_id,
            },
            &validation,
            |artifact| artifacts.read(artifact),
        )?;
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
            validation_archive: Some(archive),
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

pub(super) fn require_recorded_attempt(
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

pub(super) fn retain_attempt(
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
        match (
            &self.validation,
            review.verified.validation(),
            review.verified.assessment(),
        ) {
            (Some(expected), Some(validation), Some(assessment)) => {
                if expected.digest()? != validation.digest()? {
                    return Err(Error::new(
                        "validation_adequacy",
                        "review assessed another validation history",
                    ));
                }
                validation.validate(&self.work, &self.revision)?;
                validation.verify_assessment(assessment, true)
            }
            (None, None, None) => Ok(()),
            _ => Err(Error::new(
                "validation_adequacy",
                "review and original discovered validation differ",
            )),
        }
    }
    pub(super) fn child_schema(&self) -> u32 {
        if self.validation_archive.is_some() {
            3
        } else if self.validation.is_some() {
            2
        } else {
            1
        }
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

impl DeliveryCoordinator {
    pub(super) fn verify_review_archive(
        &self,
        signed: &Signed<evidence::ReviewAttestation>,
        publication: &Publication,
        status: Option<&evidence::ReviewStatus>,
        workspace_id: Uuid,
        requirements: evidence::ReviewRequirements<'_>,
    ) -> Result<evidence::VerifiedReview> {
        let resolved = signed
            .payload
            .review
            .archive_reference()
            .map(|reference| {
                crate::validation_archive::read(
                    &self.records,
                    &self.key,
                    &crate::validation_archive::ValidationArchiveNamespace {
                        workspace_id,
                        runtime_id: self.host.snapshot()?.intent.runtime_id,
                    },
                    reference,
                )
            })
            .transpose()?;
        evidence::verify_review_resolved(
            signed,
            publication,
            status,
            &self.key,
            requirements,
            resolved,
        )
    }
}

// This is an input limit, not a promise that the final prompt will fit. The
// instruction assembler independently bounds its complete envelope to 1 MiB.
const REVIEW_INPUT_BYTES: usize = 1024 * 1024;

fn review_bytes<T: Serialize>(value: &T, limit: usize) -> Result<usize> {
    struct Count {
        bytes: usize,
        limit: usize,
    }
    impl std::io::Write for Count {
        fn write(&mut self, input: &[u8]) -> std::io::Result<usize> {
            let next = self
                .bytes
                .checked_add(input.len())
                .filter(|n| *n <= self.limit)
                .ok_or_else(|| std::io::Error::other("review input bound"))?;
            self.bytes = next;
            Ok(input.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count { bytes: 0, limit };
    serde_json::to_writer(&mut count, value).map_err(|_| {
        Error::new(
            "bounded_input",
            "complete artifact review input requires paging beyond its finite bound",
        )
    })?;
    Ok(count.bytes)
}

/// Original bytes for every attempt, including superseded and failed attempts.
/// Duplicate exact proofs share one body; the full history retains all uses.
/// Base64 is byte access, not evidence of visual or semantic inspection.
pub(super) fn review_artifacts(
    archive: &crate::validation_archive::ResolvedValidationArchive,
    context: &Value,
) -> Result<Value> {
    use base64::Engine;
    // Charge the field name, separators, array and full existing context first.
    let mut used = review_bytes(context, REVIEW_INPUT_BYTES)?
        .checked_add(32)
        .ok_or_else(|| Error::invalid("review input overflow"))?;
    let mut seen = BTreeSet::new();
    let mut entries = Vec::new();
    for proof in archive
        .evidence()
        .attempts
        .iter()
        .flat_map(|a| &a.artifacts)
    {
        let identity = canonical_digest(1, proof)?;
        if !seen.insert(identity.clone()) {
            continue;
        }
        let remaining = REVIEW_INPUT_BYTES.saturating_sub(used);
        // Refuse large cached artifacts before copying any original bytes.
        if proof.bytes > remaining as u64 {
            return Err(Error::new(
                "bounded_input",
                "original artifact exceeds remaining review input budget",
            ));
        }
        let bytes = archive.read_artifact(proof)?;
        let text = std::str::from_utf8(&bytes)
            .ok()
            .filter(|s| !s.contains('\0'));
        let encoding = if text.is_some() { "utf8" } else { "base64" };
        let mut entry =
            json!({"proof_digest":identity,"proof":proof,"encoding":encoding,"data":""});
        let overhead = review_bytes(&entry, remaining)? + 1;
        let available = remaining.saturating_sub(overhead);
        let data_bytes = if let Some(text) = text {
            review_bytes(&text, available.saturating_add(2))?.saturating_sub(2)
        } else {
            bytes
                .len()
                .div_ceil(3)
                .checked_mul(4)
                .filter(|n| *n <= available)
                .ok_or_else(|| {
                    Error::new(
                        "bounded_input",
                        "encoded artifact exceeds review input budget",
                    )
                })?
        };
        if overhead > remaining || data_bytes > available {
            return Err(Error::new(
                "bounded_input",
                "artifact metadata exceeds review input budget",
            ));
        }
        // Allocation happens only after charging the exact encoded size.
        entry["data"] = Value::String(match text {
            Some(text) => text.to_owned(),
            None => base64::engine::general_purpose::STANDARD.encode(&bytes),
        });
        used += overhead + data_bytes;
        entries.push(entry);
    }
    Ok(Value::Array(entries))
}

#[cfg(test)]
mod archive_context_tests {
    use super::*;
    use crate::validation_archive::{self, ResolvedValidationArchive};
    fn archive(bodies: [Vec<u8>; 2]) -> (tempfile::TempDir, ResolvedValidationArchive) {
        let temp = tempfile::tempdir().unwrap();
        let records =
            PrivateDir::create(&temp.path().canonicalize().unwrap().join("records")).unwrap();
        let key = AttestationKey::new("review-artifact-input".into(), vec![45; 32]).unwrap();
        let (namespace, mut evidence, _) = validation_archive::tests::descriptive_fixture();
        let mut original = BTreeMap::new();
        for (attempt, bytes) in evidence.attempts.iter_mut().zip(bodies) {
            let proof = &mut attempt.artifacts[0];
            proof.bytes = bytes.len() as u64;
            proof.digest = bytes_digest(&bytes);
            original.insert(canonical_digest(1, proof).unwrap(), bytes);
        }
        let archive = validation_archive::retain(&records, &key, namespace, &evidence, |proof| {
            Ok(original[&canonical_digest(1, proof)?].clone())
        })
        .unwrap();
        (temp, archive)
    }
    #[test]
    fn reviewer_receives_original_failed_and_selected_text_binary_and_nul_bytes() {
        use base64::Engine;
        for binary in [b"valid UTF8 with\0NUL".to_vec(), vec![0xff, 0xfe, 0, 1]] {
            let failed = b"original failed transcript\n".to_vec();
            let (_temp, archive) = archive([failed.clone(), binary.clone()]);
            let input =
                review_artifacts(&archive, &json!({"validation":archive.evidence()})).unwrap();
            let entries = input.as_array().unwrap();
            assert_eq!(entries.len(), 2);
            assert_eq!(entries[0]["encoding"], "utf8");
            assert_eq!(entries[0]["data"].as_str().unwrap().as_bytes(), failed);
            assert_eq!(entries[1]["encoding"], "base64");
            assert_eq!(
                base64::engine::general_purpose::STANDARD
                    .decode(entries[1]["data"].as_str().unwrap())
                    .unwrap(),
                binary
            );
            for (entry, attempt) in entries.iter().zip(&archive.evidence().attempts) {
                assert_eq!(
                    entry["proof"],
                    serde_json::to_value(&attempt.artifacts[0]).unwrap()
                );
                assert_eq!(
                    entry["proof_digest"],
                    canonical_digest(1, &attempt.artifacts[0]).unwrap()
                );
            }
            assert!(!archive.evidence().attempts[0].successful);
            assert!(archive.evidence().attempts[1].successful);
        }
    }
    #[test]
    fn reviewer_artifacts_share_context_budget_and_encoded_expansion_is_bounded() {
        for bodies in [
            [vec![b'a'; 600_000], vec![b'b'; 600_000]],
            [vec![1; 200_000], vec![b'b'; 1]],
            [vec![0xff; 900_000], vec![b'b'; 1]],
        ] {
            let (_temp, archive) = archive(bodies);
            assert_eq!(
                review_artifacts(&archive, &json!({})).unwrap_err().code,
                "bounded_input"
            );
        }
        let (_temp, archive) = archive([b"failed".to_vec(), b"passed".to_vec()]);
        assert_eq!(
            review_artifacts(
                &archive,
                &json!({"existing": "x".repeat(REVIEW_INPUT_BYTES)})
            )
            .unwrap_err()
            .code,
            "bounded_input"
        );
    }
    #[test]
    fn schema_three_publication_retains_remote_bound_after_history_compaction() {
        let key = AttestationKey::new("compact-publication-bound".into(), vec![46; 32]).unwrap();
        let signed = key.sign("child_result", &json!({"schema_version":3,
            "validation_selected":[bytes_digest(b"selected")],"review_assessment":"x".repeat(60*1024)})).unwrap();
        assert_eq!(
            publication_bounds("child_result", &signed)
                .unwrap_err()
                .code,
            "bounded_input"
        );
    }
}
