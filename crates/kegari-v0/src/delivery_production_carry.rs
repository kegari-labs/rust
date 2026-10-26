//! A freshly reobserved full production gate together with its exact closed
//! owned preparation/history. This does not infer deployment or acceptance.
use super::*;
#[derive(Clone)]
pub(crate) struct CarryProduction {
    proof: evidence::VerifiedProduction,
    references: crate::delivery::CarryReferences,
}
impl CarryProduction {
    pub(crate) fn proof(&self) -> &evidence::VerifiedProduction {
        &self.proof
    }
    pub(crate) fn references(&self) -> &crate::delivery::CarryReferences {
        &self.references
    }
}
impl DeliveryCoordinator {
    pub(crate) async fn observe_carry_production(
        &self,
        observation: &DeliveryObservation,
        parent: &evidence::VerifiedParent,
        input: ReleaseStepEvidence<'_>,
        integrations: &[crate::git::VerifiedProductionIntegration],
        operation: Uuid,
    ) -> Result<CarryProduction> {
        self.current(
            observation,
            &[DeliveryPhase::Recovery, DeliveryPhase::Release],
        )?;
        let proof = match input {
            ReleaseStepEvidence::Changed { prepared, .. } => {
                if prepared.operation_id() != operation {
                    return Err(production_error("carry production operation differs"));
                }
                self.hydrate_production_validation(observation, prepared, parent)
                    .await?
            }
            ReleaseStepEvidence::Existing { .. } => {
                self.hydrate_existing_production_validation(
                    observation,
                    parent,
                    integrations,
                    operation,
                )
                .await?
            }
        };
        if proof.digest()? != input.production().digest()? {
            return Err(production_error(
                "carry production changed during observation",
            ));
        }
        let owner = self.host.operation(operation)?;
        let challenge_leaf = format!("hosted-challenge-{operation}.json");
        let challenge: Signed<HostedChallenge> = self.records.read(&challenge_leaf)?;
        self.key.verify("hosted_challenge", &challenge)?;
        let expected = &challenge.payload;
        if owner.kind != "admission"
            || owner.runtime_id != observation.scope.runtime_id
            || owner.phase != crate::control::ReceiptPhase::Reconciled
            || owner.observed_result.as_ref().is_none_or(|v| {
                v["local_execution"] != "exited"
                    || v["unresolved_effects"]
                        .as_array()
                        .is_none_or(|v| !v.is_empty())
            })
            || owner.canonical_targets != [observation.gate.work_key()?]
            || expected.schema_version != 1
            || expected.runtime_id != owner.runtime_id
            || expected.operation_id != operation
            || expected.challenge.is_nil()
            || expected.source_sha != proof.revision().tested_commit
            || owner.expected.get("scope") != Some(&expected.admission_digest)
            || [
                "run",
                "metadata",
                "worker_preparation",
                "continuation",
                "source_advanced",
                "review_round",
                "review_round_preparation",
                "review_round_resume",
                "review_round_recovery",
                "parent_reassembly",
                "session_resume",
                "session_recovery_preparation",
                "release_resume",
            ]
            .iter()
            .any(|k| owner.expected.contains_key(*k))
        {
            return Err(production_error(
                "carry production lacks exact closed actual preparation/challenge",
            ));
        }
        use crate::delivery::{CarryNamespace, CarryReferences};
        let workspace = observation.gate.workspace_id();
        let mut references = CarryReferences::default();
        references.retain_file(
            CarryNamespace::Workspace { workspace },
            &self.records,
            &challenge_leaf,
            "hosted_challenge",
            &self.key,
        )?;
        references.retain_receipt(&owner)?;
        match input {
            ReleaseStepEvidence::Changed { prepared, .. } => {
                if prepared.signed.payload.scope.digest()? != expected.admission_digest
                    || prepared.signed.payload.evidence_token != parent.token()
                {
                    return Err(production_error(
                        "carry production candidate differs from exact preparation",
                    ));
                }
                references.retain_file(
                    CarryNamespace::Workspace { workspace },
                    &self.records,
                    &production_leaf(operation),
                    "production_candidate",
                    &self.key,
                )?;
                references.retain_file(
                    CarryNamespace::Workspace { workspace },
                    &self.records,
                    &format!("production-validation-{operation}.json"),
                    "production_validation",
                    &self.key,
                )?;
            }
            ReleaseStepEvidence::Existing { .. } => {
                let validation: Signed<ChildValidationRecord> = self
                    .records
                    .read(&format!("existing-hosted-{operation}.json"))?;
                self.key.verify("existing_hosted_validation", &validation)?;
                if validation.payload.scope_digest != expected.admission_digest {
                    return Err(production_error(
                        "unchanged carry validation belongs to another preparation",
                    ));
                }
                references.retain_file(
                    CarryNamespace::Workspace { workspace },
                    &self.records,
                    &format!("existing-hosted-{operation}.json"),
                    "existing_hosted_validation",
                    &self.key,
                )?;
            }
        }
        for receipt in self
            .host
            .operations()?
            .iter()
            .filter(|r| r.expected.get("admission_operation") == Some(&operation.to_string()))
        {
            if !matches!(
                receipt.phase,
                crate::control::ReceiptPhase::Acknowledged
                    | crate::control::ReceiptPhase::Reconciled
            ) {
                return Err(production_error(
                    "carry production retains an unresolved owned effect",
                ));
            }
            references.retain_receipt(receipt)?;
        }
        self.current(
            observation,
            &[DeliveryPhase::Recovery, DeliveryPhase::Release],
        )?;
        Ok(CarryProduction { proof, references })
    }
}
