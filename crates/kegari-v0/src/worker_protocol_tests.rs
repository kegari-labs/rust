//! Real control receipts with deterministic provider transport observations.
use super::*;
use crate::control::Lifecycle;
use kegari_storage::PrivateDir;
use std::collections::BTreeSet;
struct Host {
    authority: WorkerAuthority,
    pending: AtomicBool,
}
impl WorkerHost for Host {
    fn observed_session(&self, _: Uuid, _: &SessionState) -> Result<()> {
        Ok(())
    }
    fn current_authority(&self, _: &WorkerAuthority) -> Result<WorkerAuthority> {
        if self.pending.load(Ordering::SeqCst) {
            return Err(Error::new(
                "correction_classification_pending",
                "new native revision is pending",
            ));
        }
        Ok(self.authority.clone())
    }
    fn verify_owner(&self, _: &kegari_types::control::Lease) -> Result<()> {
        Ok(())
    }
    fn rate_limits(&self, _: Uuid, _: &Value) -> Result<()> {
        Ok(())
    }
    fn reconcile_prior(&self, _: Uuid, _: Option<&Value>) -> Result<()> {
        Ok(())
    }
    fn correction_event(&self, _: Uuid, _: u64, _: &str) -> Result<String> {
        Err(Error::new("unavailable", "no fixture route mutation"))
    }
    fn observed_state(&self, _: Uuid) -> Result<Value> {
        Ok(Value::Null)
    }
}
#[tokio::test]
async fn protocol_ack_records_exact_sent_request_when_native_generation_changes() {
    let (_native, transition) = crate::corrections::test_transition_fixture().await;
    let root = tempfile::TempDir::new().unwrap();
    let runtime = Uuid::new_v4();
    let control = Arc::new(
        ControlHost::setup(
            PrivateDir::create(&root.path().canonicalize().unwrap().join("control")).unwrap(),
            runtime,
            Uuid::new_v4(),
        )
        .unwrap(),
    );
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
                kind: "start".into(),
                canonical_targets: vec![runtime.to_string()],
                expected: BTreeMap::new(),
                payload: Value::Null,
            },
            Lifecycle::Start,
            0,
        )
        .unwrap();
    let lease = kegari_types::control::Lease {
        runtime_id: runtime,
        owner_generation: 1,
        owner_id: Uuid::new_v4(),
    };
    control.attach_reconciled(lease.clone()).unwrap();
    let authority = WorkerAuthority::Authorized(transition.original_authority().clone());
    let permit = control
        .admit(
            &lease,
            Uuid::new_v4(),
            authority.work_key().unwrap(),
            kegari_digest::bytes_digest(b"scope"),
            || Ok(()),
        )
        .unwrap();
    let host = Arc::new(Host {
        authority: authority.clone(),
        pending: AtomicBool::new(false),
    });
    let gate = ProtocolGate {
        control: control.clone(),
        permit: permit.clone(),
        host: host.clone(),
        authority,
        resume_source: None,
        pending: Mutex::new(BTreeMap::new()),
    };
    let mut sent = 0;
    gate.dispatch(
        7,
        "turn/steer",
        &json!({"id":7,"method":"turn/steer","params":{}}),
        &mut || {
            sent += 1;
            Ok(())
        },
    )
    .unwrap();
    host.pending.store(true, Ordering::SeqCst);
    gate.observed(7, "turn/steer", &Ok(json!({"turnId":"actual-turn"})))
        .unwrap();
    let receipt = control
        .operations()
        .unwrap()
        .into_iter()
        .find(|r| r.kind == "codex/turn/steer")
        .unwrap();
    assert_eq!(receipt.phase, crate::control::ReceiptPhase::Acknowledged);
    assert_eq!(
        receipt.observed_result.unwrap()["provider_acknowledged"],
        true
    );
    assert_eq!(
        gate.dispatch(
            8,
            "turn/start",
            &json!({"id":8,"method":"turn/start"}),
            &mut || {
                sent += 1;
                Ok(())
            }
        )
        .unwrap_err()
        .code,
        "correction_classification_pending"
    );
    assert_eq!(sent, 1);
    assert!(gate.observed(7, "turn/steer", &Ok(Value::Null)).is_err());
}
