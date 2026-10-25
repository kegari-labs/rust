//! Actual Control journal/serialization component tests. Terminal worker/source
//! observations and authority callbacks are explicitly synthetic. These tests
//! do not create delivery/source/provider/joined-runtime capabilities.
use super::*;
use serde_json::json;
use std::sync::{Arc, Barrier};

fn digest(label: &str) -> String {
    canonical_digest(1, &label).unwrap()
}
struct Fixture {
    _dir: tempfile::TempDir,
    host: Arc<ControlHost>,
    lease: Lease,
    implementation: RunReference,
    round: ReviewRoundReference,
}
fn observation() -> Value {
    json!({"outcome":"completed","local_exit_confirmed":true,"hook_error":null,
        "session":{"terminal":"completed","input_required":false,"intervention":null}})
}
fn admission(run: RunReference, id: Uuid) -> RunAdmission {
    RunAdmission {
        request_id: id,
        work_key: run.phase.work.clone(),
        scope_digest: digest("source scope"),
        reference: run,
    }
}
fn exact(receipt: &Receipt) -> ReviewReceiptReference {
    ReviewReceiptReference {
        operation: receipt.operation_id,
        receipt_digest: canonical_digest(1, receipt).unwrap(),
    }
}
fn setup() -> Fixture {
    setup_with_review_limit(1)
}
fn setup_with_review_limit(round_limit: u32) -> Fixture {
    setup_with_bounds(round_limit, 2)
}
fn setup_with_bounds(round_limit: u32, implementation_attempts: u32) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Uuid::new_v4();
    let host = Arc::new(
        ControlHost::setup(
            PrivateDir::create(&dir.path().canonicalize().unwrap().join("control")).unwrap(),
            runtime,
            Uuid::new_v4(),
        )
        .unwrap(),
    );
    host.lifecycle(
        MutationRequest {
            request_id: Uuid::new_v4(),
            runtime_id: runtime,
            actor: Actor {
                identity: "component operator".into(),
                roles: ["lifecycle".into()].into(),
                principal: None,
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
    let lease = Lease {
        runtime_id: runtime,
        owner_generation: 1,
        owner_id: Uuid::new_v4(),
    };
    host.attach_reconciled(lease.clone()).unwrap();
    let implementation = RunReference {
        workspace: Uuid::new_v4(),
        account: "component account".into(),
        normalized_state: None,
        phase: PhaseKey {
            work: "component child".into(),
            generation: digest("generation"),
            phase: "implementation".into(),
            retry_grant: None,
        },
        profile_digest: digest("profile"),
        instruction_digest: digest("instructions"),
        max_attempts: implementation_attempts,
        max_turns: 4,
        max_review_rounds: 1,
        attempt: 1,
    };
    let permit = host
        .admit_run(
            &lease,
            admission(implementation.clone(), Uuid::new_v4()),
            || Ok(()),
        )
        .unwrap();
    let source = host
        .retire_local(permit.operation_id, observation())
        .unwrap();
    let seal = source_seal(&host, &lease, &implementation.phase.work, "a", "b");
    let mut phase = implementation.phase.clone();
    phase.phase = "child_codex_review".into();
    let mut round = ReviewRoundReference {
        schema_version: 1,
        runtime,
        workspace: implementation.workspace,
        account: implementation.account.clone(),
        review_phase: phase,
        round_id: String::new(),
        ordinal: 1,
        round_limit,
        source: ReviewedSourceReference {
            implementation_phase: implementation.phase.clone(),
            implementation: exact(&source),
            claim_digest: digest("protected claim component reference"),
            seal: exact(&seal),
            source_digest: digest("source proof reference"),
            repository_hostname: "github.invalid".into(),
            repository_id: "42".into(),
            commit: "a".repeat(40),
            tree: "b".repeat(40),
        },
        cause: ReviewRoundCause::Initial {},
    };
    round.round_id = round.expected_round_id().unwrap();
    Fixture {
        _dir: dir,
        host,
        lease,
        implementation,
        round,
    }
}
fn source_seal(host: &ControlHost, lease: &Lease, work: &str, commit: &str, tree: &str) -> Receipt {
    let permit = host
        .admit(
            lease,
            Uuid::new_v4(),
            work.into(),
            digest("sealing phase"),
            || Ok(()),
        )
        .unwrap();
    let (effect, ()) = host
        .begin_effect(
            MutationRequest {
                request_id: Uuid::new_v4(),
                runtime_id: lease.runtime_id,
                actor: Actor {
                    identity: format!("owner:{}", lease.owner_id),
                    roles: BTreeSet::new(),
                    principal: None,
                },
                kind: "child_source_sealing".into(),
                canonical_targets: vec!["42".into()],
                expected: BTreeMap::new(),
                payload: json!({"fixture":"synthetic revision observation; no Git capability"}),
            },
            &permit,
            || Ok(()),
            |_| Ok(()),
        )
        .unwrap();
    let receipt = host
        .complete_effect(
            effect,
            &permit,
            || Ok(()),
            json!({"revision":{
        "repository":{"hostname":"github.invalid","id":"42"},"head":commit.repeat(40),
        "tested_commit":commit.repeat(40),"tested_tree":tree.repeat(40)}}),
        )
        .unwrap();
    host.retire_local(
        permit.operation_id,
        json!({"fixture":"finite local metadata complete"}),
    )
    .unwrap();
    receipt
}
fn prepare(f: &Fixture, round: &ReviewRoundReference, worker: Uuid) -> ReviewReceiptReference {
    let reference = PreparationReference {
        worker_operation: worker,
        workspace: round.workspace,
        account: round.account.clone(),
        phase: round.subject_phase().clone(),
    };
    let permit = f
        .host
        .admit_round_preparation_observed(
            &f.lease,
            PreparationAdmission {
                request_id: reference.operation_id().unwrap(),
                work_key: reference.phase.work.clone(),
                scope_digest: digest("preparation"),
                reference,
            },
            round.clone(),
            |_| Ok(()),
        )
        .unwrap();
    let receipt = f
        .host
        .retire_local(
            permit.operation_id,
            json!({"phase":"worker_preparation","result":"prepared"}),
        )
        .unwrap();
    exact(&receipt)
}
fn review_run(f: &Fixture, round: &ReviewRoundReference, attempt: u32) -> RunReference {
    RunReference {
        phase: round.review_phase.clone(),
        max_review_rounds: round.round_limit,
        max_attempts: 3,
        attempt,
        ..f.implementation.clone()
    }
}
fn complete_first(f: &Fixture) -> Receipt {
    let worker = Uuid::new_v4();
    let prep = prepare(f, &f.round, worker);
    assert!(
        f.host
            .operations()
            .unwrap()
            .iter()
            .filter_map(|r| RunReference::from_receipt(r).unwrap())
            .all(|run| run.phase != f.round.review_phase)
    );
    let permit = f
        .host
        .admit_round_run_observed(
            &f.lease,
            admission(review_run(f, &f.round, 1), worker),
            ReviewRoundRunProvenance {
                round: f.round.clone(),
                preparation: prep,
                cause: ReviewRoundRunCause::Review,
            },
            |_| Ok(()),
        )
        .unwrap();
    f.host
        .retire_local(permit.operation_id, observation())
        .unwrap()
}
fn paired(f: &Fixture, review: &Receipt) -> (ReviewRoundReference, RunContinuationReference) {
    let mut round = f.round.clone();
    round.cause = ReviewRoundCause::PairedRepair {
        findings: FindingsReference {
            review: exact(review),
            round_id: round.round_id.clone(),
            round_ordinal: round.ordinal,
            round_reference_digest: canonical_digest(1, &f.round).unwrap(),
            outcome_proposal: Uuid::new_v4(),
            outcome_claim_digest: digest("actual producer must validate Findings"),
            reviewed_source_digest: round.source.source_digest.clone(),
        },
    };
    let cause = RunContinuationReference {
        predecessor: round.source.implementation.operation,
        predecessor_receipt_digest: round.source.implementation.receipt_digest.clone(),
        failure_operation: review.operation_id,
        trigger_digest: digest("paired trigger"),
        progress_digest: digest("paired progress"),
        review_rounds_consumed: 1,
    };
    (round, cause)
}
fn spacing() {
    std::thread::sleep(std::time::Duration::from_millis(1050));
}

#[test]
fn actual_preparation_is_not_allocation_and_denied_final_callback_writes_no_run() {
    let f = setup();
    let worker = Uuid::new_v4();
    let prep = prepare(&f, &f.round, worker);
    let provenance = ReviewRoundRunProvenance {
        round: f.round.clone(),
        preparation: prep,
        cause: ReviewRoundRunCause::Review,
    };
    let before = f.host.operations().unwrap().len();
    let error = f
        .host
        .admit_round_run_observed(
            &f.lease,
            admission(review_run(&f, &f.round, 1), worker),
            provenance.clone(),
            |_| Err(Error::new("stale_result", "component fresh gate denies")),
        )
        .unwrap_err();
    assert_eq!(error.code, "stale_result");
    assert_eq!(f.host.operations().unwrap().len(), before);
    assert!(f.host.operation(worker).is_err());
    let permit = f
        .host
        .admit_round_run_observed(
            &f.lease,
            admission(review_run(&f, &f.round, 1), worker),
            provenance,
            |_| Ok(()),
        )
        .unwrap();
    let receipt = f.host.operation(worker).unwrap();
    assert_eq!(
        ReviewRoundReference::from_receipt(&receipt)
            .unwrap()
            .as_ref(),
        Some(&f.round)
    );
    assert!(receipt.expected.contains_key(PREPARATION_LINK));
    f.host
        .retire_local(permit.operation_id, observation())
        .unwrap();
}
#[test]
fn final_round_findings_can_repair_but_cannot_allocate_a_confirming_review() {
    let f = setup();
    let review = complete_first(&f);
    spacing();
    let (pair, cause) = paired(&f, &review);
    let worker = Uuid::new_v4();
    let prep = prepare(&f, &pair, worker);
    let mut run = f.implementation.clone();
    run.attempt = 2;
    let permit = f
        .host
        .admit_round_run_observed(
            &f.lease,
            admission(run, worker),
            ReviewRoundRunProvenance {
                round: pair.clone(),
                preparation: prep,
                cause: ReviewRoundRunCause::PairedRepair(cause),
            },
            |_| Ok(()),
        )
        .unwrap();
    let repair = f
        .host
        .retire_local(permit.operation_id, observation())
        .unwrap();
    let actual = RunReference::from_receipt(&repair).unwrap().unwrap();
    assert_eq!(
        (
            actual.attempt,
            actual.max_attempts,
            actual.max_turns,
            actual.max_review_rounds
        ),
        (2, 2, 4, 1)
    );
    let seal = source_seal(&f.host, &f.lease, &pair.review_phase.work, "c", "d");
    let mut next = pair.clone();
    next.ordinal = 2;
    next.source.implementation = exact(&repair);
    next.source.seal = exact(&seal);
    next.source.commit = "c".repeat(40);
    next.source.tree = "d".repeat(40);
    next.source.source_digest = digest("changed actual source proof required at producer");
    next.source.claim_digest = digest("changed protected claim required at producer");
    let ReviewRoundCause::PairedRepair { findings } = pair.cause.clone() else {
        unreachable!()
    };
    next.cause = ReviewRoundCause::AfterRepair {
        findings,
        repair: exact(&repair),
        paired_round_reference_digest: canonical_digest(1, &pair).unwrap(),
    };
    next.round_id = next.expected_round_id().unwrap();
    let before = f.host.operations().unwrap().len();
    let reference = PreparationReference {
        worker_operation: Uuid::new_v4(),
        workspace: next.workspace,
        account: next.account.clone(),
        phase: next.review_phase.clone(),
    };
    assert!(
        f.host
            .admit_round_preparation_observed(
                &f.lease,
                PreparationAdmission {
                    request_id: reference.operation_id().unwrap(),
                    work_key: reference.phase.work.clone(),
                    scope_digest: digest("prep"),
                    reference
                },
                next,
                |_| Ok(())
            )
            .is_err()
    );
    assert_eq!(f.host.operations().unwrap().len(), before);
}
#[test]
fn competing_actual_pair_admissions_allocate_only_one_session() {
    let f = setup();
    let review = complete_first(&f);
    spacing();
    let (pair, cause) = paired(&f, &review);
    let workers = [Uuid::new_v4(), Uuid::new_v4()];
    let preparations = workers.map(|id| prepare(&f, &pair, id));
    let barrier = Arc::new(Barrier::new(3));
    let mut tasks = Vec::new();
    for (worker, preparation) in workers.into_iter().zip(preparations) {
        let host = f.host.clone();
        let lease = f.lease.clone();
        let barrier = barrier.clone();
        let mut run = f.implementation.clone();
        run.attempt = 2;
        let round = pair.clone();
        let cause = cause.clone();
        tasks.push(std::thread::spawn(move || {
            barrier.wait();
            host.admit_round_run_observed(
                &lease,
                admission(run, worker),
                ReviewRoundRunProvenance {
                    round,
                    preparation,
                    cause: ReviewRoundRunCause::PairedRepair(cause),
                },
                |_| Ok(()),
            )
        }));
    }
    barrier.wait();
    let results: Vec<_> = tasks.into_iter().map(|t| t.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    let permit = results.into_iter().find_map(Result::ok).unwrap();
    f.host
        .retire_local(permit.operation_id, observation())
        .unwrap();
    assert_eq!(
        f.host
            .operations()
            .unwrap()
            .iter()
            .filter(|r| r.expected.contains_key("continuation"))
            .count(),
        1
    );
}
#[test]
fn deleted_native_grant_observation_cannot_resurrect_old_source_receipts() {
    let f = setup();
    let mut newer = f.implementation.clone();
    newer.phase.retry_grant = Some(Uuid::new_v4());
    // Actual Control component admission in another budget; no native grant or
    // delivery capability is fabricated. Empty final callback models the missing
    // grant observation and must not erase this durable implementation history.
    let permit = f
        .host
        .admit_run(&f.lease, admission(newer, Uuid::new_v4()), || Ok(()))
        .unwrap();
    f.host
        .retire_local(permit.operation_id, observation())
        .unwrap();
    let reference = PreparationReference {
        worker_operation: Uuid::new_v4(),
        workspace: f.round.workspace,
        account: f.round.account.clone(),
        phase: f.round.review_phase.clone(),
    };
    let id = reference.operation_id().unwrap();
    let before = f.host.operations().unwrap().len();
    let error = f
        .host
        .admit_round_preparation_observed(
            &f.lease,
            PreparationAdmission {
                request_id: id,
                work_key: reference.phase.work.clone(),
                scope_digest: digest("prep"),
                reference,
            },
            f.round.clone(),
            |_| Ok(()),
        )
        .unwrap_err();
    assert_eq!(error.code, "review_round_lineage");
    assert!(f.host.operation(id).is_err());
    assert_eq!(f.host.operations().unwrap().len(), before);
}
#[test]
fn changed_preparation_hash_and_unmarked_successor_cannot_bypass_round_path() {
    let f = setup();
    let worker = Uuid::new_v4();
    let mut prep = prepare(&f, &f.round, worker);
    prep.receipt_digest = digest("substituted preparation");
    assert!(
        f.host
            .admit_round_run_observed(
                &f.lease,
                admission(review_run(&f, &f.round, 1), worker),
                ReviewRoundRunProvenance {
                    round: f.round.clone(),
                    preparation: prep,
                    cause: ReviewRoundRunCause::Review
                },
                |_| Ok(())
            )
            .is_err()
    );
    assert!(f.host.operation(worker).is_err());
    let review = complete_first(&f);
    let error = f
        .host
        .admit_run(
            &f.lease,
            admission(review_run(&f, &f.round, 2), Uuid::new_v4()),
            || Ok(()),
        )
        .unwrap_err();
    assert_eq!(error.code, "review_round_required");
    assert_eq!(
        f.host.operation(review.operation_id).unwrap().expected["review_round"],
        encode(&f.round).unwrap()
    );
}

#[test]
fn changed_source_allocates_next_round_without_renewing_implementation_budget() {
    let f = setup_with_review_limit(2);
    let review = complete_first(&f);
    spacing();
    let (pair, cause) = paired(&f, &review);
    let worker = Uuid::new_v4();
    let prep = prepare(&f, &pair, worker);
    let mut implementation = f.implementation.clone();
    implementation.attempt = 2;
    let permit = f
        .host
        .admit_round_run_observed(
            &f.lease,
            admission(implementation, worker),
            ReviewRoundRunProvenance {
                round: pair.clone(),
                preparation: prep,
                cause: ReviewRoundRunCause::PairedRepair(cause),
            },
            |_| Ok(()),
        )
        .unwrap();
    let repair = f
        .host
        .retire_local(permit.operation_id, observation())
        .unwrap();
    let seal = source_seal(&f.host, &f.lease, &pair.review_phase.work, "c", "d");
    spacing();
    let ReviewRoundCause::PairedRepair { findings } = pair.cause.clone() else {
        unreachable!()
    };
    let mut next = pair.clone();
    next.ordinal = 2;
    next.source.implementation = exact(&repair);
    next.source.seal = exact(&seal);
    next.source.claim_digest = digest("second source protected claim reference");
    next.source.source_digest = digest("second sealed source reference");
    next.source.commit = "c".repeat(40);
    next.source.tree = "d".repeat(40);
    next.cause = ReviewRoundCause::AfterRepair {
        findings,
        repair: exact(&repair),
        paired_round_reference_digest: canonical_digest(1, &pair).unwrap(),
    };
    next.round_id = next.expected_round_id().unwrap();
    let cause = SourceAdvancedReference {
        predecessor: review.operation_id,
        predecessor_receipt_digest: canonical_digest(1, &review).unwrap(),
        source_operation: repair.operation_id,
        source_receipt_digest: canonical_digest(1, &repair).unwrap(),
        trigger_digest: digest("fresh exact-source trigger"),
        progress_digest: digest("fresh exact-source progress"),
        review_rounds_consumed: 1,
    };
    let worker = Uuid::new_v4();
    let prep = prepare(&f, &next, worker);
    let permit = f
        .host
        .admit_round_run_observed(
            &f.lease,
            admission(review_run(&f, &next, 2), worker),
            ReviewRoundRunProvenance {
                round: next.clone(),
                preparation: prep,
                cause: ReviewRoundRunCause::AfterRepair(cause),
            },
            |_| Ok(()),
        )
        .unwrap();
    let actual = f
        .host
        .retire_local(permit.operation_id, observation())
        .unwrap();
    let recorded = ReviewRoundReference::from_receipt(&actual)
        .unwrap()
        .unwrap();
    assert_eq!((recorded.ordinal, recorded.round_limit), (2, 2));
    assert_ne!(recorded.round_id, f.round.round_id);
    let review_run = RunReference::from_receipt(&actual).unwrap().unwrap();
    assert_eq!(
        (
            review_run.attempt,
            review_run.max_attempts,
            review_run.max_turns
        ),
        (2, 3, 4)
    );
    let repair_run = RunReference::from_receipt(&repair).unwrap().unwrap();
    assert_eq!(
        (
            repair_run.attempt,
            repair_run.max_attempts,
            repair_run.max_turns,
            repair_run.max_review_rounds
        ),
        (2, 2, 4, 1)
    );
    assert_eq!(
        canonical_digest(1, &f.host.operation(repair.operation_id).unwrap()).unwrap(),
        canonical_digest(1, &repair).unwrap()
    );
    let unmarked = PreparationReference {
        worker_operation: Uuid::new_v4(),
        workspace: next.workspace,
        account: next.account,
        phase: next.review_phase,
    };
    let id = unmarked.operation_id().unwrap();
    assert_eq!(
        f.host
            .admit_preparation(
                &f.lease,
                PreparationAdmission {
                    request_id: id,
                    work_key: unmarked.phase.work.clone(),
                    scope_digest: digest("unmarked successor preparation"),
                    reference: unmarked,
                },
                || Ok(())
            )
            .unwrap_err()
            .code,
        "review_round_required"
    );
    assert!(f.host.operation(id).is_err());
}

#[test]
fn paired_source_validation_keeps_exact_receipt_without_allocating_review_round() {
    let f = setup();
    let mut validation = f.implementation.clone();
    validation.phase.phase = "child_local_validation".into();
    let permit = f
        .host
        .admit_run(
            &f.lease,
            admission(validation.clone(), Uuid::new_v4()),
            || Ok(()),
        )
        .unwrap();
    let first_validation = f
        .host
        .retire_local(permit.operation_id, observation())
        .unwrap();
    let review = complete_first(&f);
    spacing();
    let (pair, cause) = paired(&f, &review);
    let worker = Uuid::new_v4();
    let preparation = prepare(&f, &pair, worker);
    let mut implementation = f.implementation.clone();
    implementation.attempt = 2;
    let permit = f
        .host
        .admit_round_run_observed(
            &f.lease,
            admission(implementation, worker),
            ReviewRoundRunProvenance {
                round: pair.clone(),
                preparation,
                cause: ReviewRoundRunCause::PairedRepair(cause),
            },
            |_| Ok(()),
        )
        .unwrap();
    let repair = f
        .host
        .retire_local(permit.operation_id, observation())
        .unwrap();
    spacing();
    validation.attempt = 2;
    let link = SourceAdvancedReference {
        predecessor: first_validation.operation_id,
        predecessor_receipt_digest: canonical_digest(1, &first_validation).unwrap(),
        source_operation: repair.operation_id,
        source_receipt_digest: canonical_digest(1, &repair).unwrap(),
        trigger_digest: digest("paired validation trigger"),
        progress_digest: digest("paired validation progress"),
        review_rounds_consumed: pair.ordinal,
    };
    let permit = f
        .host
        .admit_run_source_advanced_observed(
            &f.lease,
            admission(validation, Uuid::new_v4()),
            link.clone(),
            |_| Ok(()),
        )
        .unwrap();
    let validated = f
        .host
        .retire_local(permit.operation_id, observation())
        .unwrap();
    assert_eq!(
        SourceAdvancedReference::from_receipt(&validated).unwrap(),
        Some(link)
    );
    assert!(
        ReviewRoundReference::from_receipt(&validated)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        ReviewRoundReference::from_receipt(&f.host.operation(repair.operation_id).unwrap())
            .unwrap(),
        Some(pair)
    );
    let actual = RunReference::from_receipt(&validated).unwrap().unwrap();
    assert_eq!(
        (
            actual.attempt,
            actual.max_attempts,
            actual.max_turns,
            actual.max_review_rounds
        ),
        (2, 2, 4, 1)
    );
    let reviews = f
        .host
        .operations()
        .unwrap()
        .iter()
        .filter_map(|r| RunReference::from_receipt(r).unwrap())
        .filter(|r| r.phase.phase == "child_codex_review")
        .count();
    assert_eq!(reviews, 1);
}

#[test]
fn continued_final_round_pair_uses_latest_validation_and_original_attempt_cap() {
    let f = setup_with_bounds(1, 3);
    let mut validation_run = f.implementation.clone();
    validation_run.phase.phase = "child_local_validation".into();
    let validation = f
        .host
        .admit_run(
            &f.lease,
            admission(validation_run.clone(), Uuid::new_v4()),
            || Ok(()),
        )
        .unwrap();
    let old_validation = f
        .host
        .retire_local(validation.operation_id, observation())
        .unwrap();
    let review = complete_first(&f);
    spacing();
    let (pair, original_cause) = paired(&f, &review);
    let worker = Uuid::new_v4();
    let prep = prepare(&f, &pair, worker);
    let mut implementation = f.implementation.clone();
    implementation.attempt = 2;
    let permit = f
        .host
        .admit_round_run_observed(
            &f.lease,
            admission(implementation.clone(), worker),
            ReviewRoundRunProvenance {
                round: pair.clone(),
                preparation: prep,
                cause: ReviewRoundRunCause::PairedRepair(original_cause),
            },
            |_| Ok(()),
        )
        .unwrap();
    let repaired = f
        .host
        .retire_local(permit.operation_id, observation())
        .unwrap();
    let seal = source_seal(&f.host, &f.lease, &pair.review_phase.work, "c", "d");
    spacing();
    validation_run.attempt = 2;
    let validation_link = SourceAdvancedReference {
        predecessor: old_validation.operation_id,
        predecessor_receipt_digest: canonical_digest(1, &old_validation).unwrap(),
        source_operation: repaired.operation_id,
        source_receipt_digest: canonical_digest(1, &repaired).unwrap(),
        trigger_digest: digest("validation of actual repaired source"),
        progress_digest: digest("validation advance"),
        review_rounds_consumed: 1,
    };
    let permit = f
        .host
        .admit_run_source_advanced_observed(
            &f.lease,
            admission(validation_run.clone(), Uuid::new_v4()),
            validation_link.clone(),
            |_| Ok(()),
        )
        .unwrap();
    // Synthetic completed model observation only. A real producer still needs
    // configured FailedCheckExecution capabilities; this Control test has none.
    let validation = f
        .host
        .retire_local(permit.operation_id, observation())
        .unwrap();
    spacing();
    let inventory: BTreeMap<_, _> = f
        .host
        .operations()
        .unwrap()
        .into_iter()
        .filter(|r| {
            RunReference::from_receipt(r)
                .unwrap()
                .is_some_and(|run| run.phase == validation_run.phase)
        })
        .map(|r| (r.operation_id, r))
        .collect();
    let mut continued = pair.clone();
    let ReviewRoundCause::PairedRepair { findings } = pair.cause.clone() else {
        unreachable!()
    };
    let mut current_source = pair.source.clone();
    current_source.implementation = exact(&repaired);
    current_source.seal = exact(&seal);
    current_source.claim_digest = digest("actual protected repair claim reference");
    current_source.source_digest = digest("actual repaired source reference");
    current_source.commit = "c".repeat(40);
    current_source.tree = "d".repeat(40);
    continued.cause = ReviewRoundCause::RepairContinuation {
        findings,
        previous_round_reference_digest: canonical_digest(1, &pair).unwrap(),
        current_source: Box::new(current_source),
        validation: Box::new(RepairValidationReference {
            validation: exact(&validation),
            phase: validation_run.phase.clone(),
            source_advanced: validation_link,
            outcome_proposal: Uuid::new_v4(),
            outcome_claim_digest: digest("actual validation claim reference"),
            failure_inventory_digest: canonical_digest(1, &inventory).unwrap(),
        }),
    };
    assert_eq!(continued.round_id, pair.round_id);
    assert_eq!((continued.ordinal, continued.round_limit), (1, 1));
    let before = f.host.operations().unwrap().len();
    let mut wrong = continued.clone();
    let ReviewRoundCause::RepairContinuation {
        previous_round_reference_digest,
        ..
    } = &mut wrong.cause
    else {
        unreachable!()
    };
    *previous_round_reference_digest = digest("substituted repair marker");
    let worker = Uuid::new_v4();
    let reference = PreparationReference {
        worker_operation: worker,
        workspace: continued.workspace,
        account: continued.account.clone(),
        phase: continued.subject_phase().clone(),
    };
    let denied = f
        .host
        .admit_round_preparation_observed(
            &f.lease,
            PreparationAdmission {
                request_id: reference.operation_id().unwrap(),
                work_key: reference.phase.work.clone(),
                scope_digest: digest("component prep scope"),
                reference,
            },
            wrong,
            |_| Ok(()),
        )
        .unwrap_err();
    assert_eq!(denied.code, "review_round_lineage");
    assert_eq!(f.host.operations().unwrap().len(), before);
    let worker = Uuid::new_v4();
    let prep = prepare(&f, &continued, worker);
    let continued_cause = RunContinuationReference {
        predecessor: repaired.operation_id,
        predecessor_receipt_digest: canonical_digest(1, &repaired).unwrap(),
        failure_operation: validation.operation_id,
        trigger_digest: digest("new actual failed validation trigger"),
        progress_digest: digest("new repaired source and actual failed obligation"),
        review_rounds_consumed: 1,
    };
    implementation.attempt = 3;
    let mut widened = implementation.clone();
    widened.max_attempts = 4;
    assert_eq!(
        f.host
            .admit_round_run_observed(
                &f.lease,
                admission(widened, worker),
                ReviewRoundRunProvenance {
                    round: continued.clone(),
                    preparation: prep.clone(),
                    cause: ReviewRoundRunCause::PairedRepair(continued_cause.clone())
                },
                |_| Ok(())
            )
            .unwrap_err()
            .code,
        "attempts_exhausted"
    );
    let permit = f
        .host
        .admit_round_run_observed(
            &f.lease,
            admission(implementation, worker),
            ReviewRoundRunProvenance {
                round: continued.clone(),
                preparation: prep,
                cause: ReviewRoundRunCause::PairedRepair(continued_cause),
            },
            |_| Ok(()),
        )
        .unwrap();
    let terminal = f
        .host
        .retire_local(permit.operation_id, observation())
        .unwrap();
    let run = RunReference::from_receipt(&terminal).unwrap().unwrap();
    assert_eq!((run.attempt, run.max_attempts, run.max_turns), (3, 3, 4));
    assert_eq!(
        ReviewRoundReference::from_receipt(&terminal).unwrap(),
        Some(continued.clone())
    );
    assert_eq!(
        canonical_digest(1, &f.host.operation(repaired.operation_id).unwrap()).unwrap(),
        canonical_digest(1, &repaired).unwrap()
    );
    let reviews = f
        .host
        .operations()
        .unwrap()
        .iter()
        .filter_map(|r| RunReference::from_receipt(r).unwrap())
        .filter(|run| run.phase.phase == "child_codex_review")
        .count();
    assert_eq!(
        reviews, 1,
        "continuing a repair never allocates another review"
    );
    spacing();
    validation_run.attempt = 3;
    let latest_validation_link = SourceAdvancedReference {
        predecessor: validation.operation_id,
        predecessor_receipt_digest: canonical_digest(1, &validation).unwrap(),
        source_operation: terminal.operation_id,
        source_receipt_digest: canonical_digest(1, &terminal).unwrap(),
        trigger_digest: digest("validation after second actual repair"),
        progress_digest: digest("latest validation source progress"),
        review_rounds_consumed: 1,
    };
    let permit = f
        .host
        .admit_run_source_advanced_observed(
            &f.lease,
            admission(validation_run, Uuid::new_v4()),
            latest_validation_link,
            |_| Ok(()),
        )
        .unwrap();
    f.host
        .retire_local(permit.operation_id, observation())
        .unwrap();
    // Later validation changes current obligations, not the immutable validity
    // of the exact previous pair chain. This reads actual protected receipts;
    // it does not mint a delivery, joined or failed-check capability.
    pair_chain::verify_repair_chain(
        &f.host.snapshot().unwrap(),
        &continued,
        &exact(&terminal),
        &f.host.operations().unwrap(),
    )
    .unwrap();
    let before = f.host.operations().unwrap().len();
    let worker = Uuid::new_v4();
    let reference = PreparationReference {
        worker_operation: worker,
        workspace: continued.workspace,
        account: continued.account.clone(),
        phase: continued.subject_phase().clone(),
    };
    assert!(
        f.host
            .admit_round_preparation_observed(
                &f.lease,
                PreparationAdmission {
                    request_id: reference.operation_id().unwrap(),
                    work_key: reference.phase.work.clone(),
                    scope_digest: digest("replayed prep scope"),
                    reference
                },
                continued,
                |_| Ok(())
            )
            .is_err()
    );
    assert_eq!(f.host.operations().unwrap().len(), before);
}
