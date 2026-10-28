//! A corrected preparation may use a known held predecessor's exact observed
//! baseline. This capability grants neither release eligibility nor a permit.
use super::*;
use crate::deployment::VerifiedRecoveryBaseline;
use kegari_digest::canonical_digest;

#[derive(Clone)]
pub struct KnownPredecessorBaseline {
    record: AttemptRecord,
    record_digest: String,
    gate: FreshGate,
    origin: VerifiedReleaseOrigin,
    baselines: Vec<VerifiedRecoveryBaseline>,
    effects: BTreeMap<String, Arc<ObservedReleaseStep>>,
    lineage: super::lineage::VerifiedReleaseLineage,
    pub(super) records: Arc<kegari_storage::PrivateDir>,
    pub(super) control: Arc<crate::control::ControlHost>,
    pub(super) key: Arc<evidence::AttestationKey>,
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
    pub(super) fn lineage(&self) -> &super::lineage::VerifiedReleaseLineage {
        &self.lineage
    }
    pub(super) fn record_digest(&self) -> &str {
        &self.record_digest
    }
    pub(crate) fn integration_sources(
        &self,
    ) -> impl Iterator<Item = (&VerifiedReleaseOrigin, &ObservedReleaseStep)> {
        self.lineage.merges()
    }
    pub(crate) fn integration_origin(
        &self,
        effect: &ObservedReleaseStep,
    ) -> Result<&VerifiedReleaseOrigin> {
        self.lineage.origin(effect)
    }
    pub(crate) fn require_origin(&self, origin: &VerifiedReleaseOrigin) -> Result<()> {
        self.revalidate_lineage()?;
        self.lineage.require_origin(origin)
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
        self.revalidate_lineage()
    }
    /// Immutable historical provenance for an already admitted finite read.
    /// Callers must separately retain the actual permit/current authority and
    /// reobserve exact provider source identities before returning an import.
    pub(crate) fn revalidate_lineage(&self) -> Result<()> {
        let stored: Signed<AttemptRecord> = self.records.read(&attempt_leaf(self.predecessor()))?;
        self.key.verify("runtime_release_attempt", &stored)?;
        if canonical_digest(1, &stored.payload)? != self.record_digest {
            return Err(Error::new(
                "stale_result",
                "predecessor delivery state changed",
            ));
        }
        validate_known_owner(&self.control, &self.record)?;
        self.lineage
            .revalidate(&self.records, &self.control, &self.key)?;
        Ok(())
    }
    /// Scope match for historical reads after the caller has checked the
    /// admitted gate against current observations. This grants no freshness.
    pub(crate) fn require_historical_gate(&self, gate: &FreshGate) -> Result<()> {
        self.revalidate_lineage()?;
        let original = &self.gate;
        if gate.workspace_id() != original.workspace_id()
            || gate.work() != original.work()
            || gate.config_revision() != original.config_revision()
            || gate.workflow_digest() != original.workflow_digest()
            || gate.correction_watermark() != original.correction_watermark()
            || gate.profile() != original.profile()
            || gate.child_scope_digest() != original.child_scope_digest()
            || gate.required_children() != original.required_children()
            || gate.dependency_digest() != original.dependency_digest()
            || gate.purpose() != original.purpose()
            || gate.purpose() == GatePurpose::Coordination
        {
            return Err(Error::new(
                "scope_violation",
                "historical read differs from observed successor authority",
            ));
        }
        Ok(())
    }
    pub(crate) fn require_historical_effect(&self, effect: &ObservedReleaseStep) -> Result<()> {
        self.revalidate_lineage()?;
        self.lineage.origin(effect)?;
        Ok(())
    }
    pub(crate) fn require_effect(
        &self,
        effect: &ObservedReleaseStep,
        gate: &FreshGate,
    ) -> Result<()> {
        self.revalidate()?;
        // `gate` belongs to the immutable admitted operation. Freshness is
        // supplied by this newly observed capability, never by renewing that
        // operation's historical timestamp.
        gate.compare_current(&self.gate, Utc::now(), Duration::seconds(30))?;
        self.lineage.origin(effect)?;
        Ok(())
    }
}
pub(super) fn validate_known_owner(
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
        let parent = projection.parent(gate.workspace_id(), gate.work().parent)?;
        self.known_predecessor_baseline_inner(&workspace, parent, gate, None)
            .await
    }
    pub(in crate::runtime_delivery) async fn active_predecessor_baseline(
        &self,
        phase: &super::super::child::ActivePhase,
    ) -> Result<Option<Arc<KnownPredecessorBaseline>>> {
        let admitted = phase.admission.fresh_gate();
        self.runtime.control.validate_permit(&phase.permit, || {
            self.runtime
                .registration
                .verify_active_owner(&phase.permit.lease)?;
            (self.retained_gate())(admitted)
        })?;
        let outcomes = self.runtime.dependency_outcomes()?;
        let parent = self
            .runtime
            .observations
            .active_parent(admitted, &outcomes)?;
        let gate =
            parent.retain_active_gate(admitted, &outcomes, Utc::now(), Duration::seconds(30))?;
        self.known_predecessor_baseline_inner(&phase.workspace, &parent, &gate, Some(phase))
            .await
    }
    async fn known_predecessor_baseline_inner(
        &self,
        workspace: &WorkspaceDelivery,
        parent: &ParentProjection,
        gate: &FreshGate,
        active: Option<&super::super::child::ActivePhase>,
    ) -> Result<Option<Arc<KnownPredecessorBaseline>>> {
        self.restore_releases()?;
        let configured = workspace
            .configuration
            .workspaces
            .values()
            .find(|w| w.id == gate.workspace_id())
            .ok_or_else(|| Error::new("scope_violation", "successor workspace disappeared"))?;
        let contract = if let Some(phase) = active {
            self.active_parent_contract(phase)?
        } else {
            workspace
                .coordinator
                .parent_contract(gate.clone(), parent, &workspace.workflow)?
        };
        let mut resources = BTreeSet::new();
        for repository in workspace
            .contracts
            .iter()
            .filter(|repository| contract.repositories().contains(&repository.repository))
        {
            resources.insert(domain::ResourceKey::Repository {
                repository: repository.repository.clone(),
            });
            if let kegari_config::Deployment::Steps { steps } =
                repository.configuration.deployment_policy()?
            {
                resources.extend(steps.iter().map(|step| domain::ResourceKey::Target {
                    provider: "github".into(),
                    id: step.target_id.clone(),
                }));
            }
        }
        let candidates = {
            let state = self.releases.state.lock().map_err(|_| unavailable())?;
            let consumed = state.completed_ancestors(gate.work(), &resources)?;
            state
                .attempts
                .values()
                .filter(|record| {
                    record.workspace == gate.workspace_id()
                        && !consumed.contains_key(&record.attempt.plan.operation_id)
                        && record.work.parent == gate.work().parent
                        && record.work.linear_endpoint == gate.work().linear_endpoint
                        && record.work.linear_workspace == gate.work().linear_workspace
                        && record.work.generation != gate.work().generation
                        && record.attempt.hold_reason.is_some()
                        && !record.attempt.resolved
                })
                .cloned()
                .collect::<Vec<_>>()
        };
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
        self.known_record_baseline_inner(workspace, gate, record, active)
            .await
            .map(Some)
    }
    pub(super) async fn known_record_baseline(
        &self,
        workspace: &WorkspaceDelivery,
        gate: &FreshGate,
        record: &AttemptRecord,
    ) -> Result<Arc<KnownPredecessorBaseline>> {
        self.known_record_baseline_inner(workspace, gate, record, None)
            .await
    }
    async fn known_record_baseline_inner(
        &self,
        workspace: &WorkspaceDelivery,
        gate: &FreshGate,
        record: &AttemptRecord,
        active: Option<&super::super::child::ActivePhase>,
    ) -> Result<Arc<KnownPredecessorBaseline>> {
        if record.workspace != gate.workspace_id()
            || issue(record)?.identity()? != gate.work_key()?
        {
            return Err(Error::new(
                "scope_violation",
                "historical baseline belongs to another current work item",
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
        let effects = self.observe_lineage_effects(workspace, record).await?;
        let lineage = self
            .release_lineage(workspace, record, origin.clone(), effects.clone())
            .await?;
        let mut baselines = Vec::new();
        // The accepted manifest contains the complete parent repository vector.
        // Unchanged deployment-free repositories intentionally have no release
        // resource, but their baseline remains part of historical provenance.
        let repositories = recovery_repositories(&origin.manifest().baselines)?;
        for repository in repositories {
            let contract = workspace
                .contracts
                .iter()
                .find(|contract| contract.repository == repository)
                .ok_or_else(|| {
                    Error::new(
                        "scope_violation",
                        "historical baseline repository is no longer configured",
                    )
                })?;
            baselines.push(workspace.observer.observe_recovery(contract).await?);
        }
        verify_known_baseline(&origin, &baselines, &effects)?;
        let outcomes = self.runtime.dependency_outcomes()?;
        let observed_gate = if active.is_some() {
            self.runtime.observations.active_gate(gate, &outcomes)?
        } else {
            self.runtime
                .observations
                .admission_snapshot()?
                .parent(gate.workspace_id(), gate.work().parent)?
                .fresh_gate(
                    gate.work().child,
                    gate.purpose(),
                    &outcomes,
                    Utc::now(),
                    Duration::seconds(30),
                )?
        };
        gate.compare_current(&observed_gate, Utc::now(), Duration::seconds(30))?;
        let cap = Arc::new(KnownPredecessorBaseline {
            record: record.clone(),
            record_digest: canonical_digest(1, record)?,
            gate: observed_gate,
            origin,
            baselines,
            effects,
            lineage,
            records: self.records.clone(),
            control: self.runtime.control.clone(),
            key: self.runtime.services.key.clone(),
            observed_at: Utc::now(),
        });
        self.runtime.control.serialize_observation(|snapshot| {
            if let Some(phase) = active {
                self.runtime
                    .registration
                    .verify_active_owner(&phase.permit.lease)?;
                if snapshot
                    .fenced_operations
                    .contains(&phase.permit.operation_id)
                    || !snapshot.active_operations.iter().any(|actual| {
                        actual.operation_id == phase.permit.operation_id
                            && actual.lease == phase.permit.lease
                            && actual.scope_digest == phase.permit.scope_digest
                            && actual.work_key == phase.permit.work_key
                            && actual.control_generation == phase.permit.control_generation
                            && actual.registered_at == phase.permit.registered_at
                    })
                {
                    return Err(Error::new(
                        "access_denied",
                        "retained baseline admission is absent or fenced",
                    ));
                }
                (self.retained_gate())(gate)?;
            } else {
                (self.current_gate())(gate)?;
            }
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
        Ok(cap)
    }
}
pub(super) fn known_effect_state(
    origin: &VerifiedReleaseOrigin,
    effects: &BTreeMap<String, Arc<ObservedReleaseStep>>,
) -> Result<KnownState> {
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
    Ok((mains, targets))
}
type KnownState = (
    BTreeMap<RepositoryId, crate::git::Revision>,
    BTreeMap<(RepositoryId, String), (crate::git::Revision, String)>,
);
pub(super) fn verify_known_baseline(
    origin: &VerifiedReleaseOrigin,
    baselines: &[VerifiedRecoveryBaseline],
    effects: &BTreeMap<String, Arc<ObservedReleaseStep>>,
) -> Result<()> {
    let now = Utc::now();
    let (mains, targets) = known_effect_state(origin, effects)?;
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

fn recovery_repositories(
    baselines: &[evidence::ProductionBaseline],
) -> Result<BTreeSet<RepositoryId>> {
    let repositories = baselines
        .iter()
        .map(|baseline| baseline.repository.clone())
        .collect::<BTreeSet<_>>();
    if repositories.is_empty() || repositories.len() != baselines.len() {
        return Err(Error::new(
            "scope_violation",
            "historical repository baseline inventory is empty or duplicated",
        ));
    }
    Ok(repositories)
}
#[cfg(test)]
mod inventory_tests {
    use super::*;
    #[test]
    fn recovery_keeps_unchanged_deployment_free_repository_outside_release_resources() {
        let make = |id: &str| evidence::ProductionBaseline {
            repository: RepositoryId {
                hostname: "github.com".into(),
                id: id.into(),
            },
            production: crate::git::Revision {
                commit: "1".repeat(40),
                tree: "2".repeat(40),
            },
            deployed_revision: None,
            deployed_artifact: None,
            target_revisions: BTreeMap::new(),
            target_artifacts: BTreeMap::new(),
            deployment_free: true,
            successor_recovery_operation: None,
        };
        let changed = make("42");
        let unchanged = make("43");
        let effect_resources = BTreeSet::from([domain::ResourceKey::Repository {
            repository: changed.repository.clone(),
        }]);
        let complete = recovery_repositories(&[changed.clone(), unchanged.clone()]).unwrap();
        assert_eq!(
            complete,
            [changed.repository.clone(), unchanged.repository.clone()].into()
        );
        assert!(
            !effect_resources.contains(&domain::ResourceKey::Repository {
                repository: unchanged.repository.clone()
            })
        );
        assert!(recovery_repositories(&[changed.clone(), changed]).is_err());
        assert!(recovery_repositories(&[]).is_err());
    }
}
