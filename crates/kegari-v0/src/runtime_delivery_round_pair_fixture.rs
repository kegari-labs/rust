//! Same-pair component boundary. Source/assignment/Control/protocol receipts are
//! actual. Completed protocol claims are synthetic, with no qualified failed
//! check or native joined completion. The raw continued marker below is data
//! for Control verification only, never a VerifiedReviewRoundPlan constructor.
use super::*;
use crate::control::{
    Receipt, RepairValidationReference, ReviewRoundReference, RunContinuationReference,
    SourceAdvancedReference,
};
use crate::runtime::VerifiedAuthorizedOutcome;
use crate::worker_run::RunCompletion;
use kegari_types::run::RunReference;

pub(super) struct PairFixture<'a, 'b> {
    pub f: &'a ReviewFixture<'b>,
    pub paired: Arc<VerifiedReviewRoundPlan>,
    pub unfinished: Arc<crate::delivery::UnfinishedChildWork>,
    pub claim: &'a Arc<CompletedChildClaim>,
    pub changed: &'a SealedChildSource,
    pub repaired: &'a Arc<RunCompletion>,
    pub previous_validation: &'a VerifiedAuthorizedOutcome,
    pub previous_validation_completion: Arc<RunCompletion>,
}
pub(super) async fn continued_pair_component(input: PairFixture<'_, '_>) {
    let PairFixture {
        f,
        paired,
        unfinished,
        claim,
        changed,
        repaired,
        previous_validation,
        previous_validation_completion,
    } = input;
    assert_eq!(
        (paired.reference().ordinal, paired.reference().round_limit),
        (1, 1)
    );
    let host = &f.built.runtime.control;
    // Actual existing paired source/Control closure remains accepted. The
    // completion is the fixture's explicitly synthetic protocol observation;
    // no native joined/resume capability is manufactured here.
    let repaired_receipt = host.operation(repaired.operation_id).unwrap();
    let records = host.operations().unwrap();
    let before = canonical_digest(1, &records).unwrap();
    paired
        .verify_completed_repair_result(&records, &[], &repaired_receipt, None)
        .unwrap();
    // A raw resumed marker cannot select the retained-capability branch or
    // silently fall back to the ordinary paired preparation/attempt decoder.
    let mut missing_cap = repaired_receipt.clone();
    missing_cap
        .expected
        .insert("review_round_resume".into(), "{}".into());
    let denied = paired
        .verify_completed_repair_result(&records, &[], &missing_cap, None)
        .unwrap_err();
    assert_eq!(denied.code, "reconciliation_required");
    assert_eq!(
        denied.message,
        "resumed repair requires its actual retained completed capability"
    );
    let mut changed_receipt = repaired_receipt.clone();
    changed_receipt.observed_result = Some(json!({"different":"whole retained terminal"}));
    assert!(
        paired
            .verify_completed_repair_result(&records, &[], &changed_receipt, None)
            .is_err()
    );
    assert_eq!(
        canonical_digest(1, &host.operations().unwrap()).unwrap(),
        before
    );

    let mut scope = f.scope.clone();
    scope.phase = DeliveryPhase::LocalValidation;
    scope.revision = changed.revision().clone();
    let phase = begin_phase(f, scope.clone()).await;
    let assignment = phase
        .coordinator
        .child_validation_assignment(
            &phase.admission,
            claim,
            changed,
            repaired,
            json!({"task":"actual changed-source validation; no qualified check claimed"}),
        )
        .unwrap();
    let advanced = Arc::new(
        maintain_phase_observations(
            &f.built.runtime,
            phase.coordinator.source_advanced_paired_validation(
                SourceAdvancedRequest {
                    source: RepairSource {
                        paired_completion: None,
                        admission: &phase.admission,
                        original: claim.clone(),
                        source: changed,
                        completion: repaired.clone(),
                    },
                    assignment: &assignment,
                    predecessor: previous_validation,
                    completion: previous_validation_completion,
                },
                &unfinished,
                &[],
            ),
        )
        .await
        .unwrap(),
    );
    assert_eq!(advanced.phase().phase, "child_local_validation");
    assert!(!advanced.requires_review_round());
    f.service
        .retire_delivery_phase(
            &phase,
            json!({"paired_validation_source":advanced.digest()}),
        )
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    refresh(f).await;
    let validation_link = SourceAdvancedReference {
        predecessor: advanced.predecessor(),
        predecessor_receipt_digest: advanced.predecessor_receipt_digest().into(),
        source_operation: advanced.source_operation(),
        source_receipt_digest: advanced.source_receipt_digest().into(),
        trigger_digest: advanced.digest().into(),
        progress_digest: advanced.progress_digest().into(),
        review_rounds_consumed: advanced.review_rounds_consumed(),
    };
    let config = f.built.runtime.configuration.snapshot().unwrap();
    let schedule = crate::runtime_schedule::plan_source_advanced(
        &config.runtime,
        advanced.phase(),
        advanced.authority().workspace_id(),
        &host.operations().unwrap(),
        &validation_link,
        Utc::now(),
    )
    .unwrap();
    assert_eq!(schedule.attempt, 2);
    let validation_id = Uuid::new_v4();
    let preparation = PreparationReference {
        worker_operation: validation_id,
        workspace: advanced.authority().workspace_id(),
        account: f.account.into(),
        phase: advanced.phase().clone(),
    };
    let prep = host
        .admit_preparation_observed(
            f.lease,
            PreparationAdmission {
                request_id: preparation.operation_id().unwrap(),
                work_key: preparation.phase.work.clone(),
                scope_digest: canonical_digest(
                    1,
                    &json!({"actual_validation_assignment":assignment.digest().unwrap()}),
                )
                .unwrap(),
                reference: preparation,
            },
            |_| {
                advanced.verify_current(&host.operations()?, &[])?;
                (f.service.current_gate())(advanced.authority())
            },
        )
        .unwrap();
    let WorkerAuthority::Authorized(gate) = f
        .service
        .current_delivery_worker_authority(advanced.authority())
        .unwrap()
    else {
        panic!("authorized fixture validation")
    };
    let source = assignment.source().unwrap();
    let validator = worker(WorkerInput {
        root: &f.root.join("same-pair-validation"),
        source: source.root(),
        binary: f.binary,
        git: f.tool.clone(),
        gate: &gate,
        workflow: &f.workspace.workflow,
        revision: source.revision(),
        repository: f.repository,
        runtime: f.lease.runtime_id,
        operation: validation_id,
        role: AgentRole::Validation,
        phase: "child_local_validation",
        context: Value::Null,
        assignment: Some(&assignment),
        account: f.account,
    });
    host.retire_local(prep.operation_id,json!({"phase":"worker_preparation","result":"prepared","fixture":"actual clone and contract; no native confinement"})).unwrap();
    let validation_run = RunReference {
        workspace: advanced.authority().workspace_id(),
        account: f.account.into(),
        normalized_state: Some("prepared".into()),
        phase: advanced.phase().clone(),
        profile_digest: canonical_digest(1, &validator.contract.instructions().manifest.profile)
            .unwrap(),
        instruction_digest: validator.contract.instructions().manifest_digest.clone(),
        max_attempts: schedule.max_attempts,
        max_turns: schedule.max_turns,
        max_review_rounds: schedule.max_review_rounds,
        attempt: schedule.attempt,
    };
    let validation_permit = host
        .admit_run_source_advanced_observed(
            f.lease,
            run_admission(&validator, validation_run.clone()),
            validation_link.clone(),
            |_| {
                advanced.verify_current(&host.operations()?, &[])?;
                advanced.verify_contract(&validator.contract)?;
                (f.service.current_gate())(advanced.authority())
            },
        )
        .unwrap();
    let validation_protocol = protocol::RoundProtocol::new(
        f.built.runtime.clone(),
        validation_permit,
        gate.clone(),
        f.service.current_gate(),
    );
    acknowledge_component_turn(&validation_protocol, "same-pair-validation");
    let (_validation_files, outcome, completion) = complete_admitted(
        host,
        &validator,
        json!({"kind":"validation","summary":"component protocol closed; no configured check executed"}),
    );
    validation_protocol.assert_closed_effect(validation_id, "turn/start");
    assert!(completion.checks.is_empty() && completion.failed_checks.is_empty());

    let phase = begin_phase(f, scope).await;
    let terminal = phase
        .coordinator
        .advance_child(
            &phase.admission,
            claim,
            &f.workspace.workflow,
            crate::delivery::ChildDeliveryAction::ValidateLocal {
                validation: &outcome,
                completion: &completion,
            },
        )
        .await
        .err()
        .unwrap();
    assert_eq!(terminal.code, "validation_gate");
    let unavailable = match maintain_phase_observations(
        &f.built.runtime,
        phase.coordinator.continue_paired_repair(
            crate::delivery::ValidationRepairRequest {
                source: RepairSource {
                    paired_completion: None,
                    admission: &phase.admission,
                    original: claim.clone(),
                    source: changed,
                    completion: repaired.clone(),
                },
                workflow: &f.workspace.workflow,
                outcome: &outcome,
                completion: completion.clone(),
            },
            paired.clone(),
            advanced.clone(),
            &config,
            &[],
        ),
    )
    .await
    {
        Ok(_) => panic!("a completed model summary cannot create a failed-check repair capability"),
        Err(error) => error,
    };
    assert_eq!(unavailable.code, "no_work_trigger");
    assert_eq!(
        unavailable.message,
        "no latest required local check remains failed"
    );
    // The pipeline maps this no-work case back to `terminal` before retirement.
    // This component exercises the real producer refusal, not a fabricated
    // ChildPipeline or a claim that its private orchestration path was run.
    assert!(crate::runtime_delivery::child::terminal_gate_failure(
        &terminal
    ));
    assert!(!crate::runtime_delivery::child::terminal_gate_failure(
        &unavailable
    ));
    f.service
        .retire_delivery_phase(
            &phase,
            json!({"terminal_validation_error":terminal,"unavailable_repair":unavailable}),
        )
        .unwrap();

    // Separate actual-Control data component. No opaque continued pair can be
    // produced without qualified failure evidence above. These references name
    // actual source/claims/receipts, but the obligation trigger is explicitly
    // synthetic. Production's opaque native callback remains unavailable.
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    refresh(f).await;
    let implementation = host.operation(repaired.operation_id).unwrap();
    let implementation_run = crate::control::run_reference_from_receipt(&implementation)
        .unwrap()
        .unwrap();
    let validation = host.operation(validation_id).unwrap();
    advanced
        .verify_completed_inventory(&host.operations().unwrap(), &[], validation_id)
        .unwrap();
    let inventory: BTreeMap<_, _> = host
        .operations()
        .unwrap()
        .into_iter()
        .filter(|receipt| {
            crate::control::run_reference_from_receipt(receipt)
                .unwrap()
                .is_some_and(|run| run.phase == validation_run.phase)
        })
        .map(|receipt| (receipt.operation_id, receipt))
        .collect();
    let mut current_source = paired.reference().source.clone();
    current_source.implementation = exact(&implementation);
    current_source.implementation_phase = implementation_run.phase.clone();
    current_source.claim_digest = claim.digest().into();
    current_source.seal = exact(&host.operation(changed.receipt().operation_id).unwrap());
    current_source.source_digest = source.digest().unwrap();
    current_source.commit = source.revision().commit.clone();
    current_source.tree = source.revision().tree.clone();
    let ReviewRoundCause::PairedRepair { findings } = paired.reference().cause.clone() else {
        panic!("actual first pair")
    };
    let mut continued = paired.reference().clone();
    continued.cause = ReviewRoundCause::RepairContinuation {
        findings: findings.clone(),
        previous_round_reference_digest: canonical_digest(1, paired.reference()).unwrap(),
        current_source: Box::new(current_source),
        validation: Box::new(RepairValidationReference {
            lineage: crate::control::RepairValidationLineage::Direct {},
            validation: exact(&validation),
            phase: validation_run.phase.clone(),
            source_advanced: validation_link,
            outcome_proposal: outcome.outcome().proposal_id,
            outcome_claim_digest: outcome.digest().into(),
            failure_inventory_digest: canonical_digest(1, &inventory).unwrap(),
        }),
    };
    assert_eq!(continued.round_id, paired.reference().round_id);
    assert_eq!(continued.source, paired.reference().source);
    assert_eq!(continued.repair_findings(), Some(&findings));
    assert_eq!((continued.ordinal, continued.round_limit), (1, 1));
    assert_eq!(
        continued.dispatch_source().implementation,
        exact(&implementation)
    );
    assert_ne!(
        continued.dispatch_source().implementation,
        continued.source.implementation
    );
    let cause = RunContinuationReference {
        predecessor:implementation.operation_id, predecessor_receipt_digest:canonical_digest(1,&implementation).unwrap(),
        failure_operation:validation.operation_id,
        trigger_digest:canonical_digest(1,&json!({"fixture_only_failed_obligation":true,"actual_validation":validation.operation_id,"qualified_failure":false})).unwrap(),
        progress_digest:canonical_digest(1,&json!({"actual_source":source.digest().unwrap(),"fixture_obligation":"component required check"})).unwrap(),
        review_rounds_consumed:unfinished.review_rounds_consumed(),
    };
    let second_id = Uuid::new_v4();
    let prep_ref = PreparationReference {
        worker_operation: second_id,
        workspace: continued.workspace,
        account: continued.account.clone(),
        phase: continued.subject_phase().clone(),
    };
    let prep_input = || PreparationAdmission {
        request_id: prep_ref.operation_id().unwrap(),
        work_key: prep_ref.phase.work.clone(),
        scope_digest: canonical_digest(
            1,
            &json!({"component_round":continued.round_id,"actual_source":source.digest().unwrap()}),
        )
        .unwrap(),
        reference: prep_ref.clone(),
    };
    let before = canonical_digest(1, &host.operations().unwrap()).unwrap();
    let mut reset = continued.clone();
    reset.cause = paired.reference().cause.clone();
    let denied = host
        .admit_round_preparation_observed(f.lease, prep_input(), reset, |_| Ok(()))
        .unwrap_err();
    assert_eq!(denied.code, "review_round_lineage");
    assert_eq!(
        canonical_digest(1, &host.operations().unwrap()).unwrap(),
        before
    );
    let prep = host
        .admit_round_preparation_observed(f.lease, prep_input(), continued.clone(), |_| {
            advanced.verify_completed_inventory(&host.operations()?, &[], validation_id)?;
            (f.service.current_gate())(advanced.authority())
        })
        .unwrap();
    let WorkerAuthority::Authorized(gate) = f
        .service
        .current_delivery_worker_authority(advanced.authority())
        .unwrap()
    else {
        panic!("authorized fixture repair")
    };
    let second = worker(WorkerInput {
        root: &f.root.join("same-pair-second-component"),
        source: source.root(),
        binary: f.binary,
        git: f.tool.clone(),
        gate: &gate,
        workflow: &f.workspace.workflow,
        revision: source.revision(),
        repository: f.repository,
        runtime: f.lease.runtime_id,
        operation: second_id,
        role: paired.assignment().role(),
        phase: "implementation",
        context: json!({"task":"Control-only continued pair; no opaque failed-check authority claimed","actual_source":source.digest().unwrap()}),
        assignment: None,
        account: f.account,
    });
    host.retire_local(prep.operation_id,json!({"phase":"worker_preparation","result":"prepared","fixture":"actual clone/assembly; data-only pair component"})).unwrap();
    let preparation = host.operation(prep.operation_id).unwrap();
    let mut second_run = implementation_run.clone();
    second_run.attempt += 1;
    second_run.instruction_digest = second.contract.instructions().manifest_digest.clone();
    second_run.normalized_state = Some("prepared".into());
    assert_eq!((second_run.attempt, second_run.max_attempts), (3, 3));
    let provenance = ReviewRoundRunProvenance {
        round: continued.clone(),
        preparation: exact(&preparation),
        cause: ReviewRoundRunCause::PairedRepair(cause),
    };
    let before = canonical_digest(1, &host.operations().unwrap()).unwrap();
    let mut widened = second_run.clone();
    widened.max_attempts += 1;
    let denied = host
        .admit_round_run_observed(
            f.lease,
            run_admission(&second, widened),
            provenance.clone(),
            |_| Ok(()),
        )
        .unwrap_err();
    assert_eq!(denied.code, "attempts_exhausted");
    assert_eq!(
        canonical_digest(1, &host.operations().unwrap()).unwrap(),
        before
    );
    let permit = host
        .admit_round_run_observed(
            f.lease,
            run_admission(&second, second_run.clone()),
            provenance,
            |_| {
                advanced.verify_completed_inventory(&host.operations()?, &[], validation_id)?;
                (f.service.current_gate())(advanced.authority())
            },
        )
        .unwrap();
    let second_protocol = protocol::RoundProtocol::new(
        f.built.runtime.clone(),
        permit,
        gate,
        f.service.current_gate(),
    );
    acknowledge_component_turn(&second_protocol, "same-pair-second-component");
    let (_second_files, _, second_completion) = complete_admitted(
        host,
        &second,
        json!({"schema_version":1,"summary":"component session closed without a qualified check","fulfilled_criteria":[],"disposition":{"kind":"no_change","reason":"Control-only component; not delivery credit"}}),
    );
    second_protocol.assert_closed_effect(second_id, "turn/start");
    let terminal = host.operation(second_id).unwrap();
    ReviewReceiptReference::preparation_for(&terminal)
        .unwrap()
        .matches_observation(&preparation)
        .unwrap();
    assert_eq!(
        ReviewRoundReference::from_receipt(&terminal).unwrap(),
        Some(continued.clone())
    );
    assert_eq!(
        canonical_digest(1, &host.operation(implementation.operation_id).unwrap()).unwrap(),
        canonical_digest(1, &implementation).unwrap()
    );
    assert_eq!(
        canonical_digest(1, &host.operation(findings.review.operation).unwrap()).unwrap(),
        findings.review.receipt_digest
    );
    let review_count = host
        .operations()
        .unwrap()
        .iter()
        .filter_map(|r| crate::control::run_reference_from_receipt(r).unwrap())
        .filter(|r| r.phase == continued.review_phase)
        .count();
    assert_eq!(
        review_count, 2,
        "one transient attempt + Findings; continuing its fix allocates no review"
    );
    assert!(
        f.built
            .runtime
            .retained_completion(second_id)
            .unwrap()
            .is_none()
    );
    assert!(second_completion.failed_checks.is_empty());
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    refresh(f).await;
    let replay_id = Uuid::new_v4();
    let replay_ref = PreparationReference {
        worker_operation: replay_id,
        ..prep_ref
    };
    let before = canonical_digest(1, &host.operations().unwrap()).unwrap();
    let replay = host
        .admit_round_preparation_observed(
            f.lease,
            PreparationAdmission {
                request_id: replay_ref.operation_id().unwrap(),
                work_key: replay_ref.phase.work.clone(),
                scope_digest: canonical_digest(1, &"replayed component pair").unwrap(),
                reference: replay_ref,
            },
            continued,
            |_| Ok(()),
        )
        .unwrap_err();
    assert_eq!(replay.code, "review_round_lineage");
    assert_eq!(
        canonical_digest(1, &host.operations().unwrap()).unwrap(),
        before
    );
}

fn run_admission(fixture: &WorkerFixture, reference: RunReference) -> RunAdmission {
    RunAdmission {
        request_id: fixture.contract.instructions().manifest.operation_id,
        work_key: reference.phase.work.clone(),
        scope_digest: fixture.contract.scope_digest().into(),
        reference,
    }
}
fn exact(receipt: &Receipt) -> ReviewReceiptReference {
    ReviewReceiptReference {
        operation: receipt.operation_id,
        receipt_digest: canonical_digest(1, receipt).unwrap(),
    }
}

/// Bounded local duplex exchange through the same effect dispatch/ACK boundary.
/// This is not the native provider run loop or a qualified terminal completion.
fn acknowledge_component_turn(gate: &protocol::RoundProtocol, thread: &str) {
    use kegari_codex::RequestGate;
    use std::io::{BufRead, Write};
    let (mut client, mut server) = std::os::unix::net::UnixStream::pair().unwrap();
    for stream in [&client, &server] {
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        stream
            .set_write_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
    }
    let frame = json!({"id":1,"method":"turn/start","params":{"threadId":thread,"input":[{"type":"text","text":"explicit fixture component"}]}});
    let bytes = format!("{frame}\n").into_bytes();
    gate.dispatch(1, "turn/start", &frame, &mut || {
        client
            .write_all(&bytes)
            .map_err(|e| Error::new("fixture_transport", e.to_string()))
    })
    .unwrap();
    let mut line = String::new();
    std::io::BufReader::new(server.try_clone().unwrap())
        .read_line(&mut line)
        .unwrap();
    assert_eq!(serde_json::from_str::<Value>(&line).unwrap(), frame);
    let response =
        json!({"id":1,"result":{"turn":{"id":"fixture-component-turn","status":"inProgress"}}});
    server
        .write_all(format!("{response}\n").as_bytes())
        .unwrap();
    line.clear();
    std::io::BufReader::new(client)
        .read_line(&mut line)
        .unwrap();
    let actual: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(actual, response);
    gate.observed(1, "turn/start", &Ok(actual["result"].clone()))
        .unwrap();
}
