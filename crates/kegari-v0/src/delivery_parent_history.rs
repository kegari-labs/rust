//! Historical accepted evidence for explicit release continuation. This reader
//! authenticates original execution and source records; it never replaces the
//! ordinary current-production parent gate or refreshes generic readiness.
use super::*;
use crate::deployment::VerifiedRecoveryBaseline;
use crate::release::{ObservedEffect, ReleasePlan, ResourceKey};

pub struct HistoricalReleaseParent {
    parent: PublishedParent,
    vector: ParentCandidateVector,
    slices: Vec<IsolatedSlice>,
    children: Vec<PublishedChild>,
    checked_at: DateTime<Utc>,
}
impl HistoricalReleaseParent {
    pub fn manifest(&self) -> &evidence::ParentEvidence {
        self.parent.verified.manifest()
    }
    pub fn token(&self) -> &str {
        self.parent.verified.token()
    }
    pub fn publication(&self) -> &Publication {
        &self.parent.publication
    }
}
/// A release-only view whose current baseline movements are fully accounted for
/// by observed effects of the same original plan. It grants no dispatch Permit.
pub struct RecoveredReleaseParent {
    historical: HistoricalReleaseParent,
    baselines: Vec<VerifiedRecoveryBaseline>,
    plan_digest: String,
}
pub(crate) struct HistoricalReleaseParts {
    pub parent: evidence::VerifiedParent,
    pub slices: Vec<IsolatedSlice>,
    pub children: Vec<PublishedChild>,
    pub vector_digest: String,
    pub baselines: Vec<VerifiedRecoveryBaseline>,
}
impl RecoveredReleaseParent {
    pub(crate) fn into_runtime_parts(self) -> HistoricalReleaseParts {
        HistoricalReleaseParts {
            parent: self.historical.parent.verified,
            slices: self.historical.slices,
            children: self.historical.children,
            vector_digest: self.historical.vector.digest,
            baselines: self.baselines,
        }
    }
    pub fn parent(&self) -> &evidence::VerifiedParent {
        &self.historical.parent.verified
    }
    pub fn slices(&self) -> &[IsolatedSlice] {
        &self.historical.slices
    }
    pub fn children(&self) -> &[PublishedChild] {
        &self.historical.children
    }
    pub fn vector_digest(&self) -> &str {
        self.historical.vector.digest()
    }
    pub fn baselines(&self) -> &[VerifiedRecoveryBaseline] {
        &self.baselines
    }
    pub fn plan_digest(&self) -> &str {
        &self.plan_digest
    }
}
impl DeliveryCoordinator {
    pub async fn hydrate_original_release_acceptance(
        &self,
        observation: &DeliveryObservation,
        parent: &evidence::VerifiedParent,
        record_id: Uuid,
        permitted: &BTreeSet<String>,
    ) -> Result<evidence::VerifiedAcceptance> {
        self.current(observation, &[DeliveryPhase::Recovery])?;
        let linear = self
            .broker
            .linear
            .as_ref()
            .ok_or_else(|| Error::new("prerequisite", "Linear connection unavailable"))?;
        let comment = linear
            .comments(parent.manifest().work.parent)
            .await?
            .into_iter()
            .find(|comment| comment.id == record_id)
            .ok_or_else(|| {
                Error::new(
                    "evidence_unpublished",
                    "original acceptance publication disappeared",
                )
            })?;
        let signed: Signed<evidence::Acceptance> =
            evidence::decode_signed_publication(&comment.body, "human_acceptance", &self.key)?;
        let publication = self
            .linear_publication(
                observation,
                parent.manifest().work.parent,
                &record_id.to_string(),
                "human_acceptance",
                &signed,
            )
            .await?;
        let acceptance = evidence::verify_acceptance(
            &signed,
            &publication,
            parent,
            &observation.scope.work,
            false,
            permitted,
            &self.key,
        )?;
        self.current(observation, &[DeliveryPhase::Recovery])?;
        Ok(acceptance)
    }
    pub async fn hydrate_historical_release_parent(
        &self,
        input: ParentHydrationInputs<'_>,
        record_id: Uuid,
        expected_token: &str,
    ) -> Result<HistoricalReleaseParent> {
        let contract = input.contract;
        (self.current_gate)(&contract.gate)?;
        let linear = self
            .broker
            .linear
            .as_ref()
            .ok_or_else(|| Error::new("prerequisite", "Linear connection unavailable"))?;
        let comment = linear
            .comments(contract.work().parent)
            .await?
            .into_iter()
            .find(|c| c.id == record_id)
            .ok_or_else(|| {
                Error::new(
                    "evidence_unpublished",
                    "original parent publication disappeared",
                )
            })?;
        let signed: Signed<ParentResultRecord> =
            evidence::decode_signed_publication(&comment.body, "parent_result", &self.key)?;
        evidence::validate_digest(expected_token)?;
        evidence::validate_digest(&signed.payload.vector_digest)?;
        evidence::validate_digest(&signed.payload.integrated_digest)?;
        if signed.payload.evidence.token()? != expected_token {
            return Err(Error::new(
                "stale_result",
                "historical parent token differs from selected accepted result",
            ));
        }
        let vector_signed: Signed<VectorRecord> = self.records.read(&format!(
            "parent-vector-{}.json",
            &signed.payload.vector_digest[7..]
        ))?;
        self.key.verify("parent_candidate_vector", &vector_signed)?;
        let vector = ParentCandidateVector {
            signed: vector_signed,
            digest: signed.payload.vector_digest.clone(),
        };
        let r = &vector.signed.payload;
        if r.schema_version != 1
            || r.runtime_id != self.host.snapshot()?.intent.runtime_id
            || r.work != *contract.work()
            || r.contract_digest != contract.digest()?
            || vector_digest(r)? != vector.digest
            || r.slices.len() != contract.repositories().len()
            || r.slices
                .iter()
                .map(|(repo, _)| repo.clone())
                .collect::<BTreeSet<_>>()
                != *contract.repositories()
            || r.candidates.len() != r.slices.len()
            || r.baselines.len() != r.slices.len()
            || r.candidates
                .iter()
                .map(|c| c.repository.clone())
                .collect::<BTreeSet<_>>()
                != *contract.repositories()
            || r.baselines
                .iter()
                .map(|b| b.repository.clone())
                .collect::<BTreeSet<_>>()
                != *contract.repositories()
            || r.targets.len() != contract.production_targets.len()
            || r.targets
                .iter()
                .map(|t| t.target.clone())
                .collect::<BTreeSet<_>>()
                != contract.production_targets
        {
            return Err(Error::new(
                "candidate_provenance",
                "historical vector differs from complete current authorized scope",
            ));
        }
        let mut slices = Vec::new();
        for candidate in vector.candidates() {
            let operation = r
                .slices
                .iter()
                .find(|(repo, _)| repo == &candidate.repository)
                .unwrap()
                .1;
            let observation =
                self.observation(contract.gate.clone(), candidate_binding(candidate)?)?;
            let slice = self.historical_release_slice(&observation, operation)?;
            if slice.manifest() != candidate {
                return Err(Error::new(
                    "candidate_provenance",
                    "historical isolated candidate object identity changed",
                ));
            }
            self.git.verify_execution_tree(
                slice.path(),
                candidate.result.as_ref().ok_or_else(|| {
                    Error::new("candidate_provenance", "historical candidate incomplete")
                })?,
                &BTreeSet::new(),
            )?;
            slices.push(slice);
        }
        let e2e = input
            .child_gates
            .get(&contract.e2e_child)
            .ok_or_else(|| Error::new("integrated_gate", "current E2E authority missing"))?;
        let integrated = IntegratedEvidence {
            signed: self.records.read(&format!(
                "parent-integrated-{}.json",
                &signed.payload.integrated_digest[7..]
            ))?,
        };
        if integrated.digest()? != signed.payload.integrated_digest {
            return Err(Error::new(
                "integrated_gate",
                "historical integrated identity changed",
            ));
        }
        self.reverify_integrated_record(contract, &vector, e2e.work(), &integrated)?;
        if signed.payload.evidence.baselines != vector.baselines()
            || signed.payload.evidence.candidates != vector.candidates()
            || signed.payload.evidence.integrated_results != integrated.runs()
        {
            return Err(Error::new(
                "stale_result",
                "historical parent differs from authenticated execution/vector",
            ));
        }
        if signed.payload.evidence.visual {
            return Err(Error::new(
                "video_gate",
                "historical visual evidence requires current authenticated asset observation",
            ));
        }
        let children = self
            .current_parent_children(
                contract,
                input.workflow,
                input.child_gates,
                &signed.payload.child_records,
            )
            .await?;
        let candidate = vector
            .candidates()
            .first()
            .ok_or_else(|| Error::new("candidate_provenance", "empty historical vector"))?;
        let observation = self.observation(contract.gate.clone(), candidate_binding(candidate)?)?;
        let publication = self
            .linear_publication(
                &observation,
                contract.work().parent,
                &record_id.to_string(),
                "parent_result",
                &signed,
            )
            .await?;
        let matching = self
            .host
            .operations()?
            .into_iter()
            .filter(|receipt| {
                receipt.runtime_id == signed.payload.runtime_id
                    && receipt.kind == "linear_comment"
                    && matches!(
                        receipt.phase,
                        crate::control::ReceiptPhase::Acknowledged
                            | crate::control::ReceiptPhase::Reconciled
                    )
                    && receipt.expected.get("admission_operation")
                        == Some(&signed.payload.evidence.publication_operation.to_string())
                    && receipt.canonical_targets.contains(&format!(
                        "linear:{}:{}:{}",
                        contract.work().linear_endpoint,
                        contract.work().linear_workspace,
                        contract.work().parent
                    ))
                    && receipt.remote_ids.get("comment_id") == Some(&record_id.to_string())
                    && receipt.expected.get("provider_body_digest") == Some(&comment.body_digest)
                    && receipt.expected.get("provider_writer")
                        == Some(&publication.writer_account_id)
            })
            .count();
        if matching != 1 {
            return Err(Error::new(
                "reconciliation_required",
                "historical parent has no unique immutable publication receipt",
            ));
        }
        let verified = self.verify_parent_record(
            contract,
            &signed,
            &children,
            input.production_integrations,
            &publication,
        )?;
        (self.current_gate)(&contract.gate)?;
        Ok(HistoricalReleaseParent {
            parent: PublishedParent {
                verified,
                signed,
                publication,
            },
            vector,
            slices,
            children,
            checked_at: Utc::now(),
        })
    }
    fn historical_release_slice(
        &self,
        observation: &DeliveryObservation,
        operation: Uuid,
    ) -> Result<IsolatedSlice> {
        self.current(observation, &[DeliveryPhase::Recovery])?;
        // The protected read-root observer authenticates the original local
        // effect receipt and filesystem identities without requiring current main
        // to equal the historical candidate's original base.
        self.retained_candidate_host_root(observation, operation)?;
        let record: Signed<SliceRecord> = self
            .records
            .read(&format!("slice-result-{operation}.json"))?;
        self.key.verify("isolated_slice", &record)?;
        let candidate = self
            .git
            .inspect_candidate(&record.payload.intent.execution_root, operation)?;
        if candidate.manifest != record.payload.candidate {
            return Err(Error::new(
                "candidate_provenance",
                "historical candidate objects changed",
            ));
        }
        Ok(IsolatedSlice { record, candidate })
    }
    /// Every movement from the accepted historical vector must be an actual
    /// successful owned effect of this exact plan. Known main/target divergence
    /// is allowed here; it is never inserted into ordinary ready parent state.
    pub fn verify_release_history_progress(
        &self,
        historical: HistoricalReleaseParent,
        plan: &ReleasePlan,
        current: Vec<VerifiedRecoveryBaseline>,
        effects: &[ObservedReleaseStep],
    ) -> Result<RecoveredReleaseParent> {
        plan.validate()?;
        let now = Utc::now();
        let manifest = historical.manifest();
        if historical.token() != plan.evidence_token
            || manifest.work.parent != plan.parent_id
            || manifest.work.linear_workspace != plan.linear_workspace
            || manifest.work.generation != plan.generation
            || now - historical.checked_at > chrono::Duration::seconds(30)
            || current.len() != manifest.baselines.len()
            || current
                .iter()
                .map(|b| b.repository().clone())
                .collect::<BTreeSet<_>>()
                != manifest
                    .baselines
                    .iter()
                    .map(|b| b.repository.clone())
                    .collect()
        {
            return Err(Error::new(
                "stale_result",
                "release history differs from exact current plan/inventory",
            ));
        }
        let original = self.host.operation(plan.operation_id)?;
        if original.kind != "admission"
            || original.runtime_id != plan.runtime_id
            || original.expected.get("scope") != Some(&plan.digest()?)
            || original.canonical_targets
                != vec![
                    crate::authority::IssueKey::new(
                        &manifest.work.linear_endpoint,
                        manifest.work.linear_workspace,
                        manifest.work.parent,
                    )?
                    .identity()?,
                ]
        {
            return Err(Error::new(
                "ownership_conflict",
                "historical continuation lacks original actual plan admission",
            ));
        }
        let mut mains = manifest
            .baselines
            .iter()
            .map(|b| (b.repository.clone(), b.production.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut targets = historical
            .vector
            .signed
            .payload
            .targets
            .iter()
            .map(|target| {
                (
                    (target.repository.clone(), target.target.clone()),
                    (target.source.clone(), target.artifact_digest.clone()),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let mut seen = BTreeSet::new();
        for observed in effects {
            if observed.plan_digest() != plan.digest()?
                || !seen.insert(observed.step_id())
                || observed.observed_at() > now + chrono::Duration::seconds(5)
                || now - observed.observed_at() > chrono::Duration::seconds(30)
                || observed.outcome() == &ReleaseStepOutcome::Pending
                || !plan.steps.iter().any(|step| step.id == observed.step_id())
            {
                return Err(Error::new(
                    "reconciliation_required",
                    "release historical effect set is unknown, stale or ambiguous",
                ));
            }
            let Some(effect) = observed.effect() else {
                continue;
            };
            match effect {
                ObservedEffect::Merge {
                    repository, commit, ..
                } => {
                    mains.insert(repository.clone(), commit.clone());
                }
                ObservedEffect::Deployment {
                    repository,
                    target: ResourceKey::Target { id, .. },
                    source_commit,
                    tree,
                    artifact_digest,
                    ..
                }
                | ObservedEffect::Verification {
                    repository,
                    target: ResourceKey::Target { id, .. },
                    source_commit,
                    tree,
                    artifact_digest,
                    ..
                } => {
                    targets.insert(
                        (repository.clone(), id.clone()),
                        (
                            crate::git::Revision {
                                commit: source_commit.clone(),
                                tree: tree.clone(),
                            },
                            artifact_digest.clone(),
                        ),
                    );
                }
                _ => {
                    return Err(Error::new(
                        "scope_violation",
                        "release effect has no canonical target",
                    ));
                }
            }
        }
        for baseline in &current {
            let contract = self
                .contracts
                .get(baseline.repository())
                .ok_or_else(|| Error::new("scope_violation", "recovery repository removed"))?;
            if baseline.contract_digest() != contract.digest()?
                || mains.get(baseline.repository()) != Some(baseline.production())
                || baseline.observed_at() > now + chrono::Duration::seconds(5)
                || now - baseline.observed_at() > chrono::Duration::seconds(30)
                || baseline
                    .targets()
                    .iter()
                    .map(|t| {
                        (
                            (t.repository.clone(), t.target.clone()),
                            (t.source.clone(), t.artifact_digest.clone()),
                        )
                    })
                    .collect::<BTreeMap<_, _>>()
                    != targets
                        .iter()
                        .filter(|((repo, _), _)| repo == baseline.repository())
                        .map(|(key, value)| (key.clone(), value.clone()))
                        .collect()
            {
                return Err(Error::new(
                    "stale_result",
                    "current main or target moved outside this plan's verified owned effects",
                ));
            }
        }
        Ok(RecoveredReleaseParent {
            historical,
            baselines: current,
            plan_digest: plan.digest()?,
        })
    }
}

#[path = "delivery_release_origin.rs"]
mod origin;
pub use origin::VerifiedReleaseOrigin;
