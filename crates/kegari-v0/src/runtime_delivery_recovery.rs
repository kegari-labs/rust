//! Explicit authenticated local recovery transitions. Provider observations are
//! prepared before control serialization; durable records never recreate permits.
use super::*;
use crate::{
    api::CommandName,
    control::Receipt,
    decisions::{AuthorizedDecision, AuthorizedOperationRecovery},
    deployment::VerifiedRecoveryBaseline,
    digest::canonical_digest,
};

impl RuntimeDelivery {
    fn recovery_record(
        &self,
        decision: &AuthorizedDecision,
        action: &str,
    ) -> Result<AttemptRecord> {
        if decision.request().command != CommandName::ReleaseRecover
            || decision.request().args["action"] != action
        {
            return Err(Error::new(
                "access_denied",
                "release recovery action differs from authenticated decision",
            ));
        }
        let operation = decision.request().args["operation"]
            .as_str()
            .and_then(|id| Uuid::parse_str(id).ok())
            .ok_or_else(|| Error::invalid("release recovery operation is invalid"))?;
        let record = self
            .releases
            .state
            .lock()
            .map_err(|_| unavailable())?
            .attempts
            .get(&operation)
            .cloned()
            .ok_or_else(|| {
                Error::new(
                    "operation_missing",
                    "operation is not an authenticated release attempt",
                )
            })?;
        if record.runtime != decision.request().runtime_id
            || record.workspace != decision.workspace()
            || issue(&record)? != *decision.parent()
        {
            return Err(Error::new(
                "scope_violation",
                "recovery operation belongs to another canonical parent or workspace",
            ));
        }
        Ok(record)
    }
    fn recovery_gate(
        &self,
        decision: &AuthorizedDecision,
        snapshot: &ControlSnapshot,
    ) -> Result<()> {
        self.runtime.registration.verify_control_owner()?;
        let configuration = self.runtime.configuration.snapshot()?;
        self.runtime
            .configuration
            .authority_fences()?
            .check_actor(decision.actor(), "recover")?;
        let projection = self.runtime.observations.admission_snapshot()?;
        decision.recheck(&configuration, &projection, snapshot)
    }
    /// Retire known unstarted work while preserving every actual/uncertain
    /// effect. This is an operator metadata action, never a rollback/cancel.
    pub async fn abandon_release(&self, decision: AuthorizedDecision) -> Result<Receipt> {
        let _serial = self.releases.progressing.lock().await;
        self.restore_releases()?;
        self.runtime.observations.refresh().await?;
        let original = self.recovery_record(&decision, "abandon")?;
        let admission = self
            .runtime
            .control
            .operation(original.attempt.plan.operation_id)?;
        let digest = original.attempt.plan.digest()?;
        let reason = decision.request().args["reason"]
            .as_str()
            .ok_or_else(|| Error::invalid("recovery reason missing"))?;
        self.runtime.control.record_release_recovery_transition(
            &decision,
            &admission,
            &digest,
            |snapshot| self.recovery_gate(&decision, snapshot),
            |receipt| {
                self.persist_recovery_transition(&original, receipt, "abandon", |record| {
                    record
                        .attempt
                        .abandon(&decision.actor().identity, reason, Utc::now(), |_| Ok(()))
                })
            },
        )
    }
    /// Clear only a known terminal, coherent failure hold after observing the
    /// entire configured repository/target vector. It does not record success.
    pub async fn resolve_release(&self, decision: AuthorizedDecision) -> Result<Receipt> {
        let _serial = self.releases.progressing.lock().await;
        self.restore_releases()?;
        let projection = self.runtime.observations.refresh().await?;
        let original = self.recovery_record(&decision, "resolve")?;
        if original.attempt.has_unknown() || !original.pending_receipts.is_empty() {
            return Err(Error::new(
                "reconciliation_required",
                "resolve requires every original effect receipt and remote action to be known",
            ));
        }
        let workspace = self.workspace(&projection, original.workspace)?;
        let baselines = self
            .recovery_baselines(&workspace, &original.attempt.plan)
            .await?;
        let baseline = reconciled_baseline(&baselines, &original.attempt.plan)?;
        let admission = self
            .runtime
            .control
            .operation(original.attempt.plan.operation_id)?;
        let digest = original.attempt.plan.digest()?;
        let reason = decision.request().args["reason"]
            .as_str()
            .ok_or_else(|| Error::invalid("recovery reason missing"))?;
        self.runtime.control.record_release_recovery_transition(
            &decision,
            &admission,
            &digest,
            |snapshot| {
                self.recovery_gate(&decision, snapshot)?;
                current_baselines(&baselines)
            },
            |receipt| {
                self.persist_recovery_transition(&original, receipt, "resolve", |record| {
                    record.attempt.resolve(
                        &decision.actor().identity,
                        reason,
                        &baseline,
                        Utc::now(),
                        |_| Ok(()),
                    )
                })
            },
        )
    }
    /// Continue the same accepted plan only after complete current evidence and
    /// a newly authenticated admission. Retained JSON never recreates a Permit.
    pub async fn resume_release(&self, decision: AuthorizedDecision) -> Result<Receipt> {
        let _serial = self.releases.progressing.lock().await;
        self.restore_releases()?;
        let projection = self.runtime.observations.refresh().await?;
        let original = self.recovery_record(&decision, "resume")?;
        if original.attempt.has_unknown()
            || !original.pending_receipts.is_empty()
            || original.attempt.abandoned
            || original.attempt.resolved
            || original.attempt.completed()
            || original.attempt.assigned_successor.is_some()
        {
            return Err(Error::new(
                "recovery_blocked",
                "resume requires known outcomes and an unfinished same-result plan",
            ));
        }
        if !original.local_retired {
            if !ready_for_local_retirement(&original) {
                return Err(Error::new(
                    "recovery_blocked",
                    "a healthy admitted release cannot be retired by a resume request",
                ));
            }
            self.retire_release(original.attempt.plan.operation_id, &original)?;
        }
        let original = self.recovery_record(&decision, "resume")?;
        let lease = self.runtime.control.active_lease()?;
        let workspace = self.workspace(&projection, original.workspace)?;
        let parent = projection.parent(original.workspace, original.work.parent)?;
        let gate = parent.fresh_gate(
            None,
            GatePurpose::NewOperation,
            &self.runtime.dependency_outcomes()?,
            Utc::now(),
            Duration::seconds(30),
        )?;
        if gate.work() != &original.work {
            return Err(Error::new(
                "stale_result",
                "resume cannot replace the original work generation",
            ));
        }
        let current = self.runtime.current_delivery(&parent.key)?;
        let mut historical =
            if current.verified_parent().is_none() || current.verified_acceptance().is_none() {
                Some(
                    self.historical_resume_sources(&original, &workspace, &gate, parent)
                        .await?,
                )
            } else {
                None
            };
        let verified_parent = historical
            .as_ref()
            .map(|h| &h.parent)
            .or_else(|| current.verified_parent())
            .ok_or_else(|| {
                Error::new(
                    "evidence_unavailable",
                    "resume needs the same authenticated parent",
                )
            })?
            .clone();
        let acceptance = historical
            .as_ref()
            .map(|h| &h.acceptance)
            .or_else(|| current.verified_acceptance())
            .ok_or_else(|| {
                Error::new(
                    "acceptance_required",
                    "resume needs the exact original acceptance",
                )
            })?
            .clone();
        if verified_parent.token() != original.attempt.plan.evidence_token
            || acceptance.digest() != original.attempt.plan.acceptance_digest
        {
            return Err(Error::new(
                "stale_result",
                "resume cannot replace the original accepted result",
            ));
        }
        if historical.is_none() {
            current.require_release_selection(
                &original.attempt.plan.evidence_token,
                &original.attempt.plan.acceptance_digest,
                acceptance.acceptance().accepted_at,
            )?;
        }
        let prepared = self
            .recover_resume_preparation(
                &original,
                &workspace,
                &gate,
                &verified_parent,
                historical.as_mut().and_then(|h| h.sources.take()),
            )
            .await?;
        let preparation = prepared.lock().await;
        let repositories = repository_evidence(&preparation, &workspace.contracts)?;
        let selected = workspace
            .configuration
            .workspaces
            .values()
            .find(|w| w.id == original.workspace)
            .ok_or_else(|| Error::new("scope_violation", "resume workspace disappeared"))?;
        let plan = build_release_plan(ReleasePlanInput {
            runtime_id: original.runtime,
            operation_id: original.attempt.plan.operation_id,
            workspace: selected,
            parent: &verified_parent,
            acceptance: &acceptance,
            carried: None,
            repositories: &repositories,
            dependencies: self.release_requirements(parent)?,
        })?;
        if plan != original.attempt.plan {
            return Err(Error::new(
                "stale_result",
                "resume configuration or accepted plan changed",
            ));
        }
        let production = preparation
            .repositories
            .values()
            .map(|r| r.production.clone().unwrap())
            .collect::<Vec<_>>();
        let origin = self.runtime.control.operation(plan.operation_id)?;
        let previous = self
            .runtime
            .control
            .operation(original.admission_operation)?;
        let verified_resume = crate::control::VerifiedReleaseResume::for_decision(
            &origin,
            &previous,
            &plan.digest()?,
            &decision,
        )?;
        let next_operation = verified_resume.reference().request_id()?;
        let evidence = || DeliveryEvidence {
            parent: &verified_parent,
            acceptance: &acceptance,
            carried: None,
            production: &production,
        };
        let facts = |snapshot: &ControlSnapshot| AdmissionFacts {
            running: snapshot.intent.desired == Intent::Running,
            included: parent.included,
            authorized: true,
            no_pending_correction: true,
            preparation_complete: true,
            provider_reads_complete: true,
            repository_policy_usable: true,
            credentials_usable: true,
            manual_mode: true,
            explicit_manual_request: Some(decision.parent_command().operation_id),
            actor_may_release: true,
            owner_generation: lease.owner_generation,
            control_generation: snapshot.intent.generation,
            required_step_ids: plan.steps.iter().map(|s| s.id.clone()).collect(),
        };
        let admission_snapshot = self.runtime.control.snapshot()?;
        let eligibility = {
            let state = self.releases.state.lock().map_err(|_| unavailable())?;
            domain::verify_release_resume_eligibility(
                &original.attempt,
                evidence(),
                facts(&admission_snapshot),
                &state.queue,
                Utc::now(),
            )?
        };
        let active = self.runtime.observations.capture_active(
            gate.workspace_id(),
            gate.config_revision(),
            gate.workflow_digest(),
        )?;
        let retained_workspace = self.release_workspace(&workspace, original.workspace)?;
        let reason = decision.request().args["reason"]
            .as_str()
            .ok_or_else(|| Error::invalid("recovery reason missing"))?;
        let checked = self.current_gate();
        let permit =
            self.runtime
                .control
                .admit_release_resume(&lease, &verified_resume, |snapshot| {
                    self.recovery_gate(&decision, snapshot)?;
                    self.runtime.registration.verify_active_owner(&lease)?;
                    checked(&gate)?;
                    if let Some(history) = &historical {
                        current_baselines(&history.baselines)?;
                    } else {
                        self.runtime
                            .current_delivery(&parent.key)?
                            .require_release_selection(
                                verified_parent.token(),
                                acceptance.digest(),
                                acceptance.acceptance().accepted_at,
                            )?;
                    }
                    Self::check_release_publications(&self.runtime, &original, None)?;
                    let mut state = self.releases.state.lock().map_err(|_| unavailable())?;
                    let prior = state.attempts.get(&plan.operation_id).ok_or_else(|| {
                        Error::new("operation_missing", "resume attempt disappeared")
                    })?;
                    if canonical_digest(1, prior)? != canonical_digest(1, &original)? {
                        return Err(Error::new(
                            "stale_result",
                            "resume attempt changed during fresh observations",
                        ));
                    }
                    domain::verify_release_resume_eligibility(
                        &prior.attempt,
                        evidence(),
                        facts(snapshot),
                        &state.queue,
                        Utc::now(),
                    )?;
                    let mut next = prior.clone();
                    next.attempt.resume(
                        &decision.actor().identity,
                        reason,
                        &verified_parent,
                        true,
                        Utc::now(),
                        |_| Ok(()),
                    )?;
                    next.attempt.owner_generation = lease.owner_generation;
                    next.attempt.control_generation = snapshot.intent.generation;
                    next.admission_operation = next_operation;
                    next.manual_actor = Some(decision.actor().clone());
                    next.manual_role = Some(ManualRole::Recover);
                    next.manual_intent = Some(decision.parent_command().operation_id);
                    next.admission_confirmed = false;
                    next.local_retired = false;
                    next.config_revision = gate.config_revision().into();
                    next.workflow_digest = gate.workflow_digest().into();
                    let mut proposed = state.attempts.clone();
                    proposed.insert(plan.operation_id, next.clone());
                    let queue = rebuild_queue(&proposed, &state.no_release)?;
                    self.save_release(&next)?;
                    state.attempts = proposed;
                    state.queue = queue;
                    Ok(())
                });
        let permit = match permit {
            Ok(permit) => permit,
            Err(error) => {
                if self
                    .releases
                    .state
                    .lock()
                    .map_err(|_| unavailable())?
                    .attempts
                    .get(&plan.operation_id)
                    .is_some_and(|r| r.admission_operation == next_operation)
                {
                    self.update_release(plan.operation_id, |r, _| {
                        r.attempt.fence(
                            format!("resume admission not confirmed: {}", error.code),
                            |_| Ok(()),
                        )
                    })?;
                }
                return Err(error);
            }
        };
        drop(preparation);
        self.releases
            .live
            .lock()
            .map_err(|_| unavailable())?
            .insert(
                plan.operation_id,
                Arc::new(tokio::sync::Mutex::new(LiveRelease {
                    permit: permit.clone(),
                    gate,
                    parent: verified_parent,
                    acceptance,
                    eligibility,
                    preparation: prepared,
                    admissions: BTreeMap::new(),
                    workspace: retained_workspace,
                    _active: active,
                })),
            );
        let receipt = self.runtime.control.operation(permit.operation_id)?;
        self.update_release(plan.operation_id, |r, _| {
            r.admission_confirmed = true;
            r.recovery_receipts
                .insert(receipt.operation_id, receipt.payload_digest.clone());
            Ok(())
        })?;
        Ok(receipt)
    }

    async fn recover_resume_preparation(
        &self,
        original: &AttemptRecord,
        workspace: &WorkspaceDelivery,
        gate: &FreshGate,
        parent: &VerifiedParent,
        historical_sources: Option<ParentSources>,
    ) -> Result<Arc<tokio::sync::Mutex<Preparation>>> {
        let key = preparation_key(original.workspace, &original.work, parent.token());
        let retained = self
            .releases
            .preparations
            .lock()
            .map_err(|_| unavailable())?
            .get(&key)
            .cloned();
        let prepared = if let Some(retained) = retained {
            retained
        } else {
            let sources = match historical_sources {
                Some(sources) => sources,
                None => self
                    .parent_sources(original.workspace, original.work.parent)
                    .await?
                    .ok_or_else(|| {
                        Error::new(
                            "candidate_provenance",
                            "complete authenticated retained candidate sources are unavailable",
                        )
                    })?,
            };
            let signed: Signed<PreparationRef> = self
                .records
                .read(&preparation_leaf(original.workspace, parent.token())?)?;
            self.runtime
                .services
                .key
                .verify("runtime_release_preparation", &signed)?;
            let reference = signed.payload;
            if reference.schema_version != 1
                || reference.workspace != original.workspace
                || reference.work != original.work
                || reference.evidence_token != parent.token()
                || reference.operation != original.attempt.plan.operation_id
                || reference.repositories != original.preparations
            {
                return Err(Error::new(
                    "record_corrupt",
                    "resume preparation identity changed",
                ));
            }
            let repositories = reference
                .repositories
                .iter()
                .map(|(repository, operation)| {
                    (
                        repository.clone(),
                        RepoPreparation {
                            operation: *operation,
                            phase: None,
                            candidate: None,
                            production: None,
                        },
                    )
                })
                .collect();
            let prepared = Arc::new(tokio::sync::Mutex::new(Preparation {
                reference,
                sources,
                repositories,
                error: None,
            }));
            self.releases
                .preparations
                .lock()
                .map_err(|_| unavailable())?
                .insert(key, prepared.clone());
            prepared
        };
        let mut retained = prepared.lock().await;
        if retained.reference.repositories != original.preparations
            || retained.reference.work != original.work
            || retained.sources.parent.token() != parent.token()
            || retained.repositories.values().any(|r| r.phase.is_some())
        {
            return Err(Error::new(
                "reconciliation_required",
                "resume preparation identity changed or still owns a live phase",
            ));
        }
        for contract in &workspace.contracts {
            let revision = release_revision(&original.attempt.plan, &contract.repository)?;
            let observation = workspace.coordinator.observation(gate.clone(), revision)?;
            let candidate = parent
                .manifest()
                .candidates
                .iter()
                .find(|c| c.repository == contract.repository)
                .ok_or_else(|| {
                    Error::new("candidate_provenance", "resume candidate inventory changed")
                })?;
            let operation = *original
                .preparations
                .get(&contract.repository)
                .ok_or_else(|| {
                    Error::new(
                        "candidate_provenance",
                        "resume repository preparation is missing",
                    )
                })?;
            let (native, proof) = if candidate.ordered_delta_digests.is_empty() {
                let baseline = retained
                    .sources
                    .baselines
                    .iter()
                    .find(|b| b.baseline().repository == contract.repository)
                    .ok_or_else(|| {
                        Error::new("candidate_provenance", "resume baseline is missing")
                    })?;
                workspace.observer.reobserve(contract, baseline).await?;
                (
                    None,
                    workspace
                        .coordinator
                        .hydrate_existing_production_validation(
                            &observation,
                            parent,
                            &retained.sources.integrations,
                            operation,
                        )
                        .await?,
                )
            } else {
                let merged = original.attempt.plan.steps.iter().find(|step|
                    matches!(&step.kind, domain::StepKind::Merge { repository, .. } if repository == &contract.repository)
                    && matches!(original.attempt.steps.get(&step.id), Some(StepState::Succeeded { .. })));
                let (native, proof) = if let Some(step) = merged {
                    workspace
                        .coordinator
                        .hydrate_owned_merge_production(MergedProductionRecovery {
                            observation: &observation,
                            parent,
                            preparation: operation,
                            plan: &original.attempt.plan,
                            merge_step: &step.id,
                            attempt: original.attempt.step_attempt(&step.id)?,
                            observer: &workspace.observer,
                        })
                        .await?
                } else {
                    let native = workspace
                        .coordinator
                        .hydrate_production_candidate(&observation, parent, operation)
                        .await?;
                    let proof = workspace
                        .coordinator
                        .hydrate_production_validation(&observation, &native, parent)
                        .await?;
                    (native, proof)
                };
                (Some(native), proof)
            };
            let repo = retained
                .repositories
                .get_mut(&contract.repository)
                .ok_or_else(|| {
                    Error::new(
                        "candidate_provenance",
                        "resume repository inventory changed",
                    )
                })?;
            repo.candidate = native;
            repo.production = Some(proof);
        }
        drop(retained);
        Ok(prepared)
    }

    /// Reconcile only the exact original owned operation requested by the
    /// operator. Provider reads can establish an outcome, never permission to
    /// replay a write or replace the immutable request payload.
    pub async fn reconcile_operation(
        &self,
        decision: AuthorizedOperationRecovery,
    ) -> Result<Receipt> {
        let _serial = self.releases.progressing.lock().await;
        let reconciled = self.runtime.control.serialize_observation(|snapshot| {
            self.operation_recovery_gate(&decision, snapshot)?;
            let actual = self
                .runtime
                .control
                .operation(decision.operation().operation_id)?;
            let expected = decision.operation();
            if actual.payload_digest != expected.payload_digest
                || actual.kind != expected.kind
                || actual.runtime_id != expected.runtime_id
                || actual.expected != expected.expected
                || actual.canonical_targets != expected.canonical_targets
            {
                return Err(Error::new(
                    "ownership_conflict",
                    "requested original operation identity changed",
                ));
            }
            Ok((actual.phase == ReceiptPhase::Reconciled).then_some(actual))
        })?;
        if let Some(receipt) = reconciled {
            return Ok(receipt);
        }
        self.restore_releases()?;
        let projection = self.runtime.observations.refresh().await?;
        let target = decision.operation();
        let records = self
            .releases
            .state
            .lock()
            .map_err(|_| unavailable())?
            .attempts
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let matching = records
            .into_iter()
            .filter(|r| {
                r.attempt.plan.operation_id == target.operation_id
                    || r.admission_operation == target.operation_id
                    || target
                        .expected
                        .get("release_plan")
                        .is_some_and(|d| r.attempt.plan.digest().as_ref() == Ok(d))
            })
            .collect::<Vec<_>>();
        if matching.len() > 1 {
            return Err(Error::new(
                "ownership_conflict",
                "operation matches multiple retained release plans",
            ));
        }
        if let Some(record) = matching.first() {
            if target.kind == "admission" && target.operation_id != record.admission_operation {
                return Err(Error::new(
                    "reconciliation_required",
                    "historical admission must be reconciled independently of a newer admission's effects",
                ));
            }
            let workspace = self.workspace(&projection, record.workspace)?;
            let selected = if target.kind == "admission" {
                record
                    .attempt
                    .steps
                    .iter()
                    .filter(|(id, state)| {
                        state.is_unknown()
                            || record
                                .pending_receipts
                                .values()
                                .any(|pending| pending == *id)
                    })
                    .map(|(id, _)| Ok((id.clone(), record.attempt.step_attempt(id)?)))
                    .collect::<Result<Vec<_>>>()?
            } else {
                let step = target.expected.get("release_step").ok_or_else(|| {
                    Error::new("record_corrupt", "release effect step is missing")
                })?;
                let attempt = target
                    .expected
                    .get("release_attempt")
                    .and_then(|a| a.parse::<u32>().ok())
                    .filter(|a| *a > 0)
                    .ok_or_else(|| {
                        Error::new("record_corrupt", "release effect attempt is missing")
                    })?;
                vec![(step.clone(), attempt)]
            };
            for (step, ordinal) in selected {
                let observed = workspace
                    .coordinator
                    .recover_owned_release_step(
                        &record.attempt.plan,
                        &step,
                        ordinal,
                        &workspace.observer,
                    )
                    .await?;
                if observed.outcome() == &ReleaseStepOutcome::Pending {
                    return Err(Error::new(
                        "operation_uncertain",
                        "exact owned release effect has no conclusive native outcome",
                    ));
                }
                let reconcile = |receipt: &Receipt| {
                    self.runtime.control.reconcile_operation_observed(decision.actor(), receipt.operation_id,
                        |actual, snapshot| {
                            self.operation_recovery_gate(&decision, snapshot)?;
                            if actual.payload_digest != receipt.payload_digest || actual.expected != receipt.expected
                                || actual.canonical_targets != receipt.canonical_targets {
                                return Err(Error::new("ownership_conflict", "release receipt changed during observation"));
                            }
                            Ok(crate::control::Reconciliation::UniqueVerified {
                                remote_ids: receipt.remote_ids.clone(),
                                result: json!({"release_plan":observed.plan_digest(),"step":observed.step_id(),"attempt":observed.attempt(),"outcome":observed.outcome(),"effect":observed.effect()}),
                                payload_digest: receipt.payload_digest.clone(), actor: receipt.actor.clone(),
                            })
                        }).map(|_| ())
                };
                if record.attempt.step_attempt(&step)? == ordinal
                    && (target.kind == "admission"
                        || target.expected.get("admission_operation")
                            == Some(&record.admission_operation.to_string()))
                {
                    self.commit_release_observation(
                        record.attempt.plan.operation_id,
                        &observed,
                        reconcile,
                    )?;
                } else if let Some(receipt) = observed.receipt() {
                    reconcile(receipt)?;
                } else {
                    return Err(Error::new(
                        "reconciliation_required",
                        "old step observation has no original provider receipt",
                    ));
                }
            }
            if target.kind == "admission" {
                let current = self
                    .releases
                    .state
                    .lock()
                    .map_err(|_| unavailable())?
                    .attempts[&record.attempt.plan.operation_id]
                    .clone();
                if !ready_for_local_retirement(&current) {
                    return Err(Error::new(
                        "reconciliation_required",
                        "release still has active or uncertain steps",
                    ));
                }
                self.runtime.control.serialize_observation(|snapshot| {
                    self.operation_recovery_gate(&decision, snapshot)
                })?;
                self.retire_release(record.attempt.plan.operation_id, &current)?;
            }
            let actual = self.runtime.control.operation(target.operation_id)?;
            if actual.phase != ReceiptPhase::Reconciled {
                return Err(Error::new(
                    "reconciliation_required",
                    "operation has not been conclusively reconciled",
                ));
            }
            return Ok(actual);
        }
        if let Some(receipt) = self.reconcile_no_release(&decision).await? {
            return Ok(receipt);
        }
        let mut publications = Vec::new();
        for workspace in projection.workspaces.values() {
            let service = self.workspace(&projection, workspace.workspace_id)?;
            if let Some((operation, action)) = service
                .coordinator
                .publication_recovery_target(target.operation_id)?
            {
                publications.push((service, operation, action));
            }
        }
        let [(workspace, operation, action)] = publications.as_slice() else {
            return Err(Error::new(
                "reconciliation_required",
                "exact operation has no unique authenticated delivery recovery record; preserve its hold",
            ));
        };
        workspace
            .coordinator
            .reconcile_receipt_checked(decision.actor(), *operation, action, |snapshot| {
                self.operation_recovery_gate(&decision, snapshot)
            })
            .await
    }
    pub(super) fn operation_recovery_gate(
        &self,
        decision: &AuthorizedOperationRecovery,
        snapshot: &ControlSnapshot,
    ) -> Result<()> {
        self.runtime.registration.verify_control_owner()?;
        self.runtime
            .configuration
            .authority_fences()?
            .check_actor(decision.actor(), "recover")?;
        decision.recheck(&self.runtime.configuration.snapshot()?, snapshot)
    }

    async fn recovery_baselines(
        &self,
        workspace: &WorkspaceDelivery,
        plan: &ReleasePlan,
    ) -> Result<Vec<VerifiedRecoveryBaseline>> {
        let repositories = plan
            .steps
            .iter()
            .map(|step| step.kind.repository().clone())
            .collect::<BTreeSet<_>>();
        if workspace
            .contracts
            .iter()
            .map(|c| c.repository.clone())
            .collect::<BTreeSet<_>>()
            != repositories
        {
            return Err(Error::new(
                "scope_violation",
                "recovery requires exact configured repository inventory",
            ));
        }
        let mut baselines = Vec::new();
        for contract in &workspace.contracts {
            baselines.push(workspace.observer.observe_recovery(contract).await?);
        }
        Ok(baselines)
    }
    // Invoked only by the control host's closed operator-transition gate. It
    // must not reenter ControlHost or perform provider/process operations.
    fn persist_recovery_transition(
        &self,
        original: &AttemptRecord,
        receipt: &Receipt,
        action: &str,
        transition: impl FnOnce(&mut AttemptRecord) -> Result<()>,
    ) -> Result<(BTreeMap<String, String>, Value)> {
        let mut state = self.releases.state.lock().map_err(|_| unavailable())?;
        let operation = original.attempt.plan.operation_id;
        let mut record = state
            .attempts
            .get(&operation)
            .cloned()
            .ok_or_else(|| Error::new("operation_missing", "release attempt disappeared"))?;
        if canonical_digest(1, &record)? != canonical_digest(1, original)? {
            return Err(Error::new(
                "stale_result",
                "release state changed during recovery observation",
            ));
        }
        transition(&mut record)?;
        record
            .recovery_receipts
            .insert(receipt.operation_id, receipt.payload_digest.clone());
        record.attempt.validate_recovered()?;
        let mut proposed = state.attempts.clone();
        proposed.insert(operation, record.clone());
        let queue = rebuild_queue(&proposed, &state.no_release)?;
        self.save_release(&record)?;
        let observed = json!({"release_operation":operation,"action":action,"attempt_digest":canonical_digest(1,&record)?,"resolved":record.attempt.resolved,"abandoned":record.attempt.abandoned,"hold":record.attempt.hold_reason,"unknown":record.attempt.has_unknown(),"released":false});
        state.attempts = proposed;
        state.queue = queue;
        Ok((BTreeMap::new(), observed))
    }
}
fn current_baselines(baselines: &[VerifiedRecoveryBaseline]) -> Result<()> {
    let now = Utc::now();
    if baselines.iter().any(|baseline| {
        baseline.observed_at() > now + Duration::seconds(5)
            || now - baseline.observed_at() > Duration::seconds(30)
    }) {
        return Err(Error::new(
            "stale_result",
            "recovery baseline observation expired",
        ));
    }
    Ok(())
}
fn reconciled_baseline(
    baselines: &[VerifiedRecoveryBaseline],
    plan: &ReleasePlan,
) -> Result<domain::ReconciledBaseline> {
    current_baselines(baselines)?;
    let observations = baselines
        .iter()
        .flat_map(VerifiedRecoveryBaseline::observations)
        .collect::<Vec<_>>();
    if observations
        .iter()
        .map(|v| v.resource.clone())
        .collect::<BTreeSet<_>>()
        != plan.resources
        || observations.len() != plan.resources.len()
    {
        return Err(Error::new(
            "recovery_blocked",
            "recovery baseline must cover each held resource exactly once",
        ));
    }
    Ok(domain::ReconciledBaseline {
        provider_reads_complete: true,
        coherent: baselines.iter().all(VerifiedRecoveryBaseline::coherent),
        observations,
        observed_at: Utc::now(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::HostFuture,
        control::{Actor, Lifecycle, MutationRequest, Principal},
        decisions::*,
        evidence::VerifiedAcceptance,
        reconcile::{DependencyOutcomes, ParentDelivery},
    };
    struct Evidence {
        delivery: Arc<RuntimeDelivery>,
    }
    impl DecisionEvidence for Evidence {
        fn current(&self, parent: &IssueKey) -> Result<ParentDelivery> {
            self.delivery.runtime.current_delivery(parent)
        }
        fn outcomes(&self) -> Result<DependencyOutcomes> {
            self.delivery.runtime.dependency_outcomes()
        }
        fn retry_target(&self, _: &WorkIdentity) -> Result<RetryTarget> {
            Err(Error::new(
                "fixture_unexpected",
                "retry not part of recovery fixture",
            ))
        }
        fn published_retry_target(&self, _: &VerifiedRetryGrant) -> Result<RetryTarget> {
            Err(Error::new(
                "fixture_unexpected",
                "retry not part of recovery fixture",
            ))
        }
        fn record_acceptance(&self, _: &IssueKey, _: VerifiedAcceptance) -> Result<()> {
            Err(Error::new(
                "fixture_unexpected",
                "acceptance not part of recovery fixture",
            ))
        }
        fn record_correction(&self, _: VerifiedExplicitCorrection) -> Result<()> {
            Err(Error::new(
                "fixture_unexpected",
                "correction not part of recovery fixture",
            ))
        }
        fn retry_committed(&self, _: VerifiedRetryGrant) -> Result<()> {
            Err(Error::new(
                "fixture_unexpected",
                "retry not part of recovery fixture",
            ))
        }
        fn release(&self, _: AuthorizedDecision) -> HostFuture<'_, Receipt> {
            Box::pin(async {
                Err(Error::new(
                    "fixture_unexpected",
                    "new release not part of recovery fixture",
                ))
            })
        }
        fn recover_release(&self, decision: AuthorizedDecision) -> HostFuture<'_, Receipt> {
            crate::delivery::RuntimeDeliveryDecisions::recover_release(
                self.delivery.as_ref(),
                decision,
            )
        }
        fn reconcile(&self, decision: AuthorizedOperationRecovery) -> HostFuture<'_, Receipt> {
            crate::delivery::RuntimeDeliveryDecisions::reconcile(self.delivery.as_ref(), decision)
        }
    }
    #[tokio::test]
    async fn actual_stopped_decision_abandons_only_known_work_and_keeps_unknown_provider_hold() {
        let (_root, built, delivery) =
            super::super::super::tests::fixture_authorization(true).await;
        let runtime = built.runtime.clone();
        let projection = runtime.observations.snapshot().unwrap();
        let parent = projection
            .parent(Uuid::from_u128(2), Uuid::from_u128(100))
            .unwrap();
        let (plan, work, _) = crate::delivery::observed_merge_for_runtime().await;
        assert_eq!(plan.parent_id, parent.key.issue);
        let lease = super::super::super::tests::start_control(&runtime);
        let permit = runtime
            .control
            .admit(
                &lease,
                plan.operation_id,
                parent.key.identity().unwrap(),
                plan.digest().unwrap(),
                || Ok(()),
            )
            .unwrap();
        let actor = Actor {
            identity: "operator".into(),
            principal: Some(Principal::Unix { uid: 501 }),
            roles: ["recover".into(), "inspect".into(), "lifecycle".into()].into(),
        };
        let external = MutationRequest {
            request_id: Uuid::new_v4(),
            runtime_id: plan.runtime_id,
            actor: actor.clone(),
            kind: "fixture_unknown_release_effect".into(),
            canonical_targets: vec![parent.key.identity().unwrap()],
            expected: BTreeMap::new(),
            payload: Value::Null,
        };
        let external_id = external.request_id;
        runtime
            .control
            .begin_effect(external, &permit, || Ok(()), |_| Ok(()))
            .unwrap();
        runtime
            .control
            .retire_local(
                permit.operation_id,
                json!({"fixture":"actual local owner ended with unknown external outcome"}),
            )
            .unwrap();
        let generation = runtime.control.snapshot().unwrap().intent.generation;
        runtime
            .control
            .lifecycle(
                MutationRequest {
                    request_id: Uuid::new_v4(),
                    runtime_id: plan.runtime_id,
                    actor: actor.clone(),
                    kind: "lifecycle".into(),
                    canonical_targets: vec![plan.runtime_id.to_string()],
                    expected: BTreeMap::new(),
                    payload: Value::Null,
                },
                Lifecycle::Stop,
                generation,
            )
            .unwrap();
        let original = super::super::tests::record(plan.clone(), work);
        delivery.save_release(&original).unwrap();
        let services = &runtime.services;
        let manager = DecisionManager::new(DecisionDependencies {
            configuration: services.configuration.clone(),
            control: services.control.clone(),
            observations: services.observations.clone(),
            secrets: services.secrets.clone(),
            key: services.key.clone(),
            evidence: Arc::new(Evidence {
                delivery: delivery.clone(),
            }),
            housekeeping: services.housekeeping.clone(),
            inputs: services.inputs.clone(),
        });
        let mut request = crate::api::CommandRequest {
            api_version: 1,
            request_id: Uuid::new_v4(),
            runtime_id: plan.runtime_id,
            command: CommandName::ReleaseRecover,
            args: json!({"parent":"TEST-100","operation":plan.operation_id,"action":"abandon","reason":"Preserve unknown remote action; retire only known unstarted work"}),
            expected_config_revision: Some(
                runtime
                    .configuration
                    .snapshot()
                    .unwrap()
                    .revision()
                    .unwrap(),
            ),
            expected_control_generation: Some(
                runtime.control.snapshot().unwrap().intent.generation,
            ),
        };
        let mut mutation = crate::app::mutation_request(
            &actor,
            &request,
            manager.command_targets(&request).unwrap(),
        );
        mutation.kind = "command/ReleaseRecover".into();
        mutation.payload = json!({"arguments":request.args,"secret_digest":null});
        let (command, fresh) = runtime
            .control
            .queue_command(mutation, "recover", || Ok(()))
            .unwrap();
        assert!(fresh);
        request.request_id = crate::app::command_effect_id(command.operation_id);
        let result = manager.execute(actor, request, command).await.unwrap();
        assert!(matches!(
            result.phase,
            ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
        ));
        let observed = delivery.release_attempts().unwrap();
        assert_eq!(observed.len(), 1);
        assert!(observed[0].abandoned);
        assert!(observed[0].has_unknown());
        assert!(observed[0].hold_reason.is_some());
        assert!(!observed[0].resolved);
        assert!(
            runtime
                .control
                .snapshot()
                .unwrap()
                .unresolved_operations
                .contains_key(&external_id)
        );
        assert_eq!(
            runtime.control.operation(external_id).unwrap().phase,
            ReceiptPhase::Sent
        );
        assert!(
            runtime
                .control
                .snapshot()
                .unwrap()
                .active_operations
                .is_empty()
        );
        let stored: Signed<AttemptRecord> = delivery
            .records
            .read(&attempt_leaf(plan.operation_id))
            .unwrap();
        runtime
            .services
            .key
            .verify("runtime_release_attempt", &stored)
            .unwrap();
        assert_eq!(stored.payload.recovery_receipts.len(), 1);
        let transition = runtime
            .control
            .operation(*stored.payload.recovery_receipts.keys().next().unwrap())
            .unwrap();
        assert_eq!(transition.kind, "release_recovery_transition");
        assert_eq!(
            transition.observed_result.as_ref().unwrap()["abandoned"],
            true
        );
    }
    async fn execute_recovery(
        delivery: &Arc<RuntimeDelivery>,
        command_name: CommandName,
        args: Value,
    ) -> Result<Receipt> {
        let runtime = &delivery.runtime;
        let services = &runtime.services;
        let manager = DecisionManager::new(DecisionDependencies {
            configuration: services.configuration.clone(),
            control: services.control.clone(),
            observations: services.observations.clone(),
            secrets: services.secrets.clone(),
            key: services.key.clone(),
            evidence: Arc::new(Evidence {
                delivery: delivery.clone(),
            }),
            housekeeping: services.housekeeping.clone(),
            inputs: services.inputs.clone(),
        });
        let actor = Actor {
            identity: "operator".into(),
            principal: Some(Principal::Unix { uid: 501 }),
            roles: ["recover".into(), "inspect".into(), "lifecycle".into()].into(),
        };
        let mut request = crate::api::CommandRequest {
            api_version: 1,
            request_id: Uuid::new_v4(),
            runtime_id: runtime.configuration.snapshot()?.runtime.id,
            command: command_name,
            args,
            expected_config_revision: Some(runtime.configuration.snapshot()?.revision()?),
            expected_control_generation: Some(runtime.control.snapshot()?.intent.generation),
        };
        let mut mutation =
            crate::app::mutation_request(&actor, &request, manager.command_targets(&request)?);
        mutation.kind = format!("command/{:?}", request.command);
        mutation.payload = json!({"arguments":request.args,"secret_digest":null});
        let (parent, fresh) = runtime
            .control
            .queue_command(mutation, "recover", || Ok(()))?;
        assert!(fresh);
        request.request_id = crate::app::command_effect_id(parent.operation_id);
        manager.execute(actor, request, parent).await
    }

    #[tokio::test]
    async fn resume_cannot_retire_a_healthy_admitted_plan_between_steps() {
        let (_root, built, delivery) =
            super::super::super::tests::fixture_authorization(true).await;
        let (plan, work, _) = crate::delivery::observed_merge_for_runtime().await;
        let lease = super::super::super::tests::start_control(&built.runtime);
        let permit = built
            .runtime
            .control
            .admit(
                &lease,
                plan.operation_id,
                IssueKey::new(&work.linear_endpoint, work.linear_workspace, work.parent)
                    .unwrap()
                    .identity()
                    .unwrap(),
                plan.digest().unwrap(),
                || Ok(()),
            )
            .unwrap();
        let mut record = super::super::tests::record(plan.clone(), work);
        for step in record.attempt.steps.values_mut() {
            *step = StepState::NotStarted;
        }
        delivery.save_release(&record).unwrap();
        delivery.restore_releases().unwrap();
        delivery
            .update_release(plan.operation_id, |record, _| {
                record.attempt.hold_reason = None;
                Ok(())
            })
            .unwrap();
        let before = built.runtime.control.operation(plan.operation_id).unwrap();
        let error = execute_recovery(&delivery, CommandName::ReleaseRecover,
            json!({"parent":"TEST-100","operation":plan.operation_id,"action":"resume","reason":"Do not preempt an active healthy plan"})).await.unwrap_err();
        assert_eq!(error.code, "recovery_blocked");
        let after = built.runtime.control.operation(plan.operation_id).unwrap();
        assert_eq!(
            canonical_digest(1, &before).unwrap(),
            canonical_digest(1, &after).unwrap()
        );
        assert!(
            built
                .runtime
                .control
                .snapshot()
                .unwrap()
                .active_operations
                .iter()
                .any(|p| p.operation_id == permit.operation_id)
        );
        assert!(
            !delivery.releases.state.lock().unwrap().attempts[&plan.operation_id].local_retired
        );
    }

    #[tokio::test]
    async fn reconcile_old_admission_returns_only_its_receipt_and_preserves_newer_unknown_effect() {
        let (_root, built, delivery) =
            super::super::super::tests::fixture_authorization(true).await;
        let runtime = &built.runtime;
        let lease = super::super::super::tests::start_control(runtime);
        let projection = runtime.observations.snapshot().unwrap();
        let parent = projection
            .parent(Uuid::from_u128(2), Uuid::from_u128(100))
            .unwrap();
        let operation = Uuid::new_v4();
        runtime
            .control
            .admit(
                &lease,
                operation,
                parent.key.identity().unwrap(),
                canonical_digest(1, &"original-plan").unwrap(),
                || Ok(()),
            )
            .unwrap();
        let original = runtime
            .control
            .retire_local(operation, json!({"fixture":"original finite plan exited"}))
            .unwrap();
        let newer = runtime
            .control
            .admit(
                &lease,
                Uuid::new_v4(),
                parent.key.identity().unwrap(),
                canonical_digest(1, &"newer-admission").unwrap(),
                || Ok(()),
            )
            .unwrap();
        let external_id = Uuid::new_v4();
        runtime
            .control
            .begin_effect(
                MutationRequest {
                    request_id: external_id,
                    runtime_id: lease.runtime_id,
                    actor: Actor {
                        identity: "fixture-owner".into(),
                        principal: None,
                        roles: BTreeSet::new(),
                    },
                    kind: "fixture_newer_unknown_effect".into(),
                    canonical_targets: vec![parent.key.identity().unwrap()],
                    expected: BTreeMap::new(),
                    payload: Value::Null,
                },
                &newer,
                || Ok(()),
                |_| Ok(()),
            )
            .unwrap();
        let unknown = runtime.control.operation(external_id).unwrap();
        let completed = execute_recovery(
            &delivery,
            CommandName::OperationReconcile,
            json!({"operation":operation}),
        )
        .await
        .unwrap();
        assert!(matches!(
            completed.phase,
            ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
        ));
        assert_eq!(
            canonical_digest(1, &runtime.control.operation(operation).unwrap()).unwrap(),
            canonical_digest(1, &original).unwrap()
        );
        assert_eq!(
            canonical_digest(1, &runtime.control.operation(external_id).unwrap()).unwrap(),
            canonical_digest(1, &unknown).unwrap()
        );
        assert!(
            runtime
                .control
                .snapshot()
                .unwrap()
                .unresolved_operations
                .contains_key(&external_id)
        );
        assert!(
            runtime
                .control
                .snapshot()
                .unwrap()
                .active_operations
                .iter()
                .any(|p| p.operation_id == newer.operation_id)
        );
    }
}

#[path = "runtime_delivery_successor.rs"]
mod successor;

impl crate::delivery::RuntimeDeliveryDecisions for RuntimeDelivery {
    fn release(&self, decision: AuthorizedDecision) -> crate::app::HostFuture<'_, Receipt> {
        Box::pin(self.queue_release(decision))
    }
    fn recover_release(&self, decision: AuthorizedDecision) -> crate::app::HostFuture<'_, Receipt> {
        Box::pin(async move {
            match decision.request().args["action"].as_str() {
                Some("resume") => self.resume_release(decision).await,
                Some("abandon") => self.abandon_release(decision).await,
                Some("resolve") => self.resolve_release(decision).await,
                Some("supersede") => self.supersede_release(decision).await,
                _ => Err(Error::invalid("release recovery action is invalid")),
            }
        })
    }
    fn reconcile(
        &self,
        decision: AuthorizedOperationRecovery,
    ) -> crate::app::HostFuture<'_, Receipt> {
        Box::pin(self.reconcile_operation(decision))
    }
}

#[path = "runtime_delivery_history.rs"]
mod history;
