//! The carried link is a separately owned, recoverable publication after every
//! repository's actual production validation. It grants no production permit.
use super::*;
impl RuntimeDelivery {
    pub(super) async fn progress_prepared_carry(
        &self,
        lease: Option<&Lease>,
        projection: &RuntimeProjection,
        prepared: &mut Preparation,
    ) -> Result<()> {
        let Some(reassembled) = prepared.sources.reassembled.clone() else {
            return Ok(());
        };
        if prepared
            .repositories
            .values()
            .any(|r| r.phase.is_some() || r.production.is_none())
        {
            return Ok(());
        }
        if let Some(carry) = prepared.carry.clone() {
            // The native publication is already retained. A reporting failure
            // must retry closing its actual owner, never publish a second link.
            let current = if let Some(phase) = prepared.carry_phase.as_ref() {
                self.retire_delivery_phase(
                    phase,
                    json!({"carried_acceptance":carry.verified().record(),"publication":carry.publication()}),
                )?;
                prepared.carry_phase = None;
                self.runtime.observations.refresh().await?
            } else {
                self.runtime.observations.admission_snapshot()?
            };
            let parent =
                current.parent(prepared.reference.workspace, prepared.reference.work.parent)?;
            carry.verify_current_projection(&self.runtime.configuration.snapshot()?, parent)?;
            self.runtime.retain_carried_acceptance(&parent.key, carry)?;
            return Ok(());
        }
        let workspace = if let Some(phase) = &prepared.carry_phase {
            phase.workspace.clone()
        } else {
            Arc::new(self.workspace(projection, prepared.reference.workspace)?)
        };
        let gate = if let Some(phase) = &prepared.carry_phase {
            // The actual active owner keeps its admitted configuration and
            // observation; changed intake filters must not select its context.
            phase.admission.fresh_gate().clone()
        } else {
            let current = self.runtime.observations.admission_snapshot()?;
            let parent =
                current.parent(prepared.reference.workspace, prepared.reference.work.parent)?;
            parent.fresh_gate(
                None,
                GatePurpose::NewOperation,
                &self.runtime.dependency_outcomes()?,
                Utc::now(),
                Duration::seconds(30),
            )?
        };
        if prepared.carry_phase.is_none() {
            let Some(lease) = lease else {
                return Ok(());
            };
            reassembled
                .source()
                .refresh_current(&workspace.configuration)
                .await?;
            let candidate = prepared
                .sources
                .parent
                .manifest()
                .candidates
                .first()
                .ok_or_else(|| {
                    Error::new("candidate_provenance", "carry candidate vector is empty")
                })?;
            let contract = workspace
                .contracts
                .iter()
                .find(|r| r.repository == candidate.repository)
                .ok_or_else(|| {
                    Error::new("scope_violation", "carry source repository disappeared")
                })?;
            let scope = DeliveryScope {
                schema_version: 1,
                runtime_id: lease.runtime_id,
                phase: DeliveryPhase::ParentEvidence,
                work: gate.work().clone(),
                repository: contract.repository.clone(),
                revision: candidate_revision(candidate)?,
                fresh_gate_digest: gate.digest()?,
                contract_digest: contract.digest()?,
                candidate_vector_digest: Some(prepared.sources.vector_digest.clone()),
            };
            let operation = delivery_id(
                lease.runtime_id,
                prepared.reference.operation,
                "carried-acceptance-publication",
            )?;
            prepared.carry_phase = Some(
                self.admit_delivery_phase(
                    &workspace,
                    &gate,
                    lease,
                    PhaseRequest { operation, scope },
                )
                .await?,
            );
        }
        let phase = prepared
            .carry_phase
            .as_ref()
            .expect("retained actual carry phase");
        let repositories = repository_evidence(prepared, &workspace.contracts)?;
        let mut production = Vec::new();
        // Re-read every actual full production proof before producing carry.
        for repository in repositories {
            let observation = phase.coordinator.observation(
                gate.clone(),
                repository.evidence.production().revision().clone(),
            )?;
            let operation = prepared.repositories[&repository.contract.repository].operation;
            let proof = phase
                .coordinator
                .observe_carry_production(
                    &observation,
                    &prepared.sources.parent,
                    repository.evidence,
                    &prepared.sources.integrations,
                    operation,
                )
                .await?;
            production.push(proof);
        }
        let carry = phase
            .coordinator
            .publish_carried_acceptance(
                &phase.admission,
                &reassembled,
                &production,
                &workspace.configuration,
                self.records.clone(),
                &self.active_parent_grant_observer(phase)?,
            )
            .await?;
        // Retain the actual publication before a fallible reporting/refresh step.
        prepared.carry = Some(carry.clone());
        self.parents
            .carries
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "carry registry unavailable"))?
            .insert(
                (
                    prepared.reference.workspace,
                    prepared.reference.work.parent,
                    prepared.reference.work.digest()?,
                ),
                carry.clone(),
            );
        self.retire_delivery_phase(phase,json!({"carried_acceptance":carry.verified().record(),"publication":carry.publication()}))?;
        prepared.carry_phase = None;
        let current = self.runtime.observations.refresh().await?;
        let parent =
            current.parent(prepared.reference.workspace, prepared.reference.work.parent)?;
        self.runtime.retain_carried_acceptance(&parent.key, carry)
    }
    pub(in crate::runtime_delivery) fn retain_current_carry(
        &self,
        workspace: &WorkspaceDelivery,
        parent: &ParentProjection,
        snapshot: &mut VerifiedDeliverySnapshot,
    ) -> Result<()> {
        let Some(current) = snapshot.parent_delivery().verified_parent() else {
            return Ok(());
        };
        if snapshot.parent_delivery().verified_acceptance().is_some() {
            return Ok(());
        }
        let key = (
            parent.workspace_id,
            current.manifest().work.parent,
            current.manifest().work.digest()?,
        );
        let carry = self
            .parents
            .carries
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "carry registry unavailable"))?
            .get(&key)
            .cloned();
        if let Some(carry) = carry {
            if carry.parent().verified().token() != current.token() {
                return Ok(());
            }
            carry.verify_current_projection(&workspace.configuration, parent)?;
            snapshot.carried_acceptance(carry)?;
        }
        Ok(())
    }
}
