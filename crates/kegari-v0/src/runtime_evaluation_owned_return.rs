//! Cleanup owners are produced by the same preparation/worker invocation. The
//! host never infers a return from absent receipts or an empty registry.
use super::*;
use crate::runtime_workers::{EvaluationWorkerTransfer, PreparationReturned};
use crate::worker_run::{
    AdmittedTrial, OwnedTrialReturn, RetainedTrialCompletion, VerifiedUnusedAdmission,
};

pub(super) enum CleanupOwner {
    Preparation(Box<PreparationReturned>),
    Transfer(Box<EvaluationWorkerTransfer>),
    Trial(Box<AdmittedTrial>),
    Unused(Box<VerifiedUnusedAdmission>),
    Completion(RetainedTrialCompletion),
}
#[derive(Default)]
pub(super) enum CleanupState {
    #[default]
    Empty,
    Ready(Box<CleanupOwner>),
    Running,
    Settled(Box<CleanupExit>),
    Closed(EvaluationTerminal),
    // An impossible duplicate handoff is a held invariant failure. Retain both
    // owners rather than destroying either while reporting the error.
    Conflict {
        _prior: Box<CleanupState>,
        _incoming: Box<CleanupState>,
    },
}
pub(super) enum CleanupExit {
    Preparation(Box<PreparationReturned>),
    Unused(Box<VerifiedUnusedAdmission>),
    Completion(RetainedTrialCompletion),
}
#[derive(Clone, Copy)]
pub(super) enum EvaluationTerminal {
    Completed {
        operation: Uuid,
    },
    Unused {
        operation: Uuid,
        receipt_operation: Uuid,
    },
}
impl EvaluationTerminal {
    pub(super) fn operation(&self) -> Uuid {
        match self {
            Self::Completed { operation } => *operation,
            Self::Unused { operation, .. } => *operation,
        }
    }
    pub(super) fn receipt_operation(&self) -> Uuid {
        match self {
            Self::Completed { operation } => *operation,
            Self::Unused {
                receipt_operation, ..
            } => *receipt_operation,
        }
    }
}
impl CleanupExit {
    fn terminal(&self) -> EvaluationTerminal {
        match self {
            Self::Preparation(owner) => EvaluationTerminal::Unused {
                operation: owner.operation(),
                receipt_operation: owner.receipt_operation(),
            },
            Self::Unused(owner) => EvaluationTerminal::Unused {
                operation: owner.operation(),
                receipt_operation: owner.operation(),
            },
            Self::Completion(value) => EvaluationTerminal::Completed {
                operation: value.completion().operation_id,
            },
        }
    }
}
impl VerifiedEvaluationRequest {
    pub(super) fn retain_cleanup(&self, owner: CleanupOwner) -> Result<()> {
        store_transition(
            &self.cleanup,
            CleanupState::Ready(Box::new(owner)),
            |state| matches!(state, CleanupState::Empty),
            conflict,
        )
    }

    pub(super) fn cleanup_pending(&self) -> Result<bool> {
        Ok(matches!(
            *self.cleanup.lock().map_err(|_| unavailable())?,
            CleanupState::Ready(_)
                | CleanupState::Running
                | CleanupState::Settled(_)
                | CleanupState::Conflict { .. }
        ))
    }
}
impl RuntimeCoordinator {
    /// This does no launch, hooks or grading. Running remains counted while the
    /// owner is temporarily outside its slot; concurrent drains cannot claim exit.
    pub(super) fn settle_evaluation_owner(
        &self,
        proof: &VerifiedEvaluationRequest,
    ) -> Result<Option<EvaluationTerminal>> {
        let owner = {
            let mut slot = proof.cleanup.lock().map_err(|_| unavailable())?;
            match &*slot {
                CleanupState::Empty | CleanupState::Running => return Ok(None),
                CleanupState::Conflict { .. } => {
                    return Err(Error::new(
                        "ownership_conflict",
                        "evaluation retains conflicting cleanup owners",
                    ));
                }
                CleanupState::Closed(terminal) => return Ok(Some(*terminal)),
                CleanupState::Settled(exit) => return Ok(Some(exit.terminal())),
                CleanupState::Ready(_) => (),
            }
            match std::mem::replace(&mut *slot, CleanupState::Running) {
                CleanupState::Ready(owner) => *owner,
                _ => unreachable!("selected actual cleanup owner"),
            }
        };
        let (state, result) = self.advance_evaluation_cleanup(owner);
        store_transition(
            &proof.cleanup,
            state,
            |state| matches!(state, CleanupState::Running),
            conflict,
        )?;
        result
    }
    fn advance_evaluation_cleanup(
        &self,
        mut owner: CleanupOwner,
    ) -> (CleanupState, Result<Option<EvaluationTerminal>>) {
        loop {
            match owner {
                CleanupOwner::Transfer(transfer) => match transfer.return_unused() {
                    Ok(unused) => owner = CleanupOwner::Unused(Box::new(unused)),
                    Err((transfer, error)) => {
                        return held(CleanupOwner::Transfer(transfer), Err(error));
                    }
                },
                CleanupOwner::Trial(trial) => match (*trial).settle_owned_return() {
                    OwnedTrialReturn::Completed(completion) => {
                        owner = CleanupOwner::Completion(RetainedTrialCompletion::from_verified(
                            *completion,
                        ))
                    }
                    OwnedTrialReturn::Unused(unused) => owner = CleanupOwner::Unused(unused),
                    OwnedTrialReturn::Held { trial, error } => {
                        return held(CleanupOwner::Trial(trial), Err(error));
                    }
                },
                CleanupOwner::Completion(completion) => {
                    match self.finish_trial_completion(&completion, true) {
                        Ok(_) => return settled(CleanupExit::Completion(completion)),
                        Err(error) => {
                            return held(CleanupOwner::Completion(completion), Err(error));
                        }
                    }
                }
                CleanupOwner::Unused(unused) => {
                    let result = unused.retire().and_then(|closed| {
                        let Some(closed) = closed else {
                            return Ok(false);
                        };
                        self.clear_unused_registration(
                            unused.operation(),
                            Some((unused.permit(), unused.contract(), unused.budget())),
                            Some(&closed),
                        )?;
                        Ok(true)
                    });
                    match result {
                        Ok(true) => return settled(CleanupExit::Unused(unused)),
                        Ok(false) => return held(CleanupOwner::Unused(unused), Ok(None)),
                        Err(error) => return held(CleanupOwner::Unused(unused), Err(error)),
                    }
                }
                CleanupOwner::Preparation(mut preparation) => {
                    let result = preparation.retire(self).and_then(|closed| {
                        if !closed {
                            return Ok(false);
                        }
                        let receipt = preparation
                            .model()
                            .map(|_| self.control.operation(preparation.operation()))
                            .transpose()?;
                        self.clear_unused_registration(
                            preparation.operation(),
                            preparation.model(),
                            receipt.as_ref(),
                        )?;
                        Ok(true)
                    });
                    match result {
                        Ok(true) => return settled(CleanupExit::Preparation(preparation)),
                        Ok(false) => return held(CleanupOwner::Preparation(preparation), Ok(None)),
                        Err(error) => {
                            return held(CleanupOwner::Preparation(preparation), Err(error));
                        }
                    }
                }
            }
        }
    }
    // Called only after an actual owned return and its exact durable retirement.
    // Keep the consumed run/attempt inventory; no joined/completion is invented.
    fn clear_unused_registration(
        &self,
        operation: Uuid,
        original: Option<(&Permit, &Arc<crate::worker::WorkerContract>, &RunReference)>,
        closed: Option<&Receipt>,
    ) -> Result<()> {
        self.control.serialize_observation(|snapshot| {
            self.registration.verify_control_owner()?;
            if snapshot
                .active_operations
                .iter()
                .any(|permit| permit.operation_id == operation)
                || snapshot.unresolved_operations.contains_key(&operation)
                || self.executions.active_operations()?.contains(&operation)
            {
                return Err(Error::new(
                    "reconciliation_required",
                    "unused worker retains actual execution or effects",
                ));
            }
            if let (Some((permit, _, budget)), Some(closed)) = (original, closed) {
                let observed = closed
                    .observed_result
                    .as_ref()
                    .and_then(|value| value.get("observation"))
                    .ok_or_else(|| {
                        Error::new("record_corrupt", "unused return has no observation")
                    })?;
                let actual = self.control.operation(operation)?;
                if canonical_digest(1, &actual)? != canonical_digest(1, closed)?
                    || !closed_model_observation(&actual, permit, budget, observed)?
                {
                    return Err(Error::new(
                        "reconciliation_required",
                        "unused worker retirement changed",
                    ));
                }
            }
            let mut state = self.state.lock().map_err(|_| unavailable())?;
            if state.completions.contains_key(&operation) {
                return Err(Error::new(
                    "ownership_conflict",
                    "unused return has a model completion",
                ));
            }
            if let Some(run) = state.runs.get_mut(&operation) {
                let Some((permit, contract, budget)) = original else {
                    return Err(Error::new(
                        "ownership_conflict",
                        "pre-admission return has a registered model",
                    ));
                };
                if run.joined
                    || !Arc::ptr_eq(&run.contract, contract)
                    || canonical_digest(1, &run.budget)? != canonical_digest(1, budget)?
                    || run.permit.operation_id != permit.operation_id
                    || run.permit.lease != permit.lease
                    || run.permit.scope_digest != permit.scope_digest
                    || run.permit.work_key != permit.work_key
                {
                    return Err(Error::new(
                        "worker_binding",
                        "unused return is not the actual registered model",
                    ));
                }
                run._active = None;
            }
            state.prepared_active.remove(&operation);
            Ok(())
        })
    }
    /// A separate local namespace exit. It consumes no model completion and is
    /// available only after the original task joined and its retained owner closed.
    pub(super) fn close_returned_evaluation(
        &self,
        proof: &VerifiedEvaluationRequest,
    ) -> Result<bool> {
        let operation = proof.request.request_id;
        if self.evaluations.tasks.joined_result(operation)?.is_none() {
            return Ok(false);
        }
        let exit = {
            let mut slot = proof.cleanup.lock().map_err(|_| unavailable())?;
            match &*slot {
                CleanupState::Closed(EvaluationTerminal::Unused { .. }) => return Ok(true),
                CleanupState::Settled(_) => (),
                _ => return Ok(false),
            }
            match std::mem::replace(&mut *slot, CleanupState::Running) {
                CleanupState::Settled(exit) => exit,
                _ => unreachable!("selected positively settled return"),
            }
        };
        let result = self.close_evaluation_return_namespace(proof, &exit);
        let terminal = exit.terminal();
        let unused = matches!(terminal, EvaluationTerminal::Unused { .. });
        let state = match &result {
            Ok(()) => CleanupState::Closed(terminal),
            Err(_) => CleanupState::Settled(exit),
        };
        store_transition(
            &proof.cleanup,
            state,
            |state| matches!(state, CleanupState::Running),
            conflict,
        )?;
        result.map(|()| unused)
    }
    fn close_evaluation_return_namespace(
        &self,
        proof: &VerifiedEvaluationRequest,
        exit: &CleanupExit,
    ) -> Result<()> {
        let source = proof
            .source
            .lock()
            .map_err(|_| unavailable())?
            .clone()
            .ok_or_else(|| {
                Error::new(
                    "ownership_conflict",
                    "returned worker lost activated source",
                )
            })?;
        let operation = proof.request.request_id;
        let preparation = crate::control::preparation_operation(operation)?;
        let ids = [source.metadata_operation(), preparation, operation];
        for id in ids {
            self.executions.observe_exited(id)?;
        }
        self.control.serialize_observation(|snapshot| {
            self.registration.verify_control_owner()?;
            if self.evaluations.tasks.joined_result(operation)?.is_none()
                || exit.terminal().operation() != operation
            {
                return Err(Error::new(
                    "ownership_conflict",
                    "returned worker differs from joined task",
                ));
            }
            let actual_command = self.control.operation(proof.command.operation_id)?;
            if actual_command.runtime_id != proof.command.runtime_id
                || actual_command.owner_generation != proof.command.owner_generation
                || actual_command.request_id != proof.command.request_id
                || actual_command.kind != proof.command.kind
                || actual_command.actor != proof.command.actor
                || actual_command.payload_digest != proof.command.payload_digest
                || actual_command.expected != proof.command.expected
                || actual_command.canonical_targets != proof.command.canonical_targets
            {
                return Err(Error::new(
                    "record_corrupt",
                    "returned worker command identity changed",
                ));
            }
            source.verify_closed_metadata(self)?;
            let active = self.executions.active_operations()?;
            let namespace = format!(
                "evaluation:{}:{}",
                proof.configuration.runtime.id, proof.workspace
            );
            let work = match exit {
                CleanupExit::Preparation(value) => Some(value.work().to_owned()),
                CleanupExit::Unused(value) => Some(value.permit().work_key.clone()),
                CleanupExit::Completion(value) => value
                    .completion()
                    .contract
                    .as_ref()
                    .map(|contract| contract.work_key())
                    .transpose()?,
            };
            if ids.iter().any(|id| active.contains(id))
                || snapshot
                    .active_operations
                    .iter()
                    .any(|permit| ids.contains(&permit.operation_id))
                || snapshot.unresolved_operations.iter().any(|(id, targets)| {
                    ids.contains(id)
                        || targets.contains(&namespace)
                        || work.as_ref().is_some_and(|work| targets.contains(work))
                })
            {
                return Err(Error::new(
                    "reconciliation_required",
                    "returned evaluation retains related effects",
                ));
            }
            match exit {
                CleanupExit::Preparation(value) => value.verify_closed(self)?,
                CleanupExit::Unused(value) => value.verify_closed()?,
                CleanupExit::Completion(verified) => {
                    let completion = verified.completion();
                    let state = self.state.lock().map_err(|_| unavailable())?;
                    let cached = state.completions.get(&operation).ok_or_else(|| {
                        Error::new("ownership_conflict", "terminal result lost actual cache")
                    })?;
                    let run = state.runs.get(&operation).ok_or_else(|| {
                        Error::new(
                            "ownership_conflict",
                            "terminal result lost actual admission",
                        )
                    })?;
                    if !Arc::ptr_eq(cached, completion)
                        || !run.joined
                        || !closed_model_observation(
                            &self.control.operation(operation)?,
                            &run.permit,
                            &run.budget,
                            &completion.observation(),
                        )?
                    {
                        return Err(Error::new(
                            "reconciliation_required",
                            "terminal result is not closed",
                        ));
                    }
                }
            }
            self.services
                .inputs
                .evaluation_providers()
                .ok_or_else(|| Error::new("evaluation_unavailable", "evaluation registry missing"))?
                .close(&VerifiedEvaluationExit {
                    workspace: proof.workspace,
                    operation,
                    source: source.digest().into(),
                })
        })
    }
}
fn held(
    owner: CleanupOwner,
    result: Result<Option<EvaluationTerminal>>,
) -> (CleanupState, Result<Option<EvaluationTerminal>>) {
    (CleanupState::Ready(Box::new(owner)), result)
}
fn settled(exit: CleanupExit) -> (CleanupState, Result<Option<EvaluationTerminal>>) {
    let terminal = exit.terminal();
    (CleanupState::Settled(Box::new(exit)), Ok(Some(terminal)))
}
fn unavailable() -> Error {
    Error::new(
        "runtime_unavailable",
        "evaluation cleanup ownership lock failed",
    )
}

fn conflict(prior: CleanupState, incoming: CleanupState) -> CleanupState {
    CleanupState::Conflict {
        _prior: Box::new(prior),
        _incoming: Box::new(incoming),
    }
}
// The caller has already moved a real owner out of its slot. Even a poisoned
// mutex or an unexpected concurrent state must not make an Err drop that owner.
fn store_transition<T>(
    slot: &Mutex<T>,
    incoming: T,
    expected: impl FnOnce(&T) -> bool,
    conflict: impl FnOnce(T, T) -> T,
) -> Result<()>
where
    T: Default,
{
    let (mut state, poisoned) = match slot.lock() {
        Ok(state) => (state, false),
        Err(error) => (error.into_inner(), true),
    };
    let valid = expected(&state);
    if valid {
        *state = incoming;
    } else {
        let prior = std::mem::take(&mut *state);
        *state = conflict(prior, incoming);
    }
    if poisoned {
        Err(unavailable())
    } else if !valid {
        Err(Error::new(
            "ownership_conflict",
            "evaluation cleanup state changed",
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod ownership_slot_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Debug)]
    struct DropCount(Arc<AtomicUsize>);
    impl Drop for DropCount {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    #[derive(Default, Debug)]
    enum DataSlot {
        #[default]
        Empty,
        Running,
        Owners(Vec<DropCount>),
    }
    fn combine(old: DataSlot, new: DataSlot) -> DataSlot {
        let mut all = Vec::new();
        for state in [old, new] {
            if let DataSlot::Owners(mut owners) = state {
                all.append(&mut owners);
            }
        }
        DataSlot::Owners(all)
    }
    #[test]
    fn poisoned_return_slot_keeps_owner_until_explicit_release() {
        // Generic ownership plumbing, not a fabricated worker/exit capability.
        let drops = Arc::new(AtomicUsize::new(0));
        let slot = Arc::new(Mutex::new(DataSlot::Running));
        let other = slot.clone();
        assert!(
            std::thread::spawn(move || {
                let _guard = other.lock().unwrap();
                panic!("component injected slot failure");
            })
            .join()
            .is_err()
        );
        let result = store_transition(
            &slot,
            DataSlot::Owners(vec![DropCount(drops.clone())]),
            |state| matches!(state, DataSlot::Running),
            combine,
        );
        assert_eq!(result.unwrap_err().code, "runtime_unavailable");
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        assert!(matches!(
            *slot.lock().unwrap_err().into_inner(),
            DataSlot::Owners(_)
        ));
        slot.clear_poison();
        *slot.lock().unwrap() = DataSlot::Empty;
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn conflicting_return_preserves_both_actual_owners() {
        let drops = Arc::new(AtomicUsize::new(0));
        let slot = Mutex::new(DataSlot::Owners(vec![DropCount(drops.clone())]));
        assert_eq!(
            store_transition(
                &slot,
                DataSlot::Owners(vec![DropCount(drops.clone())]),
                |state| matches!(state, DataSlot::Running),
                combine
            )
            .unwrap_err()
            .code,
            "ownership_conflict"
        );
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        let mut state = slot.lock().unwrap();
        assert!(matches!(&*state,DataSlot::Owners(owners) if owners.len()==2));
        *state = DataSlot::Empty;
        assert_eq!(drops.load(Ordering::SeqCst), 2);
    }
}
