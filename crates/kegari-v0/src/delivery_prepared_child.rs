//! An authenticated, unpublished E2E candidate. This preserves preparation
//! across restart without creating a completed child or a worker capability.
use super::*;
use crate::worker_run::RunCompletion;

const PURPOSE: &str = "prepared_child_candidate";
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PreparedChildCheckpoint {
    schema_version: u32,
    workspace: Uuid,
    operation: Uuid,
    publication_operation: Uuid,
    claim_digest: String,
    binding: crate::execution::ExecutionBinding,
    directory_digest: String,
    source_receipt: Uuid,
    receipts: BTreeMap<Uuid, String>,
    result: Signed<ChildResultRecord>,
}
impl PreparedChildCheckpoint {
    pub(crate) fn evidence_digest(&self) -> Result<String> {
        canonical_digest(1, &self.result.payload.evidence)
    }
    pub(crate) fn contract_digest(&self) -> &str {
        &self.result.payload.contract_digest
    }
    pub(crate) fn operation(&self) -> Uuid {
        self.operation
    }
    pub(crate) fn workspace(&self) -> Uuid {
        self.workspace
    }
    pub(crate) fn work(&self) -> &WorkIdentity {
        &self.result.payload.evidence.work
    }
    pub(crate) fn revision(&self) -> &RevisionBinding {
        &self.result.payload.evidence.revision
    }
    pub(crate) fn publication_operation(&self) -> Uuid {
        self.publication_operation
    }
    pub(crate) fn binding(&self) -> &crate::execution::ExecutionBinding {
        &self.binding
    }
    pub(crate) fn directory_digest(&self) -> &str {
        &self.directory_digest
    }
    pub(crate) fn digest(&self) -> Result<String> {
        canonical_digest(1, self)
    }
}
#[derive(Clone)]
pub(crate) struct PreparedChildCandidate {
    checkpoint: PreparedChildCheckpoint,
    staged: Option<StagedDelta>,
}
impl PreparedChildCandidate {
    pub(crate) fn checkpoint(&self) -> &PreparedChildCheckpoint {
        &self.checkpoint
    }
    pub(crate) fn staged(&self) -> Option<&StagedDelta> {
        self.staged.as_ref()
    }
}
pub(crate) enum PreparedChildCompletion<'a> {
    Changed(ChangedChildCompletion<'a>),
    NoChange(NoChangeCompletion<'a>),
}
impl DeliveryCoordinator {
    pub(crate) async fn checkpoint_prepared_child(
        &self,
        admission: &DeliveryAdmission,
        claim: &CompletedChildClaim,
        completion: &Arc<RunCompletion>,
        source: &SealedChildSource,
        workers: [Uuid; 2],
        input: PreparedChildCompletion<'_>,
    ) -> Result<PreparedChildCheckpoint> {
        child::completed_claim_run(claim, completion)?;
        self.current_child_claim(admission, claim)?;
        source.matches_worker(admission, claim)?;
        let execution = completion.execution.as_ref().ok_or_else(|| {
            Error::new(
                "execution_ownership",
                "prepared child lacks actual source ownership",
            )
        })?;
        let contract = completion.contract.as_ref().ok_or_else(|| {
            Error::new(
                "worker_binding",
                "prepared child lacks its original contract",
            )
        })?;
        execution
            .ownership
            .verify(&execution.binding, contract.isolation())?;
        let leaf = format!("prepared-child-{}.json", completion.operation_id);
        if let Some(existing) = self.read_prepared_child(completion.operation_id)? {
            if existing.binding != execution.binding
                || existing.claim_digest != claim.digest()
                || existing.revision() != source.revision()
            {
                return Err(Error::new(
                    "operation_conflict",
                    "prepared child checkpoint changed its original inputs",
                ));
            }
            return Ok(existing);
        }
        let workflow = match &input {
            PreparedChildCompletion::Changed(value) => value.workflow,
            PreparedChildCompletion::NoChange(value) => value.workflow,
        };
        let result = match input {
            PreparedChildCompletion::Changed(value) => {
                self.prepare_changed_child_result(admission, value).await?
            }
            PreparedChildCompletion::NoChange(value) => {
                self.prepare_no_change_child_result(admission, value)
                    .await?
            }
        };
        self.validate_child_draft(
            admission,
            workflow,
            &claim.claim().fulfilled_criteria,
            &result,
        )?;
        let mut receipts = BTreeMap::new();
        for operation in [completion.operation_id, workers[0], workers[1]] {
            let receipt = self.host.operation(operation)?;
            require_completed_preparation(&receipt)?;
            receipts.insert(operation, canonical_digest(1, &receipt)?);
        }
        let receipt = self.host.operation(source.receipt().operation_id)?;
        if canonical_digest(1, &receipt)? != canonical_digest(1, source.receipt())?
            || receipt.phase != crate::control::ReceiptPhase::Acknowledged
        {
            return Err(Error::new(
                "operation_uncertain",
                "prepared source seal changed or remains unresolved",
            ));
        }
        receipts.insert(receipt.operation_id, canonical_digest(1, &receipt)?);
        let checkpoint = PreparedChildCheckpoint {
            schema_version: 1,
            workspace: admission.gate.workspace_id(),
            operation: completion.operation_id,
            publication_operation: Uuid::new_v4(),
            claim_digest: claim.digest().into(),
            binding: execution.binding.clone(),
            directory_digest: execution.ownership.digest().into(),
            source_receipt: receipt.operation_id,
            receipts,
            result,
        };
        self.records
            .write(&leaf, &self.key.sign(PURPOSE, &checkpoint)?)?;
        Ok(checkpoint)
    }
    pub(crate) fn read_prepared_child(
        &self,
        operation: Uuid,
    ) -> Result<Option<PreparedChildCheckpoint>> {
        let signed: Signed<PreparedChildCheckpoint> = match self
            .records
            .read(&format!("prepared-child-{operation}.json"))
        {
            Ok(value) => value,
            Err(error) if error.code == "record_missing" => return Ok(None),
            Err(error) => return Err(error),
        };
        self.key.verify(PURPOSE, &signed)?;
        let value = signed.payload;
        if value.schema_version != 1
            || value.operation != operation
            || value.publication_operation.is_nil()
            || value.receipts.len() < 3
            || value.receipts.len() > 4
            || value.binding.operation_id != operation
            || value.work() != &value.binding.work
            || value.result.payload.runtime_id != value.binding.runtime_id
        {
            return Err(Error::new(
                "record_corrupt",
                "prepared child checkpoint identity is malformed",
            ));
        }
        Ok(Some(value))
    }
    pub(crate) async fn restore_prepared_child(
        &self,
        observation: &DeliveryObservation,
        workflow: &Workflow,
        checkpoint: PreparedChildCheckpoint,
        source: &Path,
    ) -> Result<PreparedChildCandidate> {
        let original = self
            .read_prepared_child(checkpoint.operation)?
            .ok_or_else(|| Error::new("record_missing", "prepared child checkpoint disappeared"))?;
        if original.digest()? != checkpoint.digest()?
            || checkpoint.workspace != observation.gate.workspace_id()
            || checkpoint.work() != observation.gate.work()
            || checkpoint.revision() != &observation.scope.revision
        {
            return Err(Error::new(
                "stale_result",
                "prepared child differs from its original checkpoint/current scope",
            ));
        }
        for (id, digest) in &checkpoint.receipts {
            let receipt = self.host.operation(*id)?;
            if canonical_digest(1, &receipt)? != *digest {
                return Err(Error::new(
                    "record_corrupt",
                    "prepared child retirement or source receipt changed",
                ));
            }
            if *id != checkpoint.source_receipt {
                require_completed_preparation(&receipt)?;
            }
        }
        self.verify_prepared_child_source(
            observation,
            checkpoint.source_receipt,
            &checkpoint.claim_digest,
            &checkpoint.binding,
            source,
        )?;
        self.reobserve_child_draft(
            observation,
            workflow,
            &checkpoint.result.payload.evidence.fulfilled_criteria,
            &checkpoint.result,
        )
        .await?;
        let staged =
            match &checkpoint.result.payload.evidence.outcome {
                evidence::ChildOutcome::NoChange { .. } => None,
                evidence::ChildOutcome::Changed {
                    delta,
                    pull_request,
                } => {
                    let signed: Signed<Value> = self
                        .records
                        .read(&format!("delta-{}.json", &delta.digest()?[7..]))?;
                    self.key.verify("staged_delta", &signed)?;
                    if signed.payload.get("manifest") != Some(&json!(delta)) {
                        return Err(Error::new(
                            "provenance_mismatch",
                            "prepared E2E staged contribution changed",
                        ));
                    }
                    let merge_receipt = serde_json::from_value(
                        signed
                            .payload
                            .get("merge_request")
                            .cloned()
                            .ok_or_else(|| {
                                Error::new("record_corrupt", "prepared E2E merge receipt absent")
                            })?,
                    )
                    .map_err(|_| {
                        Error::new("record_corrupt", "prepared E2E merge receipt malformed")
                    })?;
                    Some(StagedDelta {
                        delta: self.retained_delta(delta)?,
                        pull_request: *pull_request,
                        merge_receipt,
                        protection: checkpoint.result.payload.protection.clone().ok_or_else(
                            || {
                                Error::new(
                                    "provenance_mismatch",
                                    "prepared E2E staging protection absent",
                                )
                            },
                        )?,
                    })
                }
                evidence::ChildOutcome::Replacement { .. } => {
                    return Err(Error::new(
                        "reconciliation_required",
                        "prepared E2E replacement needs its separate original work lineage",
                    ));
                }
            };
        Ok(PreparedChildCandidate { checkpoint, staged })
    }
    pub(crate) async fn publish_prepared_child(
        &self,
        admission: &DeliveryAdmission,
        workflow: &Workflow,
        prepared: &PreparedChildCandidate,
    ) -> Result<PublishedChild> {
        if admission.operation_id() != prepared.checkpoint.publication_operation
            || admission.scope.work != *prepared.checkpoint.work()
            || admission.scope.revision != *prepared.checkpoint.revision()
        {
            return Err(Error::new(
                "scope_violation",
                "prepared publication changed its fixed operation or source",
            ));
        }
        self.reobserve_child_draft(
            admission,
            workflow,
            &prepared
                .checkpoint
                .result
                .payload
                .evidence
                .fulfilled_criteria,
            &prepared.checkpoint.result,
        )
        .await?;
        let mut record = prepared.checkpoint.result.payload.clone();
        record.publication_operation = admission.operation_id();
        let signed = self.key.sign("child_result", &record)?;
        self.publish_child_record(
            admission,
            workflow,
            &record.evidence.fulfilled_criteria,
            signed,
        )
        .await
    }
}
fn require_completed_preparation(receipt: &Receipt) -> Result<()> {
    let value = receipt.observed_result.as_ref().ok_or_else(|| {
        Error::new(
            "operation_uncertain",
            "prepared child lacks retirement observation",
        )
    })?;
    if receipt.phase != crate::control::ReceiptPhase::Reconciled
        || value["local_execution"] != "exited"
        || value["unresolved_effects"]
            .as_array()
            .is_none_or(|a| !a.is_empty())
        || value
            .pointer("/observation/outcome")
            .and_then(Value::as_str)
            != Some("completed")
        || value.pointer("/observation/local_exit_confirmed") != Some(&Value::Bool(true))
        || value
            .pointer("/observation/error")
            .is_none_or(|v| !v.is_null())
        || value
            .pointer("/observation/hook_error")
            .is_none_or(|v| !v.is_null())
        || value
            .pointer("/observation/validation_error")
            .is_some_and(|v| !v.is_null())
    {
        return Err(Error::new(
            "operation_uncertain",
            "prepared child worker is not an actually retired successful run",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prepared_candidate_requires_actual_retirement_before_checkpoint_acceptance() {
        // Actual Control/private storage component. This intentionally does not
        // mint a worker completion or claim native/provider qualification.
        let temp = tempfile::tempdir().unwrap();
        let runtime = Uuid::new_v4();
        let host = crate::control::ControlHost::setup(
            PrivateDir::create(&temp.path().canonicalize().unwrap().join("control")).unwrap(),
            runtime,
            Uuid::new_v4(),
        )
        .unwrap();
        host.lifecycle(
            crate::control::MutationRequest {
                request_id: Uuid::new_v4(),
                runtime_id: runtime,
                actor: kegari_control_api::identity::Actor {
                    identity: "fixture".into(),
                    roles: ["lifecycle".into()].into(),
                    principal: None,
                },
                kind: "start".into(),
                canonical_targets: vec![runtime.to_string()],
                expected: BTreeMap::new(),
                payload: Value::Null,
            },
            crate::control::Lifecycle::Start,
            0,
        )
        .unwrap();
        let lease = kegari_types::control::Lease {
            runtime_id: runtime,
            owner_generation: 1,
            owner_id: Uuid::new_v4(),
        };
        host.attach_reconciled(lease.clone()).unwrap();
        let operation = Uuid::new_v4();
        host.admit(
            &lease,
            operation,
            "prepared child component".into(),
            bytes_digest(b"scope"),
            || Ok(()),
        )
        .unwrap();
        let active = host.operation(operation).unwrap();
        assert!(require_completed_preparation(&active).is_err());
        host.retire_local(operation, json!({"component_only":true,"outcome":"completed","local_exit_confirmed":true,"error":null,"hook_error":null,"validation_error":null})).unwrap();
        let retired = host.operation(operation).unwrap();
        require_completed_preparation(&retired).unwrap();
        assert_ne!(
            canonical_digest(1, &active).unwrap(),
            canonical_digest(1, &retired).unwrap()
        );
        let mut unknown = retired.clone();
        unknown.observed_result.as_mut().unwrap()["unresolved_effects"] = json!([Uuid::new_v4()]);
        assert!(require_completed_preparation(&unknown).is_err());
        let mut failed = retired;
        failed.observed_result.as_mut().unwrap()["observation"]["outcome"] = json!("failed");
        assert!(require_completed_preparation(&failed).is_err());
    }
}
