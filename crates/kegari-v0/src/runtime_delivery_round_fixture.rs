//! Production source/round producers and real Control receipts, with explicit
//! synthetic model completion. No result is installed into the runtime joined
//! cache. Runtime admission must refuse at that actual native prerequisite.
use super::*;
use crate::control::{
    PreparationAdmission, PreparationReference, ReviewReceiptReference, ReviewRoundCause,
    ReviewRoundRunCause, ReviewRoundRunProvenance, RunAdmission,
};
use crate::delivery::{CompletedReviewRound, ReviewRoundSource, VerifiedReviewRoundPlan};
use crate::runtime_delivery::worker_fixture::{
    WorkerFixture, complete_admitted, failed_admitted_review_with_gate,
};

#[path = "runtime_delivery_round_pair_fixture.rs"]
mod pair;
#[path = "runtime_delivery_round_protocol_fixture.rs"]
mod protocol;
#[path = "runtime_delivery_round_wire_fixture.rs"]
mod wire;

pub(super) async fn run(
    f: ReviewFixture<'_>,
    limit: u32,
    prior_validation: crate::runtime::VerifiedAuthorizedOutcome,
    prior_validation_completion: Arc<crate::worker_run::RunCompletion>,
) {
    let config = f.built.runtime.configuration.snapshot().unwrap();
    assert_eq!(config.runtime.max_review_rounds, limit);
    let phase = begin_phase(&f, f.scope.clone()).await;
    let initial = Arc::new(
        maintain_phase_observations(
            &f.built.runtime,
            phase.coordinator.initial_review_round(
                ReviewRoundSource {
                    paired_completion: None,
                    admission: &phase.admission,
                    claim: f.original_claim.clone(),
                    source: f.source_proof,
                    completion: f.original_completion.clone(),
                    assignment: f.review,
                    validated_retry: None,
                },
                &config,
                &[],
            ),
        )
        .await
        .unwrap(),
    );
    assert_eq!(initial.reference().ordinal, 1);
    f.service
        .retire_delivery_phase(&phase, json!({"round_proposal":initial.digest()}))
        .unwrap();

    let (first, first_run, first_permit) = admit(
        &f,
        &initial,
        ReviewRoundRunCause::Review,
        "round-transient",
        true,
    )
    .await;
    let protocol = Arc::new(protocol::RoundProtocol::new(
        f.built.runtime.clone(),
        first_permit,
        initial.authority().clone(),
        f.service.current_gate(),
    ));
    let failed = failed_admitted_review_with_gate(
        &f.built.runtime.control,
        &first,
        first_run.max_turns,
        None,
        Some("serverOverloaded"),
        Some(protocol.clone()),
    )
    .await;
    assert!(failed.state.terminal_transient_failure().unwrap().is_some());
    protocol.assert_closed_review_effect(failed.operation_id);
    let failed_at = f
        .built
        .runtime
        .control
        .operation(failed.operation_id)
        .unwrap()
        .updated_at;
    refresh(&f).await;
    let retry = Arc::new(
        f.workspace
            .coordinator
            .transient_review_round(initial.clone(), failed, &config, &[])
            .unwrap(),
    );
    assert_eq!(retry.reference().round_id, initial.reference().round_id);
    assert_eq!(retry.reference().ordinal, 1);
    assert!(matches!(
        retry.reference().cause,
        ReviewRoundCause::TransientRetry { .. }
    ));
    // Evaluate timing at the actual retained terminal instant, without changing
    // any protected timestamp or pretending the live gate has become newer.
    let wait = schedule_at(&f, &retry, &ReviewRoundRunCause::Review, failed_at).unwrap_err();
    assert_eq!(wait.code, "retry_wait");
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let (reviewer, review_run, _review_permit) = admit(
        &f,
        &retry,
        ReviewRoundRunCause::Review,
        "round-findings-1",
        false,
    )
    .await;
    assert_eq!(review_run.attempt, 2);
    assert_eq!(review_run.max_attempts, first_run.max_attempts);
    assert_eq!(review_run.max_review_rounds, limit);
    let (_records, outcome, completed) =
        complete_admitted(&f.built.runtime.control, &reviewer, findings(1));
    assert!(
        initial
            .verify_current(&f.built.runtime.control.operations().unwrap(), &[])
            .is_err()
    );

    let mut current_plan = retry;
    let mut current_review = (*f.review).clone();
    let mut current_source: Option<SealedChildSource> = None;
    let mut current_claim = f.original_claim.clone();
    let mut current_implementation = f.original_completion.clone();
    let mut current_outcome = outcome;
    let mut current_completion = completed;
    // Keep protected synthetic outcome fixtures alive for every dependent read.
    let mut retained_claim_files = vec![_records];
    for ordinal in 1..=limit {
        refresh(&f).await;
        let mut scope = f.scope.clone();
        scope.phase = if ordinal == 1 {
            DeliveryPhase::NoChange
        } else {
            DeliveryPhase::ReviewPublication
        };
        scope.revision = current_source
            .as_ref()
            .unwrap_or(f.source_proof)
            .revision()
            .clone();
        let observed = begin_phase(&f, scope.clone()).await;
        let complete: Arc<CompletedReviewRound> = Arc::new(
            maintain_phase_observations(
                &f.built.runtime,
                observed.coordinator.completed_review_round(
                    &observed.admission,
                    &current_review,
                    current_plan.clone(),
                    current_outcome.clone(),
                    current_completion.clone(),
                    &[],
                ),
            )
            .await
            .unwrap(),
        );
        assert_eq!(complete.plan().reference().ordinal, ordinal);
        let unfinished = Arc::new(
            maintain_phase_observations(
                &f.built.runtime,
                observed.coordinator.unfinished_completed_round(
                    ReviewRepairRequest {
                        source: RepairSource {
                            paired_completion: None,
                            admission: &observed.admission,
                            original: current_claim.clone(),
                            source: current_source.as_ref().unwrap_or(f.source_proof),
                            completion: current_implementation.clone(),
                        },
                        assignment: &current_review,
                        outcome: &current_outcome,
                        completion: current_completion.clone(),
                    },
                    complete.clone(),
                    &[],
                ),
            )
            .await
            .unwrap(),
        );
        let paired = Arc::new(
            observed
                .coordinator
                .paired_review_repair(complete, unfinished.clone(), &config, &[])
                .unwrap(),
        );
        assert_eq!(paired.reference().ordinal, ordinal);
        assert_eq!(
            paired.reference().round_id,
            current_plan.reference().round_id
        );
        assert_eq!(paired.subject_phase().phase, "implementation");
        assert!(paired.subject_phase().retry_grant.is_none());
        assert!(unfinished.requires_review_round());
        assert_omitted_round(
            &f,
            paired.assignment(),
            Some(unfinished.clone()),
            None,
            Some(paired.clone()),
        )
        .await;
        let omitted = f
            .built
            .runtime
            .next_attempt_for_unfinished(&unfinished)
            .unwrap_err();
        assert_eq!(omitted.code, "worker_binding");
        assert_eq!(
            omitted.message,
            "actual common review lineage requires its opaque round plan"
        );
        f.service.retire_delivery_phase(&observed, json!({"actual_findings":unfinished.failure_operation(),"paired_round":paired.digest()})).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        let link = crate::control::RunContinuationReference {
            predecessor: unfinished.predecessor(),
            predecessor_receipt_digest: unfinished.predecessor_receipt_digest().into(),
            trigger_digest: unfinished.digest().into(),
            progress_digest: unfinished.progress_digest().into(),
            failure_operation: unfinished.failure_operation(),
            review_rounds_consumed: unfinished.review_rounds_consumed(),
        };
        let cause = ReviewRoundRunCause::PairedRepair(link);
        let next = schedule(&f, &paired, &cause).unwrap();
        assert_eq!(next.attempt, ordinal + 1);
        assert_eq!(
            next.max_attempts, 3,
            "review rounds never renew implementation attempts"
        );
        // Equality with the last review round must still permit its paired fix.
        if ordinal == limit {
            assert_eq!(paired.reference().ordinal, paired.reference().round_limit);
        }
        let (repair, repair_run, _repair_permit) = admit(
            &f,
            &paired,
            cause,
            &format!("round-repair-{ordinal}"),
            false,
        )
        .await;
        assert_eq!(repair_run.attempt, ordinal + 1);
        let file = repair
            .contract
            .isolation()
            .plan()
            .repository
            .join("tests/main.txt");
        fs::write(
            file,
            format!("scoped repair for actual Findings {ordinal}\n"),
        )
        .unwrap();
        let (files, outcome, repaired) = complete_admitted(
            &f.built.runtime.control,
            &repair,
            json!({"schema_version":1,"summary":"scoped finding fixed","fulfilled_criteria":["required behavior"],"disposition":{"kind":"changed"}}),
        );
        retained_claim_files.push(files);
        refresh(&f).await;
        let WorkerAuthority::Authorized(current_gate) =
            f.service.current_delivery_worker_authority(f.gate).unwrap()
        else {
            panic!("authorized repair only")
        };
        let claim = Arc::new(
            f.workspace
                .coordinator
                .accept_child_completion(&current_gate, outcome, repaired.clone())
                .unwrap(),
        );
        scope.phase = DeliveryPhase::SourceSealing;
        let sealing = begin_phase(&f, scope.clone()).await;
        let changed = maintain_phase_observations(&f.built.runtime, sealing.coordinator.seal_child_source(&sealing.admission, &claim, &repair.execution, repair.contract.isolation(), crate::git::SourceCommitMetadata {
            identity: crate::git::CommitIdentity { name:"Disposable Fixture".into(), email:"fixture@example.invalid".into() },
            timestamp: chrono::DateTime::from_timestamp(1_767_225_610+i64::from(ordinal),0).unwrap(),
            message:format!("test: seal round {ordinal} repair\n\nSigned-off-by: Disposable Fixture <fixture@example.invalid>\n"),
        })).await.unwrap();
        assert_ne!(
            changed.revision().tested_tree,
            current_source
                .as_ref()
                .unwrap_or(f.source_proof)
                .revision()
                .tested_tree
        );
        f.service
            .retire_delivery_phase(
                &sealing,
                json!({"actual_changed_source":changed.revision()}),
            )
            .unwrap();
        scope.phase = DeliveryPhase::LocalValidation;
        scope.revision = changed.revision().clone();
        let validation = begin_phase(&f, scope.clone()).await;
        let denied = validation
            .coordinator
            .verify_local_checks(&validation.admission, &f.workspace.workflow, &[])
            .err()
            .unwrap();
        assert_eq!(denied.code, "validation_gate");
        f.service.retire_delivery_phase(&validation, json!({"blocked":"no qualified local execution; downstream assignment fixture is separate"})).unwrap();

        // Separate downstream component, as in the existing Findings fixture:
        // no LocalPassed/PR production capability is created or gate bypassed.
        f.wire.lock().unwrap().github_transport =
            Some(Arc::new(wire::ReviewWire::new(changed.revision().clone())));
        scope.phase = DeliveryPhase::ReviewPublication;
        let downstream = begin_phase(&f, scope).await;
        let (review, assignment) = downstream
            .coordinator
            .fixture_upstream_changed_review(&downstream.admission, &claim, &changed, &repaired)
            .unwrap();
        let advanced = Arc::new(
            maintain_phase_observations(
                &f.built.runtime,
                downstream.coordinator.source_advanced_retried_review(
                    SourceAdvancedRequest {
                        source: RepairSource {
                            paired_completion: None,
                            admission: &downstream.admission,
                            original: claim.clone(),
                            source: &changed,
                            completion: repaired.clone(),
                        },
                        assignment: &assignment,
                        predecessor: &current_outcome,
                        completion: current_completion.clone(),
                    },
                    &unfinished,
                    &[],
                ),
            )
            .await
            .unwrap(),
        );
        assert!(advanced.requires_review_round());
        assert_omitted_round(&f, &assignment, None, Some(advanced.clone()), None).await;
        let omitted = f
            .built
            .runtime
            .next_attempt_for_source_advanced(&advanced)
            .unwrap_err();
        assert_eq!(omitted.code, "worker_binding");
        assert_eq!(
            omitted.message,
            "actual common review lineage requires its opaque round plan"
        );
        let next = maintain_phase_observations(
            &f.built.runtime,
            downstream.coordinator.after_repair_review_round(
                ReviewRoundSource {
                    paired_completion: None,
                    admission: &downstream.admission,
                    claim: claim.clone(),
                    source: &changed,
                    completion: repaired.clone(),
                    assignment: &review,
                    validated_retry: None,
                },
                paired.clone(),
                advanced.clone(),
                &config,
                &[],
            ),
        )
        .await;
        if ordinal == limit {
            let denied = match next {
                Ok(_) => panic!("new review cannot exceed allocated round bound"),
                Err(error) => error,
            };
            assert_eq!(denied.code, "review_rounds_exhausted");
            assert_eq!(
                denied.message,
                "no further review round remains in this exact budget"
            );
            f.service
                .retire_delivery_phase(&downstream, json!({"round_bound":limit,"new_review":false}))
                .unwrap();
            if limit == 1 {
                Box::pin(pair::continued_pair_component(pair::PairFixture {
                    f: &f,
                    paired,
                    unfinished,
                    claim: &claim,
                    changed: &changed,
                    repaired: &repaired,
                    previous_validation: &prior_validation,
                    previous_validation_completion: prior_validation_completion.clone(),
                }))
                .await;
            }
            break;
        }
        let next = Arc::new(next.unwrap());
        assert_eq!(next.reference().ordinal, ordinal + 1);
        assert_ne!(next.reference().round_id, current_plan.reference().round_id);
        assert_eq!(
            next.reference().review_phase,
            current_plan.reference().review_phase
        );
        f.service
            .retire_delivery_phase(
                &downstream,
                json!({"new_round":next.digest(),"synthetic_upstream_assignment":true}),
            )
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        let cause = ReviewRoundRunCause::AfterRepair(crate::control::SourceAdvancedReference {
            predecessor: advanced.predecessor(),
            predecessor_receipt_digest: advanced.predecessor_receipt_digest().into(),
            source_operation: advanced.source_operation(),
            source_receipt_digest: advanced.source_receipt_digest().into(),
            trigger_digest: advanced.digest().into(),
            progress_digest: advanced.progress_digest().into(),
            review_rounds_consumed: advanced.review_rounds_consumed(),
        });
        let (worker, admitted, _review_permit) = admit(
            &f,
            &next,
            cause,
            &format!("round-findings-{}", ordinal + 1),
            false,
        )
        .await;
        assert_eq!(
            admitted.attempt, 3,
            "transient consumed one attempt, no extra review round"
        );
        let (files, outcome, completion) =
            complete_admitted(&f.built.runtime.control, &worker, findings(ordinal + 1));
        retained_claim_files.push(files);
        current_plan = next;
        current_review = review;
        current_source = Some(changed);
        current_claim = claim;
        current_implementation = repaired;
        current_outcome = outcome;
        current_completion = completion;
    }
    assert!(
        f.built
            .runtime
            .retained_completion(f.original_completion.operation_id)
            .unwrap()
            .is_none()
    );
    assert_eq!(retained_claim_files.len(), if limit == 1 { 2 } else { 4 });
}

async fn assert_omitted_round(
    f: &ReviewFixture<'_>,
    assignment: &crate::delivery::DeliveryWorkerAssignment,
    unfinished: Option<Arc<crate::delivery::UnfinishedChildWork>>,
    source_advanced: Option<Arc<crate::delivery::VerifiedSourceAdvanced>>,
    round: Option<Arc<VerifiedReviewRoundPlan>>,
) {
    refresh(f).await;
    let phase = unfinished
        .as_ref()
        .map(|proof| proof.phase())
        .or_else(|| source_advanced.as_ref().map(|proof| proof.phase()))
        .unwrap()
        .clone();
    let before = canonical_digest(1, &f.built.runtime.control.operations().unwrap()).unwrap();
    let mut request = crate::runtime_workers::LaunchRequest {
        lease: f.lease.clone(),
        operation_id: Uuid::new_v4(),
        authority: f
            .service
            .current_delivery_worker_authority(assignment.authority())
            .unwrap(),
        phase,
        source: None,
        delivery: Some(assignment.clone()),
        unfinished,
        source_advanced,
        retried_unfinished: None,
        validation_retry: None,
        review_retry: None,
        review_round: None,
        max_seconds: 60,
        max_turns: 1,
    };
    let denied = request.validate_binding().unwrap_err();
    assert_eq!(denied.code, "worker_binding");
    assert_eq!(
        denied.message,
        "common review lineage cannot omit its opaque round plan"
    );
    if let Some(round) = round {
        request.review_round = Some(round);
        request.validate_binding().unwrap();
    }
    assert_eq!(
        canonical_digest(1, &f.built.runtime.control.operations().unwrap()).unwrap(),
        before
    );
}

fn findings(ordinal: u32) -> Value {
    json!({"schema_version":1,"verdict":"findings","unresolved_findings":[format!("Required scoped behavior {ordinal} remains missing in tests/main.txt")],"supporting_links":[]})
}
async fn refresh(f: &ReviewFixture<'_>) {
    f.built.runtime.observations.refresh().await.unwrap();
}
async fn begin_phase(f: &ReviewFixture<'_>, mut scope: DeliveryScope) -> ActivePhase {
    refresh(f).await;
    // Refreshing the cache does not refresh the original immutable fixture gate.
    // New phases bind the actual current native observation, as production does.
    let WorkerAuthority::Authorized(gate) =
        f.service.current_delivery_worker_authority(f.gate).unwrap()
    else {
        panic!("authorized round phase only")
    };
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
fn schedule(
    f: &ReviewFixture<'_>,
    proof: &VerifiedReviewRoundPlan,
    cause: &ReviewRoundRunCause,
) -> Result<crate::runtime_schedule::RunSchedule> {
    schedule_at(f, proof, cause, Utc::now())
}
fn schedule_at(
    f: &ReviewFixture<'_>,
    proof: &VerifiedReviewRoundPlan,
    cause: &ReviewRoundRunCause,
    now: chrono::DateTime<Utc>,
) -> Result<crate::runtime_schedule::RunSchedule> {
    crate::runtime_schedule::plan_review_round(crate::runtime_schedule::RoundScheduleInput {
        runtime: &f.built.runtime.configuration.snapshot()?.runtime,
        phase: proof.subject_phase(),
        workspace: proof.authority().workspace_id(),
        receipts: &f.built.runtime.control.operations()?,
        grants: &[],
        proof,
        cause,
        review_retry: None,
        now,
    })
}

async fn admit(
    f: &ReviewFixture<'_>,
    proof: &Arc<VerifiedReviewRoundPlan>,
    cause: ReviewRoundRunCause,
    name: &str,
    runtime_barrier: bool,
) -> (WorkerFixture, RunReference, crate::control::Permit) {
    refresh(f).await;
    let schedule = schedule(f, proof, &cause).unwrap();
    let host = &f.built.runtime.control;
    let operation = Uuid::new_v4();
    let prep_id = crate::control::preparation_operation(operation).unwrap();
    let prep = host.admit_round_preparation_observed(f.lease, PreparationAdmission {
        request_id: prep_id, work_key: proof.subject_phase().work.clone(), scope_digest: canonical_digest(1,&json!({"fixture_preparation":operation,"round":proof.digest(),"assignment":proof.assignment().digest().unwrap()})).unwrap(),
        reference: PreparationReference { worker_operation: operation, workspace: proof.authority().workspace_id(), account: f.account.into(), phase: proof.subject_phase().clone() },
    }, proof.reference().clone(), |_| { proof.verify_current(&host.operations()?, &[])?; (f.service.current_gate())(proof.authority()) }).unwrap();
    // Actual disposable clone+contract assembly is performed while this Control
    // prep owns the work. Native process containment remains unqualified.
    let source = proof.assignment().source().unwrap();
    let current = f
        .service
        .current_delivery_worker_authority(proof.authority())
        .unwrap();
    let WorkerAuthority::Authorized(gate) = current else {
        panic!("authorized round only")
    };
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
        role: proof.assignment().role(),
        phase: &proof.subject_phase().phase,
        context: Value::Null,
        assignment: Some(proof.assignment()),
        account: f.account,
    });
    host.retire_local(prep.operation_id, json!({"phase":"worker_preparation","result":"prepared","fixture":"actual clone/assembly; no native process qualification"})).unwrap();
    let preparation = host.operation(prep_id).unwrap();
    assert!(preparation.expected.contains_key("review_round"));
    assert!(
        !preparation
            .expected
            .contains_key("review_round_preparation")
    );
    let reference = RunReference {
        workspace: proof.authority().workspace_id(),
        account: f.account.into(),
        normalized_state: Some("prepared".into()),
        phase: proof.subject_phase().clone(),
        profile_digest: canonical_digest(1, &worker.contract.instructions().manifest.profile)
            .unwrap(),
        instruction_digest: worker.contract.instructions().manifest_digest.clone(),
        max_attempts: schedule.max_attempts,
        max_turns: schedule.max_turns,
        max_review_rounds: schedule.max_review_rounds,
        attempt: schedule.attempt,
    };
    let input = || RunAdmission {
        request_id: operation,
        work_key: reference.phase.work.clone(),
        scope_digest: worker.contract.scope_digest().into(),
        reference: reference.clone(),
    };
    if runtime_barrier {
        refresh(f).await;
        let request = crate::runtime_workers::LaunchRequest {
            lease: f.lease.clone(),
            operation_id: operation,
            authority: f
                .service
                .current_delivery_worker_authority(proof.authority())
                .unwrap(),
            phase: proof.subject_phase().clone(),
            source: None,
            delivery: Some(proof.assignment().clone()),
            unfinished: None,
            source_advanced: None,
            retried_unfinished: None,
            validation_retry: None,
            review_retry: None,
            review_round: Some(proof.clone()),
            max_seconds: 60,
            max_turns: schedule.max_turns,
        };
        let before = canonical_digest(1, &host.operations().unwrap()).unwrap();
        let denied = f
            .built
            .runtime
            .admit_review_round_worker(input(), &worker.contract, &request, &preparation)
            .unwrap_err();
        assert_eq!(denied.code, "review_round_lineage");
        assert_eq!(
            denied.message,
            "actual same-run joined capability is unavailable"
        );
        assert_eq!(
            canonical_digest(1, &host.operations().unwrap()).unwrap(),
            before
        );
        assert!(host.operation(operation).is_err());
    }
    let provenance = ReviewRoundRunProvenance {
        round: proof.reference().clone(),
        preparation: ReviewReceiptReference {
            operation: preparation.operation_id,
            receipt_digest: canonical_digest(1, &preparation).unwrap(),
        },
        cause,
    };
    // Positive Control component only. Its opaque current callback checks all
    // available real source/native/receipt facts; it never claims runtime join.
    let before = host.operations().unwrap().len();
    if proof.subject_phase().phase == "implementation" {
        let mut widened = input();
        widened.reference.max_attempts += 1;
        let denied = host
            .admit_round_run_observed(f.lease, widened, provenance.clone(), |_| Ok(()))
            .unwrap_err();
        assert_eq!(denied.code, "attempts_exhausted");
        assert_eq!(host.operations().unwrap().len(), before);
    }
    let permit = host
        .admit_round_run_observed(f.lease, input(), provenance, |_| {
            proof.verify_current(&host.operations()?, &[])?;
            proof.verify_contract(&worker.contract)?;
            (f.service.current_gate())(proof.authority())
        })
        .unwrap();
    let actual = host.operation(operation).unwrap();
    ReviewReceiptReference::preparation_for(&actual)
        .unwrap()
        .matches_observation(&preparation)
        .unwrap();
    assert_eq!(
        crate::control::ReviewRoundReference::from_receipt(&actual)
            .unwrap()
            .as_ref(),
        Some(proof.reference())
    );
    (worker, reference, permit)
}

/// Real round/source/Control/grant composition. The provider terminal is a
/// declared duplex observation; no native joined completion is installed.
pub(super) async fn marked_retry(f: ReviewFixture<'_>) {
    let config = f.built.runtime.configuration.snapshot().unwrap();
    let phase = begin_phase(&f, f.scope.clone()).await;
    let initial = Arc::new(
        maintain_phase_observations(
            &f.built.runtime,
            phase.coordinator.initial_review_round(
                ReviewRoundSource {
                    paired_completion: None,
                    admission: &phase.admission,
                    claim: f.original_claim.clone(),
                    source: f.source_proof,
                    completion: f.original_completion.clone(),
                    assignment: f.review,
                    validated_retry: None,
                },
                &config,
                &[],
            ),
        )
        .await
        .unwrap(),
    );
    f.service
        .retire_delivery_phase(&phase, json!({"round_proposal":initial.digest()}))
        .unwrap();
    let (reviewer, run, permit) = admit(
        &f,
        &initial,
        ReviewRoundRunCause::Review,
        "marked-grant-predecessor",
        false,
    )
    .await;
    let protocol = Arc::new(protocol::RoundProtocol::new(
        f.built.runtime.clone(),
        permit,
        initial.authority().clone(),
        f.service.current_gate(),
    ));
    let failed = failed_admitted_review_with_gate(
        &f.built.runtime.control,
        &reviewer,
        run.max_turns,
        None,
        Some("serverOverloaded"),
        Some(protocol.clone()),
    )
    .await;
    protocol.assert_closed_review_effect(failed.operation_id);
    let predecessor = f
        .built
        .runtime
        .control
        .operation(failed.operation_id)
        .unwrap();
    assert_eq!(
        crate::control::ReviewRoundReference::from_receipt(&predecessor)
            .unwrap()
            .as_ref(),
        Some(initial.reference())
    );
    refresh(&f).await;
    let grant = retry_fixture::publish_failed_review(f.built.runtime.clone()).await;
    refresh(&f).await;
    let grants = crate::runtime_workflow::current_retry_grants(
        &config,
        &f.built.runtime.observations.admission_snapshot().unwrap(),
        f.gate.work(),
        &f.built.runtime.control.operations().unwrap(),
        &f.built.runtime.services.key,
    )
    .unwrap();
    let phase = begin_phase(&f, f.scope.clone()).await;
    let retry = Arc::new(
        maintain_phase_observations(
            &f.built.runtime,
            phase.coordinator.retry_failed_review(ReviewRetryRequest {
                admission: &phase.admission,
                assignment: initial.assignment(),
                failed,
                grant: grant.clone(),
                current_grants: &grants,
            }),
        )
        .await
        .unwrap(),
    );
    assert!(retry.requires_review_round());
    assert_eq!(retry.next_attempt().unwrap(), 1);
    let receipts = f.built.runtime.control.operations().unwrap();
    assert!(!receipts.iter().any(|receipt| {
        RunReference::from_receipt(receipt)
            .unwrap()
            .is_some_and(|run| run.phase == *retry.phase())
    }));
    let now = predecessor.updated_at + Duration::seconds(60);
    assert_eq!(
        crate::runtime_schedule::plan_review_retry(
            &config.runtime,
            retry.phase(),
            f.gate.workspace_id(),
            &receipts,
            &retry,
            now,
        )
        .unwrap_err()
        .code,
        "review_round_unavailable"
    );
    let mut request = crate::runtime_workers::LaunchRequest {
        lease: f.lease.clone(),
        operation_id: Uuid::new_v4(),
        authority: f
            .service
            .current_delivery_worker_authority(retry.authority())
            .unwrap(),
        phase: retry.phase().clone(),
        source: None,
        delivery: Some(retry.assignment().clone()),
        unfinished: None,
        source_advanced: None,
        retried_unfinished: None,
        review_retry: Some(retry.clone()),
        validation_retry: None,
        review_round: None,
        max_seconds: 60,
        max_turns: grant.new_budget().max_turns,
    };
    assert_eq!(
        request.validate_binding().unwrap_err().code,
        "worker_binding"
    );
    let granted = Arc::new(
        maintain_phase_observations(
            &f.built.runtime,
            phase.coordinator.granted_review_round(
                ReviewRoundSource {
                    paired_completion: None,
                    admission: &phase.admission,
                    claim: f.original_claim.clone(),
                    source: f.source_proof,
                    completion: f.original_completion.clone(),
                    assignment: f.review,
                    validated_retry: None,
                },
                &config,
                retry.clone(),
                &grants,
            ),
        )
        .await
        .unwrap(),
    );
    request.review_round = Some(granted.clone());
    request.authority = f
        .service
        .current_delivery_worker_authority(granted.authority())
        .unwrap();
    request.validate_binding().unwrap();
    let schedule =
        crate::runtime_schedule::plan_review_round(crate::runtime_schedule::RoundScheduleInput {
            runtime: &config.runtime,
            phase: retry.phase(),
            workspace: f.gate.workspace_id(),
            receipts: &f.built.runtime.control.operations().unwrap(),
            grants: &grants,
            proof: &granted,
            cause: &ReviewRoundRunCause::Review,
            review_retry: Some(&retry),
            now,
        })
        .unwrap();
    assert_eq!(schedule.attempt, 1);
    assert_eq!(granted.reference().review_phase, grant.phase_key());
    f.service
        .retire_delivery_phase(&phase, json!({"granted_round":granted.digest()}))
        .unwrap();
    f.built.runtime.retain_retry_grant(grant).unwrap();
    assert_eq!(
        f.built
            .runtime
            .next_attempt_for_review_retry(&retry)
            .unwrap_err()
            .code,
        "worker_binding"
    );
    assert_eq!(
        f.built
            .runtime
            .next_attempt_for_review_round(&request)
            .unwrap_err()
            .code,
        "reconciliation_required"
    );
}
