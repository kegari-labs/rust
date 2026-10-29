//! Positive return of the same owned preparation invocation. This is not a
//! reconstruction from absent receipts and never grants another model attempt.
use super::*;
use crate::control::{Receipt, ReceiptPhase};
#[cfg(test)]
use kegari_types::run::PhaseKey;
use kegari_types::run::RunReference;

pub(crate) enum EvaluationPreparationReturn {
    Ready(Box<AdmittedWorker>),
    Returned {
        error: Error,
        owner: Box<PreparationReturned>,
    },
    Rejected(Error),
}

pub(crate) struct PreparationReturned {
    operation: Uuid,
    preparation: Uuid,
    lease: Lease,
    work: String,
    prep_permit: Option<Permit>,
    prep_receipt: Option<Receipt>,
    prep_reference: Option<PreparationReference>,
    prep_request_digest: Option<String>,
    model_request_digest: Option<String>,
    prep_observation: Option<Value>,
    model_receipt: Option<Receipt>,
    model_observation: Option<Value>,
    model: Option<(Permit, Arc<crate::worker::WorkerContract>, RunReference)>,
    error: Error,
}
pub(super) struct PreparationTrace {
    returned: PreparationReturned,
}
impl PreparationTrace {
    pub(super) fn new(request: &LaunchRequest) -> Result<Self> {
        // Only the evaluator's ordinary fresh review or scoped child admission.
        // Continuation/retry lanes need their own original protected identity.
        let ordinary = matches!(&request.authority, WorkerAuthority::Independent(_) if request.phase.phase == "independent_review")
            || matches!(&request.authority, WorkerAuthority::Authorized(g) if g.work().child.is_some() && request.phase.phase == "implementation");
        if !ordinary
            || request.phase.retry_grant.is_some()
            || request.delivery.is_some()
            || request.unfinished.is_some()
            || request.source_advanced.is_some()
            || request.retried_unfinished.is_some()
            || request.review_retry.is_some()
            || request.validation_retry.is_some()
            || request.review_round.is_some()
        {
            return Err(Error::new(
                "worker_binding",
                "evaluation return requires an ordinary fresh review or child admission",
            ));
        }
        Ok(Self {
            returned: PreparationReturned {
                operation: request.operation_id,
                preparation: preparation_operation(request.operation_id)?,
                lease: request.lease.clone(),
                work: request.authority.work_key()?,
                prep_permit: None,
                prep_receipt: None,
                prep_reference: None,
                prep_request_digest: None,
                model_request_digest: None,
                prep_observation: None,
                model_receipt: None,
                model_observation: None,
                model: None,
                error: Error::new(
                    "worker_preparation",
                    "preparation invocation has not returned",
                ),
            },
        })
    }
    pub(super) fn preparation(
        &mut self,
        permit: &Permit,
        reference: PreparationReference,
        control: &crate::control::ControlHost,
    ) -> Result<()> {
        self.returned.prep_permit = Some(permit.clone());
        self.returned.prep_reference = Some(reference.clone());
        let expected = crate::control::ordinary_preparation_request_digest(permit, &reference)?;
        capture_request_identity(
            &mut self.returned.prep_request_digest,
            expected,
            control,
            permit.operation_id,
        )
    }
    pub(super) fn preparation_retiring(&mut self, observation: Value) {
        self.returned.prep_observation = Some(observation);
    }
    pub(super) fn model_retiring(&mut self, observation: Value) {
        self.returned.model_observation = Some(observation);
    }
    pub(super) fn model_closed(&mut self, receipt: Receipt) {
        self.returned.model_receipt = Some(receipt);
    }
    pub(super) fn preparation_closed(&mut self, receipt: Receipt) {
        self.returned.prep_receipt = Some(receipt);
    }
    pub(super) fn admitted(
        &mut self,
        permit: &Permit,
        contract: &Arc<crate::worker::WorkerContract>,
        budget: RunReference,
        control: &crate::control::ControlHost,
    ) -> Result<()> {
        self.returned.model = Some((permit.clone(), contract.clone(), budget.clone()));
        let expected = crate::control::ordinary_run_request_digest(permit, &budget)?;
        capture_request_identity(
            &mut self.returned.model_request_digest,
            expected,
            control,
            permit.operation_id,
        )
    }
    pub(super) fn finish(mut self, error: Error) -> PreparationReturned {
        self.returned.error = error;
        self.returned
    }
}

// Store the exact original invocation identity before any fallible receipt read.
// Retry can only compare that identity; it never adopts a newly observed one.
fn capture_request_identity(
    retained: &mut Option<String>,
    expected: String,
    control: &crate::control::ControlHost,
    operation: Uuid,
) -> Result<()> {
    if retained.as_ref().is_some_and(|prior| prior != &expected) {
        return Err(Error::new(
            "worker_binding",
            "owned admission identity was already captured",
        ));
    }
    *retained = Some(expected);
    require_request_identity(retained, &control.operation(operation)?)
}
fn require_request_identity(retained: &Option<String>, receipt: &Receipt) -> Result<()> {
    if retained.as_ref() != Some(&worker_run::returned_admission_digest(receipt)?) {
        return Err(Error::new(
            "worker_binding",
            "owned admission immutable request differs",
        ));
    }
    Ok(())
}

pub(crate) async fn prepare_evaluation_admitted(
    runtime: Arc<RuntimeCoordinator>,
    request: LaunchRequest,
    operand: Option<Arc<crate::evaluation::OwnedCanaryPathOperand>>,
) -> EvaluationPreparationReturn {
    let mut trace = match PreparationTrace::new(&request) {
        Ok(trace) => trace,
        Err(error) => return EvaluationPreparationReturn::Rejected(error),
    };
    match prepare_admitted_inner(runtime, request, Some(&mut trace), operand).await {
        Ok(worker) => EvaluationPreparationReturn::Ready(Box::new(worker)),
        Err(error) => EvaluationPreparationReturn::Returned {
            owner: Box::new(trace.finish(error.clone())),
            error,
        },
    }
}

impl PreparationReturned {
    pub(crate) fn operation(&self) -> Uuid {
        self.operation
    }
    pub(crate) fn receipt_operation(&self) -> Uuid {
        if self.model.is_some() {
            self.operation
        } else {
            self.preparation
        }
    }
    pub(crate) fn work(&self) -> &str {
        &self.work
    }
    // Read-only use after this exact invocation has positively retired. Caller
    // may hold Control; do not reacquire its gate or infer a missing record.
    pub(crate) fn verify_closed(&self, runtime: &RuntimeCoordinator) -> Result<()> {
        for (operation, permit, retained) in [
            (
                self.preparation,
                self.prep_permit.as_ref(),
                self.prep_receipt.as_ref(),
            ),
            (
                self.operation,
                self.model.as_ref().map(|(permit, _, _)| permit),
                self.model_receipt.as_ref(),
            ),
        ] {
            if permit.is_some() {
                let retained = retained.ok_or_else(|| {
                    Error::new(
                        "reconciliation_required",
                        "owned return did not retain its actual retirement",
                    )
                })?;
                let actual = runtime.control.operation(operation)?;
                if canonical_digest(1, &actual)? != canonical_digest(1, retained)? {
                    return Err(Error::new(
                        "record_corrupt",
                        "owned return retirement changed",
                    ));
                }
                require_closed(&actual)?;
            } else {
                require_missing(&runtime.control, operation)?;
            }
        }
        Ok(())
    }
    pub(crate) fn model(
        &self,
    ) -> Option<(&Permit, &Arc<crate::worker::WorkerContract>, &RunReference)> {
        self.model.as_ref().map(|(p, c, b)| (p, c, b))
    }
    /// The same preparation future positively returned before transferring an
    /// executable worker. Registry/effect checks supplement that fact.
    pub(crate) fn retire(&mut self, runtime: &RuntimeCoordinator) -> Result<bool> {
        runtime.registration.verify_active_owner(&self.lease)?;
        for id in [self.preparation, self.operation] {
            runtime.control.fence(id)?;
            if runtime.executions.active_operations()?.contains(&id) {
                runtime.executions.cancel_fenced(&runtime.control, id)?;
            }
            runtime.executions.observe_exited(id)?;
        }
        let active = runtime.executions.active_operations()?;
        if active.contains(&self.preparation) || active.contains(&self.operation) {
            return Ok(false);
        }
        let clear = runtime.control.serialize_observation(|snapshot| {
            Ok(!snapshot.unresolved_operations.iter().any(|(id, targets)| {
                [self.preparation, self.operation].contains(id) || targets.contains(&self.work)
            }))
        })?;
        if !clear {
            return Ok(false);
        }
        match &self.prep_permit {
            Some(permit) => {
                let actual = runtime.control.operation(self.preparation)?;
                let reference = PreparationReference::from_receipt(&actual)?.ok_or_else(|| {
                    Error::new(
                        "worker_binding",
                        "returned preparation lost its actual reference",
                    )
                })?;
                require_request_identity(&self.prep_request_digest, &actual)?;
                if self
                    .prep_reference
                    .as_ref()
                    .map(|value| canonical_digest(1, value))
                    .transpose()?
                    != Some(canonical_digest(1, &reference)?)
                    || reference.worker_operation != self.operation
                    || actual.runtime_id != self.lease.runtime_id
                    || actual.owner_generation != self.lease.owner_generation
                    || actual.expected.get("scope") != Some(&permit.scope_digest)
                    || actual.canonical_targets != [self.work.clone()]
                {
                    return Err(Error::new(
                        "worker_binding",
                        "returned preparation identity changed",
                    ));
                }
                let observation = self.prep_observation.get_or_insert_with(|| json!({"phase":"worker_preparation","result":"blocked","error":self.error,"owned_preparation_returned":true}));
                retire_exact(
                    &runtime.control,
                    self.preparation,
                    actual,
                    observation,
                    &mut self.prep_receipt,
                )?;
            }
            None => require_missing(&runtime.control, self.preparation)?,
        }
        match &self.model {
            Some((permit, contract, budget)) => {
                let actual = runtime.control.operation(self.operation)?;
                let found =
                    crate::control::run_reference_from_receipt(&actual)?.ok_or_else(|| {
                        Error::new("worker_binding", "returned model lost its actual admission")
                    })?;
                require_request_identity(&self.model_request_digest, &actual)?;
                if canonical_digest(1, &found)? != canonical_digest(1, budget)?
                    || actual.runtime_id != permit.lease.runtime_id
                    || actual.owner_generation != permit.lease.owner_generation
                    || actual.expected.get("scope") != Some(&permit.scope_digest)
                    || actual.canonical_targets != [permit.work_key.clone()]
                    || contract.scope_digest() != permit.scope_digest
                {
                    return Err(Error::new(
                        "worker_binding",
                        "returned unused model identity changed",
                    ));
                }
                let observation = self.model_observation.get_or_insert_with(
                    || json!({"outcome":"failed","error":self.error,"phase":"worker_start"}),
                );
                retire_exact(
                    &runtime.control,
                    self.operation,
                    actual,
                    observation,
                    &mut self.model_receipt,
                )?;
            }
            None => require_missing(&runtime.control, self.operation)?,
        }
        Ok(true)
    }
}
// Retain the returned whole receipt before checking later closures. If storage
// returned an error after committing, only this invocation's exact intended
// observation may be adopted on retry; no different retirement is substituted.
fn retire_exact(
    control: &crate::control::ControlHost,
    operation: Uuid,
    actual: Receipt,
    observation: &Value,
    retained: &mut Option<Receipt>,
) -> Result<()> {
    if let Some(expected) = retained.as_ref() {
        if canonical_digest(1, &actual)? != canonical_digest(1, expected)? {
            return Err(Error::new(
                "record_corrupt",
                "owned return retirement changed",
            ));
        }
        return require_closed(&actual);
    }
    let closed = if actual.phase == ReceiptPhase::Reconciled {
        if actual
            .observed_result
            .as_ref()
            .and_then(|value| value.get("observation"))
            != Some(observation)
        {
            return Err(Error::new(
                "reconciliation_required",
                "owned return has another retirement",
            ));
        }
        actual
    } else {
        control.retire_local(operation, observation.clone())?
    };
    *retained = Some(closed);
    require_closed(retained.as_ref().expect("retained own retirement"))
}

fn require_missing(control: &crate::control::ControlHost, operation: Uuid) -> Result<()> {
    match control.operation(operation) {
        Err(error) if error.code == "record_missing" => Ok(()),
        Err(error) => Err(error),
        Ok(_) => Err(Error::new(
            "reconciliation_required",
            "returned pre-admission branch has a conflicting actual receipt",
        )),
    }
}
fn require_closed(receipt: &Receipt) -> Result<()> {
    if receipt.phase != ReceiptPhase::Reconciled
        || receipt.observed_result.as_ref().is_none_or(|value| {
            value["local_execution"] != "exited"
                || value["unresolved_effects"]
                    .as_array()
                    .is_none_or(|items| !items.is_empty())
        })
    {
        return Err(Error::new(
            "reconciliation_required",
            "returned preparation/model retains unresolved effects",
        ));
    }
    Ok(())
}

/// Created only by consuming the real AdmittedWorker's untransferred tuple.
/// Holding this on error retains its original services until no-start is proven.
pub(crate) struct EvaluationWorkerTransfer {
    pub(super) permit: Permit,
    pub(super) input: Option<(worker_run::PreparedRun, RunServices, RunOptions)>,
}
impl EvaluationWorkerTransfer {
    pub(crate) fn parts(&self) -> Result<(&worker_run::PreparedRun, &Permit, &RunServices)> {
        self.input
            .as_ref()
            .map(|(prepared, services, _)| (prepared, &self.permit, services))
            .ok_or_else(|| Error::new("ownership_conflict", "unused transfer already consumed"))
    }
    pub(crate) fn return_unused(
        mut self: Box<Self>,
    ) -> std::result::Result<worker_run::VerifiedUnusedAdmission, (Box<Self>, Error)> {
        match worker_run::VerifiedUnusedAdmission::capture_transfer(&self) {
            Ok(proof) => {
                self.input.take();
                Ok(proof)
            }
            Err(error) => Err((self, error)),
        }
    }
}
pub(crate) enum TrialTransferFailure {
    Owned {
        owner: Box<EvaluationWorkerTransfer>,
        error: Error,
    },
    Unused {
        owner: Box<worker_run::VerifiedUnusedAdmission>,
        error: Error,
    },
}
impl AdmittedWorker {
    pub(crate) fn into_trial_observed(
        mut self,
    ) -> std::result::Result<worker_run::AdmittedTrial, TrialTransferFailure> {
        let mut owner = EvaluationWorkerTransfer {
            permit: self.permit.clone(),
            input: self.input.take(),
        };
        self.transferred = true;
        let unused = match worker_run::VerifiedUnusedAdmission::capture_transfer(&owner) {
            Ok(proof) => proof,
            Err(error) => {
                return Err(TrialTransferFailure::Owned {
                    owner: Box::new(owner),
                    error,
                });
            }
        };
        let (prepared, _, options) = owner.input.as_ref().expect("validated original tuple");
        if let Err(error) =
            self.runtime
                .evaluation_trial_transfer(&owner.permit, prepared.contract(), options)
        {
            return Err(TrialTransferFailure::Unused {
                owner: Box::new(unused),
                error,
            });
        }
        let (prepared, services, options) = owner.input.take().expect("validated original tuple");
        match worker_run::AdmittedTrial::new(prepared, owner.permit.clone(), services, options) {
            Ok(trial) => Ok(trial),
            Err(error) => Err(TrialTransferFailure::Unused {
                owner: Box::new(unused),
                error,
            }),
        }
    }
}

impl Drop for EvaluationWorkerTransfer {
    fn drop(&mut self) {
        if let Some((_, services, _)) = &self.input
            && services
                .worker
                .control
                .fence(self.permit.operation_id)
                .is_ok()
        {
            let _ = services
                .worker
                .executions
                .cancel_fenced(&services.worker.control, self.permit.operation_id);
        }
    }
}

#[cfg(test)]
mod retirement_tests {
    use super::*;
    use crate::control::ControlHost;
    use crate::control::Lifecycle;
    use crate::control::MutationRequest;
    use kegari_control_api::identity::Actor;
    use kegari_digest::bytes_digest;
    use std::collections::BTreeSet;

    #[test]
    fn actual_retirement_storage_failure_retains_original_admission_for_retry() {
        // Actual Control + owned local storage; synthetic preparation observation.
        // This does not construct PreparationReturned or a native exit capability.
        let temporary = tempfile::TempDir::new().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let records = PrivateDir::create(&root.join("control")).unwrap();
        let runtime = Uuid::new_v4();
        let control = ControlHost::setup(
            PrivateDir::open(records.path()).unwrap(),
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
                        roles: BTreeSet::from(["lifecycle".into()]),
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
        let lease = Lease {
            runtime_id: runtime,
            owner_generation: 1,
            owner_id: Uuid::new_v4(),
        };
        control.attach_reconciled(lease.clone()).unwrap();
        let worker = Uuid::new_v4();
        let reference = PreparationReference {
            worker_operation: worker,
            workspace: Uuid::new_v4(),
            account: "fixture".into(),
            phase: PhaseKey {
                work: "fixture-work".into(),
                generation: bytes_digest(b"generation"),
                phase: "independent_review".into(),
                retry_grant: None,
            },
        };
        let operation = reference.operation_id().unwrap();
        control
            .admit_preparation_observed(
                &lease,
                PreparationAdmission {
                    request_id: operation,
                    work_key: reference.phase.work.clone(),
                    scope_digest: bytes_digest(b"scope"),
                    reference,
                },
                |_| Ok(()),
            )
            .unwrap();
        let original = control.operation(operation).unwrap();
        let observation = json!({"phase":"worker_preparation","result":"blocked","component":"explicit synthetic preparation return"});
        let mut retained = None;
        let receipts = records.path().join("receipts");
        let displaced = records.path().join("receipts-held");
        std::fs::rename(&receipts, &displaced).unwrap();
        let failed = retire_exact(
            &control,
            operation,
            original.clone(),
            &observation,
            &mut retained,
        );
        std::fs::rename(&displaced, &receipts).unwrap();
        assert!(failed.is_err());
        assert!(retained.is_none());
        assert_eq!(
            canonical_digest(1, &control.operation(operation).unwrap()).unwrap(),
            canonical_digest(1, &original).unwrap()
        );
        assert!(
            control
                .snapshot()
                .unwrap()
                .active_operations
                .iter()
                .any(|permit| permit.operation_id == operation)
        );
        retire_exact(
            &control,
            operation,
            control.operation(operation).unwrap(),
            &observation,
            &mut retained,
        )
        .unwrap();
        let closed = retained.as_ref().unwrap().clone();
        require_closed(&closed).unwrap();
        assert!(
            !control
                .snapshot()
                .unwrap()
                .active_operations
                .iter()
                .any(|permit| permit.operation_id == operation)
        );
        let digest = canonical_digest(1, &closed).unwrap();
        retire_exact(
            &control,
            operation,
            control.operation(operation).unwrap(),
            &observation,
            &mut retained,
        )
        .unwrap();
        assert_eq!(
            canonical_digest(1, &control.operation(operation).unwrap()).unwrap(),
            digest
        );
        let mut substituted = closed;
        substituted.observed_result.as_mut().unwrap()["observation"]["result"] = json!("prepared");
        assert_eq!(
            retire_exact(
                &control,
                operation,
                substituted,
                &observation,
                &mut retained
            )
            .unwrap_err()
            .code,
            "record_corrupt"
        );
        assert_eq!(
            control
                .operations()
                .unwrap()
                .iter()
                .filter(|receipt| receipt.kind == "admission")
                .count(),
            1
        );
    }

    #[test]
    fn actual_admission_read_failure_retains_expected_request_and_retries_exactly() {
        // Actual Control admissions and local storage only. No worker, exit,
        // PreparationReturned or completion capability is fabricated here.
        let temporary = tempfile::TempDir::new().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let records = PrivateDir::create(&root.join("control")).unwrap();
        let runtime = Uuid::new_v4();
        let control = ControlHost::setup(
            PrivateDir::open(records.path()).unwrap(),
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
                        roles: BTreeSet::from(["lifecycle".into()]),
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
        let lease = Lease {
            runtime_id: runtime,
            owner_generation: 1,
            owner_id: Uuid::new_v4(),
        };
        control.attach_reconciled(lease.clone()).unwrap();
        let worker = Uuid::new_v4();
        let reference = PreparationReference {
            worker_operation: worker,
            workspace: Uuid::new_v4(),
            account: "fixture".into(),
            phase: PhaseKey {
                work: "fixture-work".into(),
                generation: bytes_digest(b"generation"),
                phase: "independent_review".into(),
                retry_grant: None,
            },
        };
        let permit = control
            .admit_preparation_observed(
                &lease,
                PreparationAdmission {
                    request_id: reference.operation_id().unwrap(),
                    work_key: reference.phase.work.clone(),
                    scope_digest: bytes_digest(b"scope"),
                    reference: reference.clone(),
                },
                |_| Ok(()),
            )
            .unwrap();
        let expected =
            crate::control::ordinary_preparation_request_digest(&permit, &reference).unwrap();
        exercise_missing_read(&control, &records, &permit, expected);
        let mut wrong_namespace = reference.clone();
        wrong_namespace.phase.retry_grant = Some(Uuid::new_v4());
        assert_eq!(
            crate::control::ordinary_preparation_request_digest(&permit, &wrong_namespace)
                .unwrap_err()
                .code,
            "worker_binding"
        );
        control.retire_local(permit.operation_id, json!({"phase":"worker_preparation","result":"prepared","component":"synthetic fixture observation"})).unwrap();
        let run = RunReference {
            workspace: reference.workspace,
            account: reference.account.clone(),
            normalized_state: Some("in progress".into()),
            phase: reference.phase,
            profile_digest: bytes_digest(b"profile"),
            instruction_digest: bytes_digest(b"instructions"),
            max_attempts: 2,
            max_turns: 2,
            max_review_rounds: 1,
            attempt: 1,
        };
        let permit = control
            .admit_run_observed(
                &lease,
                RunAdmission {
                    request_id: worker,
                    work_key: run.phase.work.clone(),
                    scope_digest: bytes_digest(b"model scope"),
                    reference: run.clone(),
                },
                |_| Ok(()),
            )
            .unwrap();
        let expected = crate::control::ordinary_run_request_digest(&permit, &run).unwrap();
        exercise_missing_read(&control, &records, &permit, expected);
        let mut wrong_namespace = run;
        wrong_namespace.phase.retry_grant = Some(Uuid::new_v4());
        assert_eq!(
            crate::control::ordinary_run_request_digest(&permit, &wrong_namespace)
                .unwrap_err()
                .code,
            "worker_binding"
        );
        assert_eq!(
            control
                .operations()
                .unwrap()
                .iter()
                .filter(|r| r.kind == "admission")
                .count(),
            2
        );
    }

    fn exercise_missing_read(
        control: &ControlHost,
        records: &PrivateDir,
        permit: &Permit,
        expected: String,
    ) {
        let original = control.operation(permit.operation_id).unwrap();
        assert_eq!(
            worker_run::returned_admission_digest(&original).unwrap(),
            expected
        );
        let mut captured = None;
        let receipts = records.path().join("receipts");
        let displaced = records.path().join("receipts-held");
        std::fs::rename(&receipts, &displaced).unwrap();
        let failed = capture_request_identity(
            &mut captured,
            expected.clone(),
            control,
            permit.operation_id,
        );
        std::fs::rename(&displaced, &receipts).unwrap();
        assert!(failed.is_err());
        assert_eq!(captured, Some(expected.clone()));
        // This exact comparison is used by PreparationReturned::retire. No
        // current receipt is allowed to fill an unknown expected identity.
        require_request_identity(&captured, &control.operation(permit.operation_id).unwrap())
            .unwrap();
        capture_request_identity(
            &mut captured,
            expected.clone(),
            control,
            permit.operation_id,
        )
        .unwrap();
        for field in [
            "operation",
            "request",
            "runtime",
            "owner",
            "actor",
            "kind",
            "targets",
            "payload",
            "expected",
        ] {
            let mut changed = original.clone();
            match field {
                "operation" => changed.operation_id = Uuid::new_v4(),
                "request" => changed.request_id = Uuid::new_v4(),
                "runtime" => changed.runtime_id = Uuid::new_v4(),
                "owner" => changed.owner_generation += 1,
                "actor" => changed.actor = "substituted-owner".into(),
                "kind" => changed.kind = "substituted-admission".into(),
                "targets" => changed.canonical_targets.push("substituted-work".into()),
                "payload" => changed.payload_digest = bytes_digest(b"substituted payload"),
                "expected" => {
                    changed
                        .expected
                        .insert("source_advanced".into(), "substituted marker".into());
                }
                _ => unreachable!(),
            }
            assert_eq!(
                require_request_identity(&captured, &changed)
                    .unwrap_err()
                    .code,
                "worker_binding",
                "{field}"
            );
            assert_eq!(captured, Some(expected.clone()));
        }
        assert_eq!(
            capture_request_identity(
                &mut captured,
                bytes_digest(b"replacement"),
                control,
                permit.operation_id
            )
            .unwrap_err()
            .code,
            "worker_binding"
        );
        assert_eq!(captured, Some(expected));
        let mut progressed = original.clone();
        progressed.phase = ReceiptPhase::Reconciled;
        progressed.updated_at += chrono::Duration::seconds(1);
        progressed.observed_result =
            Some(json!({"component":"identity comparison only, not exit authority"}));
        require_request_identity(&captured, &progressed).unwrap();
        assert_eq!(
            canonical_digest(1, &control.operation(permit.operation_id).unwrap()).unwrap(),
            canonical_digest(1, &original).unwrap()
        );
    }
}
