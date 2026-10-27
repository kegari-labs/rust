//! Real worker/broker/Control/journal components with an explicitly synthetic
//! transport. Private test-seeded diagnostics are not evaluator enrollment.
#![allow(dead_code)] // Reused fixture also supports the full integration target.

use crate::test_pinned_git as pinned_git;
include!("../tests/support/worker_fixture.rs");
#[path = "worker_bundle_tests.rs"]
mod bundle_tests;
#[path = "worker_command_tests.rs"]
mod command_tests;
#[path = "worker_validation_tests.rs"]
mod validation_tests;

use super::super::{
    BrokerCapture, BrokerObservationState, BrokerReceiptState, CaptureBudget, OriginalRequest,
    observe_requests, receipt_state,
};
use crate::providers::{
    Credential, GithubClient, HttpPolicy, HttpRequest, HttpResponse, HttpTransport, Method,
};
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::sync::atomic::AtomicUsize;
use std::task::{Context, Poll};
use tokio::sync::oneshot;
use tokio::time::{Instant, timeout, timeout_at};

const WAIT: StdDuration = StdDuration::from_secs(10);
const BODY: &str = "component feedback, not a review attestation";

struct FeedbackWire {
    head: String,
    requests: AtomicUsize,
    posts: AtomicUsize,
    posted: Mutex<Option<oneshot::Sender<()>>>,
    reply: Mutex<Option<oneshot::Receiver<()>>>,
}
impl HttpTransport for FeedbackWire {
    fn send(
        &self,
        request: HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Result<HttpResponse>> + Send + '_>> {
        Box::pin(async move {
            let step = self.requests.fetch_add(1, Ordering::SeqCst);
            let expected = match step {
                0 => (Method::Get, "/api/v3/user"),
                1 => (Method::Get, "/api/v3/repos/fixture/repo"),
                2 => (Method::Get, "/api/v3/repos/fixture/repo/pulls/1"),
                3 => (Method::Post, "/api/v3/repos/fixture/repo/issues/1/comments"),
                _ => {
                    return Err(Error::new(
                        "component_wire",
                        "unlisted repeated provider request",
                    ));
                }
            };
            if request.method != expected.0
                || request.url.as_str() != format!("https://github.invalid{}", expected.1)
                || request
                    .headers
                    .get("authorization")
                    .and_then(|h| h.to_str().ok())
                    != Some("Bearer component-only-token")
            {
                return Err(Error::new(
                    "component_wire",
                    "provider request differs from exact fixture route",
                ));
            }
            let value = match step {
                0 => json!({"id":7,"login":"fixture","type":"User"}),
                1 => json!({"id":42,"name":"repo","owner":{"id":7,"login":"fixture","type":"User"},
                    "default_branch":"main","private":true,"archived":false,"disabled":false,"permissions":{"pull":true}}),
                2 => json!({"id":40,"number":1,"user":{"id":7},"state":"open","draft":false,
                    "head":{"sha":self.head,"ref":"feature","repo":{"id":42}},
                    "base":{"sha":self.head,"ref":"main","repo":{"id":42}},
                    "merged":false,"merge_commit_sha":null,"html_url":"https://github.invalid/fixture/repo/pull/1"}),
                _ => {
                    let body: Value = serde_json::from_slice(&request.body)
                        .map_err(|_| Error::invalid("component request JSON"))?;
                    if body != json!({"body":BODY}) {
                        return Err(Error::new("component_wire", "unexpected feedback body"));
                    }
                    self.posts.fetch_add(1, Ordering::SeqCst);
                    let reply = self
                        .reply
                        .lock()
                        .unwrap()
                        .take()
                        .ok_or_else(|| Error::invalid("duplicate component POST"))?;
                    self.posted
                        .lock()
                        .unwrap()
                        .take()
                        .ok_or_else(|| Error::invalid("duplicate POST handshake"))?
                        .send(())
                        .map_err(|_| Error::invalid("POST receiver closed"))?;
                    reply
                        .await
                        .map_err(|_| Error::new("component_wire", "reply gate closed"))?;
                    json!({"id":91})
                }
            };
            Ok(HttpResponse {
                status: 200,
                headers: Default::default(),
                body: serde_json::to_vec(&value).unwrap(),
            })
        })
    }
}

struct ReplyGuard(Option<oneshot::Sender<()>>);
impl ReplyGuard {
    fn release(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}
impl Drop for ReplyGuard {
    fn drop(&mut self) {
        self.release();
    }
}

struct Obstruction {
    path: PathBuf,
    identity: Option<(u64, u64)>,
}
impl Obstruction {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            identity: None,
        }
    }
    fn create(&mut self) -> Result<()> {
        let mut builder = std::fs::DirBuilder::new();
        builder
            .mode(0o700)
            .create(&self.path)
            .map_err(|error| Error::io("create component obstruction", error))?;
        let metadata = std::fs::symlink_metadata(&self.path)
            .map_err(|error| Error::io("capture component obstruction", error))?;
        self.identity = Some((metadata.dev(), metadata.ino()));
        Ok(())
    }
    fn remove(&self) -> Result<()> {
        let (device, inode) = self.identity.ok_or_else(|| {
            Error::new(
                "component_cleanup",
                "obstruction creation has no captured identity",
            )
        })?;
        let metadata = match std::fs::symlink_metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(Error::io("inspect component obstruction", error)),
        };
        if !metadata.is_dir() || metadata.dev() != device || metadata.ino() != inode {
            return Err(Error::new(
                "component_cleanup",
                "owned obstruction identity changed",
            ));
        }
        std::fs::remove_dir(&self.path)
            .map_err(|error| Error::io("remove owned empty obstruction", error))
    }
}
impl Drop for Obstruction {
    fn drop(&mut self) {
        let _ = self.remove();
    }
}

// Preserve the real fixture outside a panicking/expired assertion future, so its
// provider reply and owned storage are cleaned up before propagating the failure.
struct CatchAssertions<F>(Pin<Box<F>>);
impl<F: Future> Future for CatchAssertions<F> {
    type Output = std::thread::Result<F::Output>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.get_mut().0.as_mut().poll(cx)
        })) {
            Ok(Poll::Pending) => Poll::Pending,
            Ok(Poll::Ready(value)) => Poll::Ready(Ok(value)),
            Err(error) => Poll::Ready(Err(error)),
        }
    }
}

fn enroll_component_diagnostics(worker: &mut ScopedWorker) {
    // No DeclaredProtocolTrial, Runtime registry, native run or successful host
    // observation is constructed. Only this private cfg(test) collector state
    // is seeded from the actual component contract and permit.
    assert!(worker.broker_capture.is_none() && worker.protocol_capture.is_none());
    let budget = Arc::new(CaptureBudget::default());
    worker.capture_budget = Some(budget.clone());
    worker.broker_capture = Some(BrokerCapture {
        report: IndependentBrokerObservation {
            schema_version: 1,
            operation: worker.permit.operation_id,
            runtime: worker.permit.lease.runtime_id,
            workspace: worker.contract.authority.workspace(),
            trial_digest: bytes_digest(b"synthetic component diagnostic only"),
            suite_digest: bytes_digest(b"no declared evaluator"),
            fixture_digest: bytes_digest(b"actual worker test fixture"),
            instruction_digest: worker.contract.assembled.manifest_digest.clone(),
            contract_scope_digest: worker.contract.scope_digest().into(),
            context_digest: worker.contract.assembled.manifest.context_digest.clone(),
            tool_contract_digest: worker.contract.tools.digest().into(),
            isolation_digest: worker.contract.isolation.digest().into(),
            state: BrokerObservationState::Partial,
            members: vec![],
            receipts: vec![],
            authority_at_collection: None,
            gaps: vec!["test_seeded_diagnostic_not_native_enrollment".into()],
        },
        requests: vec![],
        budget,
    });
}
fn journal_bytes(journal: &PrivateDir) -> BTreeMap<String, Vec<u8>> {
    journal
        .records_bounded(128)
        .unwrap()
        .into_iter()
        .map(|name| {
            let bytes = journal.read_bytes(&name, 256 * 1024).unwrap();
            (name, bytes)
        })
        .collect()
}
fn recorded_request(journal: &PrivateDir, call: &ToolCall) -> OriginalRequest {
    let key = format!("{}:{}:{}", call.thread_id, call.turn_id, call.call_id);
    let records: Vec<Value> = journal
        .records_bounded(128)
        .unwrap()
        .iter()
        .map(|name| journal.read(name).unwrap())
        .collect();
    let starts: Vec<_> = records
        .iter()
        .filter(|v| v["kind"] == "tool_started" && v["record"]["call"] == key)
        .collect();
    assert_eq!(starts.len(), 1);
    let operation: Uuid =
        serde_json::from_value(starts[0]["record"]["request_id"].clone()).unwrap();
    let semantics: Vec<_> = records
        .iter()
        .filter(|v| {
            v["kind"] == "broker_observation"
                && v["record"]["operation"] == json!(operation)
                && v["record"]["observation"]["state"] == "feedback_intent"
        })
        .collect();
    assert_eq!(semantics.len(), 1);
    assert_eq!(
        semantics[0]["record"]["observation"]["body_digest"],
        bytes_digest(BODY.as_bytes())
    );
    OriginalRequest {
        operation,
        identity: semantics[0]["record"]["observation"]["request_identity"]
            .as_str()
            .unwrap()
            .into(),
    }
}
fn effect(
    control: &ControlHost,
    permit: &Permit,
    original: &OriginalRequest,
) -> super::super::ReceiptRead {
    control
        .validate_permit_observed(permit, |snapshot| {
            Ok(observe_requests(control, snapshot, std::slice::from_ref(original)).remove(0))
        })
        .unwrap()
}
fn safely_settled(receipt: &Receipt) -> bool {
    match receipt.phase {
        ReceiptPhase::Acknowledged => true,
        ReceiptPhase::Reconciled => receipt
            .observed_result
            .as_ref()
            .and_then(Value::as_object)
            .is_some_and(|value| {
                value.len() == 2
                    && value.get("effect") == Some(&json!("not_sent"))
                    && value
                        .get("error")
                        .is_some_and(|error| serde_json::from_value::<Error>(error.clone()).is_ok())
            }),
        _ => false,
    }
}
async fn settled_effect(
    control: &ControlHost,
    permit: &Permit,
    original: &OriginalRequest,
    deadline: Instant,
) -> Result<()> {
    timeout_at(deadline, async {
        loop {
            let observation = control.validate_permit_observed(permit, |snapshot| {
                Ok(observe_requests(control, snapshot, std::slice::from_ref(original)).remove(0))
            })?;
            let receipt = observation.receipt?;
            if !observation.pending {
                if !observation.held && safely_settled(&receipt) {
                    return Ok(());
                }
                return Err(Error::new(
                    "component_cleanup",
                    "original provider effect is not safely settled",
                ).detail(json!({"operation":original.operation,"held":observation.held,"phase":receipt.phase})));
            }
            // Wait only for this exact original dispatch. Pending-clear alone
            // is not settlement: Sent/Unknown and retained holds refuse above.
            // No sleep, resubmission, fresh UUID or native-exit inference.
            tokio::task::yield_now().await;
        }
    })
    .await
    .map_err(|_| {
        Error::new(
            "component_cleanup",
            "original provider task still lacks a safely settled Control observation",
        )
    })?
}
fn verify_retirement(receipt: &Receipt, permit: &Permit, expected: &Value) -> Result<()> {
    let closed = receipt
        .observed_result
        .as_ref()
        .and_then(Value::as_object)
        .is_some_and(|value| {
            value.len() == 3
                && value.get("local_execution") == Some(&json!("exited"))
                && value.get("observation") == Some(expected)
                && value
                    .get("unresolved_effects")
                    .and_then(Value::as_array)
                    .is_some_and(Vec::is_empty)
        });
    if receipt.operation_id != permit.operation_id
        || receipt.runtime_id != permit.lease.runtime_id
        || receipt.kind != "admission"
        || receipt.phase != ReceiptPhase::Reconciled
        || !closed
    {
        return Err(Error::new(
            "component_cleanup",
            "component retirement retains unresolved effects or differs from its actual owner",
        ));
    }
    Ok(())
}
fn pending_report(
    fixture: &Fixture,
    original: &OriginalRequest,
    has_result: bool,
) -> IndependentBrokerObservation {
    let report = fixture
        .worker
        .observe()
        .unwrap()
        .independent_broker
        .unwrap();
    report.validate().unwrap();
    assert!(report.authority_at_collection.is_none() && report.receipts.is_empty());
    assert!(
        report
            .gaps
            .iter()
            .any(|s| s == "boundary_observation_unavailable")
    );
    assert!(report.gaps.iter().any(|s| s == "broker_call_pending"));
    assert_eq!(report.members.len(), 1);
    let member = &report.members[0];
    assert_eq!(member.operation, original.operation);
    assert_eq!(member.thread, fixture.state.thread_id.as_deref().unwrap());
    assert_eq!(member.turn, fixture.state.turn_id.as_deref().unwrap());
    assert_eq!(member.tool, "review_feedback");
    assert_eq!(
        member.argument_digest,
        canonical_digest(
            1,
            &json!({"tool":"review_feedback","arguments":{"body":BODY}})
        )
        .unwrap()
    );
    let key = format!("{}:{}:{}", member.thread, member.turn, member.call);
    assert_eq!(
        fixture.worker.calls.get(&key).unwrap().operation,
        original.operation
    );
    assert!(member.finished.is_none());
    assert_eq!(member.result_digest.is_some(), has_result);
    assert_eq!(member.semantic.len(), 1);
    let capture = fixture.worker.broker_capture.as_ref().unwrap();
    assert_eq!(capture.requests.len(), 1);
    assert_eq!(capture.requests[0].operation, original.operation);
    assert_eq!(capture.requests[0].identity, original.identity);
    assert_eq!(fixture.worker.calls.len(), 1);
    assert!(
        fixture
            .worker
            .calls
            .values()
            .all(|c| c.operation == original.operation && c.result.is_none())
    );
    report
}

async fn run_component(fail_finished: bool) {
    let mut fixture = Fixture::new(AgentRole::IndependentReview);
    let (posted, mut reached) = oneshot::channel();
    let (reply, receiver) = oneshot::channel();
    let wire = Arc::new(FeedbackWire {
        head: fixture.contract.review.as_ref().unwrap().head.clone(),
        requests: AtomicUsize::new(0),
        posts: AtomicUsize::new(0),
        posted: Mutex::new(Some(posted)),
        reply: Mutex::new(Some(receiver)),
    });
    fixture.worker.providers = ProviderBroker {
        linear: None,
        github: Some(
            GithubClient::with_transport(
                HttpPolicy::new("https://github.invalid/api/v3", "7".into(), WAIT + WAIT).unwrap(),
                kegari_v0::config::GithubSelection {
                    repositories: vec!["42".into()],
                    organizations: vec![],
                    pr_authors: vec!["7".into()],
                },
                Credential::new("component-only-token".into()).unwrap(),
                Some(wire.clone()),
            )
            .unwrap(),
        ),
    };
    enroll_component_diagnostics(&mut fixture.worker);
    let journal = PrivateDir::open(fixture.worker.journal.dir.path()).unwrap();
    let call = fixture.call("review_feedback", json!({"body":BODY}));
    let control = fixture.control.clone();
    let permit = fixture.permit.clone();
    let mut release = ReplyGuard(Some(reply));
    let mut obstruction = None;
    let mut original = None;
    let mut cleanup_deadline = Instant::now() + WAIT;
    let assertions = timeout(WAIT + WAIT, CatchAssertions(Box::pin(async {
        let mut pending = fixture.worker.call(&call);
        tokio::select! {
            early = &mut pending => panic!("call finished before actual POST handshake: {early:?}"),
            observed = timeout(WAIT, &mut reached) => observed.expect("bounded POST handshake").expect("actual POST"),
        }
        cleanup_deadline = Instant::now() + WAIT;
        let request = recorded_request(&journal, &call);
        original = Some(request.clone());
        let sent = effect(&control, &permit, &request);
        assert!(sent.held && sent.pending);
        let sent_receipt = sent.receipt.unwrap();
        assert_eq!(sent_receipt.phase, ReceiptPhase::Sent);
        assert!(matches!(receipt_state(&sent_receipt, true, true), BrokerReceiptState::Unknown));
        assert_eq!(wire.posts.load(Ordering::SeqCst), 1);
        let before = journal_bytes(&journal);
        if fail_finished {
            let names: Vec<_> = before.keys().filter(|name| name.starts_with("event-")).cloned().collect();
            assert_eq!(names, (0..names.len()).map(|n| format!("event-{n:08}.json")).collect::<Vec<_>>());
            let path = journal.path().join(format!("event-{:08}.json", names.len()));
            obstruction = Some(Obstruction::new(path));
            obstruction.as_mut().unwrap().create().unwrap();
            release.release();
            let result = timeout_at(cleanup_deadline, &mut pending).await.expect("actual original call completion");
            drop(pending);
            let error = result.unwrap_err();
            assert_eq!(error.code, "unsafe_storage");
            assert_eq!(error.message, "control data must be owner-only, owned by this OS user, and not linked");
            obstruction.as_ref().unwrap().remove().unwrap();
            assert_eq!(journal_bytes(&journal), before);
            let report = pending_report(&fixture, &request, true);
            assert!(report.gaps.iter().any(|s| s == "unsafe_storage"));
            let acknowledged = effect(&control, &permit, &request);
            assert!(!acknowledged.held && !acknowledged.pending);
            let receipt = acknowledged.receipt.unwrap();
            assert_eq!(receipt.phase, ReceiptPhase::Acknowledged);
            assert_eq!(receipt.observed_result.as_ref().unwrap()["requires_observation"], true);
            let actual_result: Result<Value> = Ok(json!({"operation_id":receipt.operation_id,"state":"feedback_recorded","remote_ids":receipt.remote_ids,"attestation":false}));
            let actual_digest = canonical_digest(1, &actual_result).unwrap();
            assert_eq!(report.members[0].result_digest.as_deref(), Some(actual_digest.as_str()));
        } else {
            // Only the caller waiter is cancelled. Production's owned broker
            // task still has the original POST, and its ACK is withheld.
            drop(pending);
            let before_reply = pending_report(&fixture, &request, false);
            assert!(matches!(before_reply.state, BrokerObservationState::Unavailable));
            assert_eq!(fixture.worker.call(&call).await.unwrap_err().code, "operation_uncertain");
            assert_eq!(wire.posts.load(Ordering::SeqCst), 1);
            assert_eq!(journal_bytes(&journal), before);
            release.release();
            settled_effect(&control, &permit, &request, cleanup_deadline).await.unwrap();
            let acknowledged = effect(&control, &permit, &request);
            assert!(!acknowledged.held && !acknowledged.pending);
            let receipt = acknowledged.receipt.unwrap();
            assert!(matches!(receipt_state(&receipt, false, false), BrokerReceiptState::AcknowledgedOnly));
            assert_eq!(receipt.observed_result.as_ref().unwrap()["requires_observation"], true);
            let after_reply = pending_report(&fixture, &request, false);
            assert_eq!(canonical_digest(1, &before_reply.members).unwrap(), canonical_digest(1, &after_reply.members).unwrap());
        }
        let prior = journal_bytes(&journal);
        let report = pending_report(&fixture, &request, fail_finished);
        assert_eq!(fixture.worker.call(&call).await.unwrap_err().code, "operation_uncertain");
        let replay = pending_report(&fixture, &request, fail_finished);
        assert_eq!(canonical_digest(1, &report.members).unwrap(), canonical_digest(1, &replay.members).unwrap());
        assert_eq!(journal_bytes(&journal), prior);
        assert_eq!(wire.posts.load(Ordering::SeqCst), 1);
        assert_eq!(wire.requests.load(Ordering::SeqCst), 4);
    }))).await;

    // Always release/restore before propagating any assertion or timeout. A
    // cancelled waiter has no public JoinHandle; this only observes the actual
    // safely settled Control receipt with no hold/pending for the same operation, not native
    // join or a fabricated model completion. The other branch awaited call().
    release.release();
    let remove = obstruction.as_ref().map_or(Ok(()), Obstruction::remove);
    let request = original.or_else(|| {
        fixture
            .worker
            .broker_capture
            .as_ref()
            .and_then(|c| c.requests.first().cloned())
    });
    let cleanup = match request {
        Some(request) => settled_effect(&control, &permit, &request, cleanup_deadline).await,
        None if fixture.worker.calls.is_empty() => Ok(()),
        None => Err(Error::new(
            "component_cleanup",
            "call membership lacks an original normalized request",
        )),
    };
    let retirement = if remove.is_ok() && cleanup.is_ok() {
        let observation = json!({"component":"worker call future returned or dropped; no native worker was started"});
        control
            .retire_local(permit.operation_id, observation.clone())
            .and_then(|receipt| verify_retirement(&receipt, &permit, &observation))
    } else {
        Err(Error::new(
            "component_cleanup",
            "retirement waits for owned cleanup",
        ))
    };
    if remove.is_err() || cleanup.is_err() || retirement.is_err() {
        let retained = fixture._temp.keep();
        panic!(
            "retain owned component directory {retained:?}: obstruction={remove:?}; task={cleanup:?}; retirement={retirement:?}"
        );
    }
    match assertions {
        Ok(Ok(())) => (),
        Ok(Err(panic)) => std::panic::resume_unwind(panic),
        Err(error) => panic!("bounded component assertions expired: {error}"),
    }
}

#[tokio::test]
async fn cancelled_component_broker_call_keeps_original_pending_member() {
    run_component(false).await;
}
#[tokio::test]
async fn component_broker_ack_does_not_replace_failed_finished_record() {
    run_component(true).await;
}

#[tokio::test]
async fn uncertain_component_response_refuses_cleanup_and_preserves_retired_hold() {
    // Reuse the real Control fixture from the parent module. The synchronous
    // launch callback and injected response are explicit components, not an
    // owned native run, broker JoinHandle or remote reconciliation.
    let fixture = super::Fixture::new();
    let request = fixture.request();
    let original = OriginalRequest {
        operation: request.request_id,
        identity: crate::control::broker_request_digest(&request, &fixture.permit).unwrap(),
    };
    let super::Fixture {
        _temp,
        control,
        receipts,
        permit,
        ..
    } = fixture;
    // This case deliberately leaves a genuine uncertain effect. Disable
    // TempDir teardown before introducing it, including all assertion paths.
    let retained = _temp.keep();
    eprintln!(
        "retained uncertain broker component evidence: {}",
        retained.display()
    );
    let RemoteReservation::Prepared(prepared) = control
        .reserve_remote(request, Some(permit.clone()), None, |_| Ok(()))
        .unwrap()
    else {
        panic!("actual fresh reservation required")
    };
    let calls = AtomicUsize::new(0);
    let (in_flight, ()) = control
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
    let lost = control
        .complete_remote(
            in_flight,
            Err(Error::new(
                "component_lost_ack",
                "injected response unavailable after actual dispatch",
            )),
        )
        .unwrap_err();
    assert_eq!(lost.code, "operation_uncertain");
    let observed = effect(&control, &permit, &original);
    assert!(observed.held && !observed.pending);
    let receipt = observed.receipt.unwrap();
    assert_eq!(receipt.phase, ReceiptPhase::Sent);
    assert!(matches!(
        receipt_state(&receipt, true, false),
        BrokerReceiptState::Unknown
    ));
    let leaf = format!("{}.json", original.operation);
    let before = receipts.read_bytes(&leaf, 256 * 1024).unwrap();
    let cleanup = settled_effect(&control, &permit, &original, Instant::now() + WAIT)
        .await
        .unwrap_err();
    assert_eq!(cleanup.code, "component_cleanup");
    assert_eq!(
        cleanup.message,
        "original provider effect is not safely settled"
    );
    assert_eq!(cleanup.details["held"], true);
    assert_eq!(cleanup.details["phase"], json!(ReceiptPhase::Sent));
    assert_eq!(receipts.read_bytes(&leaf, 256 * 1024).unwrap(), before);
    // Exercise the second defensive guard against the real retirement result:
    // local exit is allowed, but it is not resolution of the uncertain effect.
    let observation = json!({"component":"synchronous local launch callback returned; no native worker was started"});
    let retired = control
        .retire_local(permit.operation_id, observation.clone())
        .unwrap();
    assert_eq!(
        retired.observed_result.as_ref().unwrap()["unresolved_effects"],
        json!([original.operation])
    );
    assert_eq!(
        verify_retirement(&retired, &permit, &observation)
            .unwrap_err()
            .code,
        "component_cleanup"
    );
    assert!(
        control
            .snapshot()
            .unwrap()
            .unresolved_operations
            .contains_key(&original.operation)
    );
    assert_eq!(receipts.read_bytes(&leaf, 256 * 1024).unwrap(), before);
    assert!(retained.is_dir() && receipts.path().join(&leaf).is_file());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
