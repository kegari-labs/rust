//! A corrected preparation may use a known held predecessor's exact observed
//! baseline. This capability grants neither release eligibility nor a permit.
use super::*;
use crate::deployment::VerifiedRecoveryBaseline;
use crate::digest::canonical_digest;

#[derive(Clone)]
pub struct KnownPredecessorBaseline {
    record: AttemptRecord,
    record_digest: String,
    gate: FreshGate,
    origin: VerifiedReleaseOrigin,
    baselines: Vec<VerifiedRecoveryBaseline>,
    effects: BTreeMap<String, Arc<ObservedReleaseStep>>,
    records: Arc<crate::storage::PrivateDir>,
    control: Arc<crate::control::ControlHost>,
    key: Arc<evidence::AttestationKey>,
    observed_at: chrono::DateTime<Utc>,
}
impl KnownPredecessorBaseline {
    pub fn predecessor(&self) -> Uuid {
        self.record.attempt.plan.operation_id
    }
    pub fn work(&self) -> &WorkIdentity {
        self.gate.work()
    }
    pub fn gate(&self) -> &FreshGate {
        &self.gate
    }
    pub fn origin(&self) -> &VerifiedReleaseOrigin {
        &self.origin
    }
    pub fn observed_at(&self) -> chrono::DateTime<Utc> {
        self.observed_at
    }
    pub fn baselines(&self) -> &[VerifiedRecoveryBaseline] {
        &self.baselines
    }
    pub fn effects(&self) -> impl Iterator<Item = &ObservedReleaseStep> {
        self.effects.values().map(Arc::as_ref)
    }
    pub fn baseline(&self, repository: &RepositoryId) -> Result<&VerifiedRecoveryBaseline> {
        self.baselines
            .iter()
            .find(|b| b.repository() == repository)
            .ok_or_else(|| {
                Error::new(
                    "scope_violation",
                    "successor repository is outside the held resource vector",
                )
            })
    }
    /// Cache/durable-read only: safe in an existing serialized control callback.
    /// This cannot renew provider observations or clear a held resource.
    pub fn revalidate(&self) -> Result<()> {
        let now = Utc::now();
        if self.observed_at > now + Duration::seconds(5)
            || now - self.observed_at > Duration::seconds(30)
        {
            return Err(Error::new(
                "stale_result",
                "known predecessor baseline observation expired",
            ));
        }
        let stored: Signed<AttemptRecord> = self.records.read(&attempt_leaf(self.predecessor()))?;
        self.key.verify("runtime_release_attempt", &stored)?;
        if canonical_digest(1, &stored.payload)? != self.record_digest {
            return Err(Error::new(
                "stale_result",
                "predecessor delivery state changed",
            ));
        }
        validate_known_owner(&self.control, &self.record)?;
        Ok(())
    }
    pub(crate) fn require_effect(
        &self,
        effect: &ObservedReleaseStep,
        gate: &FreshGate,
    ) -> Result<()> {
        self.revalidate()?;
        self.gate
            .compare_current(gate, Utc::now(), Duration::seconds(30))?;
        let actual = self
            .effects
            .get(effect.step_id())
            .ok_or_else(|| Error::new("scope_violation", "effect is outside known predecessor"))?;
        if actual.plan_digest() != effect.plan_digest()
            || actual.attempt() != effect.attempt()
            || actual.effect() != effect.effect()
            || actual.outcome() != effect.outcome()
            || actual
                .receipt()
                .map(|r| (&r.operation_id, &r.payload_digest))
                != effect
                    .receipt()
                    .map(|r| (&r.operation_id, &r.payload_digest))
        {
            return Err(Error::new(
                "scope_violation",
                "effect differs from authenticated predecessor observation",
            ));
        }
        Ok(())
    }
}
fn validate_known_owner(
    control: &crate::control::ControlHost,
    record: &AttemptRecord,
) -> Result<()> {
    record.attempt.validate_recovered()?;
    let actual = control.operation(record.admission_operation)?;
    if !record.local_retired
        || record.attempt.has_unknown()
        || !record.pending_receipts.is_empty()
        || record.attempt.hold_reason.is_none()
        || record.attempt.resolved
        || record.attempt.completed()
        || actual.kind != "admission"
        || actual.phase != ReceiptPhase::Reconciled
        || actual.runtime_id != record.runtime
        || actual.expected.get("scope") != Some(&admission_scope(&actual, &record.attempt.plan)?)
        || actual.canonical_targets != vec![issue(record)?.identity()?]
        || actual.observed_result.as_ref().is_none_or(|value| {
            value["local_execution"] != "exited"
                || value["unresolved_effects"]
                    .as_array()
                    .is_none_or(|effects| !effects.is_empty())
        })
    {
        return Err(Error::new(
            "reconciliation_required",
            "predecessor retains uncertain or live ownership",
        ));
    }
    Ok(())
}
impl RuntimeDelivery {
    pub async fn known_predecessor_baseline(
        &self,
        projection: &RuntimeProjection,
        gate: &FreshGate,
    ) -> Result<Option<Arc<KnownPredecessorBaseline>>> {
        self.restore_releases()?;
        (self.current_gate())(gate)?;
        let workspace = self.workspace(projection, gate.workspace_id())?;
        let configured = workspace
            .configuration
            .workspaces
            .values()
            .find(|w| w.id == gate.workspace_id())
            .ok_or_else(|| Error::new("scope_violation", "successor workspace disappeared"))?;
        let parent = projection.parent(gate.workspace_id(), gate.work().parent)?;
        let contract =
            workspace
                .coordinator
                .parent_contract(gate.clone(), parent, &workspace.workflow)?;
        let resources = workspace
            .contracts
            .iter()
            .filter(|repository| contract.repositories().contains(&repository.repository))
            .flat_map(|contract| {
                let mut resources = vec![domain::ResourceKey::Repository {
                    repository: contract.repository.clone(),
                }];
                if let crate::config::Deployment::Steps { steps } =
                    &contract.configuration.deployment
                {
                    resources.extend(steps.iter().map(|step| domain::ResourceKey::Target {
                        provider: "github".into(),
                        id: step.target_id.clone(),
                    }));
                }
                resources
            })
            .collect::<BTreeSet<_>>();
        let candidates = self
            .releases
            .state
            .lock()
            .map_err(|_| unavailable())?
            .attempts
            .values()
            .filter(|record| {
                record.workspace == gate.workspace_id()
                    && record.work.parent == gate.work().parent
                    && record.work.linear_endpoint == gate.work().linear_endpoint
                    && record.work.linear_workspace == gate.work().linear_workspace
                    && record.work.generation != gate.work().generation
                    && record.attempt.hold_reason.is_some()
                    && !record.attempt.resolved
            })
            .cloned()
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            return Ok(None);
        }
        let mut eligible = Vec::new();
        for record in candidates {
            if let Some(successor) = record.attempt.assigned_successor
                && !self.assigned_successor_work(&record, successor, gate.work())?
            {
                continue;
            }
            eligible.push(record);
        }
        let [record] = eligible.as_slice() else {
            return Err(Error::new(
                "release_hold",
                "corrected preparation requires one unambiguous known predecessor hold",
            ));
        };
        if !record.attempt.plan.resources.is_subset(&resources)
            || record
                .attempt
                .plan
                .resources
                .iter()
                .filter(|r| matches!(r, domain::ResourceKey::Target { .. }))
                .collect::<BTreeSet<_>>()
                != resources
                    .iter()
                    .filter(|r| matches!(r, domain::ResourceKey::Target { .. }))
                    .collect::<BTreeSet<_>>()
            || record.work.generation.scope != gate.work().generation.scope
            || configured.linear_workspace_id != record.work.linear_workspace
        {
            return Err(Error::new(
                "scope_violation",
                "recovery cannot widen or replace held repository/target scope",
            ));
        }
        validate_known_owner(&self.runtime.control, record)?;
        let origin = workspace
            .coordinator
            .release_origin(
                &record.attempt.plan,
                record.parent_publication.id,
                record.acceptance_publication.id,
            )
            .await?;
        let mut effects = BTreeMap::new();
        for (id, state) in &record.attempt.steps {
            if matches!(state, StepState::NotStarted | StepState::Retired { .. }) {
                continue;
            }
            let effect = workspace
                .coordinator
                .recover_owned_release_step(
                    &record.attempt.plan,
                    id,
                    record.attempt.step_attempt(id)?,
                    &workspace.observer,
                )
                .await?;
            match (state, effect.outcome()) {
                (
                    StepState::Succeeded {
                        effect: expected, ..
                    },
                    ReleaseStepOutcome::Succeeded,
                ) if effect.effect() == Some(expected) => (),
                (
                    StepState::Failed { .. } | StepState::Cancelled { .. },
                    ReleaseStepOutcome::Failed(_),
                ) => (),
                _ => {
                    return Err(Error::new(
                        "reconciliation_required",
                        "predecessor step outcomes need exact current reconciliation",
                    ));
                }
            }
            effects.insert(id.clone(), Arc::new(effect));
        }
        let mut baselines = Vec::new();
        for contract in workspace
            .contracts
            .iter()
            .filter(|repository| contract.repositories().contains(&repository.repository))
        {
            baselines.push(workspace.observer.observe_recovery(contract).await?);
        }
        verify_known_baseline(&origin, &baselines, &effects)?;
        let cap = Arc::new(KnownPredecessorBaseline {
            record: record.clone(),
            record_digest: canonical_digest(1, record)?,
            gate: gate.clone(),
            origin,
            baselines,
            effects,
            records: self.records.clone(),
            control: self.runtime.control.clone(),
            key: self.runtime.services.key.clone(),
            observed_at: Utc::now(),
        });
        self.runtime.control.serialize_observation(|snapshot| {
            (self.current_gate())(gate)?;
            let work = gate.work_key()?;
            if snapshot
                .unresolved_operations
                .values()
                .any(|targets| targets.contains(&work))
            {
                return Err(Error::new(
                    "reconciliation_required",
                    "corrected baseline still overlaps unknown effects",
                ));
            }
            cap.revalidate()
        })?;
        Ok(Some(cap))
    }
}
fn verify_known_baseline(
    origin: &VerifiedReleaseOrigin,
    baselines: &[VerifiedRecoveryBaseline],
    effects: &BTreeMap<String, Arc<ObservedReleaseStep>>,
) -> Result<()> {
    let now = Utc::now();
    if now - origin.observed_at() > Duration::seconds(30)
        || origin.observed_at() > now + Duration::seconds(5)
    {
        return Err(Error::new(
            "stale_result",
            "original release publication observation expired",
        ));
    }
    let mut mains = origin
        .manifest()
        .baselines
        .iter()
        .map(|b| (b.repository.clone(), b.production.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut targets = origin
        .targets()
        .iter()
        .map(|t| {
            (
                (t.repository.clone(), t.target.clone()),
                (t.source.clone(), t.artifact_digest.clone()),
            )
        })
        .collect::<BTreeMap<_, _>>();
    for effect in effects.values() {
        if effect.plan_digest() != origin.plan().digest()?
            || now - effect.observed_at() > Duration::seconds(30)
            || effect.observed_at() > now + Duration::seconds(5)
        {
            return Err(Error::new(
                "stale_result",
                "predecessor effect observation expired",
            ));
        }
        match effect.effect() {
            Some(domain::ObservedEffect::Merge {
                repository, commit, ..
            }) => {
                mains.insert(repository.clone(), commit.clone());
            }
            Some(
                domain::ObservedEffect::Deployment {
                    repository,
                    target: domain::ResourceKey::Target { id, .. },
                    source_commit,
                    tree,
                    artifact_digest,
                    ..
                }
                | domain::ObservedEffect::Verification {
                    repository,
                    target: domain::ResourceKey::Target { id, .. },
                    source_commit,
                    tree,
                    artifact_digest,
                    ..
                },
            ) => {
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
            None if matches!(effect.outcome(), ReleaseStepOutcome::Failed(_)) => (),
            _ => {
                return Err(Error::new(
                    "reconciliation_required",
                    "predecessor effects are incomplete",
                ));
            }
        }
    }
    if baselines.len() != mains.len()
        || baselines
            .iter()
            .map(|b| b.repository())
            .collect::<BTreeSet<_>>()
            != mains.keys().collect()
    {
        return Err(Error::new(
            "scope_violation",
            "known baseline inventory differs from original plan",
        ));
    }
    for b in baselines {
        let observed = b
            .targets()
            .iter()
            .map(|t| {
                (
                    (t.repository.clone(), t.target.clone()),
                    (t.source.clone(), t.artifact_digest.clone()),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let expected = targets
            .iter()
            .filter(|((repo, _), _)| repo == b.repository())
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect::<BTreeMap<_, _>>();
        if mains.get(b.repository()) != Some(b.production())
            || observed != expected
            || now - b.observed_at() > Duration::seconds(30)
            || b.observed_at() > now + Duration::seconds(5)
        {
            return Err(Error::new(
                "production_baseline",
                "current production differs from complete known owned predecessor effects",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn known_predecessor_requires_actual_retired_admission_and_no_unknown_effects() {
        let (_root, fixture, _) = crate::runtime_delivery::tests::fixture_authorization(true).await;
        let control = &fixture.runtime.control;
        let lease = crate::runtime_delivery::tests::start_control(&fixture.runtime);
        let projection = fixture.runtime.observations.refresh().await.unwrap();
        let work = projection
            .parent(Uuid::from_u128(2), Uuid::from_u128(100))
            .unwrap()
            .work
            .clone()
            .unwrap();
        let repository = RepositoryId {
            hostname: "github.invalid".into(),
            id: "42".into(),
        };
        // Explicit retained-state fixture: this test validates process/effect
        // ownership only, not successful remote production or parent evidence.
        let plan = ReleasePlan {
            schema_version: 1,
            runtime_id: lease.runtime_id,
            operation_id: Uuid::new_v4(),
            parent_id: work.parent,
            linear_workspace: work.linear_workspace,
            generation: work.generation.clone(),
            evidence_token: canonical_digest(1, &"fixture token").unwrap(),
            acceptance_digest: canonical_digest(1, &"fixture acceptance").unwrap(),
            carried_acceptance: None,
            resources: BTreeSet::from([domain::ResourceKey::Repository {
                repository: repository.clone(),
            }]),
            dependencies: vec![],
            spacing_seconds: 0,
            deployment_free: BTreeSet::from([repository.clone()]),
            steps: vec![domain::ReleaseStep {
                id: "merge".into(),
                depends_on: BTreeSet::new(),
                kind: domain::StepKind::Merge {
                    repository,
                    pull_request: 17,
                    production_base: crate::git::Revision {
                        commit: "a".repeat(40),
                        tree: "b".repeat(40),
                    },
                    candidate: crate::git::Revision {
                        commit: "c".repeat(40),
                        tree: "d".repeat(40),
                    },
                    selected_delta_digests: vec![canonical_digest(1, &"fixture delta").unwrap()],
                },
            }],
        };
        let mut record = super::super::tests::record(plan.clone(), work.clone());
        record.attempt.steps.insert(
            "merge".into(),
            StepState::Failed {
                reason: "known fixture rejection".into(),
                remote_id: None,
                observed_at: Utc::now(),
            },
        );
        record.attempt.hold_reason = Some("known fixture rejection".into());
        record.local_retired = true;
        control
            .admit(
                &lease,
                plan.operation_id,
                issue(&record).unwrap().identity().unwrap(),
                plan.digest().unwrap(),
                || Ok(()),
            )
            .unwrap();
        assert_eq!(
            validate_known_owner(control, &record).unwrap_err().code,
            "reconciliation_required"
        );
        control
            .retire_local(
                plan.operation_id,
                json!({"fixture":"no local subprocess was started"}),
            )
            .unwrap();
        validate_known_owner(control, &record).unwrap();
        let mut unknown = record.clone();
        unknown.attempt.steps.insert(
            "merge".into(),
            StepState::SentUnknown {
                payload_digest: canonical_digest(1, &"unknown fixture effect").unwrap(),
            },
        );
        assert_eq!(
            validate_known_owner(control, &unknown).unwrap_err().code,
            "reconciliation_required"
        );
        let mut changed = record.clone();
        changed.attempt.plan.operation_id = Uuid::new_v4();
        assert!(validate_known_owner(control, &changed).is_err());
        record.local_retired = false;
        assert!(validate_known_owner(control, &record).is_err());
    }
}
