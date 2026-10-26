//! Retained source-only import progression. Its signed metadata is descriptive;
//! only the actual CompletedCarryObjects capability can start an acquisition.
use super::*;
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum CarryImportStatus {
    Ready,
    Reading,
    Waiting,
    Imported,
    Failed,
    Unknown,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CarryImportReference {
    operation: Uuid,
    source_digest: String,
    repository: crate::git::RepositoryId,
    max_attempts: u32,
    attempts: u32,
    status: CarryImportStatus,
    not_before: chrono::DateTime<Utc>,
    effect: Option<Uuid>,
    source: Option<String>,
    error: Option<Error>,
}
pub(super) struct PendingCarryImport {
    proof: Arc<CompletedCarryObjects>,
    original: Arc<AcceptedParentOrigin>,
}
impl RuntimeDelivery {
    pub(super) fn retain_carry_import(
        &self,
        workspace: &WorkspaceDelivery,
        parent: &mut ParentPipeline,
        original: Arc<AcceptedParentOrigin>,
        proof: CompletedCarryObjects,
    ) -> Result<()> {
        if parent.phase.is_some() || parent.carry_import.is_some() {
            return Err(Error::new(
                "ownership_conflict",
                "parent already retains an owned carry import",
            ));
        }
        let digest = proof.digest()?;
        // A completed/failed/unknown exact source never acquires a fresh budget
        // by returning to Complete. Every identity remains in signed history.
        if parent
            .reference
            .carry_imports
            .iter()
            .any(|r| r.source_digest == digest)
        {
            return Err(Error::new(
                "reconciliation_required",
                "exact carry source already has a retained import attempt",
            ));
        }
        let operation = Uuid::new_v4();
        parent.reference.carry_imports.push(CarryImportReference {
            operation,
            source_digest: digest,
            repository: proof.repository().repository.clone(),
            max_attempts: workspace.configuration.runtime.max_attempts_per_phase,
            attempts: 0,
            status: CarryImportStatus::Ready,
            not_before: Utc::now(),
            effect: None,
            source: None,
            error: None,
        });
        parent.reference.phase_operation = Some(operation);
        parent.carry_import = Some(PendingCarryImport {
            proof: Arc::new(proof),
            original,
        });
        self.persist_parent(parent)
    }
    pub(super) async fn advance_carry_import(
        &self,
        lease: &Lease,
        workspace: &WorkspaceDelivery,
        gate: &FreshGate,
        parent: &mut ParentPipeline,
    ) -> Result<()> {
        let pending = parent.carry_import.as_ref().ok_or_else(|| {
            Error::new(
                "reconciliation_required",
                "carry import lacks actual source authority",
            )
        })?;
        let proof = pending.proof.clone();
        let original = pending.original.clone();
        let index = parent
            .reference
            .carry_imports
            .len()
            .checked_sub(1)
            .ok_or_else(|| Error::new("record_corrupt", "carry import has no signed attempt"))?;
        let reference = parent.reference.carry_imports[index].clone();
        if reference.source_digest != proof.digest()?
            || reference.repository != proof.repository().repository
            || parent.reference.phase_operation != Some(reference.operation)
            || parent.reference.stage != ParentStage::Complete
        {
            return Err(Error::new(
                "record_corrupt",
                "carry import differs from retained exact parent source",
            ));
        }
        proof.validate(gate)?;
        original.validate(&workspace.configuration)?;
        if reference.status == CarryImportStatus::Imported {
            return self.finish_carry_import(parent, index, None);
        }
        let readback_only = matches!(
            reference.status,
            CarryImportStatus::Reading | CarryImportStatus::Unknown
        );
        if reference.status == CarryImportStatus::Failed {
            return self.finish_carry_import(
                parent,
                index,
                Some(reference.error.unwrap_or_else(|| {
                    Error::new("reconciliation_required", "carry import failed")
                })),
            );
        }
        let max_attempts = reference
            .max_attempts
            .min(workspace.configuration.runtime.max_attempts_per_phase);
        if !readback_only && reference.attempts >= max_attempts {
            return self.finish_carry_import(
                parent,
                index,
                Some(Error::new(
                    "attempts_exhausted",
                    "carry object read budget is exhausted",
                )),
            );
        }
        if Utc::now() < reference.not_before {
            return Ok(());
        }
        if parent.phase.is_none() && readback_only {
            return Err(Error::new(
                "reconciliation_required",
                "uncertain carry import lost its actual owner",
            ));
        }
        if parent.phase.is_none() {
            let repository = proof.repository();
            let revision = proof.current_main();
            let retained = proof.clone();
            let accepted = original.clone();
            let configuration = workspace.configuration.clone();
            let extra: GateCheck = Arc::new(move |gate| {
                retained.validate(gate)?;
                accepted.validate(&configuration)
            });
            let scope = DeliveryScope {
                schema_version: 1,
                runtime_id: lease.runtime_id,
                phase: DeliveryPhase::CandidateAssembly,
                work: gate.work().clone(),
                repository: repository.repository.clone(),
                revision: RevisionBinding {
                    repository: repository.repository.clone(),
                    head: revision.commit.clone(),
                    base: revision.commit.clone(),
                    tested_commit: revision.commit.clone(),
                    tested_tree: revision.tree.clone(),
                },
                fresh_gate_digest: gate.digest()?,
                contract_digest: repository.digest()?,
                candidate_vector_digest: None,
            };
            parent.phase = Some(
                self.admit_delivery_phase_checked(
                    workspace,
                    gate,
                    lease,
                    PhaseRequest {
                        operation: reference.operation,
                        scope,
                    },
                    Some(extra),
                )
                .await?,
            );
        }
        let phase = parent
            .phase
            .as_ref()
            .ok_or_else(|| Error::new("ownership_conflict", "carry import owner unavailable"))?;
        let coordinator = phase.coordinator.clone();
        let admission = phase.admission.clone();
        let request = coordinator.completed_carry_object_request(&admission, &proof)?;
        let effect = request.effect_id()?;
        if reference.effect.is_some_and(|prior| prior != effect) {
            return Err(Error::new(
                "record_corrupt",
                "carry import effect changed across the same retained attempt",
            ));
        }
        // Commit the attempt before any awaited read. A dropped waiter cannot
        // infer known failure or allocate another effect on the next tick.
        if !readback_only {
            parent.reference.carry_imports[index].attempts += 1;
        }
        parent.reference.carry_imports[index].status = CarryImportStatus::Reading;
        parent.reference.carry_imports[index].effect = Some(effect);
        self.persist_parent(parent)?;
        let result = async {
            coordinator
                .reobserve_carry_import_acceptance(&admission, &original, &workspace.configuration)
                .await?;
            if readback_only {
                Box::pin(coordinator.observe_completed_carry_objects(&admission, proof)).await
            } else {
                Box::pin(coordinator.acquire_completed_carry_objects(&admission, proof)).await
            }
        }
        .await;
        match result {
            Ok(source) => {
                parent.reference.carry_imports[index].source = Some(source.digest()?);
                parent.reference.carry_imports[index].status = CarryImportStatus::Imported;
                self.persist_parent(parent)?;
                self.finish_carry_import(parent, index, None)
            }
            Err(error) => {
                let effect_receipt = match self.runtime.control.operation(effect) {
                    Ok(receipt) => Some(receipt),
                    Err(missing) if missing.code == "record_missing" => None,
                    Err(other) => return Err(other),
                };
                let attempts = parent.reference.carry_imports[index].attempts;
                let retry = !readback_only
                    && effect_receipt.is_none()
                    && error.retryable
                    && error.code == "provider_transport"
                    && attempts < max_attempts;
                let unknown = (readback_only && effect_receipt.is_none())
                    || effect_receipt.as_ref().is_some_and(|r| {
                        !matches!(
                            r.phase,
                            crate::control::ReceiptPhase::Acknowledged
                                | crate::control::ReceiptPhase::Reconciled
                        )
                    });
                let record = &mut parent.reference.carry_imports[index];
                record.error = Some(error.clone());
                record.status = if retry {
                    CarryImportStatus::Waiting
                } else if unknown {
                    CarryImportStatus::Unknown
                } else {
                    CarryImportStatus::Failed
                };
                if retry {
                    record.not_before = crate::scheduler::retry_at(
                        Utc::now(),
                        attempts,
                        workspace.configuration.runtime.retry_cap_ms,
                        error
                            .details
                            .get("retry_after_ms")
                            .and_then(serde_json::Value::as_u64),
                    )?;
                }
                self.persist_parent(parent)?;
                if retry || unknown {
                    return Err(error);
                }
                self.finish_carry_import(parent, index, Some(error))
            }
        }
    }
    fn finish_carry_import(
        &self,
        parent: &mut ParentPipeline,
        index: usize,
        error: Option<Error>,
    ) -> Result<()> {
        if let Some(phase) = &parent.phase {
            self.retire_delivery_phase(phase,json!({"carry_object_import":parent.reference.carry_imports[index].source_digest,"source":parent.reference.carry_imports[index].source,"error":error}))?;
        }
        parent.phase = None;
        parent.reference.phase_operation = None;
        if let Some(error) = error {
            parent.reference.carry_imports[index].status = CarryImportStatus::Failed;
            parent.reference.carry_imports[index].error = Some(error.clone());
            parent.reference.error = Some(error.clone());
            parent.reference.stage = ParentStage::Blocked;
            parent.carry_import = None;
            self.persist_parent(parent)?;
            return Err(error);
        }
        parent.carry_import = None;
        parent.reference.error = None;
        self.persist_parent(parent)
    }
}
