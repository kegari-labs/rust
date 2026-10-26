//! Component evidence: real Git/Control/provider wire, synthetic model protocol.
//! No runtime joined-cache or native confinement capability is manufactured.
use super::*;
use crate::evidence::Signed;
use crate::runtime_delivery::worker_fixture::{
    WorkerFixture, complete_review_retry, failed_review, failed_review_retry,
};
use std::{path::Path, sync::Mutex};
pub(super) struct ReviewFixture<'a> {
    pub findings: bool,
    pub source_proof: &'a SealedChildSource,
    pub retry_again: bool,
    pub failure_code: Option<&'a str>,
    pub service: &'a RuntimeDelivery,
    pub built: &'a crate::runtime_fixture::RuntimeFixture,
    pub wire: &'a Arc<Mutex<crate::corrections::wire_fixture::Fixture>>,
    pub workspace: &'a WorkspaceDelivery,
    pub gate: &'a FreshGate,
    pub lease: &'a crate::control::Lease,
    pub reviewer: &'a WorkerFixture,
    pub original_claim: Arc<CompletedChildClaim>,
    pub original_completion: Arc<crate::worker_run::RunCompletion>,
    pub assignment: &'a DeliveryWorkerAssignment,
    pub review: &'a ChildReviewAssignment,
    pub root: &'a Path,
    pub source: &'a Path,
    pub binary: &'a Path,
    pub tool: Arc<Git>,
    pub revision: &'a crate::git::Revision,
    pub repository: &'a crate::git::RepositoryId,
    pub account: &'a str,
    pub scope: DeliveryScope,
}
pub(super) async fn run(f: ReviewFixture<'_>) {
    let mut failed = failed_review(&f.built.runtime.control, f.lease, f.reviewer).await;
    let original = f
        .built
        .runtime
        .control
        .operation(failed.operation_id)
        .unwrap();
    let original_digest = canonical_digest(1, &original).unwrap();
    f.built.runtime.observations.refresh().await.unwrap();
    let grant = retry_fixture::publish_failed_review(f.built.runtime.clone()).await;
    f.built.runtime.observations.refresh().await.unwrap();
    let config = f.built.runtime.configuration.snapshot().unwrap();
    let current = || {
        crate::runtime_workflow::current_retry_grants(
            &config,
            &f.built.runtime.observations.admission_snapshot().unwrap(),
            f.gate.work(),
            &f.built.runtime.control.operations().unwrap(),
            &f.built.runtime.services.key,
        )
    };
    let grants = current().unwrap();
    let phase = f
        .service
        .admit_delivery_phase(
            f.workspace,
            f.gate,
            f.lease,
            PhaseRequest {
                operation: Uuid::new_v4(),
                scope: f.scope.clone(),
            },
        )
        .await
        .unwrap();
    let make = |completion| ReviewRetryRequest {
        admission: &phase.admission,
        assignment: f.assignment,
        failed: completion,
        grant: grant.clone(),
        current_grants: &grants,
    };
    Arc::get_mut(&mut failed).unwrap().local_exit_confirmed = false;
    assert!(
        phase
            .coordinator
            .retry_failed_review(make(failed.clone()))
            .await
            .is_err()
    );
    Arc::get_mut(&mut failed).unwrap().local_exit_confirmed = true;
    Arc::get_mut(&mut failed).unwrap().state.unresolved_turn = Some("unknown-provider-turn".into());
    assert!(
        phase
            .coordinator
            .retry_failed_review(make(failed.clone()))
            .await
            .is_err()
    );
    Arc::get_mut(&mut failed).unwrap().state.unresolved_turn = None;
    let original_profile = failed.state.binding.profile.model.clone();
    Arc::get_mut(&mut failed)
        .unwrap()
        .state
        .binding
        .profile
        .model = "unadmitted-model".into();
    assert!(
        phase
            .coordinator
            .retry_failed_review(make(failed.clone()))
            .await
            .is_err()
    );
    Arc::get_mut(&mut failed)
        .unwrap()
        .state
        .binding
        .profile
        .model = original_profile;
    let original_generation = failed.state.binding.generation.clone();
    Arc::get_mut(&mut failed).unwrap().state.binding.generation =
        crate::digest::bytes_digest(b"unadmitted correction");
    assert!(
        phase
            .coordinator
            .retry_failed_review(make(failed.clone()))
            .await
            .is_err()
    );
    Arc::get_mut(&mut failed).unwrap().state.binding.generation = original_generation;
    let mut proof = Arc::new(
        phase
            .coordinator
            .retry_failed_review(make(failed.clone()))
            .await
            .unwrap(),
    );
    assert!(
        !proof.requires_review_round(),
        "legacy unmarked retry remains available"
    );
    assert_eq!(proof.source().revision(), f.revision);
    let mut forked = grants.clone();
    forked.push(grant.clone());
    assert!(
        proof
            .verify_current(&f.built.runtime.control.operations().unwrap(), &forked)
            .is_err()
    );
    assert_eq!(
        proof.assignment().digest().unwrap(),
        f.assignment.digest().unwrap()
    );
    assert_eq!(proof.phase().retry_grant, Some(grant.record().operation_id));
    // The same actual source must still match Git objects; a dirty byte is not
    // a harmless new session, even when the grant and head string are unchanged.
    let file = proof.source().root().join("tests/main.txt");
    let bytes = fs::read(&file).unwrap();
    fs::write(&file, "changed after grant\n").unwrap();
    assert!(
        phase
            .coordinator
            .retry_failed_review(make(failed.clone()))
            .await
            .is_err()
    );
    fs::write(&file, &bytes).unwrap();
    f.service
        .retire_delivery_phase(&phase, json!({"retry_bound":proof.digest()}))
        .unwrap();
    // The genuinely parsed native publication can be removed/edited. Neither
    // path may authorize preparation, even with the original private token.
    let child = f.gate.work().child.unwrap();
    let saved = f.wire.lock().unwrap().comments[&child].clone();
    f.wire
        .lock()
        .unwrap()
        .comments
        .get_mut(&child)
        .unwrap()
        .retain(|v| v["id"] != grant.provenance().record_id);
    f.built.runtime.observations.refresh().await.unwrap();
    assert!(
        proof
            .verify_current(
                &f.built.runtime.control.operations().unwrap(),
                &current().unwrap()
            )
            .is_err()
    );
    f.wire.lock().unwrap().comments.insert(child, saved.clone());
    f.built.runtime.observations.refresh().await.unwrap();
    proof
        .verify_current(
            &f.built.runtime.control.operations().unwrap(),
            &current().unwrap(),
        )
        .unwrap();
    {
        let mut wire = f.wire.lock().unwrap();
        let comment = wire
            .comments
            .get_mut(&child)
            .unwrap()
            .iter_mut()
            .find(|v| v["id"] == grant.provenance().record_id)
            .unwrap();
        comment["body"] = json!(comment["body"].as_str().unwrap().replace(
            "The exact finding is understood",
            "Changed unsigned instruction"
        ));
    }
    f.built.runtime.observations.refresh().await.unwrap();
    assert!(current().is_err());
    f.wire.lock().unwrap().comments.insert(child, saved);
    f.built.runtime.observations.refresh().await.unwrap();
    // The complete inventory also rejects an unrelated/new same-source result.
    let mut changed = f.built.runtime.control.operations().unwrap();
    let mut later = original.clone();
    later.operation_id = Uuid::new_v4();
    changed.push(later);
    assert!(proof.verify_inventory(&changed).is_err());
    // Synthetic fixture completion is intentionally absent from the actual
    // runtime joined cache: the full runtime launch remains unavailable here.
    f.built.runtime.retain_retry_grant(grant.clone()).unwrap();
    assert_eq!(
        f.built
            .runtime
            .next_attempt_for_review_retry(&proof)
            .unwrap_err()
            .code,
        "reconciliation_required"
    );
    let first_proof = proof.clone();
    if f.retry_again {
        let Some(next) = another_failed_review(&f, &proof, &config).await else {
            return;
        };
        proof = Arc::new(next);
    }
    let next = worker(WorkerInput {
        root: &f.root.join("retried-review"),
        source: f.source,
        binary: f.binary,
        git: f.tool.clone(),
        gate: f.gate,
        workflow: &f.workspace.workflow,
        revision: f.revision,
        repository: f.repository,
        runtime: f.lease.runtime_id,
        operation: Uuid::new_v4(),
        role: AgentRole::Review,
        phase: "child_codex_review",
        context: Value::Null,
        assignment: Some(proof.assignment()),
        account: f.account,
    });
    proof.verify_contract(&next.contract).unwrap();
    let wrong_role = worker(WorkerInput {
        root: &f.root.join("wrong-review-role"),
        source: f.source,
        binary: f.binary,
        git: Arc::new(Git::new(f.binary).unwrap()),
        gate: f.gate,
        workflow: &f.workspace.workflow,
        revision: f.revision,
        repository: f.repository,
        runtime: f.lease.runtime_id,
        operation: Uuid::new_v4(),
        role: AgentRole::Validation,
        phase: "child_codex_review",
        context: Value::Null,
        assignment: Some(proof.assignment()),
        account: f.account,
    });
    assert!(proof.verify_contract(&wrong_role.contract).is_err());
    f.built.runtime.observations.refresh().await.unwrap();
    let request = crate::runtime_workers::LaunchRequest {
        lease: f.lease.clone(),
        operation_id: next.contract.instructions().manifest.operation_id,
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
        validation_retry: None,
        review_retry: Some(proof.clone()),
        review_round: None,
        max_seconds: 60,
        max_turns: grant.new_budget().max_turns,
    };
    request.validate_binding().unwrap();
    let mut wrong = request.clone();
    wrong.phase.retry_grant = None;
    assert!(wrong.validate_binding().is_err());
    let mut wrong_generation = request.clone();
    wrong_generation.phase.generation = crate::digest::bytes_digest(b"different correction");
    assert!(wrong_generation.validate_binding().is_err());
    let (_records, outcome, completion) = if f.findings {
        crate::runtime_delivery::worker_fixture::complete_review_retry_claim(
            &f.built.runtime.control,
            f.lease,
            &next,
            &proof,
            json!({"schema_version":1,"verdict":"findings","unresolved_findings":["Required empty-input behavior is missing"],"supporting_links":[]}),
        )
    } else {
        complete_review_retry(&f.built.runtime.control, f.lease, &next, &proof)
    };
    if f.retry_again {
        assert!(
            first_proof
                .verify_completed_inventory(
                    &f.built.runtime.control.operations().unwrap(),
                    &current().unwrap(),
                    completion.operation_id,
                )
                .is_err()
        );
    }
    assert!(
        proof
            .verify_current(
                &f.built.runtime.control.operations().unwrap(),
                &current().unwrap()
            )
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
                scope: f.scope.clone(),
            },
        )
        .await
        .unwrap();
    if f.findings {
        Box::pin(findings::run(
            &f,
            &publication,
            &proof,
            &outcome,
            &completion,
        ))
        .await;
        return;
    }
    let runtime = f.built.runtime.clone();
    let checked = proof.clone();
    let completed = completion.operation_id;
    let check: GateCheck = Arc::new(move |_| {
        let receipts = runtime.control.operations()?;
        let grants = crate::runtime_workflow::current_retry_grants(
            &runtime.configuration.snapshot()?,
            &runtime.observations.admission_snapshot()?,
            checked.authority().work(),
            &receipts,
            &runtime.services.key,
        )?;
        checked.verify_completed_inventory(&receipts, &grants, completed)
    });
    let saved = f.wire.lock().unwrap().comments[&child].clone();
    f.wire
        .lock()
        .unwrap()
        .comments
        .get_mut(&child)
        .unwrap()
        .retain(|v| v["id"] != grant.provenance().record_id);
    f.built.runtime.observations.refresh().await.unwrap();
    assert!(check(f.gate).is_err());
    f.wire.lock().unwrap().comments.insert(child, saved);
    f.built.runtime.observations.refresh().await.unwrap();
    let result = publication
        .coordinator
        .publish_retried_review(
            &publication.admission,
            f.review,
            &outcome,
            &completion,
            &proof,
            check,
        )
        .await
        .unwrap();
    assert!(!result.publication().record_id.is_empty());
    assert_eq!(
        result.verified().attestation().verdict,
        crate::evidence::Verdict::Pass
    );
    assert_eq!(
        result.verified().attestation().revision.head,
        f.revision.commit
    );
    f.service
        .retire_delivery_phase(&publication, json!({"review_passed":true}))
        .unwrap();
    assert_eq!(
        canonical_digest(
            1,
            &f.built
                .runtime
                .control
                .operation(failed.operation_id)
                .unwrap()
        )
        .unwrap(),
        original_digest
    );
    let prior = crate::control::RunReference::from_receipt(&original)
        .unwrap()
        .unwrap();
    let new = crate::control::RunReference::from_receipt(
        &f.built
            .runtime
            .control
            .operation(completion.operation_id)
            .unwrap(),
    )
    .unwrap()
    .unwrap();
    // This legacy completed-grant fixture has no admitted opaque review round.
    // It cannot advance the current pipeline or manufacture that missing cause
    // from its protected completion receipt. Preserve both operation identities.
    // The actual-round fixture separately reaches the native joined-run barrier
    // through admit_review_round_worker using its genuine round and closed prep.
    let mut child = super::super::ChildPipeline {
        reference: super::super::ChildReference {
            schema_version: 1,
            runtime: f.lease.runtime_id,
            operation: f.original_claim.operation_id(),
            proposal: Uuid::new_v4(),
            claim_digest: f.original_claim.digest().into(),
            work: f.gate.work().clone(),
            stage: super::super::Stage::LaunchReviewRetry,
            phase_operation: None,
            validation: None,
            validation_assignment_digest: None,
            validation_source_digest: None,
            validation_retry_operation: None,
            validation_retry_digest: None,
            validation_completed_digest: None,
            validation_passed_digest: None,
            validation_successor_digest: None,
            validation_history: Vec::new(),
            review: Some(failed.operation_id),
            repair_operation: None,
            unfinished_digest: None,
            source_advanced_digest: None,
            review_retry_operation: Some(completion.operation_id),
            review_retry_digest: Some(proof.digest().into()),
            review_history: vec![],
            review_round_digest: None,
            paired_round_digest: None,
            paired_completion_digest: None,
            paired_completion_origin: None,
            observed_at: Utc::now(),
            error: None,
        },
        claim: f.original_claim,
        completion: f.original_completion,
        source: None,
        local: None,
        branch: None,
        pr: None,
        hosted: None,
        exact_pending: None,
        exact: None,
        assignment: Some(f.review.clone()),
        review: None,
        pending: None,
        staged: None,
        published: None,
        phase: None,
        unfinished: None,
        source_advanced: None,
        validation_assignment: None,
        validation_source: None,
        validation_retry: None,
        validation_completed: None,
        validation_passed: None,
        validation_successor: None,
        review_worker_assignment: Some(f.assignment.clone()),
        review_retry: Some(proof.clone()),
        review_round: None,
        paired_round: None,
        paired_completion: None,
    };
    f.service.persist_child(&child).unwrap();
    let operations_before =
        canonical_digest(1, &f.built.runtime.control.operations().unwrap()).unwrap();
    let requests_before = f.wire.lock().unwrap().requests.len();
    let pipeline_leaf = format!("child-pipeline-{}.json", child.reference.operation);
    let pipeline_before = fs::read(f.service.records.path().join(&pipeline_leaf)).unwrap();
    let unavailable = f
        .service
        .launch_review_retry(f.lease, &mut child)
        .await
        .unwrap_err();
    assert_eq!(unavailable.code, "review_round_unavailable");
    assert_eq!(
        unavailable.message,
        "retry requires its retained opaque review round"
    );
    assert_eq!(
        canonical_digest(1, &f.built.runtime.control.operations().unwrap()).unwrap(),
        operations_before
    );
    assert_eq!(f.wire.lock().unwrap().requests.len(), requests_before);
    assert_eq!(
        fs::read(f.service.records.path().join(&pipeline_leaf)).unwrap(),
        pipeline_before
    );
    assert_eq!(
        child.reference.stage,
        super::super::Stage::LaunchReviewRetry
    );
    assert_eq!(child.reference.review, Some(failed.operation_id));
    assert_eq!(
        child.reference.review_retry_operation,
        Some(completion.operation_id)
    );
    let retained: Signed<super::super::ChildReference> = f
        .service
        .records
        .read(&format!(
            "child-pipeline-{}.json",
            child.reference.operation
        ))
        .unwrap();
    f.built
        .runtime
        .services
        .key
        .verify("runtime_child_pipeline", &retained)
        .unwrap();
    assert_eq!(retained.payload.review, Some(failed.operation_id));
    assert_eq!(prior.max_attempts, 1);
    assert_eq!(prior.max_review_rounds, 1);
    assert_eq!(new.attempt, if f.retry_again { 2 } else { 1 });
    assert_eq!(
        new.max_attempts,
        if f.retry_again {
            2
        } else {
            grant.new_budget().max_attempts
        }
    );
    assert_eq!(new.phase, grant.phase_key());
    if f.retry_again {
        assert_eq!((new.max_turns, new.max_review_rounds), (7, 2));
    }
}

async fn another_failed_review(
    f: &ReviewFixture<'_>,
    prior: &Arc<ReviewRetryWork>,
    config: &crate::config::Config,
) -> Option<ReviewRetryWork> {
    let host = &f.built.runtime.control;
    let inventory = host.operations().unwrap();
    let failed_at = host.operation(prior.predecessor()).unwrap().updated_at;
    // The component advances only the planner's clock. No native worker is
    // launched, and no synthetic completion enters the runtime joined cache.
    let mut lowered = config.runtime.clone();
    lowered.max_attempts_per_phase = 2;
    lowered.max_turns = 7;
    lowered.max_review_rounds = 2;
    assert_eq!(
        crate::runtime_schedule::plan_review_retry(
            &lowered,
            prior.phase(),
            f.gate.workspace_id(),
            &inventory,
            prior,
            failed_at,
        )
        .unwrap_err()
        .code,
        "retry_wait"
    );
    let schedule = crate::runtime_schedule::plan_review_retry(
        &lowered,
        prior.phase(),
        f.gate.workspace_id(),
        &inventory,
        prior,
        failed_at + Duration::seconds(60),
    )
    .unwrap();
    assert_eq!(schedule.attempt, 1);
    let fixture = worker(WorkerInput {
        root: &f.root.join("granted-failed-review"),
        source: f.source,
        binary: f.binary,
        git: f.tool.clone(),
        gate: f.gate,
        workflow: &f.workspace.workflow,
        revision: f.revision,
        repository: f.repository,
        runtime: f.lease.runtime_id,
        operation: Uuid::new_v4(),
        role: AgentRole::Review,
        phase: "child_codex_review",
        context: Value::Null,
        assignment: Some(prior.assignment()),
        account: f.account,
    });
    let mut failed = failed_review_retry(
        host,
        f.lease,
        &fixture,
        prior,
        &schedule,
        Some(30_000),
        f.failure_code.unwrap(),
    )
    .await;
    let transient = failed.state.terminal_transient_failure().unwrap();
    let receipt = host.operation(failed.operation_id).unwrap();
    let receipt_digest = canonical_digest(1, &receipt).unwrap();
    f.built.runtime.observations.refresh().await.unwrap();
    let grants = crate::runtime_workflow::current_retry_grants(
        config,
        &f.built.runtime.observations.admission_snapshot().unwrap(),
        f.gate.work(),
        &host.operations().unwrap(),
        &f.built.runtime.services.key,
    )
    .unwrap();
    assert!(
        prior
            .verify_current(&host.operations().unwrap(), &grants)
            .is_err()
    );
    let phase = f
        .service
        .admit_delivery_phase(
            f.workspace,
            f.gate,
            f.lease,
            PhaseRequest {
                operation: Uuid::new_v4(),
                scope: f.scope.clone(),
            },
        )
        .await
        .unwrap();
    if transient.is_none() {
        let count = host.operations().unwrap().len();
        let error = phase
            .coordinator
            .continue_failed_review(&phase.admission, prior, failed.clone(), &grants)
            .await
            .err()
            .expect("nontransient review cannot retry automatically");
        assert_eq!(error.code, "persistent_blocker");
        assert_eq!(host.operations().unwrap().len(), count);
        assert_eq!(
            canonical_digest(1, &host.operation(failed.operation_id).unwrap()).unwrap(),
            receipt_digest
        );
        f.service
            .retire_delivery_phase(&phase, json!({"blocked":error.code}))
            .unwrap();
        return None;
    }
    assert_eq!(
        transient,
        Some(crate::codex::TransientTurnFailure::ServerOverloaded)
    );
    Arc::get_mut(&mut failed).unwrap().local_exit_confirmed = false;
    assert!(
        phase
            .coordinator
            .continue_failed_review(&phase.admission, prior, failed.clone(), &grants)
            .await
            .is_err()
    );
    Arc::get_mut(&mut failed).unwrap().local_exit_confirmed = true;
    let path = prior.source().root().join("tests/main.txt");
    let original = fs::read(&path).unwrap();
    fs::write(&path, "movement before same-budget retry\n").unwrap();
    assert!(
        phase
            .coordinator
            .continue_failed_review(&phase.admission, prior, failed.clone(), &grants)
            .await
            .is_err()
    );
    fs::write(&path, original).unwrap();
    let next = phase
        .coordinator
        .continue_failed_review(&phase.admission, prior, failed.clone(), &grants)
        .await
        .unwrap();
    assert_ne!(next.digest(), prior.digest());
    assert_eq!(next.grant().digest(), prior.grant().digest());
    assert_eq!(
        next.assignment().digest().unwrap(),
        prior.assignment().digest().unwrap()
    );
    assert_eq!(next.source().revision(), prior.source().revision());
    assert_eq!(next.predecessor(), failed.operation_id);
    next.verify_current(&host.operations().unwrap(), &grants)
        .unwrap();
    let mut forked = grants.clone();
    forked.push(prior.grant().clone());
    assert!(
        next.verify_current(&host.operations().unwrap(), &forked)
            .is_err()
    );
    let receipts = host.operations().unwrap();
    assert_eq!(
        crate::runtime_schedule::plan(
            &config.runtime,
            next.phase(),
            f.gate.workspace_id(),
            &receipts,
            receipt.updated_at + Duration::seconds(60)
        )
        .unwrap_err()
        .code,
        "persistent_blocker"
    );
    assert_eq!(
        crate::runtime_schedule::plan_review_retry(
            &config.runtime,
            next.phase(),
            f.gate.workspace_id(),
            &receipts,
            &next,
            receipt.updated_at
        )
        .unwrap_err()
        .code,
        "retry_wait"
    );
    let scheduled = crate::runtime_schedule::plan_review_retry(
        &config.runtime,
        next.phase(),
        f.gate.workspace_id(),
        &receipts,
        &next,
        receipt.updated_at + Duration::seconds(60),
    )
    .unwrap();
    assert_eq!(
        (
            scheduled.attempt,
            scheduled.max_attempts,
            scheduled.max_turns,
            scheduled.max_review_rounds
        ),
        (2, 2, 7, 2)
    );
    assert_eq!(
        scheduled.not_before,
        receipt.updated_at + Duration::seconds(30)
    );
    let mut exhausted = config.runtime.clone();
    exhausted.max_attempts_per_phase = 1;
    assert_eq!(
        crate::runtime_schedule::plan_review_retry(
            &exhausted,
            next.phase(),
            f.gate.workspace_id(),
            &receipts,
            &next,
            receipt.updated_at + Duration::seconds(60)
        )
        .unwrap_err()
        .code,
        "attempts_exhausted"
    );
    let mut raced = receipts.clone();
    let mut extra = receipt.clone();
    extra.operation_id = Uuid::new_v4();
    raced.push(extra);
    assert!(next.verify_current(&raced, &grants).is_err());
    f.service
        .retire_delivery_phase(&phase, json!({"review_retry":next.digest()}))
        .unwrap();
    assert_eq!(
        canonical_digest(1, &host.operation(failed.operation_id).unwrap()).unwrap(),
        receipt_digest
    );
    Some(next)
}

#[path = "runtime_delivery_review_findings_tests.rs"]
mod findings;
