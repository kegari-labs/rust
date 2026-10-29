//! Restore completed workflow state through authentic retired runs and fresh
//! production readers. This never reconstructs a live task or owned child.
use super::*;

impl RuntimeDelivery {
    pub(super) async fn restore_completed_pipeline(
        &self,
        workspace: &WorkspaceDelivery,
        projection: &ParentProjection,
        gate: &FreshGate,
        contract: &ParentContract,
    ) -> Result<Option<ParentPipeline>> {
        let key = parent_key(gate.workspace_id(), gate.work())?;
        let digest = canonical_digest(1, &key)?;
        let leaf = format!("parent-pipeline-{}.json", &digest[7..]);
        let signed: kegari_secrets::Signed<ParentReference> = match self.records.read(&leaf) {
            Ok(signed) => signed,
            Err(error) if error.code == "record_missing" => return Ok(None),
            Err(error) => return Err(error),
        };
        self.runtime
            .services
            .key
            .verify("runtime_parent_pipeline", &signed)?;
        let mut reference = signed.payload;
        if reference.stage == ParentStage::Replacement {
            return self
                .restore_waiting_replacement_pipeline(
                    workspace, projection, gate, contract, reference,
                )
                .await
                .map(Some);
        }

        if reference.schema_version != 1
            || reference.runtime != self.runtime.control.snapshot()?.intent.runtime_id
            || reference.workspace != gate.workspace_id()
            || reference.work != *gate.work()
            || reference.contract_digest != contract.digest()?
            || reference.stage != ParentStage::Complete
            || reference.phase_operation.is_some()
            || reference.replacement_operation.is_some()
        {
            return Err(Error::new(
                "reconciliation_required",
                "retained parent needs compatible completed phase history; active or uncertain phases are preserved",
            ));
        }
        self.parent_generation_clear(&reference, false)?;
        let vector = reference
            .vector
            .as_deref()
            .ok_or_else(|| Error::new("record_corrupt", "completed parent vector is absent"))?;
        let mut retired = Vec::new();
        for (operation, phase) in [
            (reference.integrated, "parent_integrated_validation"),
            (reference.adequacy, "parent_integrated_review"),
            (reference.review, "parent_evidence_review"),
        ] {
            let operation = match operation {
                Some(operation) => operation,
                None if phase == "parent_integrated_review" => continue,
                None => {
                    return Err(Error::new(
                        "record_corrupt",
                        "completed parent phase identity is absent",
                    ));
                }
            };
            let proof = self.runtime.completed_parent_retirement(operation)?;
            if proof.origin().parent != *gate.work()
                || proof.origin().workspace != gate.workspace_id()
                || proof.origin().phase != phase
                || proof.origin().vector_digest() != vector
                || proof.run().phase.phase != phase
                || proof.run().workspace != gate.workspace_id()
            {
                return Err(Error::new(
                    "parent_retirement",
                    "completed phase seal differs from the original parent/vector",
                ));
            }
            retired.push(proof);
        }
        let outcomes = self.runtime.dependency_outcomes()?;
        let mut child_gates = BTreeMap::new();
        for child in &projection.required_children {
            child_gates.insert(
                *child,
                projection.fresh_gate(
                    Some(*child),
                    GatePurpose::NewOperation,
                    &outcomes,
                    Utc::now(),
                    Duration::seconds(30),
                )?,
            );
        }
        let publication = reference
            .publication
            .as_deref()
            .ok_or_else(|| Error::new("record_corrupt", "completed parent publication is absent"))?
            .parse()
            .map_err(|_| Error::new("record_corrupt", "completed parent publication is invalid"))?;
        let observed = self.runtime.observations.admission_snapshot()?;
        let predecessor = self.known_predecessor_baseline(&observed, gate).await?;
        let successor_observer = predecessor
            .as_ref()
            .map(|proof| workspace.observer.with_successor(proof.clone()))
            .transpose()?;
        let observer = successor_observer
            .as_ref()
            .unwrap_or(workspace.observer.as_ref());
        let (reader, owned) = self
            .readonly_coordinator_with_successor(workspace, gate, predecessor.as_deref())
            .await?;
        let restored = async {
            let mut integrations = Vec::new();
            if let Some(predecessor) = &predecessor {
                for baseline in predecessor.baselines() {
                    let production = baseline.production();
                    let observation = reader.observation(
                        gate.clone(),
                        RevisionBinding {
                            repository: baseline.repository().clone(),
                            head: production.commit.clone(),
                            base: production.commit.clone(),
                            tested_commit: production.commit.clone(),
                            tested_tree: production.tree.clone(),
                        },
                    )?;
                    integrations
                        .extend(reader.retained_successor_integrations(&observation, predecessor)?);
                }
            }
            let restored = reader
                .restore_historical_completed_parent(
                    ParentHydrationInputs {
                        contract,
                        observer,
                        child_gates: &child_gates,
                        workflow: &workspace.workflow,
                        production_integrations: &integrations,
                    },
                    publication,
                )
                .await?;
            Ok::<_, Error>((restored, integrations))
        }
        .await;
        let finished = owned.finish(
            json!({"parent_restore":true,"publication":publication,"verified":restored.is_ok()}),
        );
        let (restored, integrations) = match (restored, finished) {
            (Ok(restored), Ok(_)) => restored,
            (Err(error), Ok(_)) | (_, Err(error)) => return Err(error),
        };
        let slices: BTreeSet<_> = restored
            .slices
            .iter()
            .map(|slice| slice.manifest().operation_id)
            .collect();
        if restored.vector.digest() != vector
            || slices != reference.slices.iter().copied().collect()
            || slices.len() != reference.slices.len()
            || restored.integrated.has_discovered_validation() != reference.adequacy.is_some()
        {
            return Err(Error::new(
                "parent_retirement",
                "fresh parent evidence differs from its complete original phase inventory",
            ));
        }
        for proof in &retired {
            proof.verify(&self.runtime)?;
        }
        (self.current_gate())(gate)?;
        reference.error = None;
        let changed_baseline = restored
            .baselines
            .iter()
            .any(|baseline| !restored.vector.baselines().contains(baseline.baseline()));
        if reference.reassembly.is_some() && !changed_baseline {
            let current = self.runtime.current_delivery(&projection.key)?;
            let carry = current.published_carry().ok_or_else(|| Error::new(
                "parent_reassembly_required", "cold carried parent needs its actually published and freshly verified acceptance carry"))?;
            carry.verify_current_projection(&workspace.configuration, projection)?;
            if carry.parent().publication_digest()? != restored.parent.publication_digest()? {
                return Err(Error::new(
                    "acceptance_carry",
                    "restored parent differs from its exact current carried publication",
                ));
            }
        }
        let mut pipeline = ParentPipeline {
            reference,
            children: restored.children,
            child_authorities: child_gates,
            staged: Vec::new(),
            baselines: restored.baselines,
            integrations,
            slices: restored.slices,
            vector: Some(restored.vector.clone()),
            integrated: Some(restored.integrated.clone()),
            prepared_validation: None,
            adequacy_assignment: None,
            assignment: None,
            published: Some(restored.parent),
            phase: None,
            reassembly: None,
            carry_import: None,
        };
        if changed_baseline {
            // Historical candidates never enter current readiness. The existing
            // carry observer must explain the actual transition before any new
            // assignment can be installed; current validation runs anew.
            if !self
                .begin_parent_reassembly(workspace, projection, gate, &mut pipeline)
                .await?
            {
                return Err(Error::new(
                    "stale_result",
                    "changed production has no authenticated parent transition",
                ));
            }
            return Ok(Some(pipeline));
        }
        self.parents
            .integrated
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "integrated restoration lock failed"))?
            .insert(key, (restored.vector.clone(), restored.integrated.clone()));
        Ok(Some(pipeline))
    }
    async fn restore_waiting_replacement_pipeline(
        &self,
        workspace: &WorkspaceDelivery,
        projection: &ParentProjection,
        gate: &FreshGate,
        contract: &ParentContract,
        mut reference: ParentReference,
    ) -> Result<ParentPipeline> {
        let operation = reference.replacement_operation.ok_or_else(|| {
            Error::new(
                "record_corrupt",
                "retained replacement wait lacks its operation",
            )
        })?;
        if reference.schema_version != 1
            || reference.runtime != self.runtime.control.snapshot()?.intent.runtime_id
            || reference.workspace != gate.workspace_id()
            || reference.work != *gate.work()
            || reference.contract_digest != contract.digest()?
            || reference.phase_operation.is_some()
            || reference.vector.is_some()
            || reference.integrated.is_some()
            || reference.adequacy.is_some()
            || reference.review.is_some()
            || reference.publication.is_some()
            || reference.reassembly.is_some()
            || !reference.carry_imports.is_empty()
        {
            return Err(Error::new(
                "reconciliation_required",
                "replacement wait differs from its original pre-assembly state",
            ));
        }
        self.parent_generation_clear(&reference, false)?;
        let latest = self.runtime.latest_replacement_operation(operation)?;
        let work = self
            .runtime
            .retained_replacement_work(latest)?
            .ok_or_else(|| {
                Error::new(
                    "reconciliation_required",
                    "cold replacement wait needs its actually restored original assignment",
                )
            })?;
        if work.authority().workspace_id() != gate.workspace_id()
            || work.authority().work().parent != gate.work().parent
            || work.authority().work().generation != gate.work().generation
            || work.authority().work().linear_endpoint != gate.work().linear_endpoint
            || work.authority().work().linear_workspace != gate.work().linear_workspace
        {
            return Err(Error::new(
                "stale_generation",
                "replacement wait changed its original child generation",
            ));
        }
        let outcomes = self.runtime.dependency_outcomes()?;
        let child_gates = projection
            .required_children
            .iter()
            .map(|id| {
                Ok((
                    *id,
                    projection.fresh_gate(
                        Some(*id),
                        GatePurpose::NewOperation,
                        &outcomes,
                        Utc::now(),
                        Duration::seconds(30),
                    )?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        let current = self.runtime.current_delivery(&projection.key)?;
        let expected = current.verified_children().clone();
        let prepared_e2e = self
            .fresh_prepared_e2e_candidate(workspace, projection, gate.work(), contract.e2e_child())
            .await?;
        let observed = self.runtime.observations.admission_snapshot()?;
        let predecessor = self.known_predecessor_baseline(&observed, gate).await?;
        let successor_observer = predecessor
            .as_ref()
            .map(|proof| workspace.observer.with_successor(proof.clone()))
            .transpose()?;
        let observer = successor_observer
            .as_ref()
            .unwrap_or(workspace.observer.as_ref());
        let (reader, owned) = self
            .readonly_coordinator_with_successor(workspace, gate, predecessor.as_deref())
            .await?;
        let restored = async {
            let mut integrations = Vec::new();
            if let Some(predecessor) = &predecessor {
                for baseline in predecessor.baselines() {
                    let production = baseline.production();
                    let observation = reader.observation(
                        gate.clone(),
                        RevisionBinding {
                            repository: baseline.repository().clone(),
                            head: production.commit.clone(),
                            base: production.commit.clone(),
                            tested_commit: production.commit.clone(),
                            tested_tree: production.tree.clone(),
                        },
                    )?;
                    integrations
                        .extend(reader.retained_successor_integrations(&observation, predecessor)?);
                }
            }
            let restored = reader
                .restore_waiting_parent_inputs(
                    ParentHydrationInputs {
                        contract,
                        observer,
                        child_gates: &child_gates,
                        workflow: &workspace.workflow,
                        production_integrations: &integrations,
                    },
                    &expected,
                    &reference.slices,
                    prepared_e2e.as_ref(),
                )
                .await?;
            Ok::<_, Error>((restored, integrations))
        }
        .await;
        let closed = owned.finish(
            json!({"parent_replacement_wait_restore":operation,"verified":restored.is_ok()}),
        );
        let (restored, integrations) = match (restored, closed) {
            (Ok(restored), Ok(_)) => restored,
            (Err(error), _) | (_, Err(error)) => return Err(error),
        };
        let selected = restored
            .staged
            .iter()
            .filter(|entry| Some(entry.delta().manifest.child_id) == work.authority().work().child)
            .map(|entry| entry.delta().manifest.digest())
            .collect::<Result<Vec<_>>>()?;
        if selected != vec![work.original_digest()?] {
            return Err(Error::new(
                "replacement_mapping",
                "restored wait differs from its exact selected original contribution",
            ));
        }
        self.parent_generation_clear(&reference, false)?;
        (self.current_gate())(gate)?;
        reference.error = None;
        Ok(ParentPipeline {
            reference,
            children: restored.children,
            child_authorities: child_gates,
            staged: restored.staged,
            baselines: restored.baselines,
            integrations,
            slices: restored.slices,
            vector: None,
            integrated: None,
            prepared_validation: None,
            adequacy_assignment: None,
            assignment: None,
            published: None,
            phase: None,
            reassembly: None,
            carry_import: None,
        })
    }
}
