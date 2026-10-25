//! Bounded immutable completion inventory and its scoped queue projection.
use super::*;

impl RuntimeDelivery {
    /// Caller holds Control serialization and no release/runtime state guard.
    /// Whole ordinary sync owns that gate across queue and dependency projection.
    pub(in crate::runtime_delivery) fn sync_release_completion_queue_in_observation(
        &self,
    ) -> Result<()> {
        self.runtime.registration.verify_control_owner()?;
        let result = (|| {
            // First authentication and immutable anchor registration share the
            // same gate as completion persistence. Never retain ReleaseState
            // while acquiring the shared RuntimeState anchor map.
            let mut known: BTreeMap<_, _> = self
                .runtime
                .known_release_completion_anchors()?
                .into_iter()
                .map(|anchor| (anchor.operation(), anchor))
                .collect();
            let local = {
                let state = self.releases.state.lock().map_err(|_| unavailable())?;
                state.completion_anchors.clone()
            };
            for (operation, anchor) in local {
                let scope = anchor.queue_scope()?;
                if known
                    .get(&operation)
                    .map(|prior| prior.queue_scope())
                    .transpose()?
                    .is_some_and(|prior| prior != scope)
                {
                    return Err(Error::new(
                        "release_completion",
                        "known completion anchor changed",
                    ));
                }
                known.insert(operation, anchor);
            }
            if known.len() > 4096 {
                return Err(Error::new(
                    "release_completion",
                    "completion anchor inventory exceeds finite bound",
                ));
            }
            // This reader remembers each authenticated scope immediately, so a
            // later global failure cannot erase an earlier observed identity.
            // All work is bounded local file/receipt I/O, with no Control reentry.
            let mut inventory = self.learn_release_completion_queue_inventory(&known)?;
            self.runtime.remember_release_completion_anchors(
                inventory.anchors.values().cloned().collect(),
            )?;
            let mut state = self.releases.state.lock().map_err(|_| unavailable())?;
            // Preserve facade-local immutable identities as well. This never
            // replaces a shared anchor or creates completion from a scope.
            for (operation, anchor) in &state.completion_anchors {
                let observed = inventory.anchors.get(operation);
                if observed
                    .map(|current| current.queue_scope())
                    .transpose()?
                    .as_ref()
                    != Some(&anchor.queue_scope()?)
                {
                    inventory.proofs.remove(operation);
                    inventory.anchors.insert(*operation, anchor.clone());
                    inventory.errors.insert(
                        *operation,
                        Error::new(
                            "release_completion",
                            "completion inventory omitted or replaced a known immutable scope",
                        ),
                    );
                }
            }
            if inventory.anchors.len() > 4096 {
                return Err(Error::new(
                    "release_completion",
                    "completion anchor inventory exceeds finite bound",
                ));
            }
            // A complete read replaces status only for authenticated seals;
            // unrelated never-sealed diagnostics remain available to operators.
            let mut errors = state.completion_errors.clone();
            for operation in inventory.anchors.keys() {
                errors.remove(operation);
            }
            errors.extend(inventory.errors);
            state.completions = inventory.proofs;
            state.completion_anchors = inventory.anchors;
            state.completion_errors = errors;
            state.completion_inventory_error = None;
            let queue = if state.restored {
                rebuild_queue(
                    &state.attempts,
                    &state.no_release,
                    &state.completions,
                    &state.completion_anchors,
                    &state.completion_errors,
                    state.completion_inventory_error.as_ref(),
                    &state.historical_no_release,
                )
            } else {
                Err(Error::new(
                    "release_recovery_pending",
                    "strict release history must be restored before queue eligibility",
                ))
            };
            match queue {
                Ok(queue) => state.queue = queue,
                Err(error) => {
                    state.queue.invalidate_after_rebuild(error.clone());
                    if error.code == "unsafe_storage"
                        || state.restored && state.historical_no_release.require_available().is_ok()
                    {
                        // The outer handler releases this guard before making
                        // the observed global failure visible to every facade.
                        return Err(error);
                    }
                    // Unrestored attempts or independent NR unavailability keep
                    // this queue closed; they do not erase a complete ordinary
                    // historical inventory. NR has its own shared fatal latch.
                }
            }
            drop(state);
            self.runtime.clear_released_inventory_error()
        })();
        if let Err(error) = &result {
            self.runtime.invalidate_released_inventory(error.clone())?;
            let mut state = self.releases.state.lock().map_err(|_| unavailable())?;
            state.completion_inventory_error = Some(error.clone());
            state.queue.invalidate_after_rebuild(error.clone());
        }
        result
    }
}
