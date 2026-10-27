//! Real source/Control/grant evidence through the local-validation barrier.
//! Downstream PR assignment is explicitly synthetic upstream component input.
use super::*;
use crate::runtime_delivery::worker_fixture::complete;
use crate::{
    providers::{HttpRequest, HttpResponse, HttpTransport},
    worker_run::RunCompletion,
};
use std::collections::BTreeSet;

fn current(f: &ReviewFixture<'_>) -> Result<Vec<crate::decisions::VerifiedRetryGrant>> {
    crate::runtime_workflow::current_retry_grants(
        &f.built.runtime.configuration.snapshot()?,
        &f.built.runtime.observations.admission_snapshot()?,
        f.gate.work(),
        &f.built.runtime.control.operations()?,
        &f.built.runtime.services.key,
    )
}
fn repair_request<'a>(
    f: &'a ReviewFixture<'_>,
    phase: &'a super::super::super::ActivePhase,
    outcome: &'a crate::runtime::VerifiedAuthorizedOutcome,
    completion: &Arc<RunCompletion>,
) -> ReviewRepairRequest<'a> {
    ReviewRepairRequest {
        source: RepairSource {
            paired_completion: None,
            admission: &phase.admission,
            original: f.original_claim.clone(),
            source: f.source_proof,
            completion: f.original_completion.clone(),
        },
        assignment: f.review,
        outcome,
        completion: completion.clone(),
    }
}
pub(super) async fn run(
    f: &ReviewFixture<'_>,
    phase: &super::super::super::ActivePhase,
    retry: &ReviewRetryWork,
    outcome: &crate::runtime::VerifiedAuthorizedOutcome,
    completion: &Arc<RunCompletion>,
) {
    let host = &f.built.runtime.control;
    // Ordinary mixed-grant continuation remains closed.
    assert_eq!(
        phase
            .coordinator
            .unfinished_review(repair_request(f, phase, outcome, completion))
            .await
            .err()
            .unwrap()
            .code,
        "stale_result"
    );
    let proof = Arc::new(
        phase
            .coordinator
            .unfinished_retried_review(
                repair_request(f, phase, outcome, completion),
                retry,
                &current(f).unwrap(),
            )
            .await
            .unwrap(),
    );
    assert_eq!(proof.phase().retry_grant, None);
    assert_eq!(proof.phase().phase, "implementation");
    assert_eq!(proof.reference().max_attempts, 3);
    assert_eq!(
        proof.review_rounds_consumed(),
        2,
        "conservative counter includes the failed original review"
    );
    assert_eq!(proof.failure_operation(), completion.operation_id);
    assert_eq!(proof.granted_review().unwrap().phase(), retry.phase());
    let old_receipt = canonical_digest(1, &host.operation(proof.predecessor()).unwrap()).unwrap();
    let before = host.operations().unwrap();
    let mut fork = current(f).unwrap();
    fork.push(fork[0].clone());
    assert!(proof.verify_current(&before, &fork).is_err());
    assert!(completion.checks.is_empty() && completion.failed_checks.is_empty());
    let changed_completion = RunCompletion {
        operation_id: completion.operation_id,
        state: completion.state.clone(),
        outcome: crate::worker_run::RunOutcome::Completed,
        error: completion.error.clone(),
        elapsed_ms: completion.elapsed_ms,
        hook_error: completion.hook_error.clone(),
        checks: vec![],
        failed_checks: vec![],
        validation_plan: None,
        validation_artifacts: vec![],
        validation_attempts: vec![],
        validation_error: None,
        validation_history: None,
        execution: completion.execution.clone(),
        contract: completion.contract.clone(),
        local_exit_confirmed: false,
    };
    assert!(
        phase
            .coordinator
            .unfinished_retried_review(
                repair_request(f, phase, outcome, &Arc::new(changed_completion)),
                retry,
                &current(f).unwrap()
            )
            .await
            .is_err()
    );
    let child = f.gate.work().child.unwrap();
    let saved = f.wire.lock().unwrap().comments[&child].clone();
    f.wire
        .lock()
        .unwrap()
        .comments
        .get_mut(&child)
        .unwrap()
        .retain(|v| v["id"] != retry.grant().provenance().record_id);
    f.built.runtime.observations.refresh().await.unwrap();
    assert!(
        proof
            .verify_current(&host.operations().unwrap(), &current(f).unwrap())
            .is_err()
    );
    f.wire.lock().unwrap().comments.insert(child, saved.clone());
    f.built.runtime.observations.refresh().await.unwrap();
    proof
        .verify_current(&host.operations().unwrap(), &current(f).unwrap())
        .unwrap();
    f.service
        .retire_delivery_phase(phase, json!({"findings":true}))
        .unwrap();
    assert_eq!(
        f.built
            .runtime
            .next_attempt_for_unfinished(&proof)
            .unwrap_err()
            .code,
        "reconciliation_required",
        "no native joined cache is manufactured"
    );
    let mut limits = f.built.runtime.configuration.snapshot().unwrap().runtime;
    limits.max_attempts_per_phase = 1;
    let continuation = crate::control::RunContinuationReference {
        predecessor: proof.predecessor(),
        predecessor_receipt_digest: proof.predecessor_receipt_digest().into(),
        trigger_digest: proof.digest().into(),
        progress_digest: proof.progress_digest().into(),
        failure_operation: proof.failure_operation(),
        review_rounds_consumed: proof.review_rounds_consumed(),
    };
    assert_eq!(
        crate::runtime_schedule::plan_unfinished(
            &limits,
            proof.phase(),
            f.gate.workspace_id(),
            &host.operations().unwrap(),
            &continuation,
            Utc::now() + Duration::seconds(60)
        )
        .unwrap_err()
        .code,
        "attempts_exhausted"
    );
    let assignment = repair_worker_assignment(&proof).unwrap();
    let repair = worker(WorkerInput {
        root: &f.root.join("granted-findings-repair"),
        source: proof.source().root(),
        binary: f.binary,
        git: f.tool.clone(),
        gate: f.gate,
        workflow: &f.workspace.workflow,
        revision: f.revision,
        repository: f.repository,
        runtime: f.lease.runtime_id,
        operation: Uuid::new_v4(),
        role: AgentRole::Testing,
        phase: "implementation",
        context: Value::Null,
        assignment: Some(&assignment),
        account: f.account,
    });
    proof.verify_contract(&repair.contract).unwrap();
    assert!(proof.verify_contract(&f.reviewer.contract).is_err());
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    fs::write(
        repair
            .contract
            .isolation()
            .plan()
            .repository
            .join("tests/main.txt"),
        "required empty-input behavior fixed\n",
    )
    .unwrap();
    let (_records, repaired_outcome, repaired) = complete(
        host,
        f.lease,
        &repair,
        json!({"schema_version":1,"summary":"scoped finding fixed","fulfilled_criteria":["required behavior"],"disposition":{"kind":"changed"}}),
        Some(&proof),
        None,
    );
    let admitted = RunReference::from_receipt(&host.operation(repaired.operation_id).unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(admitted.phase, *proof.phase());
    assert_eq!(admitted.attempt, 2);
    assert_eq!(admitted.max_attempts, 3);
    assert_eq!(
        canonical_digest(1, &host.operation(proof.predecessor()).unwrap()).unwrap(),
        old_receipt
    );
    let claim = Arc::new(
        f.workspace
            .coordinator
            .accept_child_completion(f.gate, repaired_outcome, repaired.clone())
            .unwrap(),
    );
    f.built.runtime.observations.refresh().await.unwrap();
    let mut scope = f.scope.clone();
    scope.phase = DeliveryPhase::SourceSealing;
    let sealing = f
        .service
        .admit_delivery_phase(
            f.workspace,
            f.gate,
            f.lease,
            PhaseRequest {
                operation: Uuid::new_v4(),
                scope,
            },
        )
        .await
        .unwrap();
    let changed = sealing.coordinator.seal_child_source(&sealing.admission, &claim, &repair.execution, repair.contract.isolation(), crate::git::SourceCommitMetadata {
        identity: crate::git::CommitIdentity { name:"Disposable Fixture".into(), email:"fixture@example.invalid".into() },
        timestamp: chrono::DateTime::from_timestamp(1_767_225_602,0).unwrap(), message:"test: repair granted review finding\n\nSigned-off-by: Disposable Fixture <fixture@example.invalid>\n".into(),
    }).await.unwrap();
    assert_ne!(changed.revision().tested_tree, f.revision.tree);
    f.service
        .retire_delivery_phase(&sealing, json!({"actual_source_sealed":true}))
        .unwrap();
    let mut scope = f.scope.clone();
    scope.phase = DeliveryPhase::LocalValidation;
    scope.revision = changed.revision().clone();
    let local = f
        .service
        .admit_delivery_phase(
            f.workspace,
            f.gate,
            f.lease,
            PhaseRequest {
                operation: Uuid::new_v4(),
                scope,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        local
            .coordinator
            .verify_local_checks(&local.admission, &f.workspace.workflow, &[])
            .err()
            .unwrap()
            .code,
        "validation_gate"
    );
    f.service
        .retire_delivery_phase(
            &local,
            json!({"blocked":"missing qualified local check; no production PR authority"}),
        )
        .unwrap();

    // Separate downstream component: an explicitly test-only upstream PR
    // assignment is supplied. It does not erase the genuine barrier above.
    let wire = Arc::new(ReviewWire {
        revision: changed.revision().clone(),
        state: Mutex::new((Vec::new(), Vec::new())),
    });
    f.wire.lock().unwrap().github_transport = Some(wire.clone());
    f.built.runtime.observations.refresh().await.unwrap();
    let mut scope = f.scope.clone();
    scope.phase = DeliveryPhase::ReviewPublication;
    scope.revision = changed.revision().clone();
    let review_phase = f
        .service
        .admit_delivery_phase(
            f.workspace,
            f.gate,
            f.lease,
            PhaseRequest {
                operation: Uuid::new_v4(),
                scope: scope.clone(),
            },
        )
        .await
        .unwrap();
    let (review, assignment) = review_phase
        .coordinator
        .fixture_upstream_changed_review(&review_phase.admission, &claim, &changed, &repaired)
        .unwrap();
    let lineage = proof.granted_review().unwrap();
    let request = || SourceAdvancedRequest {
        source: RepairSource {
            paired_completion: None,
            admission: &review_phase.admission,
            original: claim.clone(),
            source: &changed,
            completion: repaired.clone(),
        },
        assignment: &assignment,
        predecessor: lineage.outcome(),
        completion: lineage.completion().clone(),
    };
    let ordinary = review_phase
        .coordinator
        .source_advanced(request())
        .await
        .err()
        .unwrap();
    assert_eq!(ordinary.code, "stale_result");
    assert_eq!(
        ordinary.message,
        "later or differently granted check history prevents automatic continuation"
    );
    // The negative producer performs its own complete Git/source read. Refresh
    // actual native observations before the next bounded producer action; the
    // original admission, assignment and completed contracts remain unchanged.
    f.built.runtime.observations.refresh().await.unwrap();
    let advanced = Arc::new(
        review_phase
            .coordinator
            .source_advanced_retried_review(request(), &proof, &current(f).unwrap())
            .await
            .unwrap(),
    );
    assert_eq!(advanced.phase(), retry.phase());
    assert_eq!(advanced.reference().attempt, 1);
    assert_eq!(advanced.source_operation(), repaired.operation_id);
    assert_eq!(advanced.review_rounds_consumed(), 2);
    f.service
        .retire_delivery_phase(&review_phase, json!({"synthetic_upstream_assignment":true}))
        .unwrap();
    assert_eq!(
        f.built
            .runtime
            .next_attempt_for_source_advanced(&advanced)
            .unwrap_err()
            .code,
        "reconciliation_required"
    );
    let reviewer = worker(WorkerInput {
        root: &f.root.join("changed-granted-review"),
        source: advanced.source().root(),
        binary: f.binary,
        git: f.tool.clone(),
        gate: f.gate,
        workflow: &f.workspace.workflow,
        revision: advanced.source().revision(),
        repository: f.repository,
        runtime: f.lease.runtime_id,
        operation: Uuid::new_v4(),
        role: AgentRole::Review,
        phase: "child_codex_review",
        context: Value::Null,
        assignment: Some(&assignment),
        account: f.account,
    });
    advanced.verify_contract(&reviewer.contract).unwrap();
    // Authenticated historical gate observations cannot be used as a current
    // launch input. The host must obtain equal semantic authority from actual
    // fresh native observations without changing the proof or assignment.
    let assignment_digest = assignment.digest().unwrap();
    let mut saved_authority = serde_json::to_value(advanced.authority()).unwrap();
    saved_authority["observed_at"] = json!(Utc::now() - Duration::minutes(10));
    let retained = f
        .built
        .runtime
        .services
        .key
        .sign(
            "pending_worker_claim",
            &json!({"schema_version":1,"authority":saved_authority,"outcome":lineage.outcome().outcome()}),
        )
        .unwrap();
    let historical = FreshGate::recover_host_claim(
        &f.built.runtime.services.key,
        &retained,
        &advanced.authority().digest().unwrap(),
    )
    .unwrap();
    let mut launch = crate::runtime_workers::LaunchRequest {
        lease: f.lease.clone(),
        operation_id: reviewer.execution.binding.operation_id,
        authority: WorkerAuthority::Authorized(historical.clone()),
        phase: advanced.phase().clone(),
        source: None,
        delivery: Some(assignment.clone()),
        unfinished: None,
        retried_unfinished: None,
        validation_retry: None,
        review_retry: None,
        review_round: None,
        source_advanced: Some(advanced.clone()),
        max_seconds: 60,
        max_turns: advanced.reference().max_turns,
    };
    assert_eq!(launch.validate_binding().unwrap_err().code, "stale_result");
    f.built.runtime.observations.refresh().await.unwrap();
    launch.authority = f
        .service
        .current_delivery_worker_authority(&historical)
        .unwrap();
    launch.validate_binding().unwrap();
    assert_eq!(assignment.digest().unwrap(), assignment_digest);
    advanced.verify_contract(&reviewer.contract).unwrap();
    let mut wrong = launch.clone();
    wrong.phase.retry_grant = None;
    assert!(wrong.validate_binding().is_err());
    let mut changed_generation = launch.clone();
    changed_generation.phase.generation = crate::digest::bytes_digest(b"other correction");
    assert!(changed_generation.validate_binding().is_err());
    f.wire
        .lock()
        .unwrap()
        .comments
        .get_mut(&child)
        .unwrap()
        .retain(|v| v["id"] != retry.grant().provenance().record_id);
    f.built.runtime.observations.refresh().await.unwrap();
    assert_granted_admission_denied(f, &reviewer, &advanced);
    f.wire.lock().unwrap().comments.insert(child, saved.clone());
    f.built.runtime.observations.refresh().await.unwrap();
    {
        let mut state = f.wire.lock().unwrap();
        let comment = state
            .comments
            .get_mut(&child)
            .unwrap()
            .iter_mut()
            .find(|value| value["id"] == retry.grant().provenance().record_id)
            .unwrap();
        let body = comment["body"].as_str().unwrap().to_owned();
        comment["body"] = json!(format!("{body}\nEdited native grant"));
    }
    f.built.runtime.observations.refresh().await.unwrap();
    assert!(current(f).is_err());
    assert_granted_admission_denied(f, &reviewer, &advanced);
    f.wire.lock().unwrap().comments.insert(child, saved.clone());
    f.built.runtime.observations.refresh().await.unwrap();
    advanced
        .verify_current(&host.operations().unwrap(), &current(f).unwrap())
        .unwrap();
    let source_file = reviewer
        .contract
        .isolation()
        .plan()
        .repository
        .join("tests/main.txt");
    let bytes = fs::read(&source_file).unwrap();
    fs::write(&source_file, "unexpected review edit\n").unwrap();
    assert!(
        reviewer
            .execution
            .git
            .verify_execution_tree(
                &reviewer.contract.isolation().plan().repository,
                advanced.source().revision(),
                &BTreeSet::new()
            )
            .is_err()
    );
    fs::write(&source_file, bytes).unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let (_review_records, clean_outcome, clean) = complete(
        host,
        f.lease,
        &reviewer,
        json!({"schema_version":1,"verdict":"pass","unresolved_findings":[],"supporting_links":[retry.grant().provenance().url]}),
        None,
        Some(&advanced),
    );
    let run = RunReference::from_receipt(&host.operation(clean.operation_id).unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(run.phase, *retry.phase());
    assert_eq!(run.attempt, 2);
    // Read-only substituted observations retain the same phase/head/ordinal;
    // none may replace the actual causal source-advanced admission reference.
    for field in [
        "absent",
        "predecessor",
        "predecessor_receipt_digest",
        "source_operation",
        "source_receipt_digest",
        "trigger_digest",
        "progress_digest",
        "review_rounds_consumed",
    ] {
        let mut inventory = host.operations().unwrap();
        let receipt = inventory
            .iter_mut()
            .find(|r| r.operation_id == clean.operation_id)
            .unwrap();
        if field == "absent" {
            receipt.expected.remove("source_advanced");
        } else {
            let mut reference: Value =
                serde_json::from_str(&receipt.expected["source_advanced"]).unwrap();
            reference[field] = match field {
                "predecessor" | "source_operation" => json!(Uuid::new_v4()),
                "review_rounds_consumed" => json!(1),
                _ => json!(crate::digest::bytes_digest(b"another causal admission")),
            };
            receipt.expected.insert(
                "source_advanced".into(),
                serde_json::to_string(&reference).unwrap(),
            );
        }
        assert!(
            advanced
                .verify_completed_inventory(&inventory, &current(f).unwrap(), clean.operation_id)
                .is_err(),
            "substituted {field}"
        );
    }
    assert!(
        advanced
            .verify_current(&host.operations().unwrap(), &current(f).unwrap())
            .is_err()
    );
    assert!(
        proof
            .verify_current(&host.operations().unwrap(), &current(f).unwrap())
            .is_err()
    );
    f.built.runtime.observations.refresh().await.unwrap();
    let publication = f
        .service
        .admit_delivery_phase(
            f.workspace,
            f.gate,
            f.lease,
            PhaseRequest {
                operation: Uuid::new_v4(),
                scope,
            },
        )
        .await
        .unwrap();
    let runtime = f.built.runtime.clone();
    let checked = advanced.clone();
    let operation = clean.operation_id;
    let check: GateCheck = Arc::new(move |_| {
        let receipts = runtime.control.operations()?;
        let grants = crate::runtime_workflow::current_retry_grants(
            &runtime.configuration.snapshot()?,
            &runtime.observations.admission_snapshot()?,
            checked.authority().work(),
            &receipts,
            &runtime.services.key,
        )?;
        checked.verify_completed_inventory(&receipts, &grants, operation)
    });
    f.wire
        .lock()
        .unwrap()
        .comments
        .get_mut(&child)
        .unwrap()
        .retain(|v| v["id"] != retry.grant().provenance().record_id);
    f.built.runtime.observations.refresh().await.unwrap();
    assert!(
        publication
            .coordinator
            .publish_source_advanced_review(
                &publication.admission,
                &review,
                &clean_outcome,
                &clean,
                &advanced,
                check.clone()
            )
            .await
            .is_err()
    );
    assert!(wire.state.lock().unwrap().0.is_empty());
    f.wire.lock().unwrap().comments.insert(child, saved);
    f.built.runtime.observations.refresh().await.unwrap();
    let result = publication
        .coordinator
        .publish_source_advanced_review(
            &publication.admission,
            &review,
            &clean_outcome,
            &clean,
            &advanced,
            check,
        )
        .await
        .unwrap();
    assert_eq!(
        result.verified().attestation().verdict,
        crate::evidence::Verdict::Pass
    );
    assert_eq!(
        result.verified().attestation().revision,
        *changed.revision()
    );
    assert_eq!(wire.state.lock().unwrap().0.len(), 1);
    f.service
        .retire_delivery_phase(&publication, json!({"component_clean_review":true}))
        .unwrap();
    // Even after successful component publication, no actual joined native run
    // or qualified repository execution exists in the runtime fixture.
    assert!(
        f.built
            .runtime
            .retained_completion(clean.operation_id)
            .unwrap()
            .is_none()
    );
}

fn assert_granted_admission_denied(
    f: &ReviewFixture<'_>,
    worker: &WorkerFixture,
    proof: &VerifiedSourceAdvanced,
) {
    let host = &f.built.runtime.control;
    let mut reference = proof.reference().clone();
    reference.attempt += 1;
    reference.instruction_digest = worker.contract.instructions().manifest_digest.clone();
    let id = worker.execution.binding.operation_id;
    let before = host.operations().unwrap().len();
    let result = host.admit_run_source_advanced_observed(
        f.lease,
        crate::control::RunAdmission {
            request_id: id,
            work_key: proof.phase().work.clone(),
            scope_digest: worker.contract.scope_digest().into(),
            reference,
        },
        crate::control::SourceAdvancedReference {
            predecessor: proof.predecessor(),
            predecessor_receipt_digest: proof.predecessor_receipt_digest().into(),
            source_operation: proof.source_operation(),
            source_receipt_digest: proof.source_receipt_digest().into(),
            trigger_digest: proof.digest().into(),
            progress_digest: proof.progress_digest().into(),
            review_rounds_consumed: proof.review_rounds_consumed(),
        },
        |_| proof.verify_current(&host.operations()?, &current(f)?),
    );
    assert!(result.is_err());
    assert_eq!(host.operations().unwrap().len(), before);
    assert!(host.operation(id).is_err());
}

struct ReviewWire {
    revision: RevisionBinding,
    state: Mutex<(Vec<Value>, Vec<Value>)>,
}
impl HttpTransport for ReviewWire {
    fn send(
        &self,
        request: HttpRequest,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<HttpResponse>> + Send + '_>>
    {
        Box::pin(async move {
            let path = request.url.path();
            let post = request.method == crate::providers::Method::Post;
            let body: Value = if request.body.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&request.body).unwrap()
            };
            let mut state = self.state.lock().unwrap();
            let repo = json!({"id":42,"owner":{"id":7,"login":"fixture","type":"User"},"name":"repo","default_branch":"main","private":true,"archived":false,"disabled":false,"allow_squash_merge":true,"permissions":{"pull":true,"push":true}});
            let value = if path == "/user" {
                json!({"id":7,"login":"fixture","type":"User"})
            } else if path == "/repos/fixture/repo" {
                repo.clone()
            } else if path == "/user/repos" {
                json!([repo])
            } else if path == "/repos/fixture/repo/pulls" {
                json!([])
            } else if path.ends_with("/pulls/1") {
                json!({"id":1,"number":1,"state":"open","draft":false,"merged":false,"user":{"id":7,"login":"fixture"},"head":{"sha":self.revision.head,"ref":"fixture-repair","repo":{"id":42}},"base":{"sha":self.revision.base,"ref":"staging","repo":{"id":42}},"html_url":"https://github.invalid/fixture/repo/pull/1"})
            } else if path.contains("/git/commits/") {
                json!({"sha":self.revision.head,"tree":{"sha":self.revision.tested_tree},"parents":[{"sha":self.revision.base}]})
            } else if path.ends_with("/issues/1/comments") {
                if post {
                    let item = json!({"id":601,"body":body["body"],"user":{"id":7},"created_at":Utc::now(),"updated_at":Utc::now(),"html_url":"https://github.invalid/comment/601"});
                    state.0.push(item.clone());
                    item
                } else {
                    json!(state.0)
                }
            } else if path.contains("/statuses/") && post {
                let mut item = body;
                item["id"] = json!(602);
                item["creator"] = json!({"id":7});
                item["created_at"] = json!(Utc::now());
                state.1.push(item.clone());
                item
            } else if path.ends_with("/statuses") {
                json!(state.1)
            } else {
                panic!(
                    "unexpected downstream review fixture request {:?} {}",
                    request.method, path
                )
            };
            Ok(HttpResponse {
                status: if post { 201 } else { 200 },
                headers: Default::default(),
                body: serde_json::to_vec(&value).unwrap(),
            })
        })
    }
}
