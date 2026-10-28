//! Component-only actual Control effect gate around the deterministic duplex.
//! Uses the shared production dispatch boundary; no WorkerHost or joined run.
use super::*;
use crate::codex::RequestGate;
use crate::control::{Actor, InFlightEffect, MutationRequest, Permit, ReceiptPhase};
use std::sync::Mutex;

pub(super) struct ValidationProtocol {
    runtime: Arc<crate::runtime::RuntimeCoordinator>,
    permit: Permit,
    authority: FreshGate,
    current: crate::delivery::GateCheck,
    pending: Mutex<BTreeMap<u64, (String, InFlightEffect)>>,
    completed: Mutex<Vec<Uuid>>,
}
impl ValidationProtocol {
    pub(super) fn new(
        runtime: Arc<crate::runtime::RuntimeCoordinator>,
        permit: Permit,
        authority: FreshGate,
        current: crate::delivery::GateCheck,
    ) -> Self {
        Self {
            runtime,
            permit,
            authority,
            current,
            pending: Mutex::new(BTreeMap::new()),
            completed: Mutex::new(Vec::new()),
        }
    }
    pub(super) fn assert_closed_effect(&self, operation: Uuid, method: &str) {
        assert_eq!(operation, self.permit.operation_id);
        assert!(self.pending.lock().unwrap().is_empty());
        let effects = self.completed.lock().unwrap();
        assert_eq!(effects.len(), 1);
        let effect = self.runtime.control.operation(effects[0]).unwrap();
        assert_eq!(effect.kind, format!("codex/{method}"));
        assert_eq!(effect.phase, ReceiptPhase::Acknowledged);
        assert_eq!(
            effect.expected["admission_operation"],
            operation.to_string()
        );
        let acknowledged = effect.observed_result.as_ref().unwrap();
        assert_eq!(acknowledged["provider_acknowledged"], true);
        assert_eq!(acknowledged["method"], method);
        let model = self.runtime.control.operation(operation).unwrap();
        assert_eq!(model.phase, ReceiptPhase::Reconciled);
        let retired = model.observed_result.as_ref().unwrap();
        assert_eq!(retired["local_execution"], "exited");
        assert_eq!(retired["unresolved_effects"], json!([]));
    }
}
impl RequestGate for ValidationProtocol {
    fn dispatch(
        &self,
        id: u64,
        method: &str,
        frame: &Value,
        transmit: &mut dyn FnMut() -> Result<()>,
    ) -> Result<()> {
        let effect = crate::worker_run::dispatch_protocol_effect(
            &self.runtime.control,
            &self.permit,
            MutationRequest {
                request_id: Uuid::new_v4(),
                runtime_id: self.permit.lease.runtime_id,
                actor: Actor {
                    identity: format!("owner:{}", self.permit.lease.owner_id),
                    principal: None,
                    roles: Default::default(),
                },
                kind: format!("codex/{method}"),
                canonical_targets: vec![format!("codex-session:{}", self.permit.operation_id)],
                expected: BTreeMap::from([("rpc_id".into(), id.to_string())]),
                payload: frame.clone(),
            },
            None,
            || {
                self.runtime
                    .registration
                    .verify_active_owner(&self.permit.lease)?;
                (self.current)(&self.authority)
            },
            transmit,
        )?;
        assert!(
            self.pending
                .lock()
                .unwrap()
                .insert(id, (method.into(), effect))
                .is_none()
        );
        Ok(())
    }
    fn observed(&self, id: u64, method: &str, result: &Result<Value>) -> Result<()> {
        let (sent_method, effect) = self.pending.lock().unwrap().remove(&id).unwrap();
        assert_eq!(sent_method, method);
        let digest = match result {
            Ok(value) => canonical_digest(1, value)?,
            Err(error) => canonical_digest(1, error)?,
        };
        let receipt = self.runtime.control.complete_effect(
            effect,
            &self.permit,
            || self.runtime.registration.verify_active_owner(&self.permit.lease),
            json!({"method":method,"rpc_id":id,"provider_acknowledged":result.is_ok(),"response_digest":digest}),
        )?;
        self.completed.lock().unwrap().push(receipt.operation_id);
        Ok(())
    }
    fn uncertain(&self, _: u64, _: &str) -> Result<()> {
        self.runtime.control.fence(self.permit.operation_id)
    }
}

// The wire terminal is actual; local process-exit metadata below remains the
// existing synthetic component boundary, never runtime joined-cache evidence.
pub(super) async fn failed_validation(
    host: &crate::control::ControlHost,
    fixture: &WorkerFixture,
    max_turns: u32,
    retry_after_ms: Option<u64>,
    failure_code: Option<&str>,
    request_gate: Arc<ValidationProtocol>,
) -> Arc<RunCompletion> {
    use crate::codex::{CodexProfile, RpcClient, SessionBinding, SessionState};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    struct Deny;
    impl crate::codex::ToolBroker for Deny {
        fn call<'a>(
            &'a mut self,
            _: &'a crate::codex::ToolCall,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send + 'a>>
        {
            Box::pin(async {
                Err(Error::new(
                    "fixture_no_execution",
                    "no actual command execution in validation component fixture",
                ))
            })
        }
    }
    let contract = &fixture.contract;
    let manifest = &contract.instructions().manifest;
    let WorkerAuthority::Authorized(gate) = contract.authority() else {
        panic!()
    };
    let actual = host.operation(manifest.operation_id).unwrap();
    let admitted = RunReference::from_receipt(&actual).unwrap().unwrap();
    assert_eq!(
        admitted.instruction_digest,
        contract.instructions().manifest_digest
    );
    assert_eq!(admitted.max_turns, max_turns);
    let mut state = SessionState::new(
        SessionBinding {
            schema_version: 1,
            account_id: contract.account_id().into(),
            work_identity: gate.work_key().unwrap(),
            operation_id: manifest.operation_id.to_string(),
            generation: manifest.work_generation.clone(),
            instruction_digest: contract.instructions().manifest_digest.clone(),
            isolation_digest: manifest.permissions.enforcement_profile.clone(),
            profile: CodexProfile {
                model: manifest.profile.model.clone(),
                reasoning: manifest.profile.reasoning.clone(),
                speed: match manifest.profile.speed {
                    kegari_config::Speed::Standard => "standard",
                    kegari_config::Speed::Fast => "fast",
                }
                .into(),
            },
            cwd: manifest.cwd.clone(),
        },
        max_turns,
    )
    .unwrap();
    state.thread_id = Some("failed-validation-fixture".into());
    let (client, server) = tokio::io::duplex(16384);
    let (cr, cw) = tokio::io::split(client);
    let (sr, mut sw) = tokio::io::split(server);
    let failure_code = failure_code.map(str::to_owned);
    let server = tokio::spawn(async move {
        let mut lines = BufReader::new(sr).lines();
        let init: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        sw.write_all(format!("{}\n",json!({"id":init["id"],"result":{"userAgent":"fixture","platformFamily":"unix","platformOs":"macos","codexHome":"/fixture/auth"}})).as_bytes()).await.unwrap();
        let _ = lines.next_line().await.unwrap().unwrap();
        let request: Value =
            serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(request["method"], "turn/start");
        sw.write_all(format!("{}\n{}\n",json!({"id":request["id"],"result":{"turn":{"id":"failed-validation-turn","status":"inProgress"}}}),json!({"method":"turn/completed","params":{"threadId":"failed-validation-fixture","turn":{"id":"failed-validation-turn","status":"failed","items":[],"error":{"message":"synthetic provider failure","codexErrorInfo":failure_code}}}})).as_bytes()).await.unwrap();
    });
    let mut rpc = RpcClient::from_transport(cr, cw, std::time::Duration::from_secs(2)).unwrap();
    rpc.bind_request_gate(request_gate).unwrap();
    rpc.initialize().await.unwrap();
    let permissions = crate::codex::PermissionConfiguration::for_isolation(
        contract.isolation(),
        &contract.isolation().plan().execution_root.join("auth"),
    )
    .unwrap();
    rpc.start_turn(
        &mut state,
        &permissions,
        "Validate the exact host-prepared source using the required checks",
        &mut Deny,
    )
    .await
    .unwrap();
    while state.terminal.is_none() {
        rpc.pump(
            &mut state,
            &mut Deny,
            std::time::Duration::from_secs(2),
            std::time::Duration::from_secs(2),
        )
        .await
        .unwrap();
    }
    server.await.unwrap();
    assert_eq!(state.terminal.as_deref(), Some("failed"));
    assert!(state.turn_id.is_none());
    let mut error = Error::new("turn_incomplete", "synthetic provider failed terminal");
    if let Some(retry_after_ms) = retry_after_ms {
        error = error.detail(json!({"retry_after_ms":retry_after_ms}));
    }
    let completion = Arc::new(RunCompletion {
        operation_id: manifest.operation_id,
        state,
        outcome: RunOutcome::Failed,
        error: Some(error),
        elapsed_ms: 1,
        hook_error: None,
        checks: vec![],
        failed_checks: vec![],
        validation_plan: None,
        validation_artifacts: vec![],
        validation_attempts: vec![],
        validation_error: None,
        validation_history: None,
        execution: Some(fixture.execution.clone()),
        contract: Some(contract.clone()),
        local_exit_confirmed: true,
    });
    host.retire_local(manifest.operation_id, completion.observation())
        .unwrap();
    completion
}
