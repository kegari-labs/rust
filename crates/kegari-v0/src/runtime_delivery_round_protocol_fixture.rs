//! Component-only actual Control effect gate around the deterministic duplex.
//! Uses the shared production dispatch boundary; no WorkerHost or joined run.
use super::*;
use crate::codex::RequestGate;
use crate::control::{Actor, InFlightEffect, MutationRequest, Permit, ReceiptPhase};
use std::sync::Mutex;

pub(super) struct RoundProtocol {
    runtime: Arc<crate::runtime::RuntimeCoordinator>,
    permit: Permit,
    authority: FreshGate,
    current: crate::delivery::GateCheck,
    pending: Mutex<BTreeMap<u64, (String, InFlightEffect)>>,
    completed: Mutex<Vec<Uuid>>,
}
impl RoundProtocol {
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
    pub(super) fn assert_closed_review_effect(&self, operation: Uuid) {
        self.assert_closed_effect(operation, "review/start");
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
impl RequestGate for RoundProtocol {
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
