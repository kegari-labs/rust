//! Cold E2E preparation is a candidate input, never a completed child. Actual
//! publication still waits for freshly verified integrated responsibility.
use super::*;

pub(super) struct PreparedState {
    candidate: PreparedChildCandidate,
    source: PathBuf,
    root: crate::git::VerifiedHostReadRoot,
    phase: Option<ActivePhase>,
    published: Option<PublishedChild>,
    completed: bool,
}
impl RuntimeDelivery {
    pub(super) async fn checkpoint_e2e_candidate(
        &self,
        lease: &Lease,
        workspace: &WorkspaceDelivery,
        gate: &FreshGate,
        child: &mut ChildPipeline,
        phase: DeliveryPhase,
    ) -> Result<()> {
        if let Some(existing) = workspace
            .coordinator
            .read_prepared_child(child.reference.operation)?
        {
            if existing.work() != gate.work()
                || existing.revision()
                    != required(&child.source, "prepared source seal")?.revision()
            {
                return Err(Error::new(
                    "stale_result",
                    "prepared E2E checkpoint changed its actual source or scope",
                ));
            }
            if child.phase.is_some() {
                self.finish_child_phase(child, Stage::Publish)?;
            }
            return Ok(());
        }
        self.phase(workspace, gate, lease, child, phase).await?;
        let active = child.phase.as_ref().ok_or_else(|| {
            Error::new("ownership_conflict", "E2E preparation admission is absent")
        })?;
        let input = match &child.claim.claim().disposition {
            ChildClaimDisposition::Changed => {
                PreparedChildCompletion::Changed(ChangedChildCompletion {
                    workflow: &workspace.workflow,
                    criteria: &child.claim.claim().fulfilled_criteria,
                    local: required(&child.local, "prepared local checks")?,
                    hosted: required(&child.hosted, "prepared hosted checks")?,
                    review: required(&child.review, "prepared review")?,
                    staged: required(&child.staged, "prepared staged contribution")?,
                })
            }
            ChildClaimDisposition::NoChange { reason } => {
                PreparedChildCompletion::NoChange(NoChangeCompletion {
                    workflow: &workspace.workflow,
                    criteria: &child.claim.claim().fulfilled_criteria,
                    reason,
                    local: required(&child.local, "prepared local checks")?,
                    exact: required(&child.exact, "prepared exact validation")?,
                    review: required(&child.review, "prepared review")?,
                })
            }
            ChildClaimDisposition::Blocked { .. } => {
                return Err(Error::new(
                    "worker_blocked",
                    "blocked child cannot prepare integrated readiness",
                ));
            }
        };
        active
            .coordinator
            .checkpoint_prepared_child(
                &active.admission,
                &child.claim,
                &child.completion,
                required(&child.source, "prepared source seal")?,
                [
                    child.reference.validation.ok_or_else(|| {
                        Error::new("record_corrupt", "prepared validation identity absent")
                    })?,
                    child.reference.review.ok_or_else(|| {
                        Error::new("record_corrupt", "prepared review identity absent")
                    })?,
                ],
                input,
            )
            .await?;
        self.finish_child_phase(child, Stage::Publish)
    }
    pub(super) async fn restore_prepared_children(
        &self,
        projection: &RuntimeProjection,
    ) -> Result<()> {
        for leaf in self.records.records_bounded(65_536)? {
            if !leaf.starts_with("child-pipeline-") || !leaf.ends_with(".json") {
                continue;
            }
            let signed: kegari_secrets::Signed<ChildReference> = self.records.read(&leaf)?;
            self.runtime
                .services
                .key
                .verify("runtime_child_pipeline", &signed)?;
            let reference = signed.payload;
            if reference.stage != Stage::Publish || reference.phase_operation.is_some() {
                continue;
            }
            if self
                .children
                .pipelines
                .lock()
                .map_err(|_| Error::new("runtime_unavailable", "child registry lock failed"))?
                .contains_key(&reference.operation)
                || self
                    .children
                    .prepared
                    .lock()
                    .map_err(|_| {
                        Error::new("runtime_unavailable", "prepared child registry lock failed")
                    })?
                    .contains_key(&reference.operation)
            {
                continue;
            }
            let receipt = self.runtime.control.operation(reference.operation)?;
            let Some(run) = crate::control::run_reference_from_receipt(&receipt)? else {
                continue;
            };
            let parent = projection.parent(run.workspace, reference.work.parent)?;
            if parent
                .metadata
                .as_ref()
                .is_none_or(|m| Some(m.e2e_child) != reference.work.child)
            {
                continue;
            }
            let result = async {
                let gate = parent.fresh_gate(
                    reference.work.child,
                    GatePurpose::NewOperation,
                    &self.runtime.dependency_outcomes()?,
                    Utc::now(),
                    Duration::seconds(30),
                )?;
                if gate.work() != &reference.work {
                    return Err(Error::new(
                        "stale_generation",
                        "prepared E2E generation changed",
                    ));
                }
                let workspace = self.workspace(projection, run.workspace)?;
                let Some(checkpoint) = workspace
                    .coordinator
                    .read_prepared_child(reference.operation)?
                else {
                    return Ok(());
                };
                let current = self.runtime.current_delivery(&parent.key)?;
                if current
                    .verified_children()
                    .get(&reference.work.child.unwrap())
                    .is_some_and(|child| {
                        child.digest() == checkpoint.evidence_digest().as_deref().unwrap_or("")
                    })
                {
                    return Ok(());
                }
                let snapshot = self.runtime.control.snapshot()?;
                if snapshot
                    .active_operations
                    .iter()
                    .any(|p| p.work_key == run.phase.work)
                    || snapshot
                        .unresolved_operations
                        .values()
                        .any(|targets| targets.contains(&run.phase.work))
                    || snapshot
                        .pending_local_processes
                        .contains(&reference.operation)
                {
                    return Err(Error::new(
                        "operation_uncertain",
                        "prepared E2E still has an active or unresolved owner",
                    ));
                }
                let (binding, boundary, owned, workflow) = self
                    .runtime
                    .services
                    .housekeeping
                    .cold_execution_source(&workspace.configuration, reference.operation)?;
                if &binding != checkpoint.binding()
                    || owned.digest() != checkpoint.directory_digest()
                    || workflow.digest != workspace.workflow.digest
                {
                    return Err(Error::new(
                        "execution_ownership",
                        "prepared E2E source registration changed",
                    ));
                }
                let root = self
                    .runtime
                    .services
                    .housekeeping
                    .retained_execution_read_root(
                        &workspace.configuration,
                        &binding,
                        &boundary,
                        &owned,
                        &workflow,
                    )?;
                let source = boundary.plan().repository.clone();
                let (reader, read_owner) = self
                    .readonly_coordinator_with_sources(&workspace, &gate, None, vec![root.clone()])
                    .await?;
                let observation =
                    reader.observation(gate.clone(), checkpoint.revision().clone())?;
                let restored = reader
                    .restore_prepared_child(&observation, &workflow, checkpoint, &source)
                    .await;
                let closed = read_owner.finish(
                    json!({"prepared_e2e_restore":reference.operation,"verified":restored.is_ok()}),
                );
                let candidate = match (restored, closed) {
                    (Ok(candidate), Ok(_)) => candidate,
                    (Err(error), _) | (_, Err(error)) => return Err(error),
                };
                self.children
                    .prepared
                    .lock()
                    .map_err(|_| {
                        Error::new("runtime_unavailable", "prepared child registry lock failed")
                    })?
                    .insert(
                        reference.operation,
                        Arc::new(tokio::sync::Mutex::new(PreparedState {
                            candidate,
                            source,
                            root,
                            phase: None,
                            published: None,
                            completed: false,
                        })),
                    );
                Ok(())
            }
            .await;
            if let Err(error) = result {
                self.runtime.invalidate_delivery(&parent.key, error)?;
            }
        }
        Ok(())
    }
    pub(super) async fn prepared_e2e_candidate(
        &self,
        work: &crate::evidence::WorkIdentity,
        e2e: Uuid,
    ) -> Result<Option<PreparedChildCandidate>> {
        let retained: Vec<_> = self
            .children
            .prepared
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "prepared child registry lock failed"))?
            .values()
            .cloned()
            .collect();
        let mut found = None;
        for state in retained {
            let state = state.lock().await;
            let candidate = &state.candidate;
            let selected = candidate.checkpoint().work();
            if selected.child != Some(e2e)
                || selected.parent != work.parent
                || selected.generation != work.generation
                || selected.linear_endpoint != work.linear_endpoint
                || selected.linear_workspace != work.linear_workspace
            {
                continue;
            }
            if found.replace(candidate.clone()).is_some() {
                return Err(Error::new(
                    "ownership_conflict",
                    "multiple prepared E2E inputs claim one work generation",
                ));
            }
        }
        Ok(found)
    }
    pub(super) async fn prepared_child_publications(
        &self,
        work: &crate::evidence::WorkIdentity,
    ) -> Result<Vec<(PublishedChild, Option<StagedDelta>)>> {
        let retained: Vec<_> = self
            .children
            .prepared
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "prepared child registry lock failed"))?
            .values()
            .cloned()
            .collect();
        let mut results = Vec::new();
        for retained in retained {
            let state = retained.lock().await;
            let selected = state.candidate.checkpoint().work();
            if selected.parent == work.parent
                && selected.generation == work.generation
                && selected.linear_endpoint == work.linear_endpoint
                && selected.linear_workspace == work.linear_workspace
                && let Some(published) = &state.published
            {
                results.push((published.clone(), state.candidate.staged().cloned()));
            }
        }
        Ok(results)
    }
    pub(in crate::runtime_delivery) async fn fresh_prepared_e2e_candidate(
        &self,
        workspace: &WorkspaceDelivery,
        parent: &ParentProjection,
        work: &crate::evidence::WorkIdentity,
        e2e: Uuid,
    ) -> Result<Option<PreparedChildCandidate>> {
        let Some(selected) = self.prepared_e2e_candidate(work, e2e).await? else {
            return Ok(None);
        };
        let retained = self
            .children
            .prepared
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "prepared child registry lock failed"))?
            .get(&selected.checkpoint().operation())
            .cloned()
            .ok_or_else(|| Error::new("record_missing", "prepared E2E candidate disappeared"))?;
        let state = retained.lock().await;
        state.root.revalidate_filesystem()?;
        let gate = parent.fresh_gate(
            Some(e2e),
            GatePurpose::NewOperation,
            &self.runtime.dependency_outcomes()?,
            Utc::now(),
            Duration::seconds(30),
        )?;
        let (reader, owner) = self
            .readonly_coordinator_with_sources(workspace, &gate, None, vec![state.root.clone()])
            .await?;
        let observation = reader.observation(gate, selected.checkpoint().revision().clone())?;
        let checked = reader
            .restore_prepared_child(
                &observation,
                &workspace.workflow,
                selected.checkpoint().clone(),
                &state.source,
            )
            .await;
        let closed = owner.finish(json!({"prepared_e2e_candidate":selected.checkpoint().operation(),"verified":checked.is_ok()}));
        match (checked, closed) {
            (Ok(value), Ok(_)) => Ok(Some(value)),
            (Err(error), _) | (_, Err(error)) => Err(error),
        }
    }
    pub(super) async fn advance_prepared_children(
        &self,
        lease: Option<&Lease>,
        projection: &RuntimeProjection,
    ) -> Result<()> {
        let retained: Vec<_> = self
            .children
            .prepared
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "prepared child registry lock failed"))?
            .values()
            .cloned()
            .collect();
        for retained in retained {
            let mut state = retained.lock().await;
            if state.completed || (lease.is_none() && state.phase.is_none()) {
                continue;
            }
            let lease = state
                .phase
                .as_ref()
                .map(|phase| phase.permit.lease.clone())
                .or_else(|| lease.cloned())
                .ok_or_else(|| {
                    Error::new(
                        "runtime_unavailable",
                        "prepared phase lacks its actual lease",
                    )
                })?;
            let checkpoint = state.candidate.checkpoint().clone();
            let parent = projection.parent(checkpoint.workspace(), checkpoint.work().parent)?;
            let result = async {
                let outcomes = self.runtime.dependency_outcomes()?;
                let gate = match &state.phase {
                    Some(phase) => self
                        .runtime
                        .observations
                        .active_gate(phase.admission.fresh_gate(), &outcomes)?,
                    None => parent.fresh_gate(
                        checkpoint.work().child,
                        GatePurpose::NewOperation,
                        &outcomes,
                        Utc::now(),
                        Duration::seconds(30),
                    )?,
                };
                let workspace = match &state.phase {
                    Some(phase) => (*phase.workspace).clone(),
                    None => self.workspace(projection, checkpoint.workspace())?,
                };
                if !self
                    .integrated_responsibility_ready(&workspace, parent, &gate)
                    .await?
                {
                    if state.phase.is_some() {
                        return Err(Error::new(
                            "integrated_gate",
                            "prepared E2E publication lost its current integrated proof",
                        ));
                    }
                    return Ok(false);
                }
                state.root.revalidate_filesystem()?;
                if state.phase.is_none() {
                    let scope = DeliveryScope {
                        schema_version: 1,
                        runtime_id: lease.runtime_id,
                        phase: if state.candidate.staged().is_some() {
                            DeliveryPhase::Staging
                        } else {
                            DeliveryPhase::NoChange
                        },
                        work: gate.work().clone(),
                        repository: checkpoint.revision().repository.clone(),
                        revision: checkpoint.revision().clone(),
                        fresh_gate_digest: gate.digest()?,
                        contract_digest: checkpoint.contract_digest().into(),
                        candidate_vector_digest: None,
                    };
                    state.phase = Some(
                        self.admit_delivery_phase(
                            &workspace,
                            &gate,
                            &lease,
                            PhaseRequest {
                                operation: checkpoint.publication_operation(),
                                scope,
                            },
                        )
                        .await?,
                    );
                }
                let active = state.phase.as_ref().unwrap();
                let coordinator = active.coordinator.clone();
                let admission = active.admission.clone();
                // The actual delivery permit guards source revalidation. This
                // introduces no second metadata admission inside the live phase.
                coordinator
                    .restore_prepared_child(
                        &admission,
                        &workspace.workflow,
                        checkpoint.clone(),
                        &state.source,
                    )
                    .await?;
                if state.published.is_none() {
                    state.published = Some(
                        coordinator
                            .publish_prepared_child(
                                &admission,
                                &workspace.workflow,
                                &state.candidate,
                            )
                            .await?,
                    );
                }
                coordinator
                    .close_child(
                        &admission,
                        state.published.as_ref().unwrap(),
                        &workspace.configuration,
                    )
                    .await?;
                self.retire_delivery_phase(
                    state.phase.as_ref().unwrap(),
                    json!({"prepared_e2e_published":checkpoint.operation()}),
                )?;
                state.phase = None;
                let leaf = format!("child-pipeline-{}.json", checkpoint.operation());
                let signed: kegari_secrets::Signed<ChildReference> = self.records.read(&leaf)?;
                self.runtime
                    .services
                    .key
                    .verify("runtime_child_pipeline", &signed)?;
                let mut reference = signed.payload;
                if reference.operation != checkpoint.operation()
                    || reference.work != *checkpoint.work()
                    || reference.stage != Stage::Publish
                {
                    return Err(Error::new(
                        "record_corrupt",
                        "prepared child pipeline changed before closure",
                    ));
                }
                reference.stage = Stage::Complete;
                reference.error = None;
                self.records.write(
                    &leaf,
                    &self
                        .runtime
                        .services
                        .key
                        .sign("runtime_child_pipeline", &reference)?,
                )?;
                Ok(true)
            }
            .await;
            match result {
                Ok(true) => {
                    state.completed = true;
                }
                Ok(false) => (),
                Err(error) => {
                    if terminal_gate_failure(&error)
                        && let Some(phase) = &state.phase
                    {
                        self.retire_delivery_phase(
                            phase,
                            json!({"prepared_e2e_publication":"blocked","error":error}),
                        )?;
                        state.phase = None;
                    }
                    self.runtime.invalidate_delivery(&parent.key, error)?;
                }
            }
        }
        Ok(())
    }
}
