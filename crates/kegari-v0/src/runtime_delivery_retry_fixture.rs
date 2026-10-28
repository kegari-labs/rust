//! Actual operator retry publication with synthetic provider wire. Selection is
//! driven by the genuine retained delivery proof, never a fabricated blocker.
use super::*;
use crate::authority::IssueKey;
use crate::control::Receipt;
use crate::decisions::*;
use crate::evidence::VerifiedAcceptance;
use crate::evidence::WorkIdentity;
use crate::reconcile::DependencyOutcomes;
use crate::reconcile::ParentDelivery;
use crate::runtime_host::HostFuture;
use kegari_control_api::identity::Actor;
use std::sync::Mutex;

/// The synthetic completed protocol cannot populate the native joined-run
/// cache. Exercise the real typed control gate with the same current native
/// grant parser and opaque proof, without inventing that cache capability.
pub(super) fn assert_preparation_denied(
    runtime: &RuntimeCoordinator,
    lease: &kegari_types::control::Lease,
    proof: &RetriedUnfinishedWork,
    account: &str,
) -> Error {
    let reference = crate::control::PreparationReference {
        worker_operation: Uuid::new_v4(),
        workspace: proof.authority().workspace_id(),
        account: account.into(),
        phase: proof.phase().clone(),
    };
    let id = reference.operation_id().unwrap();
    let before = runtime.control.operations().unwrap();
    let before_digest = canonical_digest(1, &before).unwrap();
    let mut evaluated = false;
    let error = runtime
        .control
        .admit_preparation(
            lease,
            crate::control::PreparationAdmission {
                request_id: id,
                work_key: proof.authority().work_key().unwrap(),
                scope_digest: canonical_digest(
                    1,
                    &json!({"purpose":"native-grant-preparation-race","proof":proof.digest()}),
                )
                .unwrap(),
                reference,
            },
            || {
                evaluated = true;
                let config = runtime.configuration.snapshot()?;
                let receipts = runtime.control.operations()?;
                let current = crate::runtime_workflow::current_retry_grants(
                    &config,
                    &runtime.observations.admission_snapshot()?,
                    proof.authority().work(),
                    &receipts,
                    &runtime.services.key,
                )?;
                proof.verify_current(&receipts, &current)
            },
        )
        .unwrap_err();
    assert!(
        evaluated,
        "the serialized gate must evaluate the changed native grant"
    );
    assert_eq!(
        runtime.control.operation(id).unwrap_err().code,
        "record_missing"
    );
    assert_eq!(
        canonical_digest(1, &runtime.control.operations().unwrap()).unwrap(),
        before_digest
    );
    assert!(
        runtime
            .control
            .snapshot()
            .unwrap()
            .active_operations
            .iter()
            .all(|p| p.operation_id != id)
    );
    error
}
struct Evidence {
    runtime: Arc<RuntimeCoordinator>,
    proof: Option<Arc<UnfinishedChildWork>>,
    grants: Mutex<Vec<VerifiedRetryGrant>>,
}
impl Evidence {
    fn select(
        &self,
        work: &WorkIdentity,
        expected: Option<&VerifiedRetryGrant>,
    ) -> Result<RetryTarget> {
        let config = self.runtime.configuration.snapshot()?;
        let receipts = self.runtime.control.operations()?;
        let grants = crate::runtime_workflow::current_retry_grants(
            &config,
            &self.runtime.observations.admission_snapshot()?,
            work,
            &receipts,
            &self.runtime.services.key,
        )?;
        if let Some(expected) = expected {
            return crate::runtime_workflow::published_retry_target(
                &config, expected, &grants, &receipts,
            );
        }
        crate::runtime_workflow::select_retry_target_with_unfinished(
            config.runtime.id,
            work,
            &receipts,
            &grants,
            &self.proof.iter().cloned().collect::<Vec<_>>(),
            &config.runtime,
        )
    }
}
impl DecisionEvidence for Evidence {
    fn current(&self, _: &IssueKey) -> Result<ParentDelivery> {
        Ok(ParentDelivery::default())
    }
    fn outcomes(&self) -> Result<DependencyOutcomes> {
        Ok(DependencyOutcomes::default())
    }
    fn retry_target(&self, work: &WorkIdentity) -> Result<RetryTarget> {
        self.select(work, None)
    }
    fn published_retry_target(&self, expected: &VerifiedRetryGrant) -> Result<RetryTarget> {
        self.select(expected.work(), Some(expected))
    }
    fn record_acceptance(&self, _: &IssueKey, _: VerifiedAcceptance) -> Result<()> {
        Err(Error::new(
            "fixture",
            "retry fixture has no acceptance writer",
        ))
    }
    fn record_correction(&self, _: VerifiedExplicitCorrection) -> Result<()> {
        Err(Error::new(
            "fixture",
            "retry fixture has no correction writer",
        ))
    }
    fn retry_committed(&self, grant: VerifiedRetryGrant) -> Result<()> {
        self.grants.lock().unwrap().push(grant);
        Ok(())
    }
    fn release(&self, _: AuthorizedDecision) -> HostFuture<'_, Receipt> {
        Box::pin(async { Err(Error::new("fixture", "no release in retry fixture")) })
    }
    fn recover_release(&self, _: AuthorizedDecision) -> HostFuture<'_, Receipt> {
        Box::pin(async {
            Err(Error::new(
                "fixture",
                "no release recovery in retry fixture",
            ))
        })
    }
    fn reconcile(&self, _: AuthorizedOperationRecovery) -> HostFuture<'_, Receipt> {
        Box::pin(async {
            Err(Error::new(
                "fixture",
                "no effect reconciliation in retry fixture",
            ))
        })
    }
}
pub(super) async fn publish(
    runtime: Arc<RuntimeCoordinator>,
    proof: Arc<UnfinishedChildWork>,
) -> VerifiedRetryGrant {
    publish_inner(runtime, Some(proof)).await
}
pub(super) async fn publish_failed_review(runtime: Arc<RuntimeCoordinator>) -> VerifiedRetryGrant {
    publish_inner(runtime, None).await
}
async fn publish_inner(
    runtime: Arc<RuntimeCoordinator>,
    proof: Option<Arc<UnfinishedChildWork>>,
) -> VerifiedRetryGrant {
    let evidence = Arc::new(Evidence {
        runtime: runtime.clone(),
        proof,
        grants: Mutex::new(vec![]),
    });
    let services = &runtime.services;
    let manager = DecisionManager::new(DecisionDependencies {
        configuration: services.configuration.clone(),
        control: services.control.clone(),
        observations: services.observations.clone(),
        secrets: services.secrets.clone(),
        key: services.key.clone(),
        evidence: evidence.clone(),
        housekeeping: services.housekeeping.clone(),
        inputs: services.inputs.clone(),
    });
    let actor = Actor {
        identity: "operator".into(),
        principal: Some(kegari_control_api::identity::Principal::Unix { uid: 501 }),
        roles: ["recover".into(), "inspect".into(), "lifecycle".into()].into(),
    };
    let mut request = kegari_types::api::CommandRequest {
        api_version: 1,
        request_id: Uuid::new_v4(),
        runtime_id: runtime.configuration.snapshot().unwrap().runtime.id,
        command: kegari_types::api::CommandName::Retry,
        args: json!({"work":"TEST-101","reason":"The exact finding is understood; grant a bounded repair budget after the previous limit"}),
        expected_config_revision: Some(
            crate::config::config_revision(&runtime.configuration.snapshot().unwrap()).unwrap(),
        ),
        expected_control_generation: Some(runtime.control.snapshot().unwrap().intent.generation),
    };
    let mut mutation = crate::control_command::mutation_request(
        &actor,
        &request,
        manager.command_targets(&request).unwrap(),
    );
    mutation.kind = format!("command/{:?}", request.command);
    mutation.payload = json!({"arguments":request.args,"secret_digest":null});
    let (parent, fresh) = runtime
        .control
        .queue_command(mutation, "recover", || Ok(()))
        .unwrap();
    assert!(fresh);
    request.request_id = crate::control_command::command_effect_id(parent.operation_id);
    manager.execute(actor, request, parent).await.unwrap();
    let result = evidence.grants.lock().unwrap().clone();
    assert_eq!(result.len(), 1);
    result.into_iter().next().unwrap()
}
