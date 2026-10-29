//! Native admission adapter for a parent-only released-baseline trigger. All
//! checks remain inside the ordinary capacity/authority model gate; no receipt
//! field can substitute for the retained source and actual joined or sealed retired predecessor.
use super::*;
use crate::delivery::ParentReassemblyWork;
impl RuntimeCoordinator {
    pub(crate) fn next_attempt_for_parent_reassembly(
        &self,
        request: &crate::runtime_workers::LaunchRequest,
    ) -> Result<crate::runtime_schedule::RunSchedule> {
        request.validate_binding()?;
        let proof = request
            .delivery
            .as_ref()
            .and_then(|a| a.parent_reassembly())
            .ok_or_else(|| {
                Error::new(
                    "parent_reassembly_required",
                    "parent dispatch requires its exact assignment cause",
                )
            })?;
        let config = self.configuration.snapshot()?;
        let workspace = config
            .workspaces
            .values()
            .find(|w| w.id == proof.authority().workspace_id())
            .ok_or_else(|| Error::new("scope_violation", "parent workspace disappeared"))?;
        self.schedule_parent_reassembly(
            proof,
            workspace.id,
            &config.connections[&workspace.codex_connection].account_id,
            None,
        )
    }
    pub(super) fn schedule_parent_reassembly(
        &self,
        proof: &ParentReassemblyWork,
        workspace: Uuid,
        account: &str,
        locked: Option<&crate::control::ControlSnapshot>,
    ) -> Result<crate::runtime_schedule::RunSchedule> {
        let config = self.configuration.snapshot()?;
        let ws = config
            .workspaces
            .values()
            .find(|w| w.id == workspace)
            .ok_or_else(|| Error::new("scope_violation", "parent workspace disappeared"))?;
        if proof.reference().runtime != config.runtime.id
            || proof.authority().workspace_id() != workspace
            || config.connections[&ws.codex_connection].account_id != account
            || proof.reference().account != account
        {
            return Err(Error::new(
                "identity_mismatch",
                "parent readmission changed runtime/workspace/account",
            ));
        }
        let observed;
        let snapshot = match locked {
            Some(s) => s,
            None => {
                observed = self.control.snapshot()?;
                &observed
            }
        };
        let parent_key = proof.parent_authority().work_key()?;
        if snapshot
            .active_operations
            .iter()
            .any(|p| p.work_key == proof.phase().work || p.work_key == parent_key)
            || snapshot.unresolved_operations.values().any(|targets| {
                targets.contains(&parent_key) || targets.contains(&proof.phase().work)
            })
        {
            return Err(Error::new(
                "reconciliation_required",
                "parent reassembly still has active or unresolved canonical work",
            ));
        }
        self.require_provider_session_clear(account, &proof.phase().work)?;
        self.current_admission_authority(&WorkerAuthority::Authorized(proof.authority().clone()))?;
        self.current_admission_authority(&WorkerAuthority::Authorized(
            proof.parent_authority().clone(),
        ))?;
        let receipts = self.control.operations()?;
        crate::runtime_schedule::require_provider_clear(
            config.runtime.id,
            account,
            &proof.phase().work,
            &receipts,
        )?;
        let grants = crate::runtime_workflow::current_retry_grants(
            &config,
            &self.observations.admission_snapshot()?,
            proof.authority().work(),
            &receipts,
            &self.services.key,
        )?;
        proof.verify_current(&config, snapshot, &receipts, &grants)?;
        proof.verify_joined(&|id| self.retained_completion(id), snapshot)?;
        crate::runtime_schedule::plan_parent_reassembly(
            &config.runtime,
            proof.reference(),
            &receipts,
            Utc::now(),
        )
    }
    pub(crate) fn admit_parent_reassembly_worker(
        &self,
        input: crate::control::RunAdmission,
        contract: &WorkerContract,
        request: &crate::runtime_workers::LaunchRequest,
        preparation: &crate::control::Receipt,
    ) -> Result<Permit> {
        request.validate_binding()?;
        let proof = request
            .delivery
            .as_ref()
            .and_then(|a| a.parent_reassembly())
            .ok_or_else(|| {
                Error::new(
                    "parent_reassembly_required",
                    "parent run has no sealed assignment cause",
                )
            })?;
        if input.request_id != request.operation_id || input.reference.phase != request.phase {
            return Err(Error::new(
                "worker_binding",
                "parent run differs from prepared request",
            ));
        }
        request.authority.compare(contract.authority())?;
        self.admit_worker_context(
            &request.lease,
            input,
            contract,
            WorkerAdmissionContext {
                planning: WorkerPlanning {
                    readmission: None,
                    round: None,
                },
                session_resume: None,
                paired_resume: None,
                preparation: Some(crate::control::ReviewReceiptReference {
                    operation: preparation.operation_id,
                    receipt_digest: canonical_digest(1, preparation)?,
                }),
                parent_reassembly: Some(proof),
            },
        )
    }
}
