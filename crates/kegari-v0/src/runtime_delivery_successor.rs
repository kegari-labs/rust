//! Corrected successor authorization is durable intent, never an adopted permit.
use super::*;
use chrono::DateTime;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::runtime_delivery::release) struct SuccessorIntent {
    schema_version: u32,
    predecessor: Uuid,
    predecessor_plan_digest: String,
    predecessor_admission_digest: String,
    workspace: Uuid,
    work: WorkIdentity,
    plan: ReleasePlan,
    acceptance_digest: String,
    accepted_at: DateTime<Utc>,
    config_revision: String,
    actor: crate::control::Actor,
    transition_id: Uuid,
    transition_digest: String,
    parent_command: Uuid,
    parent_command_digest: String,
}
impl SuccessorIntent {
    pub(in crate::runtime_delivery::release) fn actor(&self) -> &crate::control::Actor {
        &self.actor
    }
    pub(in crate::runtime_delivery::release) fn command(&self) -> Uuid {
        self.parent_command
    }
}
fn leaf(operation: Uuid) -> String {
    format!("release-successor-{operation}.json")
}
impl RuntimeDelivery {
    /// Assign a known predecessor hold to an exact, freshly accepted corrected
    /// result. Dispatch still waits for the driver's fresh actual admission.
    pub async fn supersede_release(&self, decision: AuthorizedDecision) -> Result<Receipt> {
        let _serial = self.releases.progressing.lock().await;
        self.restore_releases()?;
        let projection = self.runtime.observations.refresh().await?;
        let original = self.recovery_record(&decision, "supersede")?;
        if original.attempt.has_unknown()
            || !original.pending_receipts.is_empty()
            || original.attempt.hold_reason.is_none()
            || original.attempt.resolved
            || original.attempt.completed()
            || !original.local_retired
        {
            return Err(Error::new(
                "recovery_blocked",
                "supersede requires a held predecessor with known effects and proven local retirement",
            ));
        }
        self.require_retired_successor_predecessor(&original)?;
        let workspace = self.workspace(&projection, original.workspace)?;
        let parent = projection.parent(original.workspace, original.work.parent)?;
        let gate = parent.fresh_gate(
            None,
            GatePurpose::NewOperation,
            &self.runtime.dependency_outcomes()?,
            Utc::now(),
            Duration::seconds(30),
        )?;
        let current = self.runtime.current_delivery(&parent.key)?;
        let verified = current
            .verified_parent()
            .ok_or_else(|| {
                Error::new(
                    "evidence_missing",
                    "corrected successor needs current complete parent evidence",
                )
            })?
            .clone();
        let acceptance = current
            .verified_acceptance()
            .ok_or_else(|| {
                Error::new(
                    "acceptance_required",
                    "corrected successor needs fresh authenticated acceptance",
                )
            })?
            .clone();
        if decision.request().args["result"].as_str() != Some(verified.token())
            || verified.manifest().work != *decision.work()
            || verified.manifest().work.generation == original.work.generation
            || acceptance.acceptance().work != verified.manifest().work
            || acceptance.acceptance().reviewed_token != verified.token()
        {
            return Err(Error::new(
                "stale_result",
                "supersede result is not the current accepted corrected generation",
            ));
        }
        let prepared = self.releases.preparations.lock().map_err(|_| unavailable())?
            .get(&preparation_key(original.workspace, &verified.manifest().work, verified.token())).cloned()
            .ok_or_else(|| Error::new("validation_pending", "corrected production candidate must finish its configured preparation before supersede"))?;
        let preparation = prepared.lock().await;
        if preparation.sources.parent.token() != verified.token()
            || preparation
                .repositories
                .values()
                .any(|r| r.phase.is_some() || r.production.is_none())
            || preparation.error.is_some()
        {
            return Err(Error::new(
                "validation_pending",
                "corrected successor production validation is incomplete",
            ));
        }
        let repositories = repository_evidence(&preparation, &workspace.contracts)?;
        // Current proof readback precedes the serialized local-only transition.
        for item in &repositories {
            let observation = workspace
                .coordinator
                .observation(gate.clone(), item.evidence.production().revision().clone())?;
            let fresh = match item.evidence {
                ReleaseStepEvidence::Changed { prepared, .. } => {
                    workspace
                        .coordinator
                        .hydrate_production_validation(&observation, prepared, &verified)
                        .await?
                }
                ReleaseStepEvidence::Existing { baseline, .. } => {
                    workspace
                        .observer
                        .reobserve(item.contract, baseline)
                        .await?;
                    workspace
                        .coordinator
                        .hydrate_existing_production_validation(
                            &observation,
                            &verified,
                            &preparation.sources.integrations,
                            preparation.repositories[&item.contract.repository].operation,
                        )
                        .await?
                }
            };
            if fresh.digest()? != item.evidence.production().digest()? {
                return Err(Error::new(
                    "stale_result",
                    "corrected successor production evidence changed",
                ));
            }
        }
        let selected = workspace
            .configuration
            .workspaces
            .values()
            .find(|w| w.id == original.workspace)
            .ok_or_else(|| Error::new("scope_violation", "successor workspace disappeared"))?;
        let plan = build_release_plan(ReleasePlanInput {
            runtime_id: original.runtime,
            operation_id: preparation.reference.operation,
            workspace: selected,
            parent: &verified,
            acceptance: &acceptance,
            carried: None,
            repositories: &repositories,
            dependencies: self.release_requirements(parent)?,
        })?;
        let baselines = self
            .recovery_baselines(&workspace, &original.attempt.plan)
            .await?;
        let baseline = reconciled_baseline(&baselines, &original.attempt.plan)?;
        let admission = self
            .runtime
            .control
            .operation(original.attempt.plan.operation_id)?;
        let original_digest = original.attempt.plan.digest()?;
        let reason = decision.request().args["reason"]
            .as_str()
            .ok_or_else(|| Error::invalid("recovery reason missing"))?;
        let publications =
            self.publication_refs(&workspace.configuration, parent, &verified, &acceptance)?;
        let checked = self.current_gate();
        self.runtime.control.record_release_recovery_transition(
            &decision,
            &admission,
            &original_digest,
            |snapshot| {
                self.recovery_gate(&decision, snapshot)?;
                self.require_retired_successor_predecessor(&original)?;
                checked(&gate)?;
                current_baselines(&baselines)?;
                self.runtime
                    .current_delivery(&parent.key)?
                    .require_release_selection(
                        verified.token(),
                        acceptance.digest(),
                        acceptance.acceptance().accepted_at,
                    )?;
                let now = self.runtime.observations.admission_snapshot()?;
                if self.publication_refs(
                    &workspace.configuration,
                    now.parent(original.workspace, original.work.parent)?,
                    &verified,
                    &acceptance,
                )? != publications
                {
                    return Err(Error::new(
                        "stale_result",
                        "successor native publications changed",
                    ));
                }
                Ok(())
            },
            |receipt| {
                let intent = SuccessorIntent {
                    schema_version: 1,
                    predecessor: original.attempt.plan.operation_id,
                    predecessor_plan_digest: original_digest.clone(),
                    predecessor_admission_digest: admission.payload_digest.clone(),
                    workspace: original.workspace,
                    work: verified.manifest().work.clone(),
                    plan: plan.clone(),
                    acceptance_digest: acceptance.digest().into(),
                    accepted_at: acceptance.acceptance().accepted_at,
                    config_revision: gate.config_revision().into(),
                    actor: decision.actor().clone(),
                    transition_id: receipt.operation_id,
                    transition_digest: receipt.payload_digest.clone(),
                    parent_command: decision.parent_command().operation_id,
                    parent_command_digest: decision.parent_command().payload_digest.clone(),
                };
                // An orphan intent cannot grant authority: its actual transition
                // must be acknowledged and the predecessor must name this plan.
                let signed = self
                    .runtime
                    .services
                    .key
                    .sign("release_successor_intent", &intent)?;
                let _lock = self.records.lock("release-successors.lock")?;
                match self
                    .records
                    .read::<Signed<SuccessorIntent>>(&leaf(plan.operation_id))
                {
                    Ok(existing) => {
                        self.runtime
                            .services
                            .key
                            .verify("release_successor_intent", &existing)?;
                        if existing.digest()? != signed.digest()? {
                            return Err(Error::new(
                                "operation_conflict",
                                "successor plan already has another immutable authorization",
                            ));
                        }
                    }
                    Err(error) if error.code == "record_missing" => {
                        self.records.write(&leaf(plan.operation_id), &signed)?
                    }
                    Err(error) => return Err(error),
                }
                self.persist_recovery_transition(&original, receipt, "supersede", |record| {
                    record.attempt.supersede_plan(
                        domain::RecoveryDecision {
                            actor: &decision.actor().identity,
                            reason,
                            now: Utc::now(),
                        },
                        &plan,
                        &acceptance,
                        &baseline,
                        |_| Ok(()),
                    )
                })
            },
        )
    }
    fn require_retired_successor_predecessor(&self, record: &AttemptRecord) -> Result<()> {
        let receipt = self.runtime.control.operation(record.admission_operation)?;
        if receipt.kind != "admission"
            || receipt.phase != ReceiptPhase::Reconciled
            || receipt.runtime_id != record.runtime
            || receipt.expected.get("scope")
                != Some(&admission_scope(&receipt, &record.attempt.plan)?)
            || receipt.canonical_targets != vec![issue(record)?.identity()?]
            || receipt.observed_result.as_ref().is_none_or(|v| {
                v["local_execution"] != "exited"
                    || v["unresolved_effects"]
                        .as_array()
                        .is_none_or(|effects| !effects.is_empty())
            })
        {
            return Err(Error::new(
                "reconciliation_required",
                "successor predecessor has no actual terminal admission observation",
            ));
        }
        Ok(())
    }
    pub(in crate::runtime_delivery::release) fn successor_intent(
        &self,
        plan: &ReleasePlan,
        work: &WorkIdentity,
        acceptance: &VerifiedAcceptance,
    ) -> Result<Option<SuccessorIntent>> {
        let signed: Signed<SuccessorIntent> = match self.records.read(&leaf(plan.operation_id)) {
            Ok(signed) => signed,
            Err(error) if error.code == "record_missing" => return Ok(None),
            Err(error) => return Err(error),
        };
        self.runtime
            .services
            .key
            .verify("release_successor_intent", &signed)?;
        let intent = signed.payload;
        let config = self.runtime.configuration.snapshot()?;
        let original = self.runtime.control.operation(intent.predecessor)?;
        let transition = self.runtime.control.operation(intent.transition_id)?;
        let command = self.runtime.control.operation(intent.parent_command)?;
        if intent.schema_version != 1
            || &intent.plan != plan
            || &intent.work != work
            || intent.acceptance_digest != acceptance.digest()
            || intent.accepted_at != acceptance.acceptance().accepted_at
            || intent.config_revision != config.revision()?
            || original.kind != "admission"
            || original.runtime_id != plan.runtime_id
            || original.payload_digest != intent.predecessor_admission_digest
            || original.expected.get("scope") != Some(&intent.predecessor_plan_digest)
            || transition.kind != "release_recovery_transition"
            || transition.runtime_id != plan.runtime_id
            || !matches!(
                transition.phase,
                ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
            )
            || transition.payload_digest != intent.transition_digest
            || transition
                .expected
                .get("recovery_action")
                .map(String::as_str)
                != Some("supersede")
            || transition.expected.get("plan_operation_id") != Some(&intent.predecessor.to_string())
            || transition.expected.get("parent_command_id")
                != Some(&intent.parent_command.to_string())
            || transition.expected.get("parent_payload_digest")
                != Some(&intent.parent_command_digest)
            || command.kind != "command/ReleaseRecover"
            || command.actor != intent.actor.identity
            || command.payload_digest != intent.parent_command_digest
            || transition.actor != intent.actor.identity
            || original.canonical_targets
                != vec![
                    IssueKey::new(&work.linear_endpoint, work.linear_workspace, work.parent)?
                        .identity()?,
                ]
        {
            return Err(Error::new(
                "ownership_conflict",
                "successor lacks exact acknowledged corrected-plan authorization",
            ));
        }
        crate::app::verify_current_actor(&config, &intent.actor, "recover")?;
        self.runtime
            .configuration
            .authority_fences()?
            .check_actor(&intent.actor, "recover")?;
        let state = self.releases.state.lock().map_err(|_| unavailable())?;
        let predecessor = state
            .attempts
            .get(&intent.predecessor)
            .ok_or_else(|| Error::new("operation_missing", "successor predecessor disappeared"))?;
        if predecessor.attempt.plan.digest()? != intent.predecessor_plan_digest
            || predecessor.workspace != intent.workspace
            || predecessor.work.parent != work.parent
            || predecessor.attempt.assigned_successor != Some(plan.operation_id)
            || predecessor.attempt.has_unknown()
            || !predecessor.pending_receipts.is_empty()
            || !predecessor.local_retired
            || predecessor.attempt.resolved
            || predecessor.recovery_receipts.get(&intent.transition_id)
                != Some(&intent.transition_digest)
        {
            return Err(Error::new(
                "recovery_blocked",
                "predecessor hold has not been assigned to this known successor",
            ));
        }
        self.require_retired_successor_predecessor(predecessor)?;
        Ok(Some(intent))
    }
}
