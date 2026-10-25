//! Same-result continuation uses current native publications plus complete
//! original signed execution/effect accounting, never generic readiness reuse.
use super::*;
pub(super) struct HistoricalResume {
    pub(super) parent: VerifiedParent,
    pub(super) acceptance: VerifiedAcceptance,
    pub(super) sources: Option<ParentSources>,
    pub(super) baselines: Vec<VerifiedRecoveryBaseline>,
}
impl RuntimeDelivery {
    pub(super) async fn historical_resume_sources(
        &self,
        record: &AttemptRecord,
        workspace: &WorkspaceDelivery,
        gate: &FreshGate,
        parent: &crate::reconcile::ParentProjection,
    ) -> Result<HistoricalResume> {
        Self::check_release_publications(&self.runtime, record, None)?;
        let outcomes = self.runtime.dependency_outcomes()?;
        let mut children = BTreeMap::new();
        for child in &parent.required_children {
            children.insert(
                *child,
                parent.fresh_gate(
                    Some(*child),
                    GatePurpose::NewOperation,
                    &outcomes,
                    Utc::now(),
                    Duration::seconds(30),
                )?,
            );
        }
        let (coordinator, authority) = self.readonly_coordinator(workspace, gate).await?;
        let result = async {
            let contract =
                coordinator.parent_contract(gate.clone(), parent, &workspace.workflow)?;
            let historical = coordinator
                .hydrate_historical_release_parent(
                    ParentHydrationInputs {
                        contract: &contract,
                        observer: &workspace.observer,
                        child_gates: &children,
                        workflow: &workspace.workflow,
                        production_integrations: &[],
                    },
                    record.parent_publication.id,
                    &record.attempt.plan.evidence_token,
                )
                .await?;
            let baselines = self
                .recovery_baselines(workspace, &record.attempt.plan)
                .await?;
            let mut effects = Vec::new();
            for (id, state) in &record.attempt.steps {
                if matches!(state, StepState::NotStarted | StepState::Retired { .. }) {
                    continue;
                }
                let observed = coordinator
                    .recover_owned_release_step(
                        &record.attempt.plan,
                        id,
                        record.attempt.step_attempt(id)?,
                        &workspace.observer,
                    )
                    .await?;
                let matches = match (state, observed.outcome()) {
                    (StepState::Succeeded { effect, .. }, ReleaseStepOutcome::Succeeded) => {
                        observed.effect() == Some(effect)
                    }
                    (
                        StepState::Failed { .. } | StepState::Cancelled { .. },
                        ReleaseStepOutcome::Failed(_),
                    ) => true,
                    _ => false,
                };
                if !matches {
                    return Err(Error::new(
                        "reconciliation_required",
                        "current original step observation differs from retained terminal outcome",
                    ));
                }
                effects.push(observed);
            }
            let recovered = coordinator.verify_release_history_progress(
                historical,
                &record.attempt.plan,
                baselines,
                &effects,
            )?;
            let revision = recovered
                .parent()
                .manifest()
                .candidates
                .first()
                .ok_or_else(|| {
                    Error::new(
                        "candidate_provenance",
                        "historical candidate inventory is empty",
                    )
                })?;
            let observation = coordinator.observation(
                gate.clone(),
                release_revision(&record.attempt.plan, &revision.repository)?,
            )?;
            let configuration = workspace
                .configuration
                .workspaces
                .values()
                .find(|w| w.id == record.workspace)
                .ok_or_else(|| Error::new("scope_violation", "release workspace disappeared"))?;
            let permitted = configuration
                .actors
                .acceptors
                .iter()
                .map(ToString::to_string)
                .collect();
            let acceptance = coordinator
                .hydrate_original_release_acceptance(
                    &observation,
                    recovered.parent(),
                    record.acceptance_publication.id,
                    &permitted,
                )
                .await?;
            if acceptance.digest() != record.attempt.plan.acceptance_digest {
                return Err(Error::new(
                    "stale_result",
                    "original acceptance publication changed",
                ));
            }
            let parts = recovered.into_runtime_parts();
            let mut current_baseline = Vec::new();
            for candidate in &parts.parent.manifest().candidates {
                if candidate.ordered_delta_digests.is_empty() {
                    let contract = workspace
                        .contracts
                        .iter()
                        .find(|c| c.repository == candidate.repository)
                        .ok_or_else(|| {
                            Error::new("scope_violation", "historical repository disappeared")
                        })?;
                    current_baseline.push(Arc::new(workspace.observer.observe(contract).await?));
                }
            }
            Self::check_release_publications(&self.runtime, record, None)?;
            Ok(HistoricalResume {
                parent: parts.parent.clone(),
                acceptance,
                sources: Some(ParentSources {
                    parent: parts.parent,
                    slices: parts.slices.into_iter().map(Arc::new).collect(),
                    children: parts.children,
                    baselines: current_baseline,
                    vector_digest: parts.vector_digest,
                    integrations: vec![],
                }),
                baselines: parts.baselines,
            })
        }
        .await;
        let completion = authority.finish(json!({"read_only":true,"release_history":record.attempt.plan.operation_id,"verified":result.is_ok()}));
        match (result, completion) {
            (Ok(result), Ok(_)) => Ok(result),
            (Err(error), Ok(_)) | (_, Err(error)) => Err(error),
        }
    }
}
