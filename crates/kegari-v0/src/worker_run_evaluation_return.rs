//! Same-process ownership returned without entering provider/hook execution.
//! No receipt, report, missing handle or public flag can construct this proof.
use super::*;
use crate::control::{Receipt, ReceiptPhase, RunReference};

pub(crate) enum OwnedTrialReturn {
    Completed(Box<VerifiedTrialCompletion>),
    Unused(Box<VerifiedUnusedAdmission>),
    Held {
        trial: Box<AdmittedTrial>,
        error: Error,
    },
}

/// The actual native finalization capability retained across a failed runtime
/// handoff. It cannot be constructed from RunCompletion or report JSON.
pub(crate) struct RetainedTrialCompletion {
    completion: Arc<RunCompletion>,
}
impl RetainedTrialCompletion {
    pub(crate) fn from_verified(value: VerifiedTrialCompletion) -> Self {
        Self {
            completion: Arc::new(value.into_completion()),
        }
    }
    pub(crate) fn completion(&self) -> &Arc<RunCompletion> {
        &self.completion
    }
}

/// A consumed real admission whose launch object can no longer be used. This
/// proves only a local no-start return; durable closure is independently checked.
pub(crate) struct VerifiedUnusedAdmission {
    permit: Permit,
    contract: Arc<WorkerContract>,
    budget: RunReference,
    request_digest: String,
    control: Arc<ControlHost>,
    registry: Arc<ExecutionRegistry>,
    checkpoint: Option<SessionState>,
    // Retain actual filesystem/source ownership until durable local closure.
    _execution: Option<Arc<WorkerExecution>>,
}
impl VerifiedUnusedAdmission {
    pub(crate) fn operation(&self) -> Uuid {
        self.permit.operation_id
    }
    pub(crate) fn permit(&self) -> &Permit {
        &self.permit
    }
    pub(crate) fn contract(&self) -> &Arc<WorkerContract> {
        &self.contract
    }
    pub(crate) fn budget(&self) -> &RunReference {
        &self.budget
    }
    pub(crate) fn observation(&self) -> Value {
        json!({"outcome":"not_started","phase":"evaluation_owned_return","checkpoint":self.checkpoint,"provider_execution":false,"hook_execution":false})
    }
    // Called only while the original AdmittedWorker still owns its unused
    // tuple. The provisional value never escapes if trial construction succeeds.
    pub(crate) fn capture_transfer(
        transfer: &crate::runtime_workers::EvaluationWorkerTransfer,
    ) -> Result<Self> {
        let (prepared, permit, services) = transfer.parts()?;
        let control = services.worker.control.clone();
        let receipt = control.operation(permit.operation_id)?;
        let budget = RunReference::from_receipt(&receipt)?.ok_or_else(|| {
            Error::new("worker_binding", "unused admission lost its actual budget")
        })?;
        let value = Self {
            permit: permit.clone(),
            contract: prepared.contract.clone(),
            budget,
            request_digest: returned_admission_digest(&receipt)?,
            control,
            registry: services.worker.executions.clone(),
            checkpoint: None,
            _execution: services.worker.execution.clone(),
        };
        value.verify_identity()?;
        Ok(value)
    }
    fn verify_identity(&self) -> Result<Receipt> {
        let receipt = self.control.operation(self.operation())?;
        let actual = RunReference::from_receipt(&receipt)?
            .ok_or_else(|| Error::new("worker_binding", "unused admission lost its typed run"))?;
        if returned_admission_digest(&receipt)? != self.request_digest
            || receipt.operation_id != self.operation()
            || receipt.request_id != self.operation()
            || receipt.runtime_id != self.permit.lease.runtime_id
            || receipt.owner_generation != self.permit.lease.owner_generation
            || receipt.expected.get("scope") != Some(&self.permit.scope_digest)
            || receipt.canonical_targets != [self.permit.work_key.clone()]
            || canonical_digest(1, &actual)? != canonical_digest(1, &self.budget)?
            || self.contract.scope_digest() != self.permit.scope_digest
            || self.contract.work_key()? != self.permit.work_key
            || self.contract.instructions().manifest.operation_id != self.operation()
        {
            return Err(Error::new(
                "worker_binding",
                "unused return differs from its original admission",
            ));
        }
        Ok(receipt)
    }
    pub(crate) fn verify_closed(&self) -> Result<()> {
        let receipt = self.verify_identity()?;
        if receipt.phase != ReceiptPhase::Reconciled
            || receipt.observed_result.as_ref().is_none_or(|result| {
                result["local_execution"] != "exited"
                    || result["unresolved_effects"]
                        .as_array()
                        .is_none_or(|effects| !effects.is_empty())
                    || result.get("observation") != Some(&self.observation())
            })
        {
            return Err(Error::new(
                "reconciliation_required",
                "unused return is not actually closed",
            ));
        }
        Ok(())
    }
    /// The caller retains this capability on error or while handles/holds remain.
    /// This never starts a process, repeats a hook, clears an effect or refunds an attempt.
    pub(crate) fn retire(&self) -> Result<Option<Receipt>> {
        if !observe_returned_registry(&self.control, &self.registry, self.operation())? {
            return Ok(None);
        }
        let receipt = self.verify_identity()?;
        let observation = self.observation();
        if receipt.phase == ReceiptPhase::Reconciled {
            return if receipt.observed_result.as_ref().is_some_and(|result| {
                result["local_execution"] == "exited"
                    && result["unresolved_effects"]
                        .as_array()
                        .is_some_and(Vec::is_empty)
                    && result.get("observation") == Some(&observation)
            }) {
                Ok(Some(receipt))
            } else {
                Err(Error::new(
                    "reconciliation_required",
                    "unused return retirement differs or retains effects",
                ))
            };
        }
        let available = self.control.serialize_observation(|snapshot| {
            Ok(!snapshot.unresolved_operations.iter().any(|(id, targets)| {
                *id == self.operation() || targets.contains(&self.permit.work_key)
            }))
        })?;
        if !available {
            return Ok(None);
        }
        let receipt = self.control.retire_local(self.operation(), observation)?;
        if receipt.observed_result.as_ref().is_none_or(|result| {
            result["unresolved_effects"]
                .as_array()
                .is_none_or(|effects| !effects.is_empty())
        }) {
            return Err(Error::new(
                "reconciliation_required",
                "unused return retains a related effect",
            ));
        }
        Ok(Some(receipt))
    }
}

impl AdmittedTrial {
    /// Select using the actual private owner, including early run_live_trial
    /// returns. Merely calling that evaluator does not establish execution entry.
    pub(crate) fn settle_owned_return(mut self) -> OwnedTrialReturn {
        if !self.lifecycle.before_started.load(Ordering::SeqCst)
            && !self.provider.launched.load(Ordering::SeqCst)
        {
            match self.capture_unstarted() {
                Ok(proof) => {
                    // Taking the session prevents fallback Drop from replacing
                    // this proof's exact pending retirement observation.
                    self.session.take();
                    OwnedTrialReturn::Unused(Box::new(proof))
                }
                Err(error) => self.hold_for_cleanup(error),
            }
        } else {
            match self.try_finish() {
                Ok(completion) => OwnedTrialReturn::Completed(Box::new(completion)),
                Err(error) => self.hold_for_cleanup(error),
            }
        }
    }
    fn hold_for_cleanup(self, error: Error) -> OwnedTrialReturn {
        let cancelled = self
            .session
            .as_ref()
            .map(|session| {
                session
                    .control
                    .fence(session.permit.operation_id)
                    .and_then(|()| {
                        session
                            .registry
                            .cancel_fenced(&session.control, session.permit.operation_id)
                            .map(|_| ())
                    })
            })
            .transpose();
        let error = match cancelled {
            Err(cleanup) => {
                let mut primary = error;
                primary.details = json!({"original_details":primary.details,"cleanup_cancellation_error":cleanup});
                primary
            }
            Ok(_) => error,
        };
        OwnedTrialReturn::Held {
            trial: Box::new(self),
            error,
        }
    }
    fn capture_unstarted(&self) -> Result<VerifiedUnusedAdmission> {
        let session = self.session.as_ref().ok_or_else(|| {
            Error::new(
                "ownership_conflict",
                "unused trial lost its actual bound session",
            )
        })?;
        if self.lifecycle.before_started.load(Ordering::SeqCst)
            || self.lifecycle.launched.load(Ordering::SeqCst)
            || self.lifecycle.after_started.load(Ordering::SeqCst)
            || self.provider.launched.load(Ordering::SeqCst)
            || !self
                .provider
                .gate
                .pending
                .lock()
                .map_err(|_| Error::new("ownership_conflict", "provider pending lock failed"))?
                .is_empty()
        {
            return Err(Error::new(
                "reconciliation_required",
                "unused return already entered execution",
            ));
        }
        let checkpoint = session.broker.trial_checkpoint(&session.state, None)?;
        if checkpoint.turns_started != 0
            || checkpoint.thread_id.is_some()
            || checkpoint.turn_id.is_some()
            || checkpoint.unresolved_turn.is_some()
            || checkpoint.resume_origin.is_some()
        {
            return Err(Error::new(
                "reconciliation_required",
                "unused return has actual provider history",
            ));
        }
        let receipt = session.control.operation(session.permit.operation_id)?;
        let budget = RunReference::from_receipt(&receipt)?
            .ok_or_else(|| Error::new("worker_binding", "unused trial lost its actual budget"))?;
        let proof = VerifiedUnusedAdmission {
            permit: session.permit.clone(),
            contract: session.prepared.contract.clone(),
            budget,
            request_digest: returned_admission_digest(&receipt)?,
            control: session.control.clone(),
            registry: session.registry.clone(),
            checkpoint: Some(checkpoint),
            _execution: session.execution.clone(),
        };
        proof.verify_identity()?;
        Ok(proof)
    }
}

// Supplemental local-handle observation only. A true value cannot construct an
// unused return or a native result; callers must already retain the actual owner.
fn observe_returned_registry(
    control: &ControlHost,
    registry: &ExecutionRegistry,
    operation: Uuid,
) -> Result<bool> {
    control.fence(operation)?;
    if registry.active_operations()?.contains(&operation) {
        registry.cancel_fenced(control, operation)?;
    }
    registry.observe_exited(operation)?;
    Ok(!registry.active_operations()?.contains(&operation))
}

/// Stable request identity only; mutable phase/result/updated time are excluded.
/// This digest is not an admission or execution authority.
pub(crate) fn returned_admission_digest(receipt: &Receipt) -> Result<String> {
    crate::control::receipt_request_digest(receipt)
}

#[cfg(test)]
mod registry_return_tests {
    use super::*;
    use crate::control::{Actor, Lifecycle, MutationRequest};
    use std::sync::atomic::AtomicUsize;
    struct OwnedHandle {
        state: AtomicUsize,
        cancels: AtomicUsize,
    }
    impl crate::execution::CommandCancellation for OwnedHandle {
        fn cancel(&self) -> Result<()> {
            self.cancels.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn exited(&self) -> Result<bool> {
            match self.state.load(Ordering::SeqCst) {
                0 => Err(Error::new(
                    "fixture_registry_failure",
                    "synthetic owned handle observation failed",
                )),
                1 => Ok(false),
                _ => Ok(true),
            }
        }
    }
    #[test]
    fn registry_observation_error_and_pending_handle_never_retire_admission() {
        // Real Control and ExecutionRegistry, simulated owned handle only. No
        // process is launched and no worker/exit capability is manufactured.
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().canonicalize().unwrap();
        let runtime = Uuid::new_v4();
        let control = ControlHost::setup(
            kegari_storage::PrivateDir::create(&path.join("control")).unwrap(),
            runtime,
            Uuid::new_v4(),
        )
        .unwrap();
        control
            .lifecycle(
                MutationRequest {
                    request_id: Uuid::new_v4(),
                    runtime_id: runtime,
                    actor: Actor {
                        identity: "fixture".into(),
                        principal: None,
                        roles: std::collections::BTreeSet::from(["lifecycle".into()]),
                    },
                    kind: "lifecycle".into(),
                    canonical_targets: vec![runtime.to_string()],
                    expected: BTreeMap::new(),
                    payload: Value::Null,
                },
                Lifecycle::Start,
                0,
            )
            .unwrap();
        let lease = crate::control::Lease {
            runtime_id: runtime,
            owner_generation: 1,
            owner_id: Uuid::new_v4(),
        };
        control.attach_reconciled(lease.clone()).unwrap();
        let operation = Uuid::new_v4();
        control
            .admit_run(
                &lease,
                crate::control::RunAdmission {
                    request_id: operation,
                    work_key: "fixture".into(),
                    scope_digest: kegari_digest::bytes_digest(b"scope"),
                    reference: RunReference {
                        workspace: Uuid::new_v4(),
                        account: "fixture".into(),
                        normalized_state: None,
                        phase: crate::scheduler::PhaseKey {
                            work: "fixture".into(),
                            generation: kegari_digest::bytes_digest(b"generation"),
                            phase: "independent_review".into(),
                            retry_grant: None,
                        },
                        profile_digest: kegari_digest::bytes_digest(b"profile"),
                        instruction_digest: kegari_digest::bytes_digest(b"instructions"),
                        max_attempts: 1,
                        max_turns: 1,
                        max_review_rounds: 1,
                        attempt: 1,
                    },
                },
                || Ok(()),
            )
            .unwrap();
        let original = canonical_digest(1, &control.operation(operation).unwrap()).unwrap();
        let registry = ExecutionRegistry::default();
        let handle = Arc::new(OwnedHandle {
            state: AtomicUsize::new(0),
            cancels: AtomicUsize::new(0),
        });
        registry
            .register(operation, Uuid::new_v4(), handle.clone())
            .unwrap();
        assert_eq!(
            observe_returned_registry(&control, &registry, operation)
                .unwrap_err()
                .code,
            "fixture_registry_failure"
        );
        handle.state.store(1, Ordering::SeqCst);
        assert!(!observe_returned_registry(&control, &registry, operation).unwrap());
        assert!(registry.active_operations().unwrap().contains(&operation));
        handle.state.store(2, Ordering::SeqCst);
        assert!(observe_returned_registry(&control, &registry, operation).unwrap());
        assert_eq!(handle.cancels.load(Ordering::SeqCst), 3);
        assert_eq!(
            canonical_digest(1, &control.operation(operation).unwrap()).unwrap(),
            original
        );
        assert!(
            control
                .snapshot()
                .unwrap()
                .active_operations
                .iter()
                .any(|permit| permit.operation_id == operation)
        );
    }
}
