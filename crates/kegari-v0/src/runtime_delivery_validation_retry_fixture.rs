//! Actual source/Control/native grant/RPC components. Local process completion
//! remains synthetic and must never populate the runtime joined-run cache.
use super::*;
use crate::{
    control::{
        Lease, Permit, PreparationAdmission, PreparationReference, RunAdmission, RunReference,
    },
    decisions::VerifiedRetryGrant,
    delivery::{ValidationRetryOrigin, ValidationRetryRequest, ValidationRetryWork},
    runtime_delivery::worker_fixture::WorkerFixture,
    scheduler::PhaseKey,
    worker::WorkerAuthority,
    worker_run::{RunCompletion, RunOutcome},
};
use std::{path::Path, sync::Mutex};
#[path = "runtime_delivery_validation_protocol_fixture.rs"]
mod protocol;

pub(super) struct ValidationFixture<'a> {
    pub root: &'a Path,
    pub built: &'a crate::runtime_fixture::RuntimeFixture,
    pub service: &'a RuntimeDelivery,
    pub workspace: &'a WorkspaceDelivery,
    pub gate: &'a FreshGate,
    pub lease: &'a Lease,
    pub scope: DeliveryScope,
    pub assignment: &'a DeliveryWorkerAssignment,
    pub original_claim: Arc<CompletedChildClaim>,
    pub binary: &'a Path,
    pub tool: Arc<Git>,
    pub repository: &'a crate::git::RepositoryId,
    pub account: &'a str,
    pub wire: &'a Arc<Mutex<crate::corrections::wire_fixture::Fixture>>,
}

pub(super) async fn run(f: ValidationFixture<'_>, code: &str) {
    let host = &f.built.runtime.control;
    let (first, first_run, first_permit) = admit(&f, None, "failed-validation", false).await;
    let wire = Arc::new(protocol::ValidationProtocol::new(
        f.built.runtime.clone(),
        first_permit,
        authorized(&first).clone(),
        f.service.current_gate(),
    ));
    let mut failed =
        protocol::failed_validation(host, &first, first_run.max_turns, None, None, wire.clone())
            .await;
    wire.assert_closed_effect(failed.operation_id, "turn/start");
    assert!(failed.state.terminal_transient_failure().unwrap().is_none());
    let original = host.operation(failed.operation_id).unwrap();
    let original_digest = canonical_digest(1, &original).unwrap();
    refresh(&f).await;
    // Existing DecisionManager fixture selects actual failed phase receipts. Its
    // legacy helper name does not constrain the selected phase to review.
    let grant = retry_fixture::publish_failed_review(f.built.runtime.clone()).await;
    assert_eq!(grant.phase_key().phase, "child_local_validation");
    refresh(&f).await;
    let grants = current_grants(&f).unwrap();
    let phase = begin_phase(&f).await;
    let request = |failed| ValidationRetryRequest {
        admission: &phase.admission,
        assignment: f.assignment,
        origin: ValidationRetryOrigin::Original {
            source_advanced: None,
        },
        failed,
        grant: grant.clone(),
        current_grants: &grants,
    };
    // Negative-only mutations cannot create a passing capability. Restore the
    // exact actual component result before the real producer is exercised.
    Arc::get_mut(&mut failed).unwrap().local_exit_confirmed = false;
    let denied = phase
        .coordinator
        .retry_failed_validation(request(failed.clone()))
        .await
        .err()
        .unwrap();
    assert_eq!(denied.code, "reconciliation_required");
    Arc::get_mut(&mut failed).unwrap().local_exit_confirmed = true;
    let original_error = Arc::get_mut(&mut failed).unwrap().error.take();
    Arc::get_mut(&mut failed).unwrap().error = Some(Error::new(
        "operation_uncertain",
        "negative-only unknown outcome",
    ));
    let denied = phase
        .coordinator
        .retry_failed_validation(request(failed.clone()))
        .await
        .err()
        .unwrap();
    assert_eq!(denied.code, "reconciliation_required");
    Arc::get_mut(&mut failed).unwrap().error = original_error;
    let source_file = f.assignment.source().unwrap().root().join("tests/main.txt");
    let source_bytes = fs::read(&source_file).unwrap();
    fs::write(
        &source_file,
        "changed source while grant and head string stay fixed\n",
    )
    .unwrap();
    let denied = maintain_phase_observations(
        &f.built.runtime,
        phase
            .coordinator
            .retry_failed_validation(request(failed.clone())),
    )
    .await
    .err()
    .unwrap();
    assert_eq!(denied.code, "dirty_execution");
    assert_eq!(
        denied.message,
        "tracked source/index differs from the exact revision under validation"
    );
    fs::write(&source_file, &source_bytes).unwrap();
    let proof = Arc::new(
        maintain_phase_observations(
            &f.built.runtime,
            phase
                .coordinator
                .retry_failed_validation(request(failed.clone())),
        )
        .await
        .unwrap(),
    );
    assert_eq!(
        proof.assignment().digest().unwrap(),
        f.assignment.digest().unwrap()
    );
    assert_eq!(proof.phase().retry_grant, Some(grant.record().operation_id));
    assert!(proof.source_advanced().is_none());
    assert!(proof.original_source_admission().is_none());
    f.service
        .retire_delivery_phase(&phase, json!({"validation_retry":proof.digest()}))
        .unwrap();

    let mut removed = host.operations().unwrap();
    removed.retain(|r| r.operation_id != failed.operation_id);
    assert_eq!(
        proof.verify_inventory(&removed).unwrap_err().code,
        "stale_result"
    );
    let mut altered = host.operations().unwrap();
    altered
        .iter_mut()
        .find(|r| r.operation_id == failed.operation_id)
        .unwrap()
        .actor = "substituted actor".into();
    assert_eq!(
        proof.verify_inventory(&altered).unwrap_err().code,
        "stale_result"
    );
    let mut forked = grants.clone();
    forked.push(grant.clone());
    assert!(
        proof
            .verify_current(&host.operations().unwrap(), &forked)
            .is_err()
    );
    assert_eq!(
        proof
            .verify_joined(&|id| f.built.runtime.retained_completion(id))
            .unwrap_err()
            .message,
        "validation retry lost a joined predecessor"
    );
    native_grant_negatives(&f, &proof).await;
    assert_eq!(
        canonical_digest(1, &host.operation(failed.operation_id).unwrap()).unwrap(),
        original_digest
    );

    // The held native adapter must refuse
    // the actual absent joined cache before any model receipt can be created.
    f.built.runtime.retain_retry_grant(grant).unwrap();
    let failed_at = host.operation(failed.operation_id).unwrap().updated_at;
    let config = f.built.runtime.configuration.snapshot().unwrap();
    let wait = crate::runtime_schedule::plan_validation_retry(
        &config.runtime,
        proof.phase(),
        proof.authority().workspace_id(),
        &host.operations().unwrap(),
        &proof,
        failed_at,
    )
    .unwrap_err();
    assert_eq!(wait.code, "retry_wait");
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let (retried, retry_run, retry_permit) =
        admit(&f, Some(&proof), "granted-validation-1", true).await;
    assert_eq!((retry_run.attempt, retry_run.max_attempts), (1, 3));
    let retry_wire = Arc::new(protocol::ValidationProtocol::new(
        f.built.runtime.clone(),
        retry_permit,
        authorized(&retried).clone(),
        f.service.current_gate(),
    ));
    let retry_failed = protocol::failed_validation(
        host,
        &retried,
        retry_run.max_turns,
        None,
        Some(code),
        retry_wire.clone(),
    )
    .await;
    retry_wire.assert_closed_effect(retry_failed.operation_id, "turn/start");
    assert_eq!(
        retry_failed
            .state
            .terminal_transient_failure()
            .unwrap()
            .is_some(),
        code == "serverOverloaded"
    );
    let phase = begin_phase(&f).await;
    let grants = current_grants(&f).unwrap();
    let continued = maintain_phase_observations(
        &f.built.runtime,
        phase.coordinator.continue_failed_validation(
            &phase.admission,
            &proof,
            retry_failed.clone(),
            &grants,
        ),
    )
    .await;
    if code != "serverOverloaded" {
        let denied = continued.err().unwrap();
        assert_eq!(denied.code, "persistent_blocker");
        assert_eq!(
            denied.message,
            "known validation failure is not a supported transient provider failure"
        );
        f.service
            .retire_delivery_phase(&phase, json!({"known_terminal":denied.code}))
            .unwrap();
        assert_eq!(
            canonical_digest(1, &host.operation(failed.operation_id).unwrap()).unwrap(),
            original_digest
        );
        return;
    }
    let continued = Arc::new(continued.unwrap());
    assert_eq!(continued.phase(), proof.phase());
    assert_eq!(continued.grant().digest(), proof.grant().digest());
    assert_eq!(
        continued.assignment().digest().unwrap(),
        proof.assignment().digest().unwrap()
    );
    f.service
        .retire_delivery_phase(&phase, json!({"validation_retry":continued.digest()}))
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let (finished, finished_run, _permit) =
        admit(&f, Some(&continued), "granted-validation-2", false).await;
    assert_eq!(
        (finished_run.attempt, finished_run.max_attempts),
        (2, retry_run.max_attempts)
    );
    assert_eq!(finished_run.max_turns, retry_run.max_turns);
    assert_eq!(finished_run.max_review_rounds, retry_run.max_review_rounds);
    let (_files, outcome, completion) = completed_validation(
        host,
        &finished,
        json!({"kind":"validation","summary":"synthetic completed session; no required check executed"}),
    );
    assert!(completion.checks.is_empty() && completion.failed_checks.is_empty());
    let receipt = host.operation(completion.operation_id).unwrap();
    for key in [
        "source_advanced",
        "review_round",
        "review_round_preparation",
        "review_round_resume",
    ] {
        assert!(!receipt.expected.contains_key(key));
    }
    let phase = begin_phase(&f).await;
    let grants = current_grants(&f).unwrap();
    let completed = maintain_phase_observations(
        &f.built.runtime,
        phase.coordinator.completed_granted_validation(
            &phase.admission,
            &outcome,
            &completion,
            &continued,
            &grants,
        ),
    )
    .await
    .unwrap();
    assert_eq!(completed.phase(), proof.phase());
    assert!(completed.source_advanced().is_none());
    assert!(completed.original_source_admission().is_none());
    assert!(
        completed
            .verify_joined(&|id| f.built.runtime.retained_completion(id))
            .is_err()
    );
    let no_checks = phase
        .coordinator
        .advance_child(
            &phase.admission,
            &f.original_claim,
            &f.workspace.workflow,
            ChildDeliveryAction::ValidateLocal {
                validation: completed.outcome(),
                completion: completed.completion(),
            },
        )
        .await
        .err()
        .unwrap();
    assert_eq!(no_checks.code, "validation_gate");
    assert!(crate::runtime_delivery::child::terminal_gate_failure(
        &no_checks
    ));
    f.service
        .retire_delivery_phase(&phase, json!({"known_terminal":no_checks.code}))
        .unwrap();
    assert_eq!(
        canonical_digest(1, &host.operation(failed.operation_id).unwrap()).unwrap(),
        original_digest
    );
    assert_eq!(
        host.operations()
            .unwrap()
            .iter()
            .filter_map(|r| RunReference::from_receipt(r).unwrap())
            .filter(|r| r.phase.phase == "child_local_validation")
            .count(),
        3
    );
}

fn authorized(worker: &WorkerFixture) -> &FreshGate {
    let WorkerAuthority::Authorized(gate) = worker.contract.authority() else {
        panic!("authorized validation")
    };
    gate
}
async fn refresh(f: &ValidationFixture<'_>) {
    f.built.runtime.observations.refresh().await.unwrap();
}
fn current_grants(f: &ValidationFixture<'_>) -> Result<Vec<VerifiedRetryGrant>> {
    crate::runtime_workflow::current_retry_grants(
        &f.built.runtime.configuration.snapshot()?,
        &f.built.runtime.observations.admission_snapshot()?,
        f.gate.work(),
        &f.built.runtime.control.operations()?,
        &f.built.runtime.services.key,
    )
}
async fn begin_phase(f: &ValidationFixture<'_>) -> ActivePhase {
    refresh(f).await;
    // New phases bind actual current native facts, not the original fixture
    // observation whose timestamp cannot be refreshed by updating the cache.
    let WorkerAuthority::Authorized(gate) =
        f.service.current_delivery_worker_authority(f.gate).unwrap()
    else {
        panic!("authorized validation phase only")
    };
    let mut scope = f.scope.clone();
    scope.fresh_gate_digest = gate.digest().unwrap();
    f.service
        .admit_delivery_phase(
            f.workspace,
            &gate,
            f.lease,
            PhaseRequest {
                operation: Uuid::new_v4(),
                scope,
            },
        )
        .await
        .unwrap()
}

async fn admit(
    f: &ValidationFixture<'_>,
    proof: Option<&Arc<ValidationRetryWork>>,
    name: &str,
    runtime_barrier: bool,
) -> (WorkerFixture, RunReference, Permit) {
    refresh(f).await;
    let host = &f.built.runtime.control;
    let config = f.built.runtime.configuration.snapshot().unwrap();
    let phase = proof.map_or_else(
        || PhaseKey {
            work: f.gate.work_key().unwrap(),
            generation: canonical_digest(1, &f.gate.work().generation).unwrap(),
            phase: "child_local_validation".into(),
            retry_grant: None,
        },
        |proof| proof.phase().clone(),
    );
    let (attempt, max_attempts, max_turns, max_review_rounds) = if let Some(proof) = proof {
        let schedule = crate::runtime_schedule::plan_validation_retry(
            &config.runtime,
            &phase,
            f.gate.workspace_id(),
            &host.operations().unwrap(),
            proof,
            Utc::now(),
        )
        .unwrap();
        (
            schedule.attempt,
            schedule.max_attempts,
            schedule.max_turns,
            schedule.max_review_rounds,
        )
    } else {
        // A deliberately finite original component budget is exhausted by the
        // first real failed protocol result; only the actual operator grant
        // supplies the subsequent three-attempt namespace.
        (
            1,
            1,
            config.runtime.max_turns,
            config.runtime.max_review_rounds,
        )
    };
    let operation = Uuid::new_v4();
    let prep_id = crate::control::preparation_operation(operation).unwrap();
    let prep = host.admit_preparation_observed(f.lease, PreparationAdmission {
        request_id: prep_id,
        work_key: phase.work.clone(),
        scope_digest: canonical_digest(1, &json!({"fixture_preparation":operation,"assignment":f.assignment.digest().unwrap()})).unwrap(),
        reference: PreparationReference {
            worker_operation: operation, workspace:f.gate.workspace_id(), account:f.account.into(), phase:phase.clone(),
        },
    }, |_| {
        if let Some(proof) = proof { proof.verify_current(&host.operations()?, &current_grants(f)?)?; }
        (f.service.current_gate())(f.assignment.authority())
    }).unwrap();
    let current = f
        .service
        .current_delivery_worker_authority(f.assignment.authority())
        .unwrap();
    let WorkerAuthority::Authorized(gate) = current else {
        panic!("authorized validation only")
    };
    let source = f.assignment.source().unwrap();
    let worker = worker(WorkerInput {
        root: &f.root.join(name),
        source: source.root(),
        binary: f.binary,
        git: f.tool.clone(),
        gate: &gate,
        workflow: &f.workspace.workflow,
        revision: source.revision(),
        repository: f.repository,
        runtime: f.lease.runtime_id,
        operation,
        role: f.assignment.role(),
        phase: &phase.phase,
        context: Value::Null,
        assignment: Some(f.assignment),
        account: f.account,
    });
    host.retire_local(prep.operation_id,
        json!({"phase":"worker_preparation","result":"prepared","fixture":"actual clone/assembly; no native process qualification"})).unwrap();
    let preparation = host.operation(prep_id).unwrap();
    assert_eq!(
        PreparationReference::from_receipt(&preparation)
            .unwrap()
            .unwrap()
            .worker_operation,
        operation
    );
    let reference = RunReference {
        workspace: f.gate.workspace_id(),
        account: f.account.into(),
        normalized_state: Some("prepared".into()),
        phase,
        profile_digest: canonical_digest(1, &worker.contract.instructions().manifest.profile)
            .unwrap(),
        instruction_digest: worker.contract.instructions().manifest_digest.clone(),
        max_attempts,
        max_turns,
        max_review_rounds,
        attempt,
    };
    let input = || RunAdmission {
        request_id: operation,
        work_key: reference.phase.work.clone(),
        scope_digest: worker.contract.scope_digest().into(),
        reference: reference.clone(),
    };
    if let Some(proof) = proof {
        refresh(f).await;
        let request = crate::runtime_workers::LaunchRequest {
            lease: f.lease.clone(),
            operation_id: operation,
            authority: f
                .service
                .current_delivery_worker_authority(proof.authority())
                .unwrap(),
            phase: proof.phase().clone(),
            source: None,
            delivery: Some(proof.assignment().clone()),
            unfinished: None,
            source_advanced: None,
            retried_unfinished: None,
            review_retry: None,
            validation_retry: Some(proof.clone()),
            review_round: None,
            max_seconds: 60,
            max_turns,
        };
        request.validate_binding().unwrap();
        let mut wrong = request.clone();
        wrong.phase.retry_grant = None;
        assert_eq!(wrong.validate_binding().unwrap_err().code, "worker_binding");
        let mut wrong_generation = request.clone();
        wrong_generation.phase.generation =
            crate::digest::bytes_digest(b"different validation generation");
        assert_eq!(
            wrong_generation.validate_binding().unwrap_err().code,
            "worker_binding"
        );
        if runtime_barrier {
            let before = canonical_digest(1, &host.operations().unwrap()).unwrap();
            let planned = f
                .built
                .runtime
                .next_attempt_for_validation_retry(&request)
                .unwrap_err();
            assert_eq!(planned.code, "reconciliation_required");
            assert_eq!(
                planned.message,
                "validation retry lost a joined predecessor"
            );
            let denied = f
                .built
                .runtime
                .admit_validation_retry_worker(f.lease, input(), &worker.contract, proof)
                .unwrap_err();
            assert_eq!(denied.code, "reconciliation_required");
            assert_eq!(denied.message, "validation retry lost a joined predecessor");
            assert_eq!(
                canonical_digest(1, &host.operations().unwrap()).unwrap(),
                before
            );
            assert_eq!(
                host.operation(operation).unwrap_err().code,
                "record_missing"
            );
        }
    }
    // This positive is the actual Control component with a strict current
    // opaque-proof callback, explicitly below the unavailable native join gate.
    let permit = host
        .admit_run_observed(f.lease, input(), |_| {
            if let Some(proof) = proof {
                proof.verify_current(&host.operations()?, &current_grants(f)?)?;
                proof.verify_contract(&worker.contract)?;
            }
            (f.service.current_gate())(f.assignment.authority())
        })
        .unwrap();
    let actual = host.operation(operation).unwrap();
    assert_eq!(
        canonical_digest(1, &RunReference::from_receipt(&actual).unwrap().unwrap()).unwrap(),
        canonical_digest(1, &reference).unwrap()
    );
    (worker, reference, permit)
}

async fn native_grant_negatives(f: &ValidationFixture<'_>, proof: &Arc<ValidationRetryWork>) {
    let child = f.gate.work().child.unwrap();
    let saved = f.wire.lock().unwrap().comments[&child].clone();
    for edit in [false, true] {
        {
            let mut wire = f.wire.lock().unwrap();
            let comments = wire.comments.get_mut(&child).unwrap();
            if edit {
                let comment = comments
                    .iter_mut()
                    .find(|v| v["id"] == proof.grant().provenance().record_id)
                    .unwrap();
                let body = comment["body"].as_str().unwrap();
                assert!(body.contains("The exact finding is understood"));
                comment["body"] = json!(body.replace(
                    "The exact finding is understood",
                    "Changed unsigned instruction"
                ));
            } else {
                comments.retain(|v| v["id"] != proof.grant().provenance().record_id);
            }
        }
        refresh(f).await;
        let host = &f.built.runtime.control;
        let operations = canonical_digest(1, &host.operations().unwrap()).unwrap();
        let worker = Uuid::new_v4();
        let preparation = crate::control::preparation_operation(worker).unwrap();
        let mut evaluated = false;
        let denied = host
            .admit_preparation_observed(
                f.lease,
                PreparationAdmission {
                    request_id: preparation,
                    work_key: proof.phase().work.clone(),
                    scope_digest: canonical_digest(
                        1,
                        &json!({"native_grant_race":worker,"retry":proof.digest()}),
                    )
                    .unwrap(),
                    reference: PreparationReference {
                        worker_operation: worker,
                        workspace: f.gate.workspace_id(),
                        account: f.account.into(),
                        phase: proof.phase().clone(),
                    },
                },
                |_| {
                    evaluated = true;
                    let current = current_grants(f)?;
                    proof.verify_current(&host.operations()?, &current)
                },
            )
            .err()
            .unwrap();
        assert!(
            evaluated,
            "actual serialized callback must observe changed native grant"
        );
        if edit {
            // The actual signed native parser must reject the edited body.
            assert_eq!(denied.code, current_grants(f).unwrap_err().code);
        } else {
            assert_eq!(
                denied.code,
                proof
                    .verify_current(&host.operations().unwrap(), &current_grants(f).unwrap())
                    .unwrap_err()
                    .code
            );
        }
        assert_eq!(
            canonical_digest(1, &host.operations().unwrap()).unwrap(),
            operations
        );
        assert_eq!(
            host.operation(preparation).unwrap_err().code,
            "record_missing"
        );
        f.wire.lock().unwrap().comments.insert(child, saved.clone());
        refresh(f).await;
        proof
            .verify_current(&host.operations().unwrap(), &current_grants(f).unwrap())
            .unwrap();
    }
}

/// Exactly the existing protected contract/claim fixture boundary, but retire
/// once with the full observation consumed by the validation retry bridge.
/// This synthetic process result is never installed in RuntimeCoordinator.
fn completed_validation(
    host: &crate::control::ControlHost,
    fixture: &WorkerFixture,
    claim: Value,
) -> (
    tempfile::TempDir,
    crate::runtime::VerifiedAuthorizedOutcome,
    Arc<RunCompletion>,
) {
    let contract = &fixture.contract;
    let operation = contract.instructions().manifest.operation_id;
    let (files, outcome, state) = crate::runtime_claims::test_completed_contract_fixture(
        contract,
        host.operation(operation).unwrap(),
        claim,
    )
    .unwrap();
    let completion = Arc::new(RunCompletion {
        operation_id: operation,
        state,
        outcome: RunOutcome::Completed,
        error: None,
        elapsed_ms: 1,
        hook_error: None,
        checks: vec![],
        failed_checks: vec![],
        validation_plan: None,
        validation_attempts: vec![],
        validation_error: None,
        validation_history: None,
        execution: Some(fixture.execution.clone()),
        contract: Some(contract.clone()),
        local_exit_confirmed: true,
    });
    host.retire_local(operation, completion.observation())
        .unwrap();
    (files, outcome, completion)
}
