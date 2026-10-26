//! Actual protected-I/O and Control components only. No registered native
//! worker, joined completion, declared trial or accepted claim is manufactured.
use super::*;
use crate::control::{
    Actor, Lifecycle, MutationRequest, RemoteReservation, RunAdmission, RunReference,
};
use crate::providers::{
    Credential, GithubClient, HttpPolicy, HttpRequest, HttpResponse, HttpTransport,
};
use std::sync::atomic::{AtomicUsize, Ordering};

fn journal(root: &PrivateDir) -> Journal {
    let dir = root.child("journal").unwrap();
    let lock = dir.lock("worker.lock").unwrap();
    Journal {
        dir,
        _lock: lock,
        next: Mutex::new(0),
    }
}
#[test]
fn same_write_journal_anchor_refuses_mutation_before_first_read_and_missing_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let root = PrivateDir::create(&temp.path().canonicalize().unwrap().join("owned")).unwrap();
    let journal = journal(&root);
    let reference = journal
        .append_reference(
            "tool_started",
            &json!({"request_id":Uuid::new_v4(),"arguments_digest":bytes_digest(b"original")}),
        )
        .unwrap();
    let original = journal.dir.read_bytes(&reference.leaf, 256 * 1024).unwrap();
    let mut changed: Value = journal.dir.read(&reference.leaf).unwrap();
    changed["record"]["arguments_digest"] = json!(bytes_digest(b"changed"));
    journal.dir.write(&reference.leaf, &changed).unwrap();
    assert_eq!(
        reference.verify(&journal).unwrap_err().code,
        "observation_conflict"
    );
    journal.dir.write_bytes(&reference.leaf, &original).unwrap();
    reference.verify(&journal).unwrap();
    std::fs::remove_file(journal.dir.path().join(&reference.leaf)).unwrap();
    assert_eq!(
        reference.verify(&journal).unwrap_err().code,
        "record_missing"
    );
    journal.dir.write_bytes(&reference.leaf, &original).unwrap();
    reference.verify(&journal).unwrap();
}
#[test]
fn shared_capture_budget_never_renews_and_refuses_before_write() {
    let budget = Arc::new(CaptureBudget::default());
    let protocol = budget.clone();
    let broker = budget.clone();
    for _ in 0..63 {
        protocol.reserve(RECORD_BYTES).unwrap();
    }
    broker.reserve(RECORD_BYTES - 1).unwrap();
    assert_eq!(protocol.reserve(2).unwrap_err().code, "observation_limit");
    broker.reserve(1).unwrap();
    assert_eq!(broker.reserve(1).unwrap_err().code, "observation_limit");
    assert_eq!(*budget.0.lock().unwrap(), CAPTURE_BYTES);
    let temp = tempfile::tempdir().unwrap();
    let root = PrivateDir::create(&temp.path().canonicalize().unwrap().join("owned")).unwrap();
    let journal = journal(&root);
    let before = journal.dir.records().unwrap();
    assert_eq!(
        journal
            .append_observation(
                "broker_observation",
                &json!({"component":"no-space"}),
                &budget
            )
            .unwrap_err()
            .code,
        "observation_limit"
    );
    assert_eq!(journal.dir.records().unwrap(), before);
    assert_eq!(*journal.next.lock().unwrap(), 0);
    let mut state = BrokerObservationState::Partial;
    promote(&mut state, "observation_limit");
    promote(&mut state, "record_missing");
    assert!(matches!(state, BrokerObservationState::Overflow));
    promote(&mut state, "observation_conflict");
    promote(&mut state, "record_missing");
    assert!(matches!(state, BrokerObservationState::Conflict));
}
struct NeverWire(AtomicUsize);
impl HttpTransport for NeverWire {
    fn send(
        &self,
        _: HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Result<HttpResponse>> + Send + '_>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err(Error::new("test_wire", "diagnostic must not transmit")) })
    }
}
struct Fixture {
    _temp: tempfile::TempDir,
    control: ControlHost,
    receipts: PrivateDir,
    permit: Permit,
    broker: ProviderBroker,
    wire: Arc<NeverWire>,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let dir = PrivateDir::create(&temp.path().canonicalize().unwrap().join("control")).unwrap();
        let receipts = dir.child("receipts").unwrap();
        let runtime = Uuid::new_v4();
        let control = ControlHost::setup(dir, runtime, Uuid::new_v4()).unwrap();
        control
            .lifecycle(
                MutationRequest {
                    request_id: Uuid::new_v4(),
                    runtime_id: runtime,
                    actor: Actor {
                        identity: "component operator".into(),
                        roles: ["lifecycle".into()].into(),
                        principal: None,
                    },
                    kind: "start".into(),
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
        let reference = RunReference {
            workspace: Uuid::new_v4(),
            account: "component".into(),
            normalized_state: None,
            phase: crate::scheduler::PhaseKey {
                work: "independent component".into(),
                generation: bytes_digest(b"generation"),
                phase: "independent_review".into(),
                retry_grant: None,
            },
            profile_digest: bytes_digest(b"profile"),
            instruction_digest: bytes_digest(b"instruction"),
            max_attempts: 2,
            max_turns: 3,
            max_review_rounds: 1,
            attempt: 1,
        };
        let permit = control
            .admit_run(
                &lease,
                RunAdmission {
                    request_id: Uuid::new_v4(),
                    work_key: reference.phase.work.clone(),
                    scope_digest: bytes_digest(b"scope"),
                    reference,
                },
                || Ok(()),
            )
            .unwrap();
        let wire = Arc::new(NeverWire(AtomicUsize::new(0)));
        let github = GithubClient::with_transport(
            HttpPolicy::new(
                "https://github.invalid/api/v3",
                "7".into(),
                std::time::Duration::from_secs(2),
            )
            .unwrap(),
            crate::config::GithubSelection {
                repositories: vec!["42".into()],
                organizations: vec![],
                pr_authors: vec!["7".into()],
            },
            Credential::new("component-unused-credential".into()).unwrap(),
            Some(wire.clone()),
        )
        .unwrap();
        Self {
            _temp: temp,
            control,
            receipts,
            permit,
            broker: ProviderBroker {
                linear: None,
                github: Some(github),
            },
            wire,
        }
    }
    fn request(&self) -> MutationRequest {
        self.broker
            .independent_feedback_request(
                MutationRequest {
                    request_id: Uuid::new_v4(),
                    runtime_id: self.permit.lease.runtime_id,
                    actor: Actor {
                        identity: format!("worker:{}", self.permit.operation_id),
                        roles: BTreeSet::new(),
                        principal: None,
                    },
                    kind: "review_feedback".into(),
                    canonical_targets: vec![self.permit.work_key.clone()],
                    expected: BTreeMap::new(),
                    payload: json!({}),
                },
                &ProviderMutation::GithubComment {
                    repository: RepositoryIdentity {
                        id: 42,
                        owner: "fixture".into(),
                        name: "repo".into(),
                        owner_id: 7,
                        organization_id: None,
                    },
                    pr: 3,
                    expected_head: "1".repeat(40),
                    body: "component feedback".into(),
                },
            )
            .unwrap()
    }
    fn observed(&self, request: &OriginalRequest) -> (bool, bool, Result<crate::control::Receipt>) {
        self.control
            .validate_permit_observed(&self.permit, |snapshot| {
                let ReceiptRead {
                    held,
                    pending,
                    receipt,
                    ..
                } = observe_requests(&self.control, snapshot, std::slice::from_ref(request))
                    .pop()
                    .unwrap();
                Ok((held, pending, receipt))
            })
            .unwrap()
    }
}
#[test]
fn original_provider_binding_and_control_snapshot_distinguish_pending_ack_and_missing() {
    let f = Fixture::new();
    let request = f.request();
    assert_eq!(request.kind, "github_comment");
    assert_eq!(
        request.canonical_targets,
        vec!["github:https://github.invalid/api/v3:42"]
    );
    assert_eq!(request.payload["account_id"], "7");
    let original = OriginalRequest {
        operation: request.request_id,
        identity: crate::control::broker_request_digest(&request, &f.permit).unwrap(),
    };
    assert_eq!(f.observed(&original).2.unwrap_err().code, "record_missing");
    let RemoteReservation::Prepared(prepared) = f
        .control
        .reserve_remote(request, Some(f.permit.clone()), None, |_| Ok(()))
        .unwrap()
    else {
        panic!("fresh actual reservation required")
    };
    let (held, pending, receipt) = f.observed(&original);
    let receipt = receipt.unwrap();
    assert_eq!(receipt.phase, ReceiptPhase::Prepared);
    assert!(held && pending);
    assert!(matches!(
        receipt_state(&receipt, held, pending),
        BrokerReceiptState::Unknown
    ));
    let calls = AtomicUsize::new(0);
    let (effect, ()) = f
        .control
        .launch_remote(
            prepared,
            Ok(()),
            |_| Ok(()),
            |receipt| {
                assert_eq!(receipt.phase, ReceiptPhase::Sent);
                calls.fetch_add(1, Ordering::SeqCst);
            },
        )
        .unwrap();
    let (held, pending, receipt) = f.observed(&original);
    let receipt = receipt.unwrap();
    assert_eq!(receipt.phase, ReceiptPhase::Sent);
    assert!(matches!(
        receipt_state(&receipt, held, pending),
        BrokerReceiptState::Unknown
    ));
    // Injected component response is an actual local ACK, not provider readback.
    f.control
        .complete_remote(
            effect,
            Ok((
                BTreeMap::from([("record_id".into(), "component-comment".into())]),
                json!({"requires_observation":true}),
            )),
        )
        .unwrap();
    let (held, pending, receipt) = f.observed(&original);
    let receipt = receipt.unwrap();
    assert!(!held && !pending);
    assert!(matches!(
        receipt_state(&receipt, held, pending),
        BrokerReceiptState::AcknowledgedOnly
    ));
    assert_eq!(
        receipt.observed_result.as_ref().unwrap()["requires_observation"],
        true
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(f.wire.0.load(Ordering::SeqCst), 0);
    let leaf = format!("{}.json", original.operation);
    let bytes = f.receipts.read_bytes(&leaf, 256 * 1024).unwrap();
    for change in 0..4 {
        let mut changed = receipt.clone();
        match change {
            0 => {
                changed.expected.remove("admission_operation");
            }
            1 => changed.actor = "different-owner".into(),
            2 => changed.canonical_targets = vec!["different-target".into()],
            _ => changed.payload_digest = bytes_digest(b"substituted"),
        }
        f.receipts.write(&leaf, &changed).unwrap();
        assert_eq!(
            f.observed(&original).2.unwrap_err().code,
            "observation_conflict"
        );
    }
    f.receipts.write_bytes(&leaf, &bytes).unwrap();
    std::fs::remove_file(f.receipts.path().join(&leaf)).unwrap();
    assert_eq!(f.observed(&original).2.unwrap_err().code, "record_missing");
    f.receipts.write_bytes(&leaf, &bytes).unwrap();
    assert_eq!(
        canonical_digest(1, &f.observed(&original).2.unwrap()).unwrap(),
        canonical_digest(1, &receipt).unwrap()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let rejected_request = f.request();
    let rejected = OriginalRequest {
        operation: rejected_request.request_id,
        identity: crate::control::broker_request_digest(&rejected_request, &f.permit).unwrap(),
    };
    let RemoteReservation::Prepared(prepared) = f
        .control
        .reserve_remote(rejected_request, Some(f.permit.clone()), None, |_| Ok(()))
        .unwrap()
    else {
        panic!("fresh")
    };
    let result = f.control.launch_remote(
        prepared,
        Err(Error::new(
            "component_preflight",
            "actual preflight refusal",
        )),
        |_| Ok(()),
        |_| {
            calls.fetch_add(1, Ordering::SeqCst);
        },
    );
    assert_eq!(result.err().unwrap().code, "component_preflight");
    let (held, pending, receipt) = f.observed(&rejected);
    let receipt = receipt.unwrap();
    assert!(!held && !pending);
    assert!(matches!(
        receipt_state(&receipt, held, pending),
        BrokerReceiptState::NotSent
    ));
    assert_eq!(
        receipt.observed_result.as_ref().unwrap()["effect"],
        "not_sent"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
#[test]
fn exited_transport_with_unknown_outcome_keeps_original_member_and_hold() {
    let f = Fixture::new();
    let request = f.request();
    let original = OriginalRequest {
        operation: request.request_id,
        identity: crate::control::broker_request_digest(&request, &f.permit).unwrap(),
    };
    let RemoteReservation::Prepared(prepared) = f
        .control
        .reserve_remote(request, Some(f.permit.clone()), None, |_| Ok(()))
        .unwrap()
    else {
        panic!("fresh")
    };
    let (effect, ()) = f
        .control
        .launch_remote(prepared, Ok(()), |_| Ok(()), |_| ())
        .unwrap();
    let error = f
        .control
        .complete_remote(
            effect,
            Err(Error::new(
                "component_lost_ack",
                "actual local response unavailable",
            )),
        )
        .unwrap_err();
    assert_eq!(error.code, "operation_uncertain");
    f.control.remote_task_exited(original.operation).unwrap();
    let (held, pending, receipt) = f.observed(&original);
    let receipt = receipt.unwrap();
    assert!(held);
    assert!(!pending);
    assert_eq!(receipt.phase, ReceiptPhase::Sent);
    assert!(matches!(
        receipt_state(&receipt, held, pending),
        BrokerReceiptState::Unknown
    ));
    assert_eq!(f.wire.0.load(Ordering::SeqCst), 0);
}
#[test]
fn admitted_source_observation_uses_the_returned_bytes_and_refuses_traversal() {
    use crate::isolation::{IsolationPlan, PathGrant};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let repository = root.join("source");
    let scratch = root.join("scratch");
    std::fs::create_dir(&repository).unwrap();
    std::fs::create_dir(&scratch).unwrap();
    std::fs::write(repository.join("limit.py"), b"return count < limit\n").unwrap();
    let isolation = IsolationPlan {
        schema_version: 1,
        role: ExecutionRole::IndependentReview,
        execution_root: root,
        repository: repository.clone(),
        cwd: repository.clone(),
        scratch,
        read_roots: vec![PathGrant {
            path: repository.clone(),
            directory: true,
        }],
        source_writes: vec![],
        resource_writes: vec![],
        forbidden_roots: vec![],
        executables: vec![],
    }
    .validate()
    .unwrap();
    let mut observed = None;
    let value = read_source(&isolation, "limit.py", |path, bytes, value| {
        observed = Some((path.to_owned(), bytes.to_vec(), value.clone()));
        // Changing the file after the actual descriptor read cannot substitute
        // the bytes already supplied to the observer or returned to the worker.
        std::fs::write(repository.join("limit.py"), b"changed later\n").unwrap();
    })
    .unwrap();
    let (path, bytes, captured) = observed.unwrap();
    assert_eq!(path, "limit.py");
    assert_eq!(bytes, b"return count < limit\n");
    assert_eq!(captured, value);
    assert_eq!(value["digest"], bytes_digest(&bytes));
    let mut called = false;
    assert_eq!(
        read_source(&isolation, "../outside", |_, _, _| called = true)
            .unwrap_err()
            .code,
        "scope_violation"
    );
    assert!(!called);
    // The file is valid bounded UTF-8, but JSON escaping makes the public tool
    // Value exceed MAX_TOOL_BYTES. No successful returned-value fact exists.
    std::fs::write(repository.join("escaped.txt"), vec![0u8; 20_000]).unwrap();
    let mut observed_oversize = false;
    assert_eq!(
        read_source(&isolation, "escaped.txt", |_, _, _| observed_oversize =
            true)
        .unwrap_err()
        .code,
        "tool_limit"
    );
    assert!(!observed_oversize);
}

#[path = "worker_broker_call_tests.rs"]
mod call_components;
