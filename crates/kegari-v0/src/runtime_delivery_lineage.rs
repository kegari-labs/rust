//! Historical integrations are recovered from an authenticated finite chain.
//! A signed UUID reference alone never authorizes omitting a contribution.
use super::*;
use crate::digest::canonical_digest;

const MAX_RELEASE_LINKS: usize = 64;

#[path = "runtime_delivery_completion.rs"]
mod completion;
pub(crate) use completion::{ReleaseCompletionAnchor, VerifiedReleaseCompletion};

#[derive(Clone)]
struct Link {
    record: AttemptRecord,
    digest: String,
    origin: VerifiedReleaseOrigin,
    effects: BTreeMap<String, Arc<ObservedReleaseStep>>,
}

#[derive(Clone)]
pub(super) struct VerifiedReleaseLineage {
    // Oldest first. The final link is the immediate held predecessor.
    links: Vec<Link>,
    receipts: Vec<(Uuid, String)>,
    intents: Vec<(Uuid, String)>,
    terminal_root: bool,
}
impl VerifiedReleaseLineage {
    pub(super) fn require_origin(&self, origin: &VerifiedReleaseOrigin) -> Result<()> {
        let identity = |value: &VerifiedReleaseOrigin| {
            canonical_digest(
                1,
                &(
                    value.plan(),
                    value.manifest(),
                    value.targets(),
                    value.admission_digest(),
                ),
            )
        };
        let expected = identity(origin)?;
        for link in &self.links {
            if identity(&link.origin)? == expected {
                return Ok(());
            }
        }
        Err(Error::new(
            "scope_violation",
            "candidate origin is outside authenticated release lineage",
        ))
    }
    pub(super) fn merges(
        &self,
    ) -> impl Iterator<Item = (&VerifiedReleaseOrigin, &ObservedReleaseStep)> {
        self.links.iter().flat_map(|link| {
            link.effects.values().filter_map(move |effect| {
                matches!(effect.effect(), Some(domain::ObservedEffect::Merge { .. }))
                    .then_some((&link.origin, effect.as_ref()))
            })
        })
    }
    pub(super) fn origin(&self, effect: &ObservedReleaseStep) -> Result<&VerifiedReleaseOrigin> {
        self.merges()
            .find(|(_, actual)| {
                actual.plan_digest() == effect.plan_digest()
                    && actual.step_id() == effect.step_id()
                    && actual.attempt() == effect.attempt()
                    && actual.effect() == effect.effect()
                    && actual.outcome() == effect.outcome()
                    && actual
                        .receipt()
                        .map(|r| (&r.operation_id, &r.payload_digest))
                        == effect
                            .receipt()
                            .map(|r| (&r.operation_id, &r.payload_digest))
            })
            .map(|(origin, _)| origin)
            .ok_or_else(|| {
                Error::new(
                    "scope_violation",
                    "merge is outside authenticated release lineage",
                )
            })
    }
    pub(super) fn revalidate(
        &self,
        records: &crate::storage::PrivateDir,
        control: &crate::control::ControlHost,
        key: &evidence::AttestationKey,
    ) -> Result<()> {
        for (index, link) in self.links.iter().enumerate() {
            let stored: Signed<AttemptRecord> =
                records.read(&attempt_leaf(link.record.attempt.plan.operation_id))?;
            key.verify("runtime_release_attempt", &stored)?;
            if canonical_digest(1, &stored.payload)? != link.digest {
                return Err(Error::new(
                    "stale_result",
                    "historical release state changed",
                ));
            }
            if self.terminal_root && index + 1 == self.links.len() {
                completion::validate_completed_owner(control, &link.record)?;
            } else {
                super::baseline::validate_known_owner(control, &link.record)?;
            }
        }
        for (id, digest) in &self.receipts {
            if canonical_digest(1, &control.operation(*id)?)? != *digest {
                return Err(Error::new(
                    "stale_result",
                    "historical supersede receipt changed",
                ));
            }
        }
        for (id, digest) in &self.intents {
            let signed: Signed<Value> = records.read(&format!("release-successor-{id}.json"))?;
            key.verify("release_successor_intent", &signed)?;
            if signed.digest()? != *digest {
                return Err(Error::new(
                    "stale_result",
                    "historical successor intent changed",
                ));
            }
        }
        Ok(())
    }
}

impl RuntimeDelivery {
    pub(super) async fn release_lineage(
        &self,
        workspace: &WorkspaceDelivery,
        root: &AttemptRecord,
        origin: VerifiedReleaseOrigin,
        effects: BTreeMap<String, Arc<ObservedReleaseStep>>,
    ) -> Result<VerifiedReleaseLineage> {
        self.release_lineage_inner(workspace, root, origin, effects, false)
            .await
    }
    async fn release_lineage_inner(
        &self,
        workspace: &WorkspaceDelivery,
        root: &AttemptRecord,
        origin: VerifiedReleaseOrigin,
        effects: BTreeMap<String, Arc<ObservedReleaseStep>>,
        terminal_root: bool,
    ) -> Result<VerifiedReleaseLineage> {
        let mut result = VerifiedReleaseLineage {
            links: vec![Link {
                record: root.clone(),
                digest: canonical_digest(1, root)?,
                origin,
                effects,
            }],
            receipts: vec![],
            intents: vec![],
            terminal_root,
        };
        let mut seen = BTreeSet::from([root.attempt.plan.operation_id]);
        loop {
            let child = result.links.last().unwrap();
            let Some((record, receipts, intent_digest)) =
                self.recorded_predecessor(&child.record)?
            else {
                break;
            };
            if result.links.len() >= MAX_RELEASE_LINKS
                || !seen.insert(record.attempt.plan.operation_id)
            {
                return Err(Error::new(
                    "record_corrupt",
                    "release lineage is cyclic or exceeds its finite bound",
                ));
            }
            super::baseline::validate_known_owner(&self.runtime.control, &record)?;
            let origin = workspace
                .coordinator
                .release_origin(
                    &record.attempt.plan,
                    record.parent_publication.id,
                    record.acceptance_publication.id,
                )
                .await?;
            let effects = self.observe_lineage_effects(workspace, &record).await?;
            let (mains, targets) = super::baseline::known_effect_state(&origin, &effects)?;
            let next_mains = child
                .origin
                .manifest()
                .baselines
                .iter()
                .map(|b| (b.repository.clone(), b.production.clone()))
                .collect::<BTreeMap<_, _>>();
            let next_targets = child
                .origin
                .targets()
                .iter()
                .map(|t| {
                    (
                        (t.repository.clone(), t.target.clone()),
                        (t.source.clone(), t.artifact_digest.clone()),
                    )
                })
                .collect::<BTreeMap<_, _>>();
            if mains != next_mains || targets != next_targets {
                return Err(Error::new(
                    "production_baseline",
                    "successor baseline is not the exact known predecessor result",
                ));
            }
            result
                .intents
                .push((child.record.attempt.plan.operation_id, intent_digest));
            for receipt in receipts {
                result
                    .receipts
                    .push((receipt.operation_id, canonical_digest(1, &receipt)?));
            }
            result.links.push(Link {
                digest: canonical_digest(1, &record)?,
                record,
                origin,
                effects,
            });
        }
        result.links.reverse();
        result.verify_inventory()?;
        result.revalidate(
            &self.records,
            &self.runtime.control,
            &self.runtime.services.key,
        )?;
        Ok(result)
    }

    pub(super) async fn observe_lineage_effects(
        &self,
        workspace: &WorkspaceDelivery,
        record: &AttemptRecord,
    ) -> Result<BTreeMap<String, Arc<ObservedReleaseStep>>> {
        let mut effects = BTreeMap::new();
        for (id, state) in &record.attempt.steps {
            if matches!(state, StepState::NotStarted | StepState::Retired { .. }) {
                continue;
            }
            let effect = if let StepState::Succeeded { effect, .. } = state
                && matches!(
                    effect.as_ref(),
                    domain::ObservedEffect::Deployment { .. }
                        | domain::ObservedEffect::Verification { .. }
                ) {
                workspace
                    .coordinator
                    .recover_terminal_release_step(
                        &record.attempt.plan,
                        id,
                        record.attempt.step_attempt(id)?,
                        effect,
                        &workspace.observer,
                    )
                    .await?
            } else {
                workspace
                    .coordinator
                    .recover_owned_release_step(
                        &record.attempt.plan,
                        id,
                        record.attempt.step_attempt(id)?,
                        &workspace.observer,
                    )
                    .await?
            };
            if !match (state, effect.outcome()) {
                (
                    StepState::Succeeded {
                        effect: expected, ..
                    },
                    ReleaseStepOutcome::Succeeded,
                ) => effect.effect() == Some(expected),
                (
                    StepState::Failed { .. } | StepState::Cancelled { .. },
                    ReleaseStepOutcome::Failed(_),
                ) => true,
                _ => false,
            } {
                return Err(Error::new(
                    "reconciliation_required",
                    "historical step differs from its exact current owned observation",
                ));
            }
            effects.insert(id.clone(), Arc::new(effect));
        }
        Ok(effects)
    }
}

impl VerifiedReleaseLineage {
    fn verify_inventory(&self) -> Result<()> {
        let mut merged = BTreeMap::new();
        for link in &self.links {
            let mut declared = BTreeSet::new();
            for candidate in &link.origin.manifest().candidates {
                for record in &candidate.production_integrations {
                    let key = (record.release_operation, record.repository.clone());
                    let expected: &crate::git::ProductionIntegration =
                        merged.get(&key).ok_or_else(|| {
                            Error::new(
                                "production_provenance",
                                "historical integration has no authenticated predecessor merge",
                            )
                        })?;
                    let mut current = expected.clone();
                    current.observed_main = candidate.production.clone();
                    if &current != record || !declared.insert(key) {
                        return Err(Error::new(
                            "production_provenance",
                            "historical integration differs from its original accepted merge or repeats it",
                        ));
                    }
                }
            }
            if declared != merged.keys().cloned().collect() {
                return Err(Error::new(
                    "production_provenance",
                    "successor omitted an earlier integrated contribution",
                ));
            }
            for effect in link.effects.values() {
                let Some(domain::ObservedEffect::Merge {
                    repository,
                    pull_request,
                    commit,
                    parent_commit,
                }) = effect.effect()
                else {
                    continue;
                };
                let candidate = link
                    .origin
                    .manifest()
                    .candidates
                    .iter()
                    .find(|c| &c.repository == repository)
                    .ok_or_else(|| {
                        Error::new("production_provenance", "merge has no accepted candidate")
                    })?;
                let result = candidate.result.as_ref().ok_or_else(|| {
                    Error::new(
                        "production_provenance",
                        "historical candidate is incomplete",
                    )
                })?;
                if candidate.ordered_delta_digests.is_empty()
                    || candidate.production.commit != *parent_commit
                    || result.tree != commit.tree
                {
                    return Err(Error::new(
                        "production_provenance",
                        "historical merge differs from the accepted product patch",
                    ));
                }
                let record = crate::git::ProductionIntegration {
                    repository: repository.clone(),
                    parent_id: link.record.work.parent,
                    release_operation: link.record.attempt.plan.operation_id,
                    pull_request: *pull_request,
                    source_candidate_digest: canonical_digest(1, candidate)?,
                    accepted_evidence_token: link.origin.plan().evidence_token.clone(),
                    acceptance_digest: link.origin.plan().acceptance_digest.clone(),
                    production_parent: candidate.production.clone(),
                    candidate: result.clone(),
                    merged: commit.clone(),
                    observed_main: commit.clone(),
                    selected_delta_digests: candidate.ordered_delta_digests.clone(),
                    selected_patch_digests: candidate.ordered_patch_digests.clone(),
                    selected_contributions: candidate.selected_contributions.clone(),
                    combined_patch_digest: candidate.combined_patch_digest.clone().ok_or_else(
                        || Error::new("production_provenance", "historical patch digest missing"),
                    )?,
                };
                if merged
                    .insert((record.release_operation, repository.clone()), record)
                    .is_some()
                {
                    return Err(Error::new(
                        "production_provenance",
                        "release lineage repeats a merge",
                    ));
                }
            }
        }
        Ok(())
    }
}

/// Narrow queue projection authority. It neither removes durable holds nor
/// substitutes for accepted evidence, configured eligibility or an admission.
pub(crate) struct VerifiedSuccessorHolds {
    plan_digest: String,
    predecessor: Arc<KnownPredecessorBaseline>,
    holds: BTreeMap<Uuid, domain::ResourceHold>,
    receipts: Vec<(Uuid, String)>,
    intent_digest: String,
}
impl VerifiedSuccessorHolds {
    pub(crate) fn validate(
        &self,
        plan: &ReleasePlan,
    ) -> Result<&BTreeMap<Uuid, domain::ResourceHold>> {
        if plan.digest()? != self.plan_digest {
            return Err(Error::new(
                "scope_violation",
                "successor hold proof belongs to another exact plan",
            ));
        }
        self.predecessor.revalidate()?;
        let control = &self.predecessor.control;
        for (id, digest) in &self.receipts {
            if canonical_digest(1, &control.operation(*id)?)? != *digest {
                return Err(Error::new(
                    "stale_result",
                    "successor hold authorization receipt changed",
                ));
            }
        }
        let signed: Signed<Value> = self
            .predecessor
            .records
            .read(&format!("release-successor-{}.json", plan.operation_id))?;
        self.predecessor
            .key
            .verify("release_successor_intent", &signed)?;
        if signed.digest()? != self.intent_digest {
            return Err(Error::new(
                "stale_result",
                "successor hold authorization changed",
            ));
        }
        Ok(&self.holds)
    }
}
impl RuntimeDelivery {
    pub(super) fn successor_holds(
        &self,
        plan: &ReleasePlan,
        work: &WorkIdentity,
        predecessor: Arc<KnownPredecessorBaseline>,
    ) -> Result<VerifiedSuccessorHolds> {
        predecessor.revalidate()?;
        if work != predecessor.work() {
            return Err(Error::new(
                "scope_violation",
                "successor hold proof has another current work generation",
            ));
        }
        let (record, receipts, intent_digest) = self
            .recorded_plan_predecessor(plan, work, predecessor.gate().workspace_id())?
            .ok_or_else(|| {
                Error::new(
                    "release_hold",
                    "successor has no actual acknowledged assignment",
                )
            })?;
        if record.attempt.plan.operation_id != predecessor.predecessor()
            || canonical_digest(1, &record)? != predecessor.record_digest()
        {
            return Err(Error::new(
                "stale_result",
                "successor assignment differs from observed predecessor",
            ));
        }
        let mut holds = BTreeMap::new();
        let links = &predecessor.lineage().links;
        for (index, link) in links.iter().enumerate() {
            let next = links
                .get(index + 1)
                .map(|next| next.record.attempt.plan.operation_id)
                .unwrap_or(plan.operation_id);
            let attempt = &link.record.attempt;
            if attempt.assigned_successor != Some(next) || attempt.plan.resources != plan.resources
            {
                return Err(Error::new(
                    "release_hold",
                    "historical hold is not assigned along this exact resource chain",
                ));
            }
            holds.insert(
                attempt.plan.operation_id,
                domain::ResourceHold {
                    operation_id: attempt.plan.operation_id,
                    parent_id: attempt.plan.parent_id,
                    resources: attempt.plan.resources.clone(),
                    assigned_successor: attempt.assigned_successor,
                },
            );
        }
        let proof = VerifiedSuccessorHolds {
            plan_digest: plan.digest()?,
            predecessor,
            holds,
            receipts: receipts
                .into_iter()
                .map(|r| Ok((r.operation_id, canonical_digest(1, &r)?)))
                .collect::<Result<_>>()?,
            intent_digest,
        };
        proof.validate(plan)?;
        Ok(proof)
    }
}
