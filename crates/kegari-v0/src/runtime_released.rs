//! Historical completion credits are independent of current delivery readiness.
//! Every use rechecks the immutable proof and current provider/account visibility.
use super::*;
use crate::{
    configuration::AuthorityFences,
    reconcile::RuntimeProjection,
    runtime_delivery::{
        ReleaseCompletionAnchor, VerifiedNoReleaseCompletion, VerifiedReleaseCompletion,
    },
};
use kegari_config::Config;

const MAX_COMPLETIONS: usize = 4096;

impl RuntimeCoordinator {
    /// Called only after the delivery service has released its own locks. The
    /// inventory contains capabilities recovered from actual signed records;
    /// diagnostics carry no outcome or admission authority.
    #[cfg(test)]
    pub(crate) fn replace_released_completions(
        &self,
        proofs: Vec<VerifiedReleaseCompletion>,
        errors: BTreeMap<Uuid, Error>,
    ) -> Result<()> {
        self.control.serialize_observation(|_| {
            let result = self.replace_released_completions_in_observation(proofs, errors);
            if let Err(error) = &result {
                self.invalidate_released_inventory(error.clone())?;
            }
            result
        })
    }

    /// Caller owns Control serialization. A whole-sync caller must latch any
    /// returned inventory failure before releasing its existing observation.
    pub(crate) fn replace_released_completions_in_observation(
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
        self.registration.verify_control_owner()?;
        let mut retained = BTreeMap::new();
        let mut anchors = BTreeMap::new();
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
            let anchor = proof.queue_anchor()?;
            proof.validate_queue_envelope()?;
            if let Err(error) = proof.validate() {
                if error.code == "unsafe_storage" {
                    return Err(error);
                }
                errors.insert(operation, error);
            }
            anchors.insert(operation, anchor);
            retained.insert(operation, proof);
        }
        let mut state = self.state.lock().map_err(|_| {
            Error::new(
                "runtime_unavailable",
                "released dependency cache lock failed",
            )
        })?;
        // A proof from a replacement file cannot replace a previously
        // authenticated immutable identity, even in a new delivery facade.
        let changed: Vec<_> = anchors
            .iter()
            .filter_map(|(operation, anchor)| {
                state
                    .known_release_completion_anchors
                    .get(operation)
                    .is_some_and(|prior| prior.digest() != anchor.digest())
                    .then_some(*operation)
            })
            .collect();
        for operation in changed {
            anchors.remove(&operation);
            retained.remove(&operation);
            errors.insert(
                operation,
                Error::new(
                    "release_completion",
                    "a known immutable release completion was replaced",
                ),
            );
        }
        merge_release_anchors(&mut state, self.runtime_id, anchors)?;
        for operation in state.known_release_completion_anchors.keys() {
            if !retained.contains_key(operation) {
                errors.entry(*operation).or_insert_with(|| {
                    Error::new(
                        "record_missing",
                        "known release completion is absent from recovered inventory",
                    )
                });
            }
        }
        if errors.len() > MAX_COMPLETIONS {
            return Err(Error::new(
                "release_completion",
                "released dependency errors exceed finite bound",
            ));
        }
        state.released_completions = retained;
        state.released_recovery_errors = errors;
        Ok(())
    }
    /// Immutable identity only. This getter issues no current completion credit
    /// or mutation permit, and does not enter Control serialization.
    pub(crate) fn known_release_completion_anchors(&self) -> Result<Vec<ReleaseCompletionAnchor>> {
        self.registration.verify_control_owner()?;
        let state = self
            .state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "release anchor cache lock failed"))?;
        Ok(state
            .known_release_completion_anchors
            .values()
            .cloned()
            .collect())
    }
    /// Failure-only shared fence. Caller holds Control serialization and no
    /// delivery-state guard; this helper never re-enters Control.
    pub(crate) fn invalidate_released_inventory(&self, error: Error) -> Result<()> {
        self.registration.verify_control_owner()?;
        let mut state = self.state.lock().map_err(|_| {
            Error::new(
                "runtime_unavailable",
                "release dependency cache lock failed",
            )
        })?;
        state.released_inventory_error = Some(error);
        Ok(())
    }

    /// Only an actual complete ordinary inventory under Control may clear this
    /// fence. Replacing cached dependency proofs does not establish completeness.
    pub(crate) fn clear_released_inventory_error(&self) -> Result<()> {
        self.registration.verify_control_owner()?;
        let mut state = self.state.lock().map_err(|_| {
            Error::new(
                "runtime_unavailable",
                "release dependency cache lock failed",
            )
        })?;
        state.released_inventory_error = None;
        Ok(())
    }

    /// A fresh facade must honor an inventory failure learned by another one.
    pub(crate) fn require_released_inventory_available(&self) -> Result<()> {
        self.registration.verify_control_owner()?;
        let state = self.state.lock().map_err(|_| {
            Error::new(
                "runtime_unavailable",
                "release dependency cache lock failed",
            )
        })?;
        match &state.released_inventory_error {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }

    pub(crate) fn remember_release_completion_anchors(
        &self,
        anchors: Vec<ReleaseCompletionAnchor>,
    ) -> Result<()> {
        self.registration.verify_control_owner()?;
        if anchors.len() > MAX_COMPLETIONS {
            return Err(Error::new(
                "release_completion",
                "release anchor inventory exceeds bound",
            ));
        }
        let mut unique = BTreeMap::new();
        for anchor in anchors {
            let operation = anchor.operation();
            if let Some(prior) = unique.get(&operation) {
                let prior: &ReleaseCompletionAnchor = prior;
                if prior.queue_scope()? != anchor.queue_scope()? {
                    return Err(Error::new(
                        "release_completion",
                        "release anchor inventory repeats a changed identity",
                    ));
                }
            }
            unique.insert(operation, anchor);
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "release anchor cache lock failed"))?;
        merge_release_anchors(&mut state, self.runtime_id, unique)
    }
}

impl RuntimeCoordinator {
    /// Read-only immutable anchors survive ordinary cache replacement and a new
    /// delivery facade in this runtime. They are not currently usable outcomes.
    pub(crate) fn known_no_release_completions(&self) -> Result<Vec<VerifiedNoReleaseCompletion>> {
        let state = self
            .state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "no-release seal anchor lock failed"))?;
        Ok(state.known_no_release_seals.values().cloned().collect())
    }

    /// Retain immutable identity only, while the caller holds Control
    /// serialization. No completion credit, current outcome or permit is issued.
    /// This deliberately does not enter Control again: first authenticated reads
    /// and new seal writes must share the producer's existing serialization.
    pub(crate) fn remember_no_release_completion_anchors(
        &self,
        anchors: Vec<VerifiedNoReleaseCompletion>,
    ) -> Result<()> {
        self.registration.verify_control_owner()?;
        if anchors.len() > MAX_COMPLETIONS {
            return Err(Error::new(
                "no_release_completion",
                "no-release anchor inventory exceeds its finite bound",
            ));
        }
        let mut proposed = BTreeMap::new();
        for proof in anchors {
            let operation = proof.operation();
            if proof.runtime_id() != self.runtime_id || operation.is_nil() {
                return Err(Error::new(
                    "identity_mismatch",
                    "no-release anchor belongs to another runtime or operation",
                ));
            }
            if let Some(prior) = proposed.get(&operation) {
                let prior: &VerifiedNoReleaseCompletion = prior;
                if prior.digest() != proof.digest() {
                    return Err(Error::new(
                        "no_release_completion",
                        "no-release anchor inventory repeats a changed seal",
                    ));
                }
            }
            proposed.insert(operation, proof);
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "no-release seal anchor lock failed"))?;
        let mut known = state.no_release_sealed_operations.clone();
        known.extend(state.known_no_release_seals.keys().copied());
        known.extend(proposed.keys().copied());
        if known.len() > MAX_COMPLETIONS {
            return Err(Error::new(
                "no_release_completion",
                "known no-release identity inventory exceeds its finite bound",
            ));
        }
        // Validate the entire set before changing any immutable identity.
        for (operation, proof) in &proposed {
            if let Some(prior) = state.known_no_release_seals.get(operation)
                && prior.digest() != proof.digest()
            {
                return Err(Error::new(
                    "no_release_completion",
                    "a known immutable no-release seal was replaced",
                ));
            }
        }
        for (operation, proof) in proposed {
            state
                .known_no_release_seals
                .entry(operation)
                .or_insert(proof);
        }
        state.no_release_sealed_operations = known;
        Ok(())
    }

    /// A new facade must honor a failure latched by another serialized reader.
    /// This reads only the shared failure flag and issues no proof or permit.
    pub(crate) fn require_no_release_inventory_available(&self) -> Result<()> {
        self.registration.verify_control_owner()?;
        let state = self.state.lock().map_err(|_| {
            Error::new(
                "runtime_unavailable",
                "no-release dependency cache lock failed",
            )
        })?;
        match &state.no_release_inventory_error {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }

    /// Failure-only map update under the caller's Control gate. Preserve every
    /// known anchor and retained proof; ordinary complete replacement is the
    /// only path that may clear this inventory-wide dependency fence.
    pub(crate) fn invalidate_no_release_inventory(&self, error: Error) -> Result<()> {
        self.registration.verify_control_owner()?;
        let mut state = self.state.lock().map_err(|_| {
            Error::new(
                "runtime_unavailable",
                "no-release dependency cache lock failed",
            )
        })?;
        state.no_release_inventory_error = Some(error);
        Ok(())
    }

    /// Caller owns Control serialization. This keeps a complete inventory and
    /// its dependency-cache replacement atomic without re-entering Control.
    pub(crate) fn replace_no_release_completions_in_observation(
        &self,
        proofs: Vec<VerifiedNoReleaseCompletion>,
        anchors: Vec<VerifiedNoReleaseCompletion>,
        mut errors: BTreeMap<Uuid, Error>,
        sealed_operations: BTreeSet<Uuid>,
        inventory_error: Option<Error>,
    ) -> Result<()> {
        if proofs.len() > MAX_COMPLETIONS
            || anchors.len() > MAX_COMPLETIONS
            || errors.len() > MAX_COMPLETIONS
            || sealed_operations.len() > MAX_COMPLETIONS
        {
            return Err(Error::new(
                "no_release_completion",
                "historical no-release dependency inventory exceeds its finite bound",
            ));
        }
        self.registration.verify_control_owner()?;
        // These opaque anchors were authenticated by the real seal reader.
        // Missing immutable refs may deny completion without erasing the
        // original signed identity or allowing a later remint.
        let mut anchored = BTreeMap::new();
        for anchor in anchors {
            anchor.validate_queue_envelope()?;
            let operation = anchor.operation();
            if anchor.runtime_id() != self.runtime_id || !sealed_operations.contains(&operation) {
                return Err(Error::new(
                    "identity_mismatch",
                    "no-release seal anchor has an unbound runtime or identity",
                ));
            }
            if let Some(prior) = anchored.get(&operation) {
                let prior: &VerifiedNoReleaseCompletion = prior;
                if prior.digest() != anchor.digest() {
                    return Err(Error::new(
                        "no_release_completion",
                        "no-release anchor inventory repeats a changed seal",
                    ));
                }
            }
            anchored.insert(operation, anchor);
        }
        let mut retained = BTreeMap::new();
        for proof in proofs {
            let operation = proof.operation();
            if proof.runtime_id() != self.runtime_id
                || !sealed_operations.contains(&operation)
                || anchored
                    .get(&operation)
                    .is_none_or(|anchor| anchor.digest() != proof.digest())
            {
                return Err(Error::new(
                    "identity_mismatch",
                    "historical no-release dependency has an unbound runtime or sealed identity",
                ));
            }
            if let Some(prior) = retained.get(&operation) {
                let prior: &VerifiedNoReleaseCompletion = prior;
                if prior.digest() != proof.digest() {
                    return Err(Error::new(
                        "no_release_completion",
                        "historical inventory repeats a changed completion",
                    ));
                }
                continue;
            }
            proof.validate_queue_envelope()?;
            if let Err(error) = proof.validate() {
                if error.code == "unsafe_storage" {
                    return Err(error);
                }
                errors.insert(operation, error);
            }
            retained.insert(operation, proof);
        }
        let mut state = self.state.lock().map_err(|_| {
            Error::new(
                "runtime_unavailable",
                "historical no-release dependency cache lock failed",
            )
        })?;
        let mut known = state.no_release_sealed_operations.clone();
        known.extend(sealed_operations);
        if known.len() > MAX_COMPLETIONS {
            return Err(Error::new(
                "no_release_completion",
                "known no-release identity inventory exceeds its finite bound",
            ));
        }
        for (operation, proof) in &anchored {
            if let Some(prior) = state.known_no_release_seals.get(operation)
                && prior.digest() != proof.digest()
            {
                return Err(Error::new(
                    "no_release_completion",
                    "a known immutable no-release seal was replaced",
                ));
            }
        }
        for operation in &known {
            if !retained.contains_key(operation) {
                errors.entry(*operation).or_insert_with(|| {
                    inventory_error.clone().unwrap_or_else(|| {
                        Error::new(
                            "record_missing",
                            "known no-release completion is absent from recovered inventory",
                        )
                    })
                });
            }
        }
        for (operation, proof) in &anchored {
            state
                .known_no_release_seals
                .entry(*operation)
                .or_insert_with(|| proof.clone());
        }
        state.no_release_sealed_operations = known;
        state.no_release_completions = retained;
        state.no_release_recovery_errors = errors;
        state.no_release_inventory_error = inventory_error;
        Ok(())
    }
}

/// Pure map update; caller already checked the actual runtime owner. Validate
/// the entire proposed set before changing any retained immutable identity.
fn merge_release_anchors(
    state: &mut State,
    runtime: Uuid,
    anchors: BTreeMap<Uuid, ReleaseCompletionAnchor>,
) -> Result<()> {
    let mut keys: BTreeSet<_> = state
        .known_release_completion_anchors
        .keys()
        .copied()
        .collect();
    keys.extend(anchors.keys().copied());
    if keys.len() > MAX_COMPLETIONS {
        return Err(Error::new(
            "release_completion",
            "known release anchors exceed bound",
        ));
    }
    for (operation, anchor) in &anchors {
        if anchor.runtime_id() != runtime || anchor.operation() != *operation {
            return Err(Error::new(
                "identity_mismatch",
                "release anchor belongs to another runtime or operation",
            ));
        }
        let scope = anchor.queue_scope()?;
        if let Some(prior) = state.known_release_completion_anchors.get(operation)
            && prior.queue_scope()? != scope
        {
            return Err(Error::new(
                "release_completion",
                "a known immutable release anchor was replaced",
            ));
        }
    }
    for (operation, anchor) in anchors {
        state
            .known_release_completion_anchors
            .entry(operation)
            .or_insert(anchor);
    }
    Ok(())
}

pub(super) fn effective_outcomes(
    state: &State,
    config: &Config,
    observed: Result<RuntimeProjection>,
    fences: &AuthorityFences,
) -> (DependencyOutcomes, BTreeMap<Uuid, Error>) {
    let mut outcomes = DependencyOutcomes::default();
    let mut errors = state.released_recovery_errors.clone();
    if let Some(error) = &state.released_inventory_error {
        for operation in state
            .known_release_completion_anchors
            .keys()
            .chain(state.released_completions.keys())
        {
            errors.insert(*operation, error.clone());
        }
        return (outcomes, errors);
    }
    for (operation, proof) in &state.released_completions {
        let result = (|| {
            let projection = observed.as_ref().map_err(Clone::clone)?;
            require_current_observation(
                HistoricalCompletion::Released(proof),
                config,
                projection,
                fences,
            )?;
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

pub(super) fn effective_no_release_outcomes(
    state: &State,
    config: &Config,
    observed: Result<RuntimeProjection>,
    fences: &AuthorityFences,
) -> (DependencyOutcomes, BTreeMap<Uuid, Error>) {
    let mut outcomes = DependencyOutcomes::default();
    let mut errors = state.no_release_recovery_errors.clone();
    if let Some(error) = &state.no_release_inventory_error {
        for operation in &state.no_release_sealed_operations {
            errors.insert(*operation, error.clone());
        }
        return (outcomes, errors);
    }
    // A first producer/read can learn an immutable seal before a complete sync
    // installs dependency credit. Its failure must still close current cached
    // no-release fallback; success here grants no historical outcome.
    for (operation, anchor) in &state.known_no_release_seals {
        if !state.no_release_completions.contains_key(operation)
            && let Err(error) = anchor.validate()
        {
            errors.insert(*operation, error);
        }
    }
    for (operation, proof) in &state.no_release_completions {
        let result = (|| {
            let projection = observed.as_ref().map_err(Clone::clone)?;
            require_current_observation(
                HistoricalCompletion::NoRelease(proof),
                config,
                projection,
                fences,
            )?;
            // This is the sole insertion path. It validates the sealed proof
            // against current configuration and derives the same generation
            // and outcome digest used by the release queue.
            outcomes.historical_no_release(proof, config)
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

enum HistoricalCompletion<'a> {
    Released(&'a VerifiedReleaseCompletion),
    NoRelease(&'a VerifiedNoReleaseCompletion),
}
impl HistoricalCompletion<'_> {
    fn work(&self) -> &crate::evidence::WorkIdentity {
        match self {
            Self::Released(proof) => proof.work(),
            Self::NoRelease(proof) => proof.work(),
        }
    }
    fn workspace_id(&self) -> Uuid {
        match self {
            Self::Released(proof) => proof.workspace_id(),
            Self::NoRelease(proof) => proof.workspace_id(),
        }
    }
    fn dependency_repositories(&self) -> Result<&BTreeSet<kegari_git::RepositoryId>> {
        match self {
            Self::Released(proof) => proof.dependency_repositories(),
            Self::NoRelease(proof) => proof.dependency_repositories(),
        }
    }
}
fn require_current_observation(
    proof: HistoricalCompletion<'_>,
    config: &Config,
    projection: &RuntimeProjection,
    fences: &AuthorityFences,
) -> Result<()> {
    fences.check_workspace(proof.workspace_id())?;
    let now = Utc::now();
    let fresh = |at| at <= now + Duration::seconds(5) && now - at <= Duration::seconds(30);
    if projection.runtime_id != config.runtime.id
        || projection.config_revision != crate::config::config_revision(config)?
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
