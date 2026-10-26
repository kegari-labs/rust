//! Real owned Git/source seals and production contract/claim validation. The
//! completed model protocol is synthetic; no native agent qualification claimed.
use super::*;
use crate::{
    instructions::AgentRole,
    runtime_delivery::worker_fixture::{
        WorkerInput, complete, complete_capped, complete_retried, git, maintain_phase_observations,
        worker,
    },
    test_process_probe as probe,
};
use serde_json::Value;
use std::fs;

#[tokio::test]
async fn owned_observation_maintenance_reads_native_changes_while_phase_remains_admitted() {
    use crate::runtime_driver::{ObservationMaintenance, ObservationTick};

    let (_root, built, _service, wire, owner) =
        super::super::tests::fixture_authorization_with_wire(true).await;
    let before = built.runtime.observations.snapshot().unwrap();
    let gate = before
        .parent(Uuid::from_u128(2), Uuid::from_u128(100))
        .unwrap()
        .fresh_gate(
            Some(Uuid::from_u128(101)),
            GatePurpose::NewOperation,
            &crate::reconcile::DependencyOutcomes::default(),
            Utc::now(),
            Duration::seconds(30),
        )
        .unwrap();
    let _active = built
        .runtime
        .observations
        .capture_active(
            gate.workspace_id(),
            gate.config_revision(),
            gate.workflow_digest(),
        )
        .unwrap();
    let (lease, _owner_lock) = owner.start(&built.runtime);
    let permit = built
        .runtime
        .control
        .admit(
            &lease,
            Uuid::new_v4(),
            gate.work_key().unwrap(),
            canonical_digest(1, &json!({"purpose":"observation-maintenance-barrier"})).unwrap(),
            || {
                built
                    .runtime
                    .observations
                    .active_gate(&gate, &crate::reconcile::DependencyOutcomes::default())?;
                Ok(())
            },
        )
        .unwrap();
    let admission_digest = canonical_digest(
        1,
        &built
            .runtime
            .control
            .operation(permit.operation_id)
            .unwrap(),
    )
    .unwrap();
    let request_count = {
        let mut provider = wire.lock().unwrap();
        provider.set_parent_title("Changed native parent while a delivery phase is pending");
        provider.requests.len()
    };
    let maintenance = ObservationMaintenance::new(
        built.runtime.observations.clone(),
        Arc::new(tokio::sync::Notify::new()),
    );
    let tick = maintenance.tick(30_000, true).await;
    // Joining the real bounded read task is the deterministic barrier. There is
    // no elapsed-age sleep, injected observation, or synthetic execution result.
    let refreshed = maintenance.join().await;
    let pending = maintenance.pending();
    let still_admitted = built
        .runtime
        .control
        .snapshot()
        .unwrap()
        .active_operations
        .iter()
        .any(|p| p.operation_id == permit.operation_id);
    let unchanged_admission = canonical_digest(
        1,
        &built
            .runtime
            .control
            .operation(permit.operation_id)
            .unwrap(),
    )
    .unwrap();
    let current_authority = built
        .runtime
        .observations
        .active_gate(&gate, &crate::reconcile::DependencyOutcomes::default());
    let (request_count_after, issue_reads) = {
        let provider = wire.lock().unwrap();
        // Reconciliation observes issues through these bulk queries; the wire's
        // single-issue KegariIssue hook is not on this production refresh path.
        let issue_reads = provider.requests[request_count..]
            .iter()
            .filter(|(_, body)| {
                body["query"].as_str().is_some_and(|query| {
                    query.contains("KegariParents") || query.contains("KegariRefresh")
                })
            })
            .count();
        (provider.requests.len(), issue_reads)
    };
    built
        .runtime
        .control
        .retire_local(
            permit.operation_id,
            json!({"phase":"observation-maintenance-barrier","result":"no local process or external effect was launched"}),
        )
        .unwrap();

    assert!(matches!(tick.unwrap(), ObservationTick::Started));
    let refreshed = refreshed.unwrap().unwrap();
    assert!(issue_reads > 0);
    assert!(request_count_after > request_count);
    assert!(refreshed.observed_at >= before.observed_at);
    assert_eq!(
        refreshed
            .parent(Uuid::from_u128(2), Uuid::from_u128(100))
            .unwrap()
            .issue
            .as_ref()
            .unwrap()
            .title,
        "Changed native parent while a delivery phase is pending"
    );
    assert!(still_admitted);
    assert_eq!(unchanged_admission, admission_digest);
    assert!(current_authority.is_err());
    assert!(!pending.unwrap());
}

#[tokio::test]
async fn exact_review_findings_produce_repair_from_real_owned_source_and_contracts() {
    repair_cycle(false, false, None, false).await;
}
#[tokio::test]
async fn exhausted_completed_work_uses_actual_operator_retry_budget_and_bound_repair_source() {
    repair_cycle(true, false, None, false).await;
}
#[path = "runtime_delivery_retry_fixture.rs"]
mod retry_fixture;
#[tokio::test]
async fn failed_review_uses_exact_source_and_actual_native_grant_before_clean_review() {
    repair_cycle(false, true, None, false).await;
}
#[tokio::test]
async fn granted_review_retries_another_known_failure_with_same_budget_and_source() {
    repair_cycle(false, true, Some("serverOverloaded"), false).await;
}
#[tokio::test]
async fn granted_review_does_not_automatically_retry_auth_unknown_or_nontransient_failure() {
    for code in ["unauthorized", "other", "badRequest"] {
        repair_cycle(false, true, Some(code), false).await;
    }
}
#[tokio::test]
async fn granted_findings_preserve_implementation_budget_and_follow_changed_source_in_granted_review_budget()
 {
    repair_cycle(false, true, None, true).await;
}
fn repair_cycle(
    explicit_retry: bool,
    review_retry: bool,
    failure_code: Option<&str>,
    findings: bool,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + '_>> {
    // The owned Git/protocol fixture has large nested futures. Allocate its
    // frame independently rather than expanding every test's stack frame.
    Box::pin(repair_cycle_inner(
        explicit_retry,
        review_retry,
        failure_code,
        findings,
        None,
        None,
        false,
    ))
}
async fn repair_cycle_inner(
    explicit_retry: bool,
    review_retry: bool,
    failure_code: Option<&str>,
    findings: bool,
    round_limit: Option<u32>,
    validation_retry: Option<&str>,
    marked_review_retry: bool,
) {
    let retry_again = failure_code.is_some();
    let (root, built, old_service, wire, owner) =
        super::super::tests::fixture_authorization_with_configuration(true, |config| {
            if validation_retry.is_some() {
                config.runtime.max_attempts_per_phase = 3;
                config.runtime.retry_cap_ms = 1_000;
            }
            if let Some(limit) = round_limit {
                config.runtime.max_review_rounds = limit;
                config.runtime.max_attempts_per_phase = 3;
                config.runtime.retry_cap_ms = 1_000;
            }
        })
        .await;
    let binary = old_service.git_path().unwrap();
    let root = root.path().canonicalize().unwrap();
    let source = root.join("source");
    git(&binary, &source, &["init", "-b", "main"]);
    fs::create_dir(source.join("tests")).unwrap();
    fs::write(source.join("tests/main.txt"), "initial\n").unwrap();
    git(&binary, &source, &["add", "tests"]);
    git(
        &binary,
        &source,
        &[
            "commit",
            "--signoff",
            "-m",
            "test: initialize disposable continuation source",
        ],
    );
    let tool = Arc::new(Git::new(&binary).unwrap());
    let revision = tool.revision(&source, "HEAD").unwrap();
    let processes = Arc::new(
        crate::process_records::ProcessJournal::with_probe(
            built
                .runtime
                .registration
                .root()
                .child("continuation-process-fixture")
                .unwrap(),
            built.runtime.configuration.snapshot().unwrap().runtime.id,
            Arc::new(probe::FixtureBootProbe::new()),
        )
        .unwrap(),
    );
    let service =
        RuntimeDelivery::with_fixture(built.runtime.clone(), processes, binary.clone()).unwrap();
    let projection = built.runtime.observations.snapshot().unwrap();
    let parent = projection
        .parent(Uuid::from_u128(2), Uuid::from_u128(100))
        .unwrap();
    let gate = parent
        .fresh_gate(
            Some(Uuid::from_u128(101)),
            GatePurpose::NewOperation,
            &crate::reconcile::DependencyOutcomes::default(),
            Utc::now(),
            Duration::seconds(30),
        )
        .unwrap();
    let workspace = service.workspace(&projection, gate.workspace_id()).unwrap();
    let repository = workspace.contracts[0].repository.clone();
    let (lease, _lock) = owner.start(&built.runtime);
    let runtime = lease.runtime_id;
    let account = workspace.configuration.connections
        [&workspace.configuration.workspaces["example"].codex_connection]
        .account_id
        .clone();
    let original = worker(WorkerInput {
        root: &root.join("implementation"),
        source: &source,
        binary: &binary,
        git: tool.clone(),
        gate: &gate,
        workflow: &workspace.workflow,
        revision: &revision,
        repository: &repository,
        runtime,
        operation: Uuid::new_v4(),
        role: AgentRole::Testing,
        phase: "implementation",
        context: json!({"task":"check the required behavior"}),
        assignment: None,
        account: &account,
    });
    let (_original_records, outcome, completion) = complete_capped(
        &built.runtime.control,
        &lease,
        &original,
        json!({"schema_version":1,"summary":"existing source assessed","fulfilled_criteria":["required behavior"],"disposition":{"kind":"no_change","reason":"claimed existing behavior suffices"}}),
        if explicit_retry { 1 } else { 3 },
    );
    let claim = Arc::new(
        workspace
            .coordinator
            .accept_child_completion(&gate, outcome, completion.clone())
            .unwrap(),
    );
    let binding = RevisionBinding {
        repository: repository.clone(),
        head: revision.commit.clone(),
        base: revision.commit.clone(),
        tested_commit: revision.commit.clone(),
        tested_tree: revision.tree.clone(),
    };
    let scope = |phase| DeliveryScope {
        schema_version: 1,
        runtime_id: runtime,
        phase,
        work: gate.work().clone(),
        repository: repository.clone(),
        revision: binding.clone(),
        fresh_gate_digest: gate.digest().unwrap(),
        contract_digest: workspace.contracts[0].digest().unwrap(),
        candidate_vector_digest: None,
    };
    let sealing = service
        .admit_delivery_phase(
            &workspace,
            &gate,
            &lease,
            PhaseRequest {
                operation: Uuid::new_v4(),
                scope: scope(DeliveryPhase::SourceSealing),
            },
        )
        .await
        .unwrap();
    let source_proof = maintain_phase_observations(
        &built.runtime,
        sealing.coordinator.seal_child_source(
            &sealing.admission,
            &claim,
            &original.execution,
            original.contract.isolation(),
            crate::git::SourceCommitMetadata {
                identity: crate::git::CommitIdentity {
                    name: "Disposable Fixture".into(),
                    email: "fixture@example.invalid".into(),
                },
                timestamp: chrono::DateTime::from_timestamp(1_767_225_600, 0).unwrap(),
                message: "test: seal disposable source\n\nSigned-off-by: Disposable Fixture <fixture@example.invalid>\n".into(),
            },
        ),
    )
    .await
    .unwrap();
    service
        .retire_delivery_phase(&sealing, json!({"sealed":true}))
        .unwrap();
    built.runtime.observations.refresh().await.unwrap();
    let validation_phase = service
        .admit_delivery_phase(
            &workspace,
            &gate,
            &lease,
            PhaseRequest {
                operation: Uuid::new_v4(),
                scope: scope(DeliveryPhase::LocalValidation),
            },
        )
        .await
        .unwrap();
    let validation_assignment = validation_phase
        .coordinator
        .child_validation_assignment(
            &validation_phase.admission,
            &claim,
            &source_proof,
            &completion,
            json!({"task":"required local checks"}),
        )
        .unwrap();
    service
        .retire_delivery_phase(&validation_phase, json!({"assignment_prepared":true}))
        .unwrap();
    if let Some(code) = validation_retry {
        Box::pin(validation_retry_fixture::run(
            validation_retry_fixture::ValidationFixture {
                root: &root,
                built: &built,
                service: &service,
                workspace: &workspace,
                gate: &gate,
                lease: &lease,
                scope: scope(DeliveryPhase::LocalValidation),
                assignment: &validation_assignment,
                original_claim: claim.clone(),
                binary: &binary,
                tool: tool.clone(),
                repository: &repository,
                account: &account,
                wire: &wire,
            },
            code,
        ))
        .await;
        return;
    }
    let validator = worker(WorkerInput {
        root: &root.join("validation"),
        source: &source,
        binary: &binary,
        git: tool.clone(),
        gate: &gate,
        workflow: &workspace.workflow,
        revision: &revision,
        repository: &repository,
        runtime,
        operation: Uuid::new_v4(),
        role: AgentRole::Validation,
        phase: "child_local_validation",
        context: Value::Null,
        assignment: Some(&validation_assignment),
        account: &account,
    });
    let (_validation_records, validation_outcome, validation_completion) = complete(
        &built.runtime.control,
        &lease,
        &validator,
        json!({"kind":"validation","summary":"synthetic protocol; no passed check claimed"}),
        None,
        None,
    );
    built.runtime.observations.refresh().await.unwrap();
    let review_phase = service
        .admit_delivery_phase(
            &workspace,
            &gate,
            &lease,
            PhaseRequest {
                operation: Uuid::new_v4(),
                scope: scope(DeliveryPhase::NoChange),
            },
        )
        .await
        .unwrap();
    let review = review_phase
        .coordinator
        .prepare_child_review(
            &review_phase.admission,
            &claim,
            ChildReviewTarget::NoChange {
                source: &source_proof,
            },
            json!({"task":"review required behavior"}),
        )
        .unwrap();
    let review_worker = review_phase
        .coordinator
        .child_review_worker_assignment(
            &review_phase.admission,
            &claim,
            &source_proof,
            &completion,
            &review,
        )
        .unwrap();
    service
        .retire_delivery_phase(&review_phase, json!({"assignment_prepared":true}))
        .unwrap();
    // Recover only historical host-authenticated provenance with an old
    // observation time. It is deliberately unusable as a current provider gate;
    // the actual delivery admission still uses the fresh native projection.
    let historical_review_gate = if retry_again {
        let mut saved = serde_json::to_value(&gate).unwrap();
        saved["observed_at"] = json!(Utc::now() - Duration::minutes(10));
        let envelope = built
            .runtime
            .services
            .key
            .sign(
                "pending_worker_claim",
                &json!({
                    "schema_version":1,"authority":saved,"outcome":validation_outcome.outcome(),
                }),
            )
            .unwrap();
        let historical = FreshGate::recover_host_claim(
            &built.runtime.services.key,
            &envelope,
            &gate.digest().unwrap(),
        )
        .unwrap();
        assert!(
            gate.compare_current(&historical, Utc::now(), Duration::seconds(30))
                .is_err()
        );
        historical
    } else {
        gate.clone()
    };
    let reviewer = worker(WorkerInput {
        root: &root.join("review"),
        source: &source,
        binary: &binary,
        git: tool.clone(),
        gate: &historical_review_gate,
        workflow: &workspace.workflow,
        revision: &revision,
        repository: &repository,
        runtime,
        operation: Uuid::new_v4(),
        role: AgentRole::Review,
        phase: "child_codex_review",
        context: review.context().clone(),
        assignment: Some(&review_worker),
        account: &account,
    });
    if review_retry || round_limit.is_some() {
        let fixture = ReviewFixture {
            findings,
            source_proof: &source_proof,
            retry_again,
            failure_code,
            service: &service,
            built: &built,
            wire: &wire,
            workspace: &workspace,
            gate: &gate,
            lease: &lease,
            reviewer: &reviewer,
            original_claim: claim.clone(),
            original_completion: completion.clone(),
            assignment: &review_worker,
            review: &review,
            root: &root,
            source: &source,
            binary: &binary,
            tool: tool.clone(),
            revision: &revision,
            repository: &repository,
            account: &account,
            scope: scope(DeliveryPhase::NoChange),
        };
        if marked_review_retry {
            Box::pin(round_fixture::marked_retry(fixture)).await;
        } else if let Some(limit) = round_limit {
            Box::pin(round_fixture::run(
                fixture,
                limit,
                validation_outcome,
                validation_completion,
            ))
            .await;
        } else {
            review_retry_fixture::run(fixture).await;
        }
        return;
    }
    let (_review_records, review_outcome, review_completion) = complete(
        &built.runtime.control,
        &lease,
        &reviewer,
        json!({"schema_version":1,"verdict":"findings","unresolved_findings":["Required empty-input behavior has no implementation in tests/main.txt"],"supporting_links":[]}),
        None,
        None,
    );
    built.runtime.observations.refresh().await.unwrap();
    let review_phase = service
        .admit_delivery_phase(
            &workspace,
            &gate,
            &lease,
            PhaseRequest {
                operation: Uuid::new_v4(),
                scope: scope(DeliveryPhase::NoChange),
            },
        )
        .await
        .unwrap();
    let proof = maintain_phase_observations(
        &built.runtime,
        review_phase
            .coordinator
            .unfinished_review(ReviewRepairRequest {
                source: RepairSource {
                    paired_completion: None,
                    admission: &review_phase.admission,
                    original: claim,
                    source: &source_proof,
                    completion,
                },
                assignment: &review,
                outcome: &review_outcome,
                completion: review_completion.clone(),
            }),
    )
    .await
    .unwrap();
    assert_eq!(proof.predecessor(), original.execution.binding.operation_id);
    assert_eq!(proof.phase().phase, "implementation");
    assert_eq!(proof.review_rounds_consumed(), 1);
    assert_eq!(proof.source().revision(), &revision);
    assert_eq!(
        proof.reference().max_attempts,
        if explicit_retry { 1 } else { 3 }
    );
    let proof = Arc::new(proof);
    service
        .retire_delivery_phase(&review_phase, json!({"findings":true}))
        .unwrap();
    let limits = built
        .runtime
        .configuration
        .snapshot()
        .unwrap()
        .runtime
        .clone();
    let receipts = built.runtime.control.operations().unwrap();
    // Exercise budget separation using mutated observations only. These are
    // never written to ControlHost or used to manufacture a retry grant.
    let mut other_budget = proof.predecessor_receipt().clone();
    other_budget.operation_id = Uuid::new_v4();
    other_budget.request_id = other_budget.operation_id;
    let mut other_run = proof.reference().clone();
    other_run.phase.retry_grant = Some(Uuid::new_v4());
    other_budget
        .expected
        .insert("run".into(), serde_json::to_string(&other_run).unwrap());
    let mut history = receipts.clone();
    history.push(other_budget.clone());
    proof.verify_retry_basis(&history).unwrap();
    other_run.phase = proof.phase().clone();
    other_run.attempt += 1;
    other_run.max_attempts = other_run.max_attempts.max(other_run.attempt);
    other_budget
        .expected
        .insert("run".into(), serde_json::to_string(&other_run).unwrap());
    history.push(other_budget);
    assert_eq!(
        proof.verify_retry_basis(&history).unwrap_err().code,
        "stale_result"
    );
    assert_eq!(
        crate::runtime_workflow::select_retry_target(runtime, gate.work(), &receipts, &[])
            .unwrap_err()
            .code,
        "no_retry_target"
    );
    let target = crate::runtime_workflow::select_retry_target_with_unfinished(
        runtime,
        gate.work(),
        &receipts,
        &[],
        std::slice::from_ref(&proof),
        &limits,
    );
    if explicit_retry {
        assert_eq!(target.unwrap().phase, *proof.phase());
    } else {
        assert_eq!(target.unwrap_err().code, "no_retry_target");
    }
    let retried = if explicit_retry {
        let grant = retry_fixture::publish(built.runtime.clone(), proof.clone()).await;
        let config = built.runtime.configuration.snapshot().unwrap();
        let current = crate::runtime_workflow::current_retry_grants(
            &config,
            &built.runtime.observations.snapshot().unwrap(),
            gate.work(),
            &built.runtime.control.operations().unwrap(),
            &built.runtime.services.key,
        )
        .unwrap();
        let retried = Arc::new(
            workspace
                .coordinator
                .retry_unfinished(proof.clone(), grant, &current)
                .unwrap(),
        );
        assert_eq!(retried.grant().new_budget().max_attempts, 3);
        assert_ne!(retried.phase(), proof.phase());
        assert!(
            retried
                .verify_current(&built.runtime.control.operations().unwrap(), &[])
                .is_err()
        );
        let planned = crate::runtime_schedule::plan(
            &config.runtime,
            retried.phase(),
            gate.workspace_id(),
            &built.runtime.control.operations().unwrap(),
            Utc::now(),
        )
        .unwrap();
        assert_eq!((planned.attempt, planned.max_attempts), (1, 3));
        retried
            .verify_current(&built.runtime.control.operations().unwrap(), &current)
            .unwrap();
        let child = gate.work().child.unwrap();
        let saved = wire.lock().unwrap().comments[&child].clone();
        wire.lock()
            .unwrap()
            .comments
            .get_mut(&child)
            .unwrap()
            .retain(|comment| {
                comment["id"].as_str() != Some(retried.grant().provenance().record_id.as_str())
            });
        built.runtime.observations.refresh().await.unwrap();
        let removed = crate::runtime_workflow::current_retry_grants(
            &config,
            &built.runtime.observations.snapshot().unwrap(),
            gate.work(),
            &built.runtime.control.operations().unwrap(),
            &built.runtime.services.key,
        )
        .unwrap();
        assert!(
            retried
                .verify_current(&built.runtime.control.operations().unwrap(), &removed)
                .is_err()
        );
        assert_eq!(
            retry_fixture::assert_preparation_denied(&built.runtime, &lease, &retried, &account,)
                .code,
            "stale_result"
        );
        wire.lock().unwrap().comments.insert(child, saved.clone());
        built.runtime.observations.refresh().await.unwrap();
        let before_edit = crate::runtime_workflow::current_retry_grants(
            &config,
            &built.runtime.observations.snapshot().unwrap(),
            gate.work(),
            &built.runtime.control.operations().unwrap(),
            &built.runtime.services.key,
        )
        .unwrap();
        retried
            .verify_current(&built.runtime.control.operations().unwrap(), &before_edit)
            .unwrap();
        {
            let mut wire = wire.lock().unwrap();
            let comment = wire
                .comments
                .get_mut(&child)
                .unwrap()
                .iter_mut()
                .find(|comment| {
                    comment["id"].as_str() == Some(retried.grant().provenance().record_id.as_str())
                })
                .unwrap();
            let body = comment["body"].as_str().unwrap().replace(
                "The exact finding is understood",
                "Changed unverified retry direction",
            );
            comment["body"] = json!(body);
        }
        built.runtime.observations.refresh().await.unwrap();
        assert!(
            crate::runtime_workflow::current_retry_grants(
                &config,
                &built.runtime.observations.snapshot().unwrap(),
                gate.work(),
                &built.runtime.control.operations().unwrap(),
                &built.runtime.services.key
            )
            .is_err()
        );
        assert_eq!(
            retry_fixture::assert_preparation_denied(&built.runtime, &lease, &retried, &account,)
                .code,
            "invalid_attestation"
        );
        wire.lock().unwrap().comments.insert(child, saved);
        built.runtime.observations.refresh().await.unwrap();
        let restored = crate::runtime_workflow::current_retry_grants(
            &config,
            &built.runtime.observations.snapshot().unwrap(),
            gate.work(),
            &built.runtime.control.operations().unwrap(),
            &built.runtime.services.key,
        )
        .unwrap();
        retried
            .verify_current(&built.runtime.control.operations().unwrap(), &restored)
            .unwrap();
        Some(retried)
    } else {
        None
    };
    let assignment = match &retried {
        Some(retried) => retried_repair_worker_assignment(retried).unwrap(),
        None => repair_worker_assignment(&proof).unwrap(),
    };
    match &retried {
        Some(retried) => retried.verify_assignment(&assignment).unwrap(),
        None => proof.verify_assignment(&assignment).unwrap(),
    };
    let repair = worker(WorkerInput {
        root: &root.join("repair"),
        source: proof.source().root(),
        binary: &binary,
        git: tool.clone(),
        gate: &gate,
        workflow: &workspace.workflow,
        revision: &revision,
        repository: &repository,
        runtime,
        operation: Uuid::new_v4(),
        role: AgentRole::Testing,
        phase: "implementation",
        context: proof.remaining_task().clone(),
        assignment: Some(&assignment),
        account: &account,
    });
    match &retried {
        Some(retried) => retried.verify_contract(&repair.contract).unwrap(),
        None => proof.verify_contract(&repair.contract).unwrap(),
    };
    assert!(proof.verify_contract(&original.contract).is_err());
    built.runtime.observations.refresh().await.unwrap();
    let request = crate::runtime_workers::LaunchRequest {
        lease: lease.clone(),
        operation_id: repair.execution.binding.operation_id,
        authority: service
            .current_delivery_worker_authority(proof.authority())
            .unwrap(),
        phase: retried
            .as_ref()
            .map(|p| p.phase())
            .unwrap_or_else(|| proof.phase())
            .clone(),
        source: None,
        delivery: Some(assignment),
        unfinished: if retried.is_some() {
            None
        } else {
            Some(proof.clone())
        },
        retried_unfinished: retried.clone(),
        validation_retry: None,
        review_retry: None,
        review_round: None,
        source_advanced: None,
        max_seconds: 60,
        max_turns: 20,
    };
    request.validate_binding().unwrap();
    if retried.is_some() {
        let mut wrong = request.clone();
        wrong.phase = proof.phase().clone();
        assert!(wrong.validate_binding().is_err());
        let mut wrong = request.clone();
        wrong.unfinished = Some(proof.clone());
        assert!(wrong.validate_binding().is_err());
    }
    proof
        .verify_failure_inventory(&built.runtime.control.operations().unwrap())
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    fs::write(
        repair
            .contract
            .isolation()
            .plan()
            .repository
            .join("tests/main.txt"),
        "required empty-input case covered\n",
    )
    .unwrap();
    let repair_claim_json = json!({"schema_version":1,"summary":"required finding corrected","fulfilled_criteria":["required behavior"],"disposition":{"kind":"changed"}});
    let (_repair_records, repair_outcome, repair_completion) = match &retried {
        Some(retried) => complete_retried(
            &built.runtime.control,
            &lease,
            &repair,
            repair_claim_json,
            retried,
        ),
        None => complete(
            &built.runtime.control,
            &lease,
            &repair,
            repair_claim_json,
            Some(&proof),
            None,
        ),
    };
    if let Some(retried) = &retried {
        let run = RunReference::from_receipt(
            &built
                .runtime
                .control
                .operation(repair.execution.binding.operation_id)
                .unwrap(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(run.attempt, 1);
        assert_eq!(run.max_attempts, 3);
        assert_eq!(&run.phase, retried.phase());
        assert!(
            retried
                .verify_current(
                    &built.runtime.control.operations().unwrap(),
                    std::slice::from_ref(retried.grant())
                )
                .is_err()
        );
        // The first-session capability is consumed, but acknowledging the
        // already published grant must remain valid after its budget starts.
        let config = built.runtime.configuration.snapshot().unwrap();
        let inventory = built.runtime.control.operations().unwrap();
        let current = crate::runtime_workflow::current_retry_grants(
            &config,
            &built.runtime.observations.snapshot().unwrap(),
            gate.work(),
            &inventory,
            &built.runtime.services.key,
        )
        .unwrap();
        let original_target = crate::runtime_workflow::published_retry_target(
            &config,
            retried.grant(),
            &current,
            &inventory,
        )
        .unwrap();
        assert_eq!(original_target.phase, *proof.phase());
        assert_eq!(original_target.admissions, [proof.predecessor()].into());
    }
    let repair_claim = Arc::new(
        workspace
            .coordinator
            .accept_child_completion(&gate, repair_outcome, repair_completion.clone())
            .unwrap(),
    );
    built.runtime.observations.refresh().await.unwrap();
    let repair_sealing = service
        .admit_delivery_phase(
            &workspace,
            &gate,
            &lease,
            PhaseRequest {
                operation: Uuid::new_v4(),
                scope: scope(DeliveryPhase::SourceSealing),
            },
        )
        .await
        .unwrap();
    let changed = maintain_phase_observations(
        &built.runtime,
        repair_sealing.coordinator.seal_child_source(
            &repair_sealing.admission,
            &repair_claim,
            &repair.execution,
            repair.contract.isolation(),
            crate::git::SourceCommitMetadata {
                identity: crate::git::CommitIdentity {
                    name: "Disposable Fixture".into(),
                    email: "fixture@example.invalid".into(),
                },
                timestamp: chrono::DateTime::from_timestamp(1_767_225_601, 0).unwrap(),
                message: "test: repair disposable required case\n\nSigned-off-by: Disposable Fixture <fixture@example.invalid>\n".into(),
            },
        ),
    )
    .await
    .unwrap();
    service
        .retire_delivery_phase(&repair_sealing, json!({"sealed":true}))
        .unwrap();
    assert_ne!(changed.revision().tested_tree, revision.tree);
    let mut advanced_scope = scope(DeliveryPhase::LocalValidation);
    advanced_scope.revision = changed.revision().clone();
    built.runtime.observations.refresh().await.unwrap();
    let revalidate = service
        .admit_delivery_phase(
            &workspace,
            &gate,
            &lease,
            PhaseRequest {
                operation: Uuid::new_v4(),
                scope: advanced_scope,
            },
        )
        .await
        .unwrap();
    let assignment = revalidate
        .coordinator
        .child_validation_assignment(
            &revalidate.admission,
            &repair_claim,
            &changed,
            &repair_completion,
            json!({"task":"validate corrected exact source"}),
        )
        .unwrap();
    let advanced = maintain_phase_observations(
        &built.runtime,
        revalidate
            .coordinator
            .source_advanced(SourceAdvancedRequest {
                source: RepairSource {
                    paired_completion: None,
                    admission: &revalidate.admission,
                    original: repair_claim,
                    source: &changed,
                    completion: repair_completion,
                },
                assignment: &assignment,
                predecessor: &validation_outcome,
                completion: validation_completion,
            }),
    )
    .await
    .unwrap();
    assert_eq!(
        advanced.predecessor(),
        validator.execution.binding.operation_id
    );
    assert_eq!(
        advanced.source_operation(),
        repair.execution.binding.operation_id
    );
    assert_eq!(advanced.phase().phase, "child_local_validation");
    assert_eq!(advanced.reference().max_attempts, 3);
    advanced.verify_assignment(&assignment).unwrap();
    advanced
        .verify_predecessor_inventory(&built.runtime.control.operations().unwrap())
        .unwrap();
    service
        .retire_delivery_phase(&revalidate, json!({"advanced_assignment_prepared":true}))
        .unwrap();
    // This fixture has a synthetic completed provider protocol, not an actual
    // owned driver join. Do not manufacture runtime-cache process authority.
    assert_eq!(
        built
            .runtime
            .next_attempt_for_source_advanced(&advanced)
            .unwrap_err()
            .code,
        "reconciliation_required"
    );
    let revalidator = worker(WorkerInput {
        root: &root.join("revalidation"),
        source: advanced.source().root(),
        binary: &binary,
        git: tool.clone(),
        gate: &gate,
        workflow: &workspace.workflow,
        revision: advanced.source().revision(),
        repository: &repository,
        runtime,
        operation: Uuid::new_v4(),
        role: AgentRole::Validation,
        phase: "child_local_validation",
        context: Value::Null,
        assignment: Some(&assignment),
        account: &account,
    });
    advanced.verify_contract(&revalidator.contract).unwrap();
    let prior_validation = built
        .runtime
        .control
        .operation(advanced.predecessor())
        .unwrap();
    let (_revalidation_records, _revalidation_outcome, _) = complete(
        &built.runtime.control,
        &lease,
        &revalidator,
        json!({"kind":"validation","summary":"synthetic completed protocol; no passed execution proof claimed"}),
        None,
        Some(&advanced),
    );
    let admitted = RunReference::from_receipt(
        &built
            .runtime
            .control
            .operation(revalidator.execution.binding.operation_id)
            .unwrap(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(admitted.attempt, 2);
    assert_eq!(admitted.max_attempts, 3);
    assert_eq!(admitted.phase, advanced.reference().phase);
    assert_eq!(
        advanced
            .verify_predecessor_inventory(&built.runtime.control.operations().unwrap())
            .unwrap_err()
            .code,
        "stale_result"
    );
    assert_eq!(
        canonical_digest(
            1,
            &built
                .runtime
                .control
                .operation(advanced.predecessor())
                .unwrap()
        )
        .unwrap(),
        canonical_digest(1, &prior_validation).unwrap()
    );
    // A subsequent clean same-head review must invalidate the retained failure,
    // even though its original failed-review receipt remains byte-for-byte equal.
    let clean_reviewer = worker(WorkerInput {
        root: &root.join("clean-review"),
        source: &source,
        binary: &binary,
        git: tool,
        gate: &gate,
        workflow: &workspace.workflow,
        revision: &revision,
        repository: &repository,
        runtime,
        operation: Uuid::new_v4(),
        role: AgentRole::Review,
        phase: "child_codex_review",
        context: Value::Null,
        assignment: Some(&review_worker),
        account: &account,
    });
    let prior_failure = built
        .runtime
        .control
        .operation(proof.failure_operation())
        .unwrap();
    let (_clean_records, _clean_outcome, _) = complete(
        &built.runtime.control,
        &lease,
        &clean_reviewer,
        json!({"schema_version":1,"verdict":"pass","unresolved_findings":[],"supporting_links":[]}),
        None,
        None,
    );
    assert_eq!(
        proof
            .verify_failure_inventory(&built.runtime.control.operations().unwrap())
            .unwrap_err()
            .code,
        "stale_result"
    );
    assert_eq!(
        canonical_digest(
            1,
            &built
                .runtime
                .control
                .operation(proof.failure_operation())
                .unwrap()
        )
        .unwrap(),
        canonical_digest(1, &prior_failure).unwrap()
    );
    fs::write(
        repair
            .contract
            .isolation()
            .plan()
            .repository
            .join("tests/main.txt"),
        "changed after source seal\n",
    )
    .unwrap();
    assert!(
        advanced
            .source()
            .verify_for_clone(&repair.execution.git)
            .is_err()
    );
}

#[path = "runtime_delivery_review_retry_tests.rs"]
mod review_retry_fixture;
use review_retry_fixture::ReviewFixture;

#[path = "runtime_delivery_round_fixture.rs"]
mod round_fixture;
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exact_round_preparation_reaches_native_join_barrier_and_final_round_repair() {
    Box::pin(repair_cycle_inner(
        false,
        false,
        None,
        false,
        Some(1),
        None,
        false,
    ))
    .await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recurring_findings_reuse_implementation_budget_after_actual_changed_source() {
    Box::pin(repair_cycle_inner(
        false,
        false,
        None,
        false,
        Some(2),
        None,
        false,
    ))
    .await;
}

#[path = "runtime_delivery_validation_retry_fixture.rs"]
mod validation_retry_fixture;
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exact_validation_retry_uses_native_grant_and_new_checks_without_join_fiction() {
    Box::pin(repair_cycle_inner(
        false,
        false,
        None,
        false,
        None,
        Some("serverOverloaded"),
        false,
    ))
    .await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exact_validation_retry_refuses_automatic_auth_failure_within_granted_budget() {
    Box::pin(repair_cycle_inner(
        false,
        false,
        None,
        false,
        None,
        Some("unauthorized"),
        false,
    ))
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn first_granted_review_cannot_omit_actual_prior_round_provenance() {
    Box::pin(repair_cycle_inner(
        false,
        false,
        None,
        false,
        Some(2),
        None,
        true,
    ))
    .await;
}
