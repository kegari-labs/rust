//! Effective correction epochs for existing registered workers. No new operation
//! is admitted here, and installation never rewrites the original contract.
use super::*;
use crate::corrections::{CorrectionPublisher, VerifiedCorrectionTransition};

impl RuntimeCoordinator {
    pub(crate) fn waiting_for_correction(&self, operation: Uuid) -> Result<bool> {
        let expected = {
            let state = self
                .state
                .lock()
                .map_err(|_| Error::new("runtime_unavailable", "correction waiting lock failed"))?;
            let run = state.runs.get(&operation).ok_or_else(|| {
                Error::new(
                    "worker_binding",
                    "correction waiting has no registered worker",
                )
            })?;
            if run.joined || run._active.is_none() {
                return Err(Error::new(
                    "ownership_conflict",
                    "correction waiting lost actual active ownership",
                ));
            }
            let WorkerAuthority::Authorized(original) = run.contract.authority() else {
                return Ok(false);
            };
            if original.work().child.is_none() {
                return Ok(false);
            }
            state
                .correction_transitions
                .get(&operation)
                .map_or(original, |transition| transition.current_authority())
                .clone()
        };
        self.configuration
            .authority_fences()?
            .check_workspace(expected.workspace_id())?;
        let (_, parent) = self.observations.active_correction_context(&expected)?;
        correction_waiting(&expected, &parent, &self.dependency_outcomes()?)
    }
    pub(crate) fn current_correction_transition(
        &self,
        operation: Uuid,
    ) -> Result<Option<Arc<VerifiedCorrectionTransition>>> {
        let transition = {
            let state = self
                .state
                .lock()
                .map_err(|_| Error::new("runtime_unavailable", "correction epoch lock failed"))?;
            let Some(transition) = state.correction_transitions.get(&operation) else {
                return Ok(None);
            };
            let run = state.runs.get(&operation).ok_or_else(|| {
                Error::new(
                    "ownership_conflict",
                    "correction epoch lost its registered admission",
                )
            })?;
            if run.joined
                || run._active.is_none()
                || run.contract.authority().work() != Some(transition.original_work())
            {
                return Err(Error::new(
                    "ownership_conflict",
                    "correction epoch is not bound to an active original worker",
                ));
            }
            transition.clone()
        };
        self.configuration
            .authority_fences()?
            .check_workspace(transition.current_authority().workspace_id())?;
        self.observations
            .active_gate(transition.current_authority(), &self.dependency_outcomes()?)?;
        Ok(Some(transition))
    }
    /// Retain an actual installed correction capability after the owned worker
    /// joins, but authorize its use only against a fresh new-operation gate.
    /// Historical signed records alone cannot reconstruct execution ownership.
    pub fn completed_correction_transition(
        &self,
        completed: &crate::runtime::VerifiedAuthorizedOutcome,
    ) -> Result<Option<Arc<VerifiedCorrectionTransition>>> {
        let Some(expected) = completed.correction_transition_digest() else {
            return Ok(None);
        };
        let operation = completed.outcome().operation_id;
        let transition = {
            let state = self.state.lock().map_err(|_| {
                Error::new("runtime_unavailable", "completed correction lock failed")
            })?;
            let transition = state
                .correction_transitions
                .get(&operation)
                .ok_or_else(|| {
                    Error::new(
                        "reconciliation_required",
                        "completed correction lacks its actual installed execution capability",
                    )
                })?;
            let completion = state.completions.get(&operation).ok_or_else(|| {
                Error::new(
                    "reconciliation_required",
                    "completed correction lacks its owned joined worker",
                )
            })?;
            let contract = completion.contract.as_ref().ok_or_else(|| {
                Error::new(
                    "worker_binding",
                    "completed correction lacks its immutable original contract",
                )
            })?;
            if !completion.local_exit_confirmed
                || !matches!(completion.outcome, crate::worker_run::RunOutcome::Completed)
                || canonical_digest(1, &completion.state)?
                    != canonical_digest(1, &completed.outcome().session)?
                || contract.authority().work() != Some(transition.original_work())
                || transition.digest() != expected
                || transition.original_authority().digest()?
                    != completed.original_authority().digest()?
                || transition.current_authority().digest()?
                    != completed.effective_authority().digest()?
            {
                return Err(Error::new(
                    "worker_binding",
                    "completed correction differs from actual original ownership, terminal session or effective native history",
                ));
            }
            transition.clone()
        };
        let current = self.current_admission_authority(&WorkerAuthority::Authorized(
            transition.current_authority().clone(),
        ))?;
        WorkerAuthority::Authorized(transition.current_authority().clone()).compare(&current)?;
        Ok(Some(transition))
    }
    pub(crate) fn install_correction_transition(
        &self,
        operation: Uuid,
        transition: VerifiedCorrectionTransition,
    ) -> Result<()> {
        let run = self
            .state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "correction registration lock failed"))?
            .runs
            .get(&operation)
            .cloned()
            .ok_or_else(|| {
                Error::new("ownership_conflict", "correction has no registered worker")
            })?;
        self.control.validate_permit(&run.permit, || {
            self.registration.verify_active_owner(&run.permit.lease)?;
            let WorkerAuthority::Authorized(original) = run.contract.authority() else { return Err(Error::new("scope_violation", "correction cannot change independent review authority")); };
            if run.joined || run._active.is_none() || original.digest()? != transition.original_authority().digest()? {
                return Err(Error::new("worker_binding", "correction differs from original admitted worker"));
            }
            transition.revalidate(&self.observations, &self.dependency_outcomes()?)?;
            let mut state = self.state.lock().map_err(|_| Error::new("runtime_unavailable", "correction epoch lock failed"))?;
            if state.runs.get(&operation).is_none_or(|current|current.joined || current._active.is_none()) { return Err(Error::new("ownership_conflict", "worker finished before correction installation")); }
            if state.correction_transitions.get(&operation).map(|p|p.digest()) != transition.previous().map(|p|p.digest()) {
                return Err(Error::new("stale_result", "another correction epoch won the serialized installation"));
            }
            let record = serde_json::json!({"schema_version":1,"runtime":self.runtime_id,"operation":operation,"work":run.permit.work_key,"scope":run.permit.scope_digest,"instructions":run.contract.instructions().manifest_digest,"tool_contract":run.contract.tools().digest(),"transition":transition.record()?});
            let signed = self.services.key.sign("worker_correction_transition", &record)?;
            let leaf = format!("worker-transition-{operation}-{}.json", transition.watermark());
            let bytes = serde_json::to_vec(&signed).map_err(|_| Error::invalid("correction record encoding failed"))?;
            // Durable exact immutable lineage precedes the in-memory transition.
            self.claims.create_bytes_exclusive(&leaf, &bytes)?;
            state.correction_inputs.insert((operation, transition.watermark()), (transition.input_digest(), leaf));
            state.correction_transitions.insert(operation, Arc::new(transition));
            state.correction_errors.remove(&operation);
            if let Some(wake) = state.correction_wakes.get(&operation) { wake.notify_one(); }
            self.services.wake.notify_one();
            Ok(())
        })
    }
    /// Inspect current native facts and update only existing worker epochs. A
    /// failed or ambiguous child stays held without stopping unrelated workers.
    pub(crate) async fn refresh_active_corrections(
        &self,
        publisher: &CorrectionPublisher,
    ) -> Result<()> {
        let runs = self
            .state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "active correction registry failed"))?
            .runs
            .iter()
            .filter(|(_, run)| !run.joined && run._active.is_some())
            .map(|(id, run)| (*id, run.contract.clone()))
            .collect::<Vec<_>>();
        for (operation, contract) in runs {
            let WorkerAuthority::Authorized(original) = contract.authority() else {
                continue;
            };
            if original.work().child.is_none() {
                continue;
            }
            let previous = self
                .state
                .lock()
                .map_err(|_| Error::new("runtime_unavailable", "correction epoch lock failed"))?
                .correction_transitions
                .get(&operation)
                .cloned();
            let expected = previous
                .as_ref()
                .map_or(original, |p| p.current_authority());
            let current = self.observations.active_correction_context(expected);
            // A fresh observation can resolve an informational pending comment
            // without changing correction generation. Wake a quiescent worker
            // as well as workers that will receive an installed successor.
            if current.is_ok()
                && let Some(wake) = self
                    .state
                    .lock()
                    .map_err(|_| Error::new("runtime_unavailable", "correction wake lock failed"))?
                    .correction_wakes
                    .get(&operation)
            {
                wake.notify_one();
            }
            let result = match current {
                Ok((_, parent))
                    if parent.corrections.generation()?
                        == expected.work().generation.correction =>
                {
                    continue;
                }
                Ok((_, parent)) if !parent.corrections.pending_ids().is_empty() => continue,
                Ok(_) => publisher
                    .transition(original, previous, &self.dependency_outcomes()?)
                    .await
                    .and_then(|transition| {
                        self.install_correction_transition(operation, transition)
                    }),
                Err(error) => Err(error),
            };
            if let Err(error) = result {
                self.state
                    .lock()
                    .map_err(|_| {
                        Error::new("runtime_unavailable", "correction diagnostic lock failed")
                    })?
                    .correction_errors
                    .insert(operation, error);
            }
        }
        Ok(())
    }
}

// Observation only: this never creates a gate or authorizes effects. All other
// authority failures remain errors while classification/routing is pending.
fn correction_waiting(
    expected: &crate::reconcile::FreshGate,
    parent: &crate::reconcile::ParentProjection,
    outcomes: &DependencyOutcomes,
) -> Result<bool> {
    let age = Utc::now() - parent.observed_at;
    if age < Duration::zero() || age > Duration::seconds(30) {
        return Err(Error::new(
            "stale_result",
            "correction observation is outside freshness bound",
        ));
    }
    if let Some(reason) = parent.blockers.iter().find(|reason| {
        !matches!(
            reason.code.as_str(),
            "excluded" | "correction_classification_pending"
        )
    }) {
        return Err(Error::new(&reason.code, &reason.reason));
    }
    let child_id = expected.work().child.ok_or_else(|| {
        Error::new(
            "worker_binding",
            "correction waiting requires an admitted child",
        )
    })?;
    let child = parent
        .children
        .get(&child_id)
        .ok_or_else(|| Error::new("scope_violation", "correction child disappeared"))?;
    if let Some(reason) = child.blockers.first() {
        return Err(Error::new(&reason.code, &reason.reason));
    }
    if let Some(reason) = parent
        .dependency_blockers(Some(child_id), outcomes)?
        .first()
    {
        return Err(Error::new(&reason.code, &reason.reason));
    }
    let work = child.work.as_ref().ok_or_else(|| {
        Error::new(
            "authorization_unpublished",
            "correction child lacks current authority",
        )
    })?;
    if !parent.authorization_published
        || parent.workspace_id != expected.workspace_id()
        || parent.required_children != *expected.required_children()
        || work.linear_endpoint != expected.work().linear_endpoint
        || work.linear_workspace != expected.work().linear_workspace
        || work.parent != expected.work().parent
        || work.child != expected.work().child
        || work.generation.authorization != expected.work().generation.authorization
        || work.generation.scope != expected.work().generation.scope
        || child
            .scope
            .as_ref()
            .map(|scope| canonical_digest(1, scope))
            .transpose()?
            .as_deref()
            != expected.child_scope_digest()
    {
        return Err(Error::new(
            "stale_result",
            "correction waiting cannot retain changed authority or permissions",
        ));
    }
    Ok(!parent.corrections.pending_ids().is_empty()
        || work.generation.correction != expected.work().generation.correction)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn second_native_pending_revision_waits_without_authorizing_effects_or_revocation() {
        let (_native, transition, mut parent) =
            crate::corrections::test_pending_transition_fixture().await;
        let expected = transition.current_authority();
        assert!(correction_waiting(expected, &parent, &DependencyOutcomes::default()).unwrap());
        assert_eq!(
            parent
                .fresh_gate(
                    expected.work().child,
                    GatePurpose::DrainExisting,
                    &DependencyOutcomes::default(),
                    Utc::now(),
                    Duration::seconds(30)
                )
                .unwrap_err()
                .code,
            "correction_classification_pending"
        );
        parent.authorization_published = false;
        assert!(correction_waiting(expected, &parent, &DependencyOutcomes::default()).is_err());
        parent.authorization_published = true;
        parent
            .children
            .get_mut(&expected.work().child.unwrap())
            .unwrap()
            .scope
            .as_mut()
            .unwrap()
            .write_paths
            .push("another-directory/".into());
        assert!(correction_waiting(expected, &parent, &DependencyOutcomes::default()).is_err());
    }
}
