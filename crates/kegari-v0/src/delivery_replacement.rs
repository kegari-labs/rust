//! Scoped replacement work preserves the independently verified staging result.
//! Fresh production-baseline work never authorizes a coordinator repair or merge.
use super::*;
use kegari_git::{DeltaArtifact, DeltaManifest};

#[path = "delivery_replacement_recovery.rs"]
mod recovery;
pub(crate) use recovery::ReplacementCheckpoint;

/// Select the separately observed staging ancestry. A replacement head is
/// never treated as the head of the already merged related pull request.
pub(crate) fn replacement_staging_original(selected: &DeltaManifest) -> Result<&DeltaManifest> {
    selected.original_contributions()?;
    let mut current = selected;
    while let Some(mapping) = &current.replacement {
        let [original] = mapping.originals.as_slice() else {
            return Err(Error::new(
                "replacement_mapping",
                "this replacement route requires one selected contribution at each generation",
            ));
        };
        if original.repository != current.repository
            || original.parent_id != current.parent_id
            || original.child_id != current.child_id
            || original.work_generation != current.work_generation
            || original.correction_generation != current.correction_generation
            || original.scope != current.scope
            || original.staging_commit != current.staging_commit
            || current.staging_parent != current.staging_commit
            || mapping.reason.trim().is_empty()
            || original
                .replacement
                .as_ref()
                .is_some_and(|prior| prior.staging_result_digest != mapping.staging_result_digest)
        {
            return Err(Error::new(
                "replacement_mapping",
                "replacement source differs from its original scoped staging ancestry",
            ));
        }
        evidence::validate_digest(&mapping.staging_result_digest)?;
        current = original;
    }
    Ok(current)
}

#[derive(Clone)]
pub(crate) struct ReplacementWork {
    gate: FreshGate,
    contract_digest: String,
    staged: StagedDelta,
    published: PublishedChild,
    staging_result: PublishedChild,
    source: Arc<crate::remote_objects::VerifiedObjectSource>,
    reason: String,
    digest: String,
}
impl ReplacementWork {
    pub(crate) fn authority(&self) -> &FreshGate {
        &self.gate
    }
    pub(crate) fn source(&self) -> &Arc<crate::remote_objects::VerifiedObjectSource> {
        &self.source
    }
    pub(crate) fn contract_digest(&self) -> &str {
        &self.contract_digest
    }
    pub(crate) fn digest(&self) -> &str {
        &self.digest
    }
    pub(crate) fn original_digest(&self) -> Result<String> {
        self.staged.delta.manifest.digest()
    }
    pub(crate) fn context(&self) -> Result<Value> {
        Ok(json!({"replacement_assignment":{
            "schema_version":1,"binding":self.digest,"production":self.source.revision(),
            "originals":[self.staged.delta.manifest],"staged_result":self.staging_result.verified.evidence(),
            "reason":self.reason,
            "task":"Reauthor only this child's original contribution against this exact production baseline. Preserve every original contribution and stay inside its original write scope. Do not edit other children's work or repair the aggregate candidate. The staged behavior already has independent successful evidence; if your solution requires additional staging changes, report that blocker instead of claiming completion. Fresh local validation, exact-SHA hosted checks and Codex review follow this run.",
            "outcome_claim":{"schema_version":1,"summary":"actual scoped replacement and behavior equivalence", "fulfilled_criteria":self.published.verified.evidence().fulfilled_criteria,"disposition":{"kind":"changed"}}
        }}))
    }
    pub(crate) fn verify_completion(
        &self,
        completion: &crate::worker_run::RunCompletion,
    ) -> Result<()> {
        let contract = completion.contract.as_ref().ok_or_else(|| {
            Error::new(
                "replacement_mapping",
                "replacement completion lost its actual contract",
            )
        })?;
        super::assignment::verify_assignment_contract(
            contract,
            &self.gate,
            "child_replacement",
            &self.context()?,
        )?;
        let execution = completion.execution.as_ref().ok_or_else(|| {
            Error::new(
                "execution_ownership",
                "replacement has no owned child clone",
            )
        })?;
        if execution.binding.repository != self.staged.delta.manifest.repository
            || execution.binding.revision != *self.source.revision()
        {
            return Err(Error::new(
                "replacement_mapping",
                "replacement did not start from its exact isolated production baseline",
            ));
        }
        Ok(())
    }
}
#[derive(Clone)]
pub struct ReplacementDelta {
    work: Arc<ReplacementWork>,
    delta: DeltaArtifact,
}
impl ReplacementDelta {
    pub(crate) fn review_context(&self) -> Result<Value> {
        Ok(
            json!({"replacement":self.delta.manifest,"original_work":self.work.context()?,
            "review_requirement":"Assess that the isolated replacement preserves every original contribution and fulfilled criterion inside the original child scope. Verify the separate staged result already represents the intended behavior; if additional staging changes are needed, return findings. Do not treat old staging checks as evidence that this replacement passed its fresh source-bound checks."}),
        )
    }
    pub(crate) fn delta(&self) -> &DeltaArtifact {
        &self.delta
    }
    pub(crate) fn matches(&self, admission: &DeliveryObservation) -> Result<()> {
        if self.work.gate.work() != &admission.scope.work
            || self.work.contract_digest != admission.scope.contract_digest
            || self.delta.manifest.validated_base.commit != admission.scope.revision.base
            || self.delta.manifest.validated_head.commit != admission.scope.revision.head
            || self.delta.manifest.validated_head.tree != admission.scope.revision.tested_tree
        {
            return Err(Error::new(
                "replacement_mapping",
                "replacement differs from current scoped source admission",
            ));
        }
        Ok(())
    }
    pub(crate) fn branch(
        &self,
        git: &Git,
        identifier: &str,
        title: &str,
    ) -> Result<kegari_git::BranchName> {
        let suffix = self.delta.manifest.digest()?;
        git.branch_name(
            &format!("{identifier}-replacement-{}", &suffix[7..23]),
            title,
            255,
        )
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplacementRecord {
    schema_version: u32,
    runtime: Uuid,
    delta: DeltaManifest,
    patch_chunks: Vec<String>,
    staged: Signed<ChildResultRecord>,
    staged_publication: Publication,
    related_publication: Publication,
    provenance: Signed<Value>,
}
impl DeliveryCoordinator {
    pub(crate) async fn prepare_replacement_work(
        &self,
        admission: &DeliveryAdmission,
        gate: &FreshGate,
        staged: &StagedDelta,
        published: &PublishedChild,
        workflow: &Workflow,
        reason: &str,
    ) -> Result<Arc<ReplacementWork>> {
        self.current(admission, &[DeliveryPhase::CandidateAssembly])?;
        (self.current_gate)(gate)?;
        let original = &staged.delta.manifest;
        let (pull_request, delta) = match &published.verified.evidence().outcome {
            evidence::ChildOutcome::Changed {
                pull_request,
                delta,
            } => (pull_request, delta),
            evidence::ChildOutcome::Replacement {
                related_pull_request,
                delta,
            } => (related_pull_request, delta),
            evidence::ChildOutcome::NoChange { .. } => {
                return Err(Error::new(
                    "replacement_gate",
                    "replacement requires a selected changed contribution with independent staging evidence",
                ));
            }
        };
        if admission.scope.work.child.is_some()
            || gate.work().parent != admission.scope.work.parent
            || gate.work().generation != admission.scope.work.generation
            || gate.work().child != Some(original.child_id)
            || published.verified.evidence().work != *gate.work()
            || delta.as_ref() != original
            || *pull_request != staged.pull_request
            || reason.trim().is_empty()
            || reason.len() > 4096
        {
            return Err(Error::new(
                "replacement_mapping",
                "replacement work does not select exactly this current child's original staged contribution",
            ));
        }
        let observation =
            self.observation(gate.clone(), published.verified.evidence().revision.clone())?;
        self.validate_child_record(
            &observation,
            workflow,
            &published.verified.evidence().fulfilled_criteria,
            &published.signed,
            &published.publication,
        )?;
        let staging_result = if original.replacement.is_some() {
            self.reobserve_replacement(&observation, original).await?;
            let (record, verified) = self.replacement_staged(&observation, workflow, original)?;
            PublishedChild {
                verified,
                signed: record.staged,
                publication: record.staged_publication,
            }
        } else {
            published.clone()
        };
        let contract = self.contract(admission)?;
        let scope = gate.child_scope().ok_or_else(|| {
            Error::new(
                "scope_violation",
                "replacement requires original child scope",
            )
        })?;
        if scope.write_repository.as_deref() != Some(contract.configuration_name.as_str())
            || original.scope.paths != scope.write_paths
        {
            return Err(Error::new(
                "scope_violation",
                "replacement cannot enlarge the original repository boundary",
            ));
        }
        let production = self.main_revision(admission).await?;
        if production.commit != admission.scope.revision.base {
            return Err(Error::new(
                "stale_result",
                "production baseline changed before replacement preparation",
            ));
        }
        let request = crate::remote_objects::AuthorizedObjectRequest::for_candidate(
            contract,
            production,
            &admission.permit,
            admission,
            &[staged],
            &[],
        )?;
        let source = Arc::new(
            self.object_acquisition(admission, self.processes.clone(), self.executions.clone())?
                .acquire(request)
                .await?,
        );
        self.key.verify("child_result", &published.signed)?;
        let digest = canonical_digest(
            1,
            &json!({"gate":gate.digest()?, "contract":contract.digest()?,
            "staged":published.verified.digest(),"original":original.digest()?,"source":source.digest()?,"reason":reason}),
        )?;
        Ok(Arc::new(ReplacementWork {
            gate: gate.clone(),
            contract_digest: contract.digest()?,
            staged: staged.clone(),
            published: published.clone(),
            staging_result,
            source,
            reason: reason.into(),
            digest,
        }))
    }
    pub(crate) async fn capture_replacement_source(
        &self,
        admission: &DeliveryAdmission,
        work: Arc<ReplacementWork>,
        claim: &CompletedChildClaim,
        source: &SealedChildSource,
        completion: &crate::worker_run::RunCompletion,
    ) -> Result<ReplacementDelta> {
        self.current(admission, &[DeliveryPhase::PublishChild])?;
        self.current_child_claim(admission, claim)?;
        source.matches_worker(admission, claim)?;
        work.verify_completion(completion)?;
        if claim.claim().fulfilled_criteria != work.published.verified.evidence().fulfilled_criteria
        {
            return Err(Error::new(
                "replacement_mapping",
                "replacement must preserve the original fulfilled criteria",
            ));
        }
        let execution = completion.execution.as_ref().unwrap();
        let root = completion
            .contract
            .as_ref()
            .unwrap()
            .isolation()
            .plan()
            .repository
            .clone();
        execution.ownership.verify(
            &execution.binding,
            completion.contract.as_ref().unwrap().isolation(),
        )?;
        let git = self.git.clone();
        let owned = work.clone();
        let head = admission.scope.revision.head.clone();
        let delta = tokio::task::spawn_blocking(move || {
            let original = &owned.staged.delta.manifest;
            git.capture_replacement(
                &root,
                crate::git::CaptureReplacement {
                    repository: original.repository.clone(),
                    parent_id: original.parent_id,
                    child_id: original.child_id,
                    work_generation: &original.work_generation,
                    correction_generation: &original.correction_generation,
                    scope: original.scope.clone(),
                    production_base: &owned.source.revision().commit,
                    head: &head,
                    staging_revision: &original.staging_commit.commit,
                    staging_result_digest: owned.staging_result.verified.digest(),
                    originals: std::slice::from_ref(&owned.staged.delta),
                    reason: &owned.reason,
                },
            )
        })
        .await
        .map_err(|_| Error::new("operation_uncertain", "replacement capture did not join"))??;
        let result = ReplacementDelta { work, delta };
        result.matches(admission)?;
        Ok(result)
    }
    pub(crate) async fn publish_replacement_branch(
        &self,
        admission: &DeliveryAdmission,
        replacement: &ReplacementDelta,
        source: &SealedChildSource,
        local: &LocalPassed,
        branch: &kegari_git::BranchName,
    ) -> Result<PublishedChildBranch> {
        self.publish_child_objects_inner(
            admission,
            ChildObjectPublication {
                change: source.change().ok_or_else(|| {
                    Error::new("replacement_mapping", "replacement has no actual patch")
                })?,
                committed: source.committed().ok_or_else(|| {
                    Error::new("replacement_mapping", "replacement was not committed")
                })?,
                local,
                branch,
                previous: None,
            },
            Some(replacement),
        )
        .await
    }
    pub(crate) async fn publish_replacement_child(
        &self,
        admission: &DeliveryAdmission,
        workflow: &Workflow,
        replacement: &ReplacementDelta,
        local: &LocalPassed,
        exact: &ExactValidationPassed,
        review: &PublishedReview,
    ) -> Result<(PublishedChild, StagedDelta)> {
        self.current(admission, &[DeliveryPhase::Replacement])?;
        replacement.matches(admission)?;
        local.matches(admission)?;
        local.verify_review_adequacy(review)?;
        exact.hosted.matches(admission, CheckKind::HostedCi)?;
        if workflow.digest != admission.gate.workflow_digest()
            || review.verified.attestation().delta_digest
                != replacement.delta.manifest.review_digest()?
        {
            return Err(Error::new(
                "replacement_gate",
                "replacement requires fresh exact-source validation and review",
            ));
        }
        if self.main_revision(admission).await? != replacement.delta.manifest.validated_base {
            return Err(Error::new(
                "stale_result",
                "production changed before replacement publication",
            ));
        }
        self.observe_owned_validation_current(admission, exact)
            .await?;
        self.reobserve_review(admission, review).await?;
        let original = &replacement.work;
        let staged_original = replacement_staging_original(&original.staged.delta.manifest)?;
        let pr = self
            .github()?
            .pull_request(
                &self.contract(admission)?.provider,
                original.staged.pull_request,
            )
            .await?;
        if pr.merged != Some(true)
            || pr.head_sha != staged_original.validated_head.commit
            || pr.merge_commit_sha.as_deref()
                != Some(staged_original.staging_commit.commit.as_str())
        {
            return Err(Error::new(
                "replacement_gate",
                "original independently staged result no longer matches its related PR",
            ));
        }
        let checks: Vec<_> = local
            .checks
            .iter()
            .chain(&exact.hosted.checks)
            .cloned()
            .collect();
        let mut results: Vec<_> = checks
            .iter()
            .flat_map(|c| c.artifacts.iter().cloned())
            .collect();
        results.push(ArtifactReference {
            reference: format!("retained-delta:{}", replacement.delta.manifest.digest()?),
            digest: replacement.delta.manifest.digest()?,
        });
        let evidence = evidence::ChildEvidence {
            schema_version: local.child_schema(),
            validation_selected: local
                .validation_archive
                .as_ref()
                .map(|a| a.evidence().selected_digests())
                .transpose()?,
            work: admission.scope.work.clone(),
            revision: admission.scope.revision.clone(),
            fulfilled_criteria: original
                .published
                .verified
                .evidence()
                .fulfilled_criteria
                .clone(),
            outcome: evidence::ChildOutcome::Replacement {
                related_pull_request: original.staged.pull_request,
                delta: Box::new(replacement.delta.manifest.clone()),
            },
            checks: CheckSnapshot {
                pagination_complete: true,
                observed_at: self.operation_time(admission, "replacement-result")?,
                runs: if local.validation_archive.is_some() {
                    checks
                        .into_iter()
                        .filter(|r| r.kind != CheckKind::Local)
                        .collect()
                } else {
                    checks
                },
            },
            review_digest: review.verified.digest().into(),
            results_for_dependents: results,
        };
        let publication_payload = json!({"evidence":evidence,"review":review.signed});
        let signed_publication = self
            .key
            .sign("replacement_provenance", &publication_payload)?;
        let receipt = self
            .effect(
                admission,
                "publish-replacement-provenance",
                ProviderMutation::GithubComment {
                    repository: self.contract(admission)?.provider.clone(),
                    pr: original.staged.pull_request,
                    expected_head: pr.head_sha,
                    body: record_body("replacement_provenance", &signed_publication)?,
                },
            )
            .await?;
        let mut related = self
            .github_publication(
                admission,
                original.staged.pull_request,
                &remote_id(&receipt, "record_id")?,
                "replacement_provenance",
                &signed_publication,
            )
            .await?;
        related.payload_digest = canonical_digest(1, &evidence)?;
        let digest = replacement.delta.manifest.digest()?;
        let chunks = replacement
            .delta
            .patch
            .chunks(4 * 1024 * 1024)
            .enumerate()
            .map(|(index, bytes)| {
                let leaf = format!("delta-{}-{index}.bin", &digest[7..]);
                self.records.write_bytes(&leaf, bytes)?;
                Ok(leaf)
            })
            .collect::<Result<Vec<_>>>()?;
        let record = ReplacementRecord {
            schema_version: 1,
            runtime: admission.scope.runtime_id,
            delta: replacement.delta.manifest.clone(),
            patch_chunks: chunks,
            staged: original.staging_result.signed.clone(),
            staged_publication: original.staging_result.publication.clone(),
            related_publication: related,
            provenance: signed_publication,
        };
        self.records.write(
            &format!("replacement-{}.json", &digest[7..]),
            &self.key.sign("replacement_delta", &record)?,
        )?;
        let signed = self.key.sign(
            "child_result",
            &ChildResultRecord {
                schema_version: local.child_schema(),
                runtime_id: admission.scope.runtime_id,
                publication_operation: admission.operation_id(),
                contract_digest: admission.scope.contract_digest.clone(),
                workflow_digest: workflow.digest.clone(),
                evidence,
                review: review.signed.clone(),
                review_publication: review.publication.clone(),
                review_status: review.status.clone(),
                hosted_proofs: exact.hosted.proofs.clone(),
                protection: None,
            },
        )?;
        let published = self
            .publish_child_record(
                admission,
                workflow,
                &original.published.verified.evidence().fulfilled_criteria,
                signed,
            )
            .await?;
        let staged = StagedDelta {
            delta: replacement.delta.clone(),
            pull_request: original.staged.pull_request,
            merge_receipt: original.staged.merge_receipt,
            protection: original.staged.protection.clone(),
        };
        Ok((published, staged))
    }
    fn replacement_record(&self, delta: &DeltaManifest) -> Result<ReplacementRecord> {
        let signed: Signed<ReplacementRecord> = self
            .records
            .read(&format!("replacement-{}.json", &delta.digest()?[7..]))?;
        self.key.verify("replacement_delta", &signed)?;
        if signed.payload.schema_version != 1
            || signed.payload.delta != *delta
            || delta.replacement.is_none()
        {
            return Err(Error::new(
                "replacement_mapping",
                "retained replacement record differs from selected delta",
            ));
        }
        Ok(signed.payload)
    }
    fn replacement_staged(
        &self,
        observation: &DeliveryObservation,
        workflow: &Workflow,
        delta: &DeltaManifest,
    ) -> Result<(ReplacementRecord, evidence::VerifiedChild)> {
        let record = self.replacement_record(delta)?;
        if record.runtime != observation.scope.runtime_id
            || !matches!(
                record.staged.payload.evidence.outcome,
                evidence::ChildOutcome::Changed { .. }
            )
        {
            return Err(Error::new(
                "replacement_gate",
                "replacement staged proof is not a normal result from this runtime",
            ));
        }
        let evidence::ChildOutcome::Changed {
            delta: staged_delta,
            ..
        } = &record.staged.payload.evidence.outcome
        else {
            unreachable!()
        };
        if replacement_staging_original(delta)? != staged_delta.as_ref() {
            return Err(Error::new(
                "replacement_mapping",
                "retained replacement ancestry differs from its original normal staging proof",
            ));
        }
        let mut original = observation.clone();
        original.scope.revision = record.staged.payload.evidence.revision.clone();
        let staged = self.validate_child_record(
            &original,
            workflow,
            &record.staged.payload.evidence.fulfilled_criteria,
            &record.staged,
            &record.staged_publication,
        )?;
        Ok((record, staged))
    }
    pub(super) fn replacement_staging_protection(
        &self,
        observation: &DeliveryObservation,
        workflow: &Workflow,
        delta: &DeltaManifest,
    ) -> Result<evidence::RepositoryProtection> {
        let (record, _) = self.replacement_staged(observation, workflow, delta)?;
        record.staged.payload.protection.ok_or_else(|| {
            Error::new(
                "replacement_mapping",
                "original staged protection is unavailable",
            )
        })
    }

    pub(super) fn validate_replacement_draft(
        &self,
        observation: &DeliveryObservation,
        workflow: &Workflow,
        child: &evidence::ChildEvidence,
        requirements: evidence::ChildRequirements<'_>,
        review: &evidence::VerifiedReview,
    ) -> Result<()> {
        let evidence::ChildOutcome::Replacement {
            related_pull_request,
            delta,
        } = &child.outcome
        else {
            unreachable!()
        };
        let (record, staged) = self.replacement_staged(observation, workflow, delta)?;
        evidence::validate_replacement_candidate(
            child,
            requirements,
            review,
            evidence::ReplacementGates {
                staged_result: &staged,
                related_pull_request: *related_pull_request,
                repository: &delta.repository,
                github_publication: &record.related_publication,
                github_writer: &record.related_publication.writer_account_id,
            },
        )
    }
    pub(super) fn verify_replacement_record(
        &self,
        observation: &DeliveryObservation,
        workflow: &Workflow,
        child: &evidence::ChildEvidence,
        requirements: evidence::ChildRequirements<'_>,
        review: &evidence::VerifiedReview,
        publication: &Publication,
        writer: &str,
    ) -> Result<evidence::VerifiedChild> {
        let evidence::ChildOutcome::Replacement {
            related_pull_request,
            delta,
        } = &child.outcome
        else {
            unreachable!()
        };
        let (record, staged) = self.replacement_staged(observation, workflow, delta)?;
        evidence::verify_replacement_child(
            child,
            requirements,
            review,
            evidence::ReplacementGates {
                staged_result: &staged,
                related_pull_request: *related_pull_request,
                repository: &delta.repository,
                github_publication: &record.related_publication,
                github_writer: &record.related_publication.writer_account_id,
            },
            publication,
            writer,
        )
    }
    pub(super) async fn reobserve_replacement(
        &self,
        observation: &DeliveryObservation,
        delta: &DeltaManifest,
    ) -> Result<()> {
        self.current(observation, &[observation.scope.phase])?;
        let record = self.replacement_record(delta)?;
        self.key
            .verify("replacement_provenance", &record.provenance)?;
        let evidence::ChildOutcome::Changed {
            pull_request,
            delta: original,
        } = &record.staged.payload.evidence.outcome
        else {
            return Err(Error::new(
                "replacement_gate",
                "replacement original was not normal staging work",
            ));
        };
        if record.runtime != observation.scope.runtime_id
            || delta.parent_id != observation.scope.work.parent
            || delta.work_generation != observation.scope.work.generation.definition
            || delta.correction_generation != observation.scope.work.generation.correction
            || delta.repository != observation.scope.repository
        {
            return Err(Error::new(
                "replacement_mapping",
                "replacement provenance is outside current candidate authority",
            ));
        }
        let original_publication = self
            .linear_publication(
                observation,
                record
                    .staged
                    .payload
                    .evidence
                    .work
                    .child
                    .ok_or_else(|| Error::new("replacement_gate", "original child missing"))?,
                &record.staged_publication.record_id,
                "child_result",
                &record.staged,
            )
            .await?;
        if original_publication.writer_account_id != record.staged_publication.writer_account_id {
            return Err(Error::new(
                "replacement_gate",
                "original staging publication writer changed",
            ));
        }
        let pr = self
            .github()?
            .pull_request(&self.contract(observation)?.provider, *pull_request)
            .await?;
        if pr.merged != Some(true)
            || pr.head_sha != original.validated_head.commit
            || pr.merge_commit_sha.as_deref() != Some(original.staging_commit.commit.as_str())
            || pr.base_ref != self.contract(observation)?.configuration.staging
        {
            return Err(Error::new(
                "replacement_gate",
                "replacement original staging PR changed",
            ));
        }
        let publication = self
            .github_publication(
                observation,
                *pull_request,
                &record.related_publication.record_id,
                "replacement_provenance",
                &record.provenance,
            )
            .await?;
        if publication.writer_account_id != record.related_publication.writer_account_id
            || record.provenance.payload.get("evidence").is_none_or(|e| {
                canonical_digest(1, e).as_deref()
                    != Ok(record.related_publication.payload_digest.as_str())
            })
        {
            return Err(Error::new(
                "evidence_unpublished",
                "replacement provenance no longer matches its actual related PR publication",
            ));
        }
        Ok(())
    }
    pub(super) fn retained_replacement_delta(
        &self,
        delta: &DeltaManifest,
    ) -> Result<DeltaArtifact> {
        let record = self.replacement_record(delta)?;
        replacement_staging_original(delta)?;
        if record.patch_chunks.len() > 64 {
            return Err(Error::new(
                "bounded_input",
                "replacement patch exceeds 256 MiB",
            ));
        }
        let mut patch = Vec::new();
        let mut seen = BTreeSet::new();
        for leaf in record.patch_chunks {
            if !seen.insert(leaf.clone()) {
                return Err(Error::new(
                    "record_corrupt",
                    "duplicate replacement patch chunk",
                ));
            }
            let bytes = self.records.read_bytes(&leaf, 4 * 1024 * 1024)?;
            if patch.len().saturating_add(bytes.len()) > 256 * 1024 * 1024 {
                return Err(Error::new(
                    "bounded_input",
                    "replacement patch exceeds bound",
                ));
            }
            patch.extend_from_slice(&bytes);
        }
        if bytes_digest(&patch) != delta.patch_digest {
            return Err(Error::new(
                "replacement_mapping",
                "retained replacement patch changed",
            ));
        }
        Ok(DeltaArtifact {
            manifest: delta.clone(),
            patch,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn original() -> DeltaManifest {
        let revision = |digit: &str| kegari_git::Revision {
            commit: digit.repeat(40),
            tree: digit.repeat(40),
        };
        DeltaManifest {
            schema_version: kegari_git::DELTA_SCHEMA,
            git_version: kegari_git::SUPPORTED_GIT_VERSION.into(),
            repository: RepositoryId {
                hostname: "github.invalid".into(),
                id: "42".into(),
            },
            parent_id: Uuid::from_u128(1),
            child_id: Uuid::from_u128(2),
            work_generation: bytes_digest(b"work"),
            correction_generation: bytes_digest(b"correction"),
            scope: kegari_git::WriteScope {
                paths: vec!["src/".into()],
            },
            validated_base: revision("1"),
            validated_head: revision("2"),
            staging_parent: revision("1"),
            staging_commit: revision("3"),
            patch_digest: bytes_digest(b"original"),
            paths: vec!["src/a".into()],
            replacement: None,
        }
    }
    #[test]
    fn replacement_acquisition_preserves_original_pr_ancestry_and_child_scope() {
        let original = original();
        let mut replacement = original.clone();
        replacement.validated_base = kegari_git::Revision {
            commit: "4".repeat(40),
            tree: "4".repeat(40),
        };
        replacement.validated_head = kegari_git::Revision {
            commit: "5".repeat(40),
            tree: "5".repeat(40),
        };
        replacement.patch_digest = bytes_digest(b"replacement");
        replacement.staging_parent = original.staging_commit.clone();
        replacement.replacement = Some(kegari_git::ReplacementMapping {
            originals: vec![original.clone()],
            reason: "remove dependency on unrelated staging context".into(),
            staging_result_digest: bytes_digest(b"independent staged proof"),
        });
        let selected = replacement_staging_original(&replacement).unwrap();
        assert_eq!(selected, &original);
        assert_ne!(selected.validated_head, replacement.validated_head);
        assert_eq!(
            replacement.original_contributions().unwrap(),
            BTreeSet::from([original.digest().unwrap()])
        );
        let mut successor = replacement.clone();
        successor.validated_base = kegari_git::Revision {
            commit: "6".repeat(40),
            tree: "6".repeat(40),
        };
        successor.validated_head = kegari_git::Revision {
            commit: "7".repeat(40),
            tree: "7".repeat(40),
        };
        successor.replacement.as_mut().unwrap().originals = vec![replacement.clone()];
        assert_eq!(replacement_staging_original(&successor).unwrap(), &original);
        assert_eq!(
            successor.original_contributions().unwrap(),
            replacement.original_contributions().unwrap()
        );
        assert_eq!(
            successor.replacement.as_ref().unwrap().originals,
            vec![replacement.clone()]
        );
        let mut changed_staging_proof = successor.clone();
        changed_staging_proof
            .replacement
            .as_mut()
            .unwrap()
            .staging_result_digest = bytes_digest(b"another staged proof");
        assert_eq!(
            replacement_staging_original(&changed_staging_proof)
                .unwrap_err()
                .code,
            "replacement_mapping"
        );
        let mut duplicate = successor;
        duplicate
            .replacement
            .as_mut()
            .unwrap()
            .originals
            .push(replacement.clone());
        assert_eq!(
            replacement_staging_original(&duplicate).unwrap_err().code,
            "replacement_mapping"
        );
        let mut wrong_child = replacement.clone();
        wrong_child.child_id = Uuid::from_u128(3);
        assert_eq!(
            replacement_staging_original(&wrong_child).unwrap_err().code,
            "replacement_mapping"
        );
        let mut dropped = replacement.clone();
        dropped.replacement.as_mut().unwrap().originals.clear();
        assert_eq!(
            replacement_staging_original(&dropped).unwrap_err().code,
            "replacement_mapping"
        );
        let mut enlarged = replacement.clone();
        enlarged.scope.paths.push("other-child/".into());
        assert_eq!(
            replacement_staging_original(&enlarged).unwrap_err().code,
            "replacement_mapping"
        );
        let mut transplanted = replacement;
        transplanted.staging_commit = kegari_git::Revision {
            commit: "6".repeat(40),
            tree: "6".repeat(40),
        };
        assert_eq!(
            replacement_staging_original(&transplanted)
                .unwrap_err()
                .code,
            "replacement_mapping"
        );
    }
}
