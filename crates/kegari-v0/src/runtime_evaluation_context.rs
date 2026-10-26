//! Evaluator-only pathname selection, checked against the retained actual
//! command/source/owner. No public LaunchRequest field or JSON capability.
use super::*;
use crate::evaluation::{
    CanaryIdentity, CanaryOperandBinding, OwnedCanaryPathOperand, PreparedFixture,
};

impl VerifiedEvaluationRequest {
    fn operand_binding(&self, request: &LaunchRequest) -> Result<CanaryOperandBinding> {
        let WorkerAuthority::Independent(gate) = &request.authority else {
            return Err(Error::new(
                "canary_binding",
                "canary operand requires independent authority",
            ));
        };
        let prepared = self.prepared.lock().map_err(|_| {
            Error::new(
                "runtime_unavailable",
                "evaluation source binding lock failed",
            )
        })?;
        let prepared = prepared
            .as_ref()
            .ok_or_else(|| Error::new("canary_binding", "actual evaluation source is absent"))?;
        let profile = self
            .suite
            .profiles
            .iter()
            .find(|p| p.name == self.trial.profile)
            .and_then(|p| p.binding.as_ref())
            .ok_or_else(|| Error::new("canary_binding", "actual evaluation profile is absent"))?;
        let policy = self
            .variant
            .canary_diagnostic
            .as_ref()
            .ok_or_else(|| Error::new("canary_binding", "declared canary policy is absent"))?;
        let source = request
            .source
            .as_ref()
            .ok_or_else(|| Error::new("canary_binding", "actual selected source is absent"))?;
        if !prepared.canary_context_required
            || request.operation_id != self.request.request_id
            || request.lease != self.lease
            || request.phase.phase != "independent_review"
            || request.phase.retry_grant.is_some()
            || request.max_seconds != self.variant.bounds.max_seconds
            || request.max_turns != self.variant.bounds.max_turns
            || &source.revision != prepared.source.head()
            || source.repository != gate.repository_id().to_string()
            || gate.pull_request().base_sha != prepared.source.base().commit
            || gate.pull_request().head_sha != prepared.source.head().commit
            || canonical_digest(1, gate.profile())? != canonical_digest(1, &profile.profile)?
        {
            return Err(Error::new(
                "canary_binding",
                "canary context differs from the exact selected evaluation request",
            ));
        }
        Ok(CanaryOperandBinding {
            identity: CanaryIdentity {
                runtime: self.request.runtime_id,
                workspace: self.workspace,
                command: self.command.operation_id,
                operation: self.request.request_id,
                trial_digest: canonical_digest(1, &self.trial)?,
                suite_digest: self.suite.digest()?,
                fixture_digest: self.variant.fixture_digest.clone(),
                scorer_digest: self.variant.scorer_digest.clone(),
                policy_digest: policy.digest.clone(),
                profile_digest: canonical_digest(1, profile)?,
            },
            command_digest: canonical_digest(1, &self.command)?,
            request_digest: canonical_digest(1, &self.request)?,
            lease_digest: canonical_digest(1, &request.lease)?,
            source_digest: prepared.source.digest().into(),
            source_selection_digest: canonical_digest(1, &request.source)?,
            authority_digest: request.authority.digest()?,
            phase_digest: canonical_digest(1, &request.phase)?,
            profile: gate.profile().clone(),
            head_revision: prepared.source.head().commit.clone(),
            pull_request_digest: canonical_digest(1, gate.pull_request())?,
            max_turns: request.max_turns,
            max_seconds: request.max_seconds,
        })
    }
}
impl RuntimeCoordinator {
    pub(super) fn make_evaluation_context_operand(
        &self,
        proof: &Arc<VerifiedEvaluationRequest>,
        request: &LaunchRequest,
        fixture: &mut PreparedFixture,
    ) -> Result<Option<Arc<OwnedCanaryPathOperand>>> {
        if !fixture.requires_canary_context()? {
            return Ok(None);
        }
        self.control.serialize_observation(|snapshot| {
            proof.current_worker(
                self,
                snapshot,
                request.operation_id,
                &request.authority,
                &request.phase,
                request.max_turns,
            )
        })?;
        let owner = proof
            .canary
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "canary retention lock failed"))?
            .clone()
            .ok_or_else(|| {
                Error::new(
                    "canary_binding",
                    "version 3 requires the original staged canary owner",
                )
            })?;
        let operand = owner.context_operand(proof.operand_binding(request)?)?;
        // Retain the same original owner on both sides of all preparation awaits.
        // File reads happen outside Control; the final callback only checks identities.
        self.control.serialize_observation(|snapshot| {
            proof.current_worker(
                self,
                snapshot,
                request.operation_id,
                &request.authority,
                &request.phase,
                request.max_turns,
            )?;
            let current = self
                .evaluations
                .requests
                .lock()
                .map_err(|_| {
                    Error::new("runtime_unavailable", "evaluation command map lock failed")
                })?
                .get(&request.operation_id)
                .cloned()
                .ok_or_else(|| {
                    Error::new("canary_binding", "actual evaluation request disappeared")
                })?;
            if !Arc::ptr_eq(&current, proof) {
                return Err(Error::new(
                    "canary_binding",
                    "actual evaluation request was replaced",
                ));
            }
            operand.verify_binding(&owner, &proof.operand_binding(request)?)?;
            let mut retained = proof
                .context_operand
                .lock()
                .map_err(|_| Error::new("runtime_unavailable", "canary operand lock failed"))?;
            if retained.is_some() {
                return Err(Error::new(
                    "canary_conflict",
                    "canary context operand cannot be replaced",
                ));
            }
            *retained = Some(operand.clone());
            Ok(())
        })?;
        fixture.attach_canary_operand(operand.clone())?;
        Ok(Some(operand))
    }
    pub(crate) fn verify_evaluation_context_operand(
        &self,
        request: &LaunchRequest,
        operand: Option<&Arc<OwnedCanaryPathOperand>>,
    ) -> Result<()> {
        if let Some(operand) = operand {
            operand.revalidate()?;
        }
        self.control.serialize_observation(|snapshot| {
            self.check_evaluation_context_operand(snapshot, request, operand)
        })
    }
    /// Called only under the caller's Control observation/admission callback.
    pub(crate) fn check_evaluation_context_operand(
        &self,
        snapshot: &crate::control::ControlSnapshot,
        request: &LaunchRequest,
        operand: Option<&Arc<OwnedCanaryPathOperand>>,
    ) -> Result<()> {
        let proof = self
            .evaluations
            .requests
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "evaluation command map lock failed"))?
            .get(&request.operation_id)
            .cloned();
        let Some(proof) = proof else {
            return if operand.is_none() {
                Ok(())
            } else {
                Err(Error::new(
                    "canary_binding",
                    "canary operand lacks actual evaluation request",
                ))
            };
        };
        proof.current_worker(
            self,
            snapshot,
            request.operation_id,
            &request.authority,
            &request.phase,
            request.max_turns,
        )?;
        let required = proof
            .prepared
            .lock()
            .map_err(|_| {
                Error::new(
                    "runtime_unavailable",
                    "evaluation source binding lock failed",
                )
            })?
            .as_ref()
            .is_some_and(|p| p.canary_context_required);
        let retained = proof
            .context_operand
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "canary operand lock failed"))?
            .clone();
        match (required, retained, operand) {
            (false, None, None) => Ok(()),
            (true, Some(actual), Some(supplied)) if Arc::ptr_eq(&actual, supplied) => {
                let owner = proof
                    .canary
                    .lock()
                    .map_err(|_| Error::new("runtime_unavailable", "canary retention lock failed"))?
                    .clone()
                    .ok_or_else(|| Error::new("canary_binding", "original canary owner absent"))?;
                actual.verify_binding(&owner, &proof.operand_binding(request)?)
            }
            _ => Err(Error::new(
                "canary_binding",
                "evaluation context operand was omitted, substituted or unexpectedly supplied",
            )),
        }
    }
    /// Additional exact-contract check inside the existing final model gate.
    /// It uses only retained expectations; observed JSON never supplies a path.
    pub(crate) fn check_evaluation_context_contract(
        &self,
        contract: &WorkerContract,
    ) -> Result<()> {
        let proof = self
            .evaluations
            .requests
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "evaluation command map lock failed"))?
            .get(&contract.instructions().manifest.operation_id)
            .cloned();
        let Some(proof) = proof else {
            return Ok(());
        };
        let required = proof
            .prepared
            .lock()
            .map_err(|_| {
                Error::new(
                    "runtime_unavailable",
                    "evaluation source binding lock failed",
                )
            })?
            .as_ref()
            .is_some_and(|p| p.canary_context_required);
        let operand = proof
            .context_operand
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "canary operand lock failed"))?
            .clone();
        match (required, operand) {
            (true, Some(operand)) => operand.verify_contract(contract),
            (false, None) if contract.context().get("evidence") == Some(&json!({})) => Ok(()),
            _ => Err(Error::new(
                "canary_binding",
                "evaluation contract lacks its original declared context operand",
            )),
        }
    }
}
