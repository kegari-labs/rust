//! Historical completion credits are independent of current delivery readiness.
//! Every use rechecks the immutable proof and current provider/account visibility.
use super::*;
use crate::{
    config::Config, configuration::AuthorityFences, reconcile::RuntimeProjection,
    runtime_delivery::VerifiedReleaseCompletion,
};

const MAX_COMPLETIONS: usize = 4096;

impl RuntimeCoordinator {
    /// Called only after the delivery service has released its own locks. The
    /// inventory contains capabilities recovered from actual signed records;
    /// diagnostics carry no outcome or admission authority.
    pub(crate) fn replace_released_completions(
        &self,
        proofs: Vec<VerifiedReleaseCompletion>,
        mut errors: BTreeMap<Uuid, Error>,
    ) -> Result<()> {
        if proofs.len() > MAX_COMPLETIONS || errors.len() > MAX_COMPLETIONS {
            return Err(Error::new(
                "release_completion",
                "released dependency inventory exceeds its finite bound",
            ));
        }
        self.control.serialize_observation(|_| {
            self.registration.verify_control_owner()?;
            let mut retained = BTreeMap::new();
            for proof in proofs {
                let operation = proof.terminal().plan.operation_id;
                if proof.terminal().plan.runtime_id != self.runtime_id {
                    return Err(Error::new(
                        "identity_mismatch",
                        "released dependency belongs to another runtime",
                    ));
                }
                if let Some(prior) = retained.get(&operation) {
                    let prior: &VerifiedReleaseCompletion = prior;
                    if prior.digest() != proof.digest() {
                        return Err(Error::new(
                            "release_completion",
                            "released dependency inventory repeats a changed completion",
                        ));
                    }
                    continue;
                }
                // A reference can disappear between disk recovery and this
                // serialized install. Keep its identity and diagnostic; every
                // consumer below still requires the actual proof to validate.
                if let Err(error) = proof.validate() {
                    errors.insert(operation, error);
                }
                retained.insert(operation, proof);
            }
            let mut state = self.state.lock().map_err(|_| {
                Error::new(
                    "runtime_unavailable",
                    "released dependency cache lock failed",
                )
            })?;
            state.released_completions = retained;
            state.released_recovery_errors = errors;
            Ok(())
        })
    }
}

pub(super) fn effective_outcomes(
    state: &State,
    config: &Config,
    observed: Result<RuntimeProjection>,
    fences: &AuthorityFences,
) -> (DependencyOutcomes, BTreeMap<Uuid, Error>) {
    let mut outcomes = DependencyOutcomes::default();
    let mut errors = state.released_recovery_errors.clone();
    for (operation, proof) in &state.released_completions {
        let result = (|| {
            let projection = observed.as_ref().map_err(Clone::clone)?;
            require_current_observation(proof, config, projection, fences)?;
            // This is the sole insertion path. It validates the sealed proof
            // against current configuration and derives the same generation
            // and outcome digest used by the release queue.
            outcomes.released(proof, config)
        })();
        match result {
            Ok(()) => {
                errors.remove(operation);
            }
            Err(error) => {
                errors.insert(*operation, error);
            }
        }
    }
    (outcomes, errors)
}

fn require_current_observation(
    proof: &VerifiedReleaseCompletion,
    config: &Config,
    projection: &RuntimeProjection,
    fences: &AuthorityFences,
) -> Result<()> {
    fences.check_workspace(proof.workspace_id())?;
    let now = Utc::now();
    let fresh = |at| at <= now + Duration::seconds(5) && now - at <= Duration::seconds(30);
    if projection.runtime_id != config.runtime.id
        || projection.config_revision != config.revision()?
        || !fresh(projection.observed_at)
    {
        return Err(Error::new(
            "observation_unavailable",
            "released dependency requires current configured provider observations",
        ));
    }
    let workspace = projection
        .workspaces
        .get(&proof.workspace_id())
        .filter(|workspace| {
            workspace.complete
                && workspace.error.is_none()
                && workspace.config_revision == projection.config_revision
                && fresh(workspace.observed_at)
        })
        .ok_or_else(|| {
            Error::new(
                "incomplete_snapshot",
                "released dependency workspace is absent, stale or incompletely observed",
            )
        })?;
    let work = proof.work();
    let expected = IssueKey::new(&work.linear_endpoint, work.linear_workspace, work.parent)?;
    let mut visible = false;
    for parent in workspace.parents.values() {
        if let Some(observation) = parent.observations.get(&work.parent) {
            let issue = &observation.issue;
            if IssueKey::new(&issue.endpoint, issue.workspace_id, issue.id)? != expected {
                return Err(Error::new(
                    "identity_mismatch",
                    "released prerequisite observation belongs to another native identity",
                ));
            }
            visible = true;
        }
    }
    if !visible {
        return Err(Error::new(
            "missing_dependency",
            "released prerequisite is currently missing or inaccessible",
        ));
    }
    // An unchanged deployment-free repository has no plan step. It remains
    // part of the accepted outcome and must still be currently readable.
    for repository in proof.dependency_repositories()? {
        let observed = repository
            .id
            .parse::<u64>()
            .ok()
            .and_then(|id| workspace.repositories.get(&id));
        if observed.is_none_or(|repository| {
            repository.archived
                || repository.disabled
                || repository.permissions.get("pull").and_then(Value::as_bool) != Some(true)
        }) {
            return Err(Error::new(
                "provider_visibility",
                "released prerequisite repository is no longer readable in its configured scope",
            ));
        }
    }
    // Deliberately no current-generation or current-production equality: the
    // dependent's authenticated pin selects the exact historical generation.
    Ok(())
}
