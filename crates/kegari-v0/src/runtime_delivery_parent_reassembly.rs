//! A completed parent is reopened only by an actual authenticated baseline
//! transition. Old model admissions/attempts remain immutable and count toward
//! the same finite phase budgets; historical retirement never invents a joined task.
use super::*;
use crate::runtime_delivery::carry_baseline::select_released_transitions;
type ParentGrantObserver = Arc<
    dyn Fn(&crate::evidence::WorkIdentity) -> Result<Vec<crate::decisions::VerifiedRetryGrant>>
        + Send
        + Sync,
>;

pub(super) struct Reassembly {
    pub(super) original: Arc<AcceptedParentOrigin>,
    pub(super) previous_parent: PublishedParent,
    pub(super) transition: Arc<VerifiedBaselineTransition>,
    pub(super) previous_integrated: ParentRunPredecessor,
    pub(super) previous_review: ParentRunPredecessor,
    previous_adequacy: Option<ParentRunPredecessor>,
    validation_prefix: Option<crate::validation::ValidationPrefix>,
    pub(super) previous_integrated_assignment: Option<DeliveryWorkerAssignment>,
    pub(super) previous_review_assignment: Option<DeliveryWorkerAssignment>,
    previous_adequacy_assignment: Option<DeliveryWorkerAssignment>,
    previous_retired: BTreeMap<String, Arc<RetiredParentReassembly>>,
    /// Persisted operation lives in ParentReference; this retained capability
    /// survives a bounded launch wait or reporting failure without re-minting.
    pub(super) retry: Option<DeliveryWorkerAssignment>,
    pub(super) source: Option<Arc<ParentReassemblySource>>,
    pub(super) integrated: Option<DeliveryWorkerAssignment>,
    pub(super) review: Option<DeliveryWorkerAssignment>,
    adequacy: Option<DeliveryWorkerAssignment>,
    pub(super) completed: Option<ReassembledParent>,
}
impl RuntimeDelivery {
    pub(super) async fn begin_parent_reassembly(
        &self,
        workspace: &WorkspaceDelivery,
        projection: &ParentProjection,
        gate: &FreshGate,
        parent: &mut ParentPipeline,
    ) -> Result<bool> {
        if parent.reference.stage != ParentStage::Complete {
            return Ok(false);
        }
        let published = required_parent(&parent.published, "published original parent")?;
        let contract =
            workspace
                .coordinator
                .parent_contract(gate.clone(), projection, &workspace.workflow)?;
        let selected = workspace
            .contracts
            .iter()
            .filter(|c| contract.repositories().contains(&c.repository))
            .cloned()
            .collect::<Vec<_>>();
        let mut baselines = Vec::new();
        for repository in &selected {
            baselines.push(Arc::new(workspace.observer.observe(repository).await?));
        }
        if baselines.iter().all(|b| {
            published
                .verified()
                .manifest()
                .baselines
                .contains(b.baseline())
        }) && baselines.len() == published.verified().manifest().baselines.len()
        {
            return Ok(false);
        }
        let discovered = parent.reference.adequacy.is_some()
            || parent
                .integrated
                .as_ref()
                .is_some_and(IntegratedEvidence::has_discovered_validation);
        let previous_parent = published.clone();
        self.parent_generation_clear(&parent.reference, parent.phase.is_some())?;
        let predecessor = |operation: Option<Uuid>| -> Result<ParentRunPredecessor> {
            let operation = operation.ok_or_else(|| {
                Error::new(
                    "reconciliation_required",
                    "parent has no actual prior phase identity",
                )
            })?;
            let completion = match self.runtime.retained_completion(operation)? {
                Some(completion) => ParentRunPredecessor::Joined(completion),
                None => ParentRunPredecessor::Retired(Arc::new(
                    self.runtime.completed_parent_retirement(operation)?,
                )),
            };
            let receipt = self.runtime.control.operation(operation)?;
            let run = crate::control::run_reference_from_receipt(&receipt)?.ok_or_else(|| {
                Error::new(
                    "reconciliation_required",
                    "parent predecessor is not an actual run",
                )
            })?;
            if run.attempt
                >= run
                    .max_attempts
                    .min(workspace.configuration.runtime.max_attempts_per_phase)
            {
                return Err(Error::new(
                    "attempts_exhausted",
                    "baseline movement cannot reset the original parent phase budget",
                ));
            }
            Ok(completion)
        };
        let previous_integrated = predecessor(parent.reference.integrated)?;
        let previous_review = predecessor(parent.reference.review)?;
        let previous_adequacy = if discovered {
            Some(predecessor(parent.reference.adequacy)?)
        } else {
            None
        };
        let validation_prefix = match &previous_integrated {
            ParentRunPredecessor::Joined(completion) => {
                crate::validation::ValidationPrefix::from_completion(completion)?
            }
            ParentRunPredecessor::Retired(_) => {
                required_parent(&parent.integrated, "original integrated history")?
                    .validation_prefix()?
            }
        };
        if let (ParentRunPredecessor::Retired(retired), Some(prefix)) =
            (&previous_integrated, &validation_prefix)
        {
            let prior = prefix
                .history()
                .last()
                .ok_or_else(|| {
                    Error::new(
                        "validation_lineage_pending",
                        "original archive history is empty",
                    )
                })?
                .current()?;
            if prior.binding.operation_id != retired.operation_id()
                || prior.instruction_digest != retired.run().instruction_digest
                || prior.binding.candidate_vector_digest.as_deref()
                    != Some(retired.origin().vector_digest())
            {
                return Err(Error::new(
                    "validation_lineage_pending",
                    "retired integrated worker differs from its original validation archive",
                ));
            }
        }
        if discovered != validation_prefix.is_some() {
            return Err(Error::new(
                "validation_lineage_pending",
                "parent lost its complete original discovered validation history",
            ));
        }
        let historical = if parent.reference.reassembly.is_some() && parent.reassembly.is_none() {
            Some(
                self.restore_retired_reassembly(workspace, &contract, gate, parent)
                    .await?,
            )
        } else {
            None
        };
        let original = if let Some((original, _)) = &historical {
            original.clone()
        } else if let Some(prior) = &parent.reassembly {
            Arc::new(
                workspace
                    .coordinator
                    .capture_parent_acceptance(
                        &contract,
                        prior.original.published(),
                        &workspace.configuration,
                    )
                    .await?,
            )
        } else {
            Arc::new(
                workspace
                    .coordinator
                    .capture_parent_acceptance(&contract, published, &workspace.configuration)
                    .await?,
            )
        };
        let completions = self.retained_carry_completions(gate.workspace_id())?;
        let selected_links =
            select_released_transitions(original.parent(), &baselines, &completions)?;
        let mut origins = Vec::new();
        for completion in completions {
            if let Some(repositories) = selected_links.get(&completion.terminal().plan.operation_id)
            {
                origins.push(
                    workspace
                        .coordinator
                        .completed_carry_origin(
                            gate,
                            completion,
                            &workspace.configuration,
                            repositories.clone(),
                        )
                        .await?,
                );
            }
        }
        let roots = workspace
            .coordinator
            .completed_carry_roots(&contract, &origins)?;
        let (reader, owned) = self
            .readonly_coordinator_with_sources(workspace, gate, None, roots)
            .await?;
        let missing = reader
            .missing_completed_carry_objects(&contract, &origins, &baselines, &workspace.observer)
            .await;
        match missing {
            Ok(Some(proof)) => {
                owned.finish(
                    json!({"carry_baseline":"exact_objects_missing","source":proof.digest()?}),
                )?;
                self.retain_carry_import(workspace, parent, original, proof)?;
                return Ok(true);
            }
            Err(error) => {
                owned.finish(json!({"carry_baseline":"blocked","error":error}))?;
                return Err(error);
            }
            Ok(None) => (),
        }
        let observed = reader
            .observe_completed_carry_integrations(
                &contract,
                &origins,
                &baselines,
                &workspace.observer,
            )
            .await;
        // The real metadata scope is always joined/retired before using its
        // result or returning an error; no detached source observer is retained.
        owned.finish(match &observed {
            Ok(_) => json!({"carry_baseline":"observed"}),
            Err(error) => json!({"carry_baseline":"blocked","error":error}),
        })?;
        let observed = observed?;
        let links = observed
            .iter()
            .map(|(completion, integration)| ReleasedCarryLink {
                completion,
                integration,
            })
            .collect::<Vec<_>>();
        let actual = baselines
            .iter()
            .map(|b| b.as_ref().clone())
            .collect::<Vec<_>>();
        let transition = Arc::new(VerifiedBaselineTransition::observe(
            original.parent(),
            &actual,
            &links,
            &selected,
            &workspace.configuration,
            gate.workspace_id(),
        )?);
        self.runtime.control.serialize_observation(|_| {
            (self.current_gate())(gate)?;
            transition.verify_current(&workspace.configuration, &actual)?;
            original.validate(&workspace.configuration)?;
            let previous = self
                .runtime
                .services
                .key
                .sign("runtime_parent_pipeline", &parent.reference)?;
            let archive = format!("parent-reassembly-prior-{}.json", &previous.digest()?[7..]);
            match self
                .records
                .read::<kegari_secrets::Signed<ParentReference>>(&archive)
            {
                Ok(prior) => {
                    self.runtime
                        .services
                        .key
                        .verify("runtime_parent_pipeline", &prior)?;
                    if prior.digest()? != previous.digest()? {
                        return Err(Error::new(
                            "operation_conflict",
                            "parent reassembly archive changed",
                        ));
                    }
                }
                Err(error) if error.code == "record_missing" => {
                    self.records.write(&archive, &previous)?
                }
                Err(error) => return Err(error),
            }
            parent.reference.reassembly = Some(transition.digest().into());
            parent.reference.stage = ParentStage::Assemble;
            parent.reference.phase_operation = None;
            parent.reference.slices.clear();
            parent.reference.vector = None;
            parent.reference.integrated = None;
            parent.reference.adequacy = None;
            parent.reference.review = None;
            parent.reference.publication = None;
            parent.reference.error = None;
            parent.baselines.clear();
            parent.integrations.clear();
            parent.slices.clear();
            parent.vector = None;
            parent.integrated = None;
            parent.prepared_validation = None;
            parent.adequacy_assignment = None;
            parent.assignment = None;
            parent.published = None;
            let previous_integrated_assignment = parent
                .reassembly
                .as_ref()
                .and_then(|r| r.integrated.clone());
            let previous_review_assignment =
                parent.reassembly.as_ref().and_then(|r| r.review.clone());
            let previous_adequacy_assignment =
                parent.reassembly.as_ref().and_then(|r| r.adequacy.clone());
            parent.reassembly = Some(Reassembly {
                original,
                previous_parent,
                transition,
                previous_integrated,
                previous_review,
                previous_adequacy,
                validation_prefix,
                previous_integrated_assignment,
                previous_review_assignment,
                previous_adequacy_assignment,
                previous_retired: historical.map(|(_, phases)| phases).unwrap_or_default(),
                retry: None,
                source: None,
                integrated: None,
                review: None,
                adequacy: None,
                completed: None,
            });
            self.parents
                .integrated
                .lock()
                .map_err(|_| Error::new("runtime_unavailable", "integrated registry unavailable"))?
                .remove(&parent_key(
                    parent.reference.workspace,
                    &parent.reference.work,
                )?);
            self.persist_parent(parent)
        })?;
        Ok(true)
    }
    /// Restore only completed marked predecessors, using the exact carry's
    /// original acceptance and protected retirement records. The old baseline
    /// is historical; a fresh transition is still required by the caller.
    async fn restore_retired_reassembly(
        &self,
        workspace: &WorkspaceDelivery,
        contract: &ParentContract,
        gate: &FreshGate,
        parent: &ParentPipeline,
    ) -> Result<(
        Arc<AcceptedParentOrigin>,
        BTreeMap<String, Arc<RetiredParentReassembly>>,
    )> {
        let published = required_parent(&parent.published, "historical carried publication")?;
        let subject = workspace
            .coordinator
            .retained_current_carry_subject(
                gate,
                published.verified().token(),
                self.records.clone(),
            )?
            .ok_or_else(|| {
                Error::new(
                    "parent_reassembly_required",
                    "marked parent has no exact completed carry record",
                )
            })?;
        if parent.reference.reassembly.as_deref() != Some(subject.reassembly_baseline_digest())
            || subject.current_evidence() != published.verified().manifest()
        {
            return Err(Error::new(
                "parent_reassembly_required",
                "retained parent changed its exact completed carry",
            ));
        }
        let receipts = self.runtime.control.operations()?;
        let observation = self.runtime.observations.admission_snapshot()?;
        let mut phases = BTreeMap::new();
        for (operation, phase) in [
            (parent.reference.integrated, "parent_integrated_validation"),
            (parent.reference.adequacy, "parent_integrated_review"),
            (parent.reference.review, "parent_evidence_review"),
        ] {
            let operation = match operation {
                Some(operation) => operation,
                None if phase == "parent_integrated_review" => continue,
                None => {
                    return Err(Error::new(
                        "parent_reassembly_required",
                        "completed carry omitted a required phase",
                    ));
                }
            };
            let retirement = Arc::new(self.runtime.completed_parent_retirement(operation)?);
            let phase_gate = match retirement.origin().worker.child {
                Some(child) => parent.child_authorities.get(&child).ok_or_else(|| {
                    Error::new(
                        "parent_reassembly_required",
                        "carried validation lost current child authority",
                    )
                })?,
                None => gate,
            };
            let grants = crate::runtime_workflow::current_retry_grants(
                &workspace.configuration,
                &observation,
                phase_gate.work(),
                &receipts,
                &self.runtime.services.key,
            )?;
            let proof = workspace
                .coordinator
                .retired_parent_reassembly(&subject, retirement, phase_gate, &grants)?;
            phases.insert(phase.into(), proof);
        }
        let (reader, owned) = self
            .readonly_coordinator_with_successor(workspace, gate, None)
            .await?;
        let origin = reader
            .reobserve_carry_reassembly_origin(
                &subject,
                ParentHydrationInputs {
                    contract,
                    observer: &workspace.observer,
                    child_gates: &parent.child_authorities,
                    workflow: &workspace.workflow,
                    production_integrations: &parent.integrations,
                },
                &workspace.configuration,
            )
            .await;
        let closed = owned.finish(
            json!({"retired_parent_reassembly":subject.digest()?,"verified":origin.is_ok()}),
        );
        let origin = match (origin, closed) {
            (Ok(origin), Ok(_)) => origin,
            (Err(error), Ok(_)) | (_, Err(error)) => return Err(error),
        };
        (self.current_gate())(gate)?;
        Ok((Arc::new(origin), phases))
    }

    pub(super) async fn bind_parent_reassembly_assignment(
        &self,
        workspace: &WorkspaceDelivery,
        parent: &mut ParentPipeline,
        operation: Uuid,
        mut assignment: DeliveryWorkerAssignment,
    ) -> Result<DeliveryWorkerAssignment> {
        let Some(reassembly) = &mut parent.reassembly else {
            return Ok(assignment);
        };
        if !matches!(
            assignment.phase(),
            "parent_integrated_validation" | "parent_integrated_review" | "parent_evidence_review"
        ) {
            return Err(Error::new(
                "validation_lineage_pending",
                "parent reassembly cannot replace a distinct independent adequacy phase with synthesis",
            ));
        }
        let source =
            required_parent(&reassembly.source, "closed reassembly vector source")?.clone();
        source.refresh_current(&workspace.configuration).await?;
        let predecessor = match assignment.phase() {
            "parent_integrated_validation" => {
                if let Some(prefix) = &reassembly.validation_prefix {
                    assignment.continue_validation(prefix.clone())?;
                }
                reassembly.previous_integrated.clone()
            }
            "parent_integrated_review" => required_parent(
                &reassembly.previous_adequacy,
                "original independent adequacy phase",
            )?
            .clone(),
            _ => reassembly.previous_review.clone(),
        };
        let receipts = self.runtime.control.operations()?;
        let grants = crate::runtime_workflow::current_retry_grants(
            &workspace.configuration,
            &self.runtime.observations.admission_snapshot()?,
            assignment.authority().work(),
            &receipts,
            &self.runtime.services.key,
        )?;
        let previous = match assignment.phase() {
            "parent_integrated_validation" => &reassembly.previous_integrated_assignment,
            "parent_integrated_review" => &reassembly.previous_adequacy_assignment,
            _ => &reassembly.previous_review_assignment,
        }
        .as_ref()
        .and_then(|a| a.parent_reassembly());
        let historical = reassembly.previous_retired.get(assignment.phase());
        let assignment = workspace.coordinator.parent_reassembly_assignment(
            operation,
            source,
            assignment,
            ParentReassemblyPredecessor {
                completion: predecessor,
                previous,
                historical,
            },
            &workspace.configuration,
            &grants,
        )?;
        match assignment.phase() {
            "parent_integrated_validation" => reassembly.integrated = Some(assignment.clone()),
            "parent_integrated_review" => reassembly.adequacy = Some(assignment.clone()),
            _ => reassembly.review = Some(assignment.clone()),
        }
        Ok(assignment)
    }
    pub(super) fn verify_reassembled_parent_result(
        &self,
        parent: &ParentPipeline,
        completion: &Arc<crate::worker_run::RunCompletion>,
        phase: &str,
    ) -> Result<()> {
        let Some(reassembly) = &parent.reassembly else {
            return Ok(());
        };
        let assignment = match phase {
            "parent_integrated_validation" => &reassembly.integrated,
            "parent_integrated_review" => &reassembly.adequacy,
            _ => &reassembly.review,
        };
        let proof = required_parent(assignment, "actual reassembly worker assignment")?
            .parent_reassembly()
            .ok_or_else(|| {
                Error::new(
                    "parent_reassembly_required",
                    "parent result lost its sealed cause",
                )
            })?;
        if self
            .runtime
            .retained_completion(completion.operation_id)?
            .as_ref()
            .is_none_or(|actual| !Arc::ptr_eq(actual, completion))
        {
            return Err(Error::new(
                "reconciliation_required",
                "parent result lacks actual joined identity",
            ));
        }
        let receipts = self.runtime.control.operations()?;
        proof.verify_grants(
            &receipts,
            &self
                .active_parent_grant_observer(required_parent(
                    &parent.phase,
                    "active parent result phase",
                )?)?
                .observe(proof.authority())?,
        )?;
        proof.verify_joined(
            &|id| self.runtime.retained_completion(id),
            &self.runtime.control.snapshot()?,
        )?;
        proof.verify_result(completion, &receipts)
    }
}

impl RuntimeDelivery {
    pub(super) fn seal_completed_parent_reassembly(
        &self,
        workspace: &WorkspaceDelivery,
        parent: &mut ParentPipeline,
        published: &PublishedParent,
    ) -> Result<()> {
        if parent.reassembly.is_none() {
            return Ok(());
        }
        let active_grants = self.active_parent_grant_observer(required_parent(
            &parent.phase,
            "active parent publication phase",
        )?)?;
        let reassembly = parent
            .reassembly
            .as_mut()
            .expect("checked retained reassembly");
        let source = required_parent(&reassembly.source, "parent reassembly source")?.clone();
        let actual = |operation: Option<Uuid>| -> Result<Arc<crate::worker_run::RunCompletion>> {
            self.runtime
                .retained_completion(operation.ok_or_else(|| {
                    Error::new("reconciliation_required", "parent worker operation missing")
                })?)?
                .ok_or_else(|| {
                    Error::new(
                        "reconciliation_required",
                        "actual completed parent worker missing",
                    )
                })
        };
        let integrated = actual(parent.reference.integrated)?;
        let review = actual(parent.reference.review)?;
        let integrated_proof =
            required_parent(&reassembly.integrated, "integrated reassembly assignment")?
                .parent_reassembly()
                .ok_or_else(|| {
                    Error::new(
                        "parent_reassembly_required",
                        "integrated assignment lost its source",
                    )
                })?;
        let review_proof = required_parent(&reassembly.review, "review reassembly assignment")?
            .parent_reassembly()
            .ok_or_else(|| {
                Error::new(
                    "parent_reassembly_required",
                    "review assignment lost its source",
                )
            })?;
        let adequacy = if reassembly.previous_adequacy.is_some() {
            Some(actual(parent.reference.adequacy)?)
        } else {
            None
        };
        let adequacy_proof = reassembly
            .adequacy
            .as_ref()
            .and_then(|assignment| assignment.parent_reassembly());
        let adequacy_pair = match (adequacy_proof, adequacy.as_ref()) {
            (Some(proof), Some(result)) => Some((proof, result)),
            (None, None) => None,
            _ => {
                return Err(Error::new(
                    "parent_reassembly_required",
                    "independent adequacy lost its actual reassembly result",
                ));
            }
        };
        reassembly.completed = Some(workspace.coordinator.completed_parent_reassembly(
            source,
            published.clone(),
            CompletedParentReassemblyPhases {
                integrated: (integrated_proof, &integrated),
                review: (review_proof, &review),
                adequacy: adequacy_pair,
            },
            &workspace.configuration,
            &|id| self.runtime.retained_completion(id),
            (self.parent_grant_observer(), active_grants),
        )?);
        Ok(())
    }
}

impl RuntimeDelivery {
    pub(in crate::runtime_delivery) fn parent_grant_observer(&self) -> ParentGrantObserver {
        let runtime = self.runtime.clone();
        Arc::new(move |work| {
            let config = runtime.configuration.snapshot()?;
            crate::runtime_workflow::current_retry_grants(
                &config,
                &runtime.observations.admission_snapshot()?,
                work,
                &runtime.control.operations()?,
                &runtime.services.key,
            )
        })
    }
    /// Handles only a retained marked parent worker with an actual known failed
    /// exit. New operation identity is persisted before launch and never used
    /// as authority in the absence of the exact new grant/source capability.
    pub(super) async fn retry_parent_reassembly_worker(
        &self,
        lease: &Lease,
        workspace: &WorkspaceDelivery,
        parent: &mut ParentPipeline,
    ) -> Result<bool> {
        let phase = match parent.reference.stage {
            ParentStage::Integrated => "parent_integrated_validation",
            ParentStage::Adequacy => "parent_integrated_review",
            ParentStage::Publish => "parent_evidence_review",
            _ => return Ok(false),
        };
        let Some(reassembly) = &parent.reassembly else {
            return Ok(false);
        };
        if let Some(assignment) = reassembly.retry.clone() {
            let operation = assignment
                .parent_reassembly()
                .ok_or_else(|| {
                    Error::new(
                        "parent_reassembly_required",
                        "pending parent retry lost exact cause",
                    )
                })?
                .reference()
                .operation;
            if !self.worker_launch_admitted(operation)? {
                assignment
                    .parent_reassembly()
                    .unwrap()
                    .refresh_current(&workspace.configuration)
                    .await?;
                self.launch_assignment(lease, operation, assignment.clone())
                    .await?;
            }
            let reassembly = parent.reassembly.as_mut().unwrap();
            match phase {
                "parent_integrated_validation" => reassembly.integrated = Some(assignment),
                "parent_integrated_review" => reassembly.adequacy = Some(assignment),
                _ => reassembly.review = Some(assignment),
            }
            reassembly.retry = None;
            parent.reference.error = None;
            self.persist_parent(parent)?;
            return Ok(true);
        }
        let operation = match phase {
            "parent_integrated_validation" => parent.reference.integrated,
            "parent_integrated_review" => parent.reference.adequacy,
            _ => parent.reference.review,
        }
        .ok_or_else(|| {
            Error::new(
                "reconciliation_required",
                "marked parent worker has no operation",
            )
        })?;
        let Some(completion) = self.runtime.retained_completion(operation)? else {
            return Ok(false);
        };
        if !matches!(completion.outcome, crate::worker_run::RunOutcome::Failed) {
            return Ok(false);
        }
        let assignment = match phase {
            "parent_integrated_validation" => &reassembly.integrated,
            "parent_integrated_review" => &reassembly.adequacy,
            _ => &reassembly.review,
        };
        let proof = required_parent(assignment, "actual failed parent assignment")?
            .parent_reassembly()
            .ok_or_else(|| {
                Error::new(
                    "parent_reassembly_required",
                    "failed parent worker has no marked source",
                )
            })?;
        let grants = (self.parent_grant_observer())(proof.authority().work())?;
        let matching = grants
            .iter()
            .filter(|grant| grant.phase() == proof.phase())
            .collect::<Vec<_>>();
        let grant = match matching.as_slice() {
            [] if proof.phase().retry_grant.is_none() => return Ok(false),
            [] => None,
            [grant] => Some(*grant),
            _ => {
                return Err(Error::new(
                    "parent_reassembly",
                    "failed parent phase has forked grants",
                ));
            }
        };
        proof.refresh_current(&workspace.configuration).await?;
        proof.verify_joined(
            &|id| self.runtime.retained_completion(id),
            &self.runtime.control.snapshot()?,
        )?;
        let actual_grants = (self.parent_grant_observer())(proof.authority().work())?;
        let next = Uuid::new_v4();
        let assignment = if let Some(grant) = grant {
            crate::runtime_workflow::require_current_grant(grant, &actual_grants)?;
            workspace.coordinator.retry_parent_reassembly(
                next,
                proof,
                completion,
                grant,
                &workspace.configuration,
                &actual_grants,
            )?
        } else {
            workspace.coordinator.continue_parent_reassembly(
                next,
                proof,
                completion,
                &workspace.configuration,
                &actual_grants,
            )?
        };
        parent.reassembly.as_mut().unwrap().retry = Some(assignment.clone());
        match phase {
            "parent_integrated_validation" => parent.reference.integrated = Some(next),
            "parent_integrated_review" => parent.reference.adequacy = Some(next),
            _ => parent.reference.review = Some(next),
        }
        self.persist_parent(parent)?;
        self.launch_assignment(lease, next, assignment.clone())
            .await?;
        let reassembly = parent.reassembly.as_mut().unwrap();
        match phase {
            "parent_integrated_validation" => reassembly.integrated = Some(assignment),
            "parent_integrated_review" => reassembly.adequacy = Some(assignment),
            _ => reassembly.review = Some(assignment),
        }
        reassembly.retry = None;
        parent.reference.error = None;
        self.persist_parent(parent)?;
        Ok(true)
    }
}
