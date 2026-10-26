//! Actual same-process registration observation. Diagnostic JSON never restores
//! an active registration or authorizes worker work.
use super::*;
use crate::evaluation::IndependentBoundaryIdentity;
impl RuntimeCoordinator {
    pub(super) fn observe_registered_independent_boundary(
        &self,
        control: &Arc<ControlHost>,
        permit: &Permit,
        contract: &Arc<WorkerContract>,
    ) -> Result<IndependentBoundaryIdentity> {
        if !Arc::ptr_eq(control, &self.control) {
            return Err(Error::new(
                "worker_binding",
                "independent observation belongs to another Control owner",
            ));
        }
        let mut observed = None;
        self.control.validate_permit(permit, || {
            self.registration.verify_active_owner(&permit.lease)?;
            let (actual, budget) = {
                let state = self.state.lock().map_err(|_| {
                    Error::new("runtime_unavailable", "boundary registration lock failed")
                })?;
                let run = state.runs.get(&permit.operation_id).ok_or_else(|| {
                    Error::new(
                        "worker_binding",
                        "independent observation has no actual registered operation",
                    )
                })?;
                if run.joined
                    || run._active.is_none()
                    || !Arc::ptr_eq(&run.contract, contract)
                    || canonical_digest(1, &run.permit)? != canonical_digest(1, permit)?
                {
                    return Err(Error::new(
                        "worker_binding",
                        "independent observation lost its original active registration",
                    ));
                }
                (run.contract.clone(), run.budget.clone())
            };
            // Independent only: no general delivery evidence, network or nested Control.
            let WorkerAuthority::Independent(original) = actual.authority() else {
                return Err(Error::new(
                    "worker_binding",
                    "registered worker has another authority kind",
                ));
            };
            let request_digest = boundary_model_request(&self.control, permit, &budget)?;
            let current = self.observations.active_independent_gate(original)?;
            observed =
                Some(actual.independent_boundary_identity(permit, request_digest, &current)?);
            Ok(())
        })?;
        observed.ok_or_else(|| {
            Error::new(
                "boundary_observation_unavailable",
                "serialized independent observation was not collected",
            )
        })
    }
}
/// Caller holds the actual permit gate. Reads protected bytes only; no nested
/// snapshot, serialization, raw marker-to-capability conversion or write.
fn boundary_model_request(
    control: &ControlHost,
    permit: &Permit,
    budget: &RunReference,
) -> Result<String> {
    if budget.phase.phase != "independent_review" || budget.phase.retry_grant.is_some() {
        return Err(Error::new(
            "boundary_observation_unavailable",
            "independent diagnostic requires its original ordinary model admission",
        ));
    }
    let expected = crate::control::ordinary_run_request_digest(permit, budget)?;
    let actual = control.operation(permit.operation_id)?;
    if crate::control::receipt_request_digest(&actual)? != expected
        || actual.phase != crate::control::ReceiptPhase::Prepared
        || actual.observed_result.is_some()
    {
        return Err(Error::new(
            "worker_binding",
            "independent immutable model admission changed",
        ));
    }
    Ok(expected)
}
#[cfg(test)]
#[path = "runtime_worker_boundary_tests.rs"]
mod tests;
