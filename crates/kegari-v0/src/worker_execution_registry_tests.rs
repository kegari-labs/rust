//! Real registry and Control, simulated cancellation callbacks. No process,
//! joined worker or native exit capability is created by these component tests.
use super::*;
use crate::control::{Lifecycle, RunAdmission, RunReference};
use crate::execution::CommandCancellation;
use std::sync::atomic::AtomicUsize;

struct Handle {
    calls: AtomicUsize,
    exited: AtomicBool,
    failure: Option<Error>,
}
impl Handle {
    fn new(failure: Option<Error>) -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
            exited: AtomicBool::new(false),
            failure,
        })
    }
}
impl CommandCancellation for Handle {
    fn cancel(&self) -> Result<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.failure.clone().map_or(Ok(()), Err)
    }
    fn exited(&self) -> Result<bool> {
        Ok(self.exited.load(Ordering::SeqCst))
    }
}

fn owner() -> (tempfile::TempDir, ControlHost, Uuid, Uuid) {
    let temp = tempfile::tempdir().unwrap();
    let root = PrivateDir::create(&temp.path().canonicalize().unwrap().join("control")).unwrap();
    let runtime = Uuid::new_v4();
    let control = ControlHost::setup(root, runtime, Uuid::new_v4()).unwrap();
    lifecycle(&control, runtime, Lifecycle::Start);
    let lease = Lease {
        runtime_id: runtime,
        owner_generation: 1,
        owner_id: Uuid::new_v4(),
    };
    control.attach_reconciled(lease.clone()).unwrap();
    let operation = Uuid::new_v4();
    control
        .admit_run(
            &lease,
            RunAdmission {
                request_id: operation,
                work_key: "registry-component".into(),
                scope_digest: bytes_digest(b"scope"),
                reference: RunReference {
                    workspace: Uuid::new_v4(),
                    account: "component".into(),
                    normalized_state: None,
                    phase: crate::scheduler::PhaseKey {
                        work: "registry-component".into(),
                        generation: bytes_digest(b"generation"),
                        phase: "independent_review".into(),
                        retry_grant: None,
                    },
                    profile_digest: bytes_digest(b"profile"),
                    instruction_digest: bytes_digest(b"instructions"),
                    max_attempts: 1,
                    max_turns: 1,
                    max_review_rounds: 1,
                    attempt: 1,
                },
            },
            || Ok(()),
        )
        .unwrap();
    (temp, control, runtime, operation)
}
fn lifecycle(control: &ControlHost, runtime: Uuid, action: Lifecycle) {
    control
        .lifecycle(
            MutationRequest {
                request_id: Uuid::new_v4(),
                runtime_id: runtime,
                actor: Actor {
                    identity: "component".into(),
                    roles: BTreeSet::from(["lifecycle".into()]),
                    principal: None,
                },
                kind: "lifecycle".into(),
                canonical_targets: vec![runtime.to_string()],
                expected: BTreeMap::new(),
                payload: Value::Null,
            },
            action,
            control.snapshot().unwrap().intent.generation,
        )
        .unwrap();
}

#[test]
fn reservation_precedes_start_and_only_explicit_no_start_releases_it() {
    let registry = ExecutionRegistry::default();
    let operation = Uuid::new_v4();
    let effect = Uuid::from_u128(1);
    let reservation = registry.reserve(operation, effect).unwrap();
    assert!(registry.active_operations().unwrap().contains(&operation));
    assert_eq!(registry.observe_exited(operation).unwrap(), 0);
    assert!(registry.reserve(operation, effect).is_err());
    // Cancellation/drop of a start future supplies no absence/exit evidence.
    drop(reservation);
    assert_eq!(registry.observe_exited(operation).unwrap(), 0);
    registry
        .reserve(operation, Uuid::from_u128(2))
        .unwrap()
        .never_started();
    assert_eq!(registry.observe_exited(operation).unwrap(), 1);
    assert!(registry.active_operations().unwrap().contains(&operation));
    for index in 1..MAX_CALLS {
        drop(
            registry
                .reserve(operation, Uuid::from_u128(index as u128 + 100))
                .unwrap(),
        );
    }
    assert_eq!(
        registry
            .reserve(operation, Uuid::new_v4())
            .err()
            .unwrap()
            .code,
        "record_limit"
    );
}

#[test]
fn fenced_cancel_before_attach_cancels_and_retains_the_original_handle() {
    let (_temp, control, runtime, operation) = owner();
    let registry = ExecutionRegistry::default();
    let reservation = registry.reserve(operation, Uuid::new_v4()).unwrap();
    assert_eq!(
        registry
            .cancel_fenced(&control, operation)
            .unwrap_err()
            .code,
        "access_denied"
    );
    lifecycle(&control, runtime, Lifecycle::ForceStop);
    assert_eq!(
        registry
            .cancel_fenced(&control, operation)
            .unwrap_err()
            .code,
        "operation_uncertain"
    );
    let handle = Handle::new(Some(Error::new(
        "fixture_cancel",
        "simulated cancellation failure",
    )));
    assert_eq!(
        reservation.attach(handle.clone()).unwrap_err().code,
        "fixture_cancel"
    );
    assert_eq!(handle.calls.load(Ordering::SeqCst), 1);
    assert_eq!(registry.observe_exited(operation).unwrap(), 0);
    registry.cancel_fenced(&control, operation).unwrap_err();
    assert_eq!(handle.calls.load(Ordering::SeqCst), 2);
    handle.exited.store(true, Ordering::SeqCst);
    assert_eq!(registry.observe_exited(operation).unwrap(), 1);
    // Removing a simulated exited handle never retires its real admission.
    assert!(
        control
            .snapshot()
            .unwrap()
            .active_operations
            .iter()
            .any(|permit| permit.operation_id == operation)
    );
}

#[test]
fn cancellation_visits_every_owned_handle_and_preserves_all_errors() {
    let (_temp, control, runtime, operation) = owner();
    let registry = ExecutionRegistry::default();
    let first = Error::new("first_failure", "first simulated failure")
        .detail(json!({"retained":"first detail"}));
    let second = Error::new("second_failure", "second simulated failure");
    let handles = [
        Handle::new(Some(first.clone())),
        Handle::new(Some(second.clone())),
        Handle::new(None),
    ];
    for (index, handle) in handles.iter().enumerate() {
        registry
            .reserve(operation, Uuid::from_u128(index as u128 + 1))
            .unwrap()
            .attach(handle.clone())
            .unwrap();
    }
    lifecycle(&control, runtime, Lifecycle::ForceStop);
    let error = registry.cancel_fenced(&control, operation).unwrap_err();
    assert_eq!(error.code, first.code);
    assert_eq!(error.message, first.message);
    assert_eq!(error.details["original_details"], first.details);
    let failures = error.details["owned_cancellation_failures"]
        .as_array()
        .unwrap();
    assert_eq!(failures.len(), 2);
    assert_eq!(failures[0][1], serde_json::to_value(&first).unwrap());
    assert_eq!(failures[1][1], serde_json::to_value(&second).unwrap());
    for handle in handles {
        assert_eq!(handle.calls.load(Ordering::SeqCst), 1);
    }
    assert_eq!(registry.observe_exited(operation).unwrap(), 0);
    assert!(registry.active_operations().unwrap().contains(&operation));
}
