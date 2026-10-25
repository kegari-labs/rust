//! Historical no-release recovery is independent of current release readiness.
//! A failed strict queue rebuild preserves ownership while explicitly fencing use.
use super::*;

impl RuntimeDelivery {
    pub(crate) fn sync_no_release_completions(&self) -> Result<()> {
        // The complete read and both installers share one Control boundary.
        // No older successful inventory can clear a newer observed failure.
        self.runtime.control.serialize_observation(|_| {
            self.runtime.registration.verify_control_owner()?;
            let result = (|| match self.no_release_completion_inventory_in_observation() {
                Ok(inventory) => {
                    self.runtime.replace_no_release_completions_in_observation(
                        inventory.proofs.clone(),
                        inventory.anchors.clone(),
                        inventory.errors.clone(),
                        inventory.sealed_operations.clone(),
                        None,
                    )?;
                    self.install_no_release_queue_history_in_observation(
                        inventory.proofs,
                        inventory.anchors,
                        inventory.errors,
                        inventory.sealed_operations,
                        None,
                    )
                }
                Err(error) => {
                    self.runtime.replace_no_release_completions_in_observation(
                        Vec::new(),
                        Vec::new(),
                        BTreeMap::new(),
                        BTreeSet::new(),
                        Some(error.clone()),
                    )?;
                    self.install_no_release_queue_history_in_observation(
                        Vec::new(),
                        Vec::new(),
                        BTreeMap::new(),
                        BTreeSet::new(),
                        Some(error.clone()),
                    )?;
                    Err(error)
                }
            })();
            if let Err(error) = &result {
                // Revalidation can fail after the initial complete inventory.
                // Publish that failure in shared and local state before this
                // same observation releases; no new facade may miss it.
                self.runtime
                    .invalidate_no_release_inventory(error.clone())?;
                let mut state = self.releases.state.lock().map_err(|_| unavailable())?;
                state.historical_no_release.fatal_error = Some(error.clone());
                state.queue.invalidate_after_rebuild(error.clone());
            }
            result
        })
    }
    fn install_no_release_queue_history_in_observation(
        &self,
        proofs: Vec<VerifiedNoReleaseCompletion>,
        anchors: Vec<VerifiedNoReleaseCompletion>,
        errors: BTreeMap<Uuid, Error>,
        sealed_operations: BTreeSet<Uuid>,
        inventory_error: Option<Error>,
    ) -> Result<()> {
        self.runtime.registration.verify_control_owner()?;
        let mut state = self.releases.state.lock().map_err(|_| unavailable())?;
        if let Some(error) = inventory_error {
            // A failed enumeration does not establish an empty history.
            // Keep its error in the rebuild input, not only the old queue.
            state.historical_no_release.fatal_error = Some(error);
        } else {
            match state
                .historical_no_release
                .observed(proofs, anchors, errors, sealed_operations)
            {
                Ok(history) => state.historical_no_release = history,
                Err(error) => {
                    state.historical_no_release.fatal_error = Some(error.clone());
                    state.queue.invalidate_after_rebuild(error.clone());
                    return Err(error);
                }
            }
        }
        // Once a seal has been authenticated, current cached evidence must
        // never substitute for a missing or changed immutable reference.
        let quarantined: BTreeSet<_> = state
            .no_release
            .keys()
            .filter(|operation| state.historical_no_release.is_sealed(operation))
            .copied()
            .collect();
        state
            .no_release
            .retain(|operation, _| !quarantined.contains(operation));
        let queue = state
            .historical_no_release
            .require_available()
            .and_then(|()| {
                if !state.restored {
                    Err(Error::new(
                        "release_recovery_pending",
                        "strict release history must be restored before queue eligibility",
                    ))
                } else {
                    rebuild_queue(
                        &state.attempts,
                        &state.no_release,
                        &state.completions,
                        &state.completion_anchors,
                        &state.completion_errors,
                        state.completion_inventory_error.as_ref(),
                        &state.historical_no_release,
                    )
                }
            });
        match queue {
            Ok(queue) => state.queue = queue,
            Err(error) => {
                state.queue.invalidate_after_rebuild(error.clone());
                if error.code == "unsafe_storage" {
                    state.historical_no_release.fatal_error = Some(error.clone());
                    return Err(error);
                }
            }
        }
        // An unrelated failed strict rebuild must not erase a valid
        // immutable dependency fact or prevent its diagnostic inspection.
        Ok(())
    }
}
