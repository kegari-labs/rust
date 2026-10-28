//! Actual Control component tests with explicitly synthetic source, terminal
//! session and readback observations. No native joined/provider/source capability
//! is constructed; callbacks model the separately mandatory opaque gate.
use super::*;
use crate::control::{
    ReviewRoundRecoveryReference, ReviewRoundResumeReference, RoundRecoveryProvenance,
    RoundResumeProvenance,
};
use kegari_types::run::RunReference;

struct FailedPair {
    f: Fixture,
    round: ReviewRoundReference,
    run: RunReference,
    failed: Receipt,
    recovery: ReviewRoundRecoveryReference,
}
fn failed_session(run: &RunReference, operation: Uuid) -> kegari_codex::SessionState {
    let mut state = kegari_codex::SessionState::new(
        kegari_codex::SessionBinding {
            schema_version: 1,
            account_id: run.account.clone(),
            work_identity: run.phase.work.clone(),
            operation_id: operation.to_string(),
            generation: run.phase.generation.clone(),
            instruction_digest: run.instruction_digest.clone(),
            isolation_digest: digest("synthetic isolation"),
            profile: kegari_codex::CodexProfile {
                model: "fixture".into(),
                reasoning: "high".into(),
                speed: "standard".into(),
            },
            cwd: std::path::PathBuf::from("/synthetic-control-component"),
        },
        run.max_turns,
    )
    .unwrap();
    state.thread_id = Some("synthetic-thread".into());
    state.turns_started = 1;
    state.terminal = Some("failed".into());
    state
        .completed_turns
        .insert("failed-turn".into(), "failed".into());
    state.last_completed_turn = Some("failed-turn".into());
    state.provider_history = Some(serde_json::from_value(json!({"turns":[{
        "id":"failed-turn","status":"failed","content_digest":digest("synthetic full turn"),
        "transient_failure":{"classification":"server_overloaded","terminal_digest":digest("synthetic full turn"),
            "error_digest":digest("synthetic error"),"policy_free":true}
    }]})).unwrap());
    assert!(state.terminal_transient_failure().unwrap().is_some());
    state
}
fn failed_pair() -> FailedPair {
    failed_pair_with_scalar(1, 1)
}
fn failed_pair_with_scalar(floor: u32, implementation_limit: u32) -> FailedPair {
    failed_pair_from_fixture(
        setup_with_implementation_limits(5, 8, implementation_limit),
        floor,
    )
}
fn failed_pair_from_fixture(f: Fixture, floor: u32) -> FailedPair {
    let reviewed = complete_first(&f);
    spacing();
    let (round, mut cause) = paired(&f, &reviewed);
    cause.review_rounds_consumed = floor;
    let worker = Uuid::new_v4();
    let prep = prepare(&f, &round, worker);
    let mut run = f.implementation.clone();
    run.attempt = 2;
    let permit = f
        .host
        .admit_round_run_observed(
            &f.lease,
            admission(run.clone(), worker),
            ReviewRoundRunProvenance {
                round: round.clone(),
                preparation: prep.clone(),
                cause: ReviewRoundRunCause::PairedRepair(cause),
            },
            |_| Ok(()),
        )
        .unwrap();
    let session = failed_session(&run, permit.operation_id);
    let failed = f.host.retire_local(permit.operation_id, json!({"outcome":"failed","local_exit_confirmed":true,"hook_error":null,"session":session})).unwrap();
    let recovery = ReviewRoundRecoveryReference {
        schema_version: 1,
        round_reference_digest: canonical_digest(1, &round).unwrap(),
        round_plan_digest: digest("synthetic opaque plan reference"),
        origin: exact(&failed),
        origin_preparation: prep,
        predecessor: exact(&failed),
        protected_candidate_digest: digest("synthetic protected candidate"),
        terminal_state_digest: canonical_digest(1, &session).unwrap(),
        classification: TransientFailureCode::ServerOverloaded,
        previous_resume_digest: None,
    };
    spacing();
    FailedPair {
        f,
        round,
        run,
        failed,
        recovery,
    }
}
impl FailedPair {
    fn metadata(&self, attempt: u32) -> SessionRecoveryPreparationReference {
        SessionRecoveryPreparationReference {
            predecessor: self.recovery.predecessor.operation,
            predecessor_receipt_digest: self.recovery.predecessor.receipt_digest.clone(),
            candidate_digest: self.recovery.protected_candidate_digest.clone(),
            workspace: self.run.workspace,
            account: self.run.account.clone(),
            phase: self.run.phase.clone(),
            attempt,
            max_attempts: self.run.max_attempts,
        }
    }
    fn provenance(&self) -> RoundRecoveryProvenance {
        RoundRecoveryProvenance {
            round: self.round.clone(),
            recovery: self.recovery.clone(),
        }
    }
    fn metadata_success(&self) -> Receipt {
        let permit = self
            .f
            .host
            .admit_round_session_recovery_preparation(
                &self.f.lease,
                self.metadata(1),
                self.provenance(),
                digest("metadata scope"),
                |_| Ok(()),
            )
            .unwrap();
        self.f.host.retire_local(permit.operation_id, json!({"metadata_recovery":"verified","proof_digest":digest("synthetic observed readback"),"local_exit_confirmed":true})).unwrap()
    }
    fn resume(
        &self,
        metadata: &Receipt,
    ) -> (RunAdmission, RunResumeReference, RoundResumeProvenance) {
        let mut run = self.run.clone();
        run.attempt += 1;
        let marker = ReviewRoundResumeReference {
            schema_version: 1,
            recovery: self.recovery.clone(),
            metadata: exact(metadata),
            provider_recovery_digest: canonical_digest(1, &metadata.operation_id).unwrap(),
            instruction_compatibility_digest: digest("synthetic compatibility"),
            directory_reuse_digest: digest("synthetic directory reuse"),
        };
        let resume = RunResumeReference {
            predecessor: marker.recovery.predecessor.operation,
            predecessor_receipt_digest: marker.recovery.predecessor.receipt_digest.clone(),
            recovery_digest: marker.provider_recovery_digest.clone(),
            instruction_compatibility_digest: marker.instruction_compatibility_digest.clone(),
            directory_reuse_digest: marker.directory_reuse_digest.clone(),
            review_rounds_consumed: self.round.ordinal,
        };
        (
            admission(run, Uuid::new_v4()),
            resume,
            RoundResumeProvenance {
                round: self.round.clone(),
                resume: marker,
            },
        )
    }
}
#[test]
fn paired_recovery_admits_exact_metadata_then_resume_without_new_round_or_budget() {
    let p = failed_pair();
    assert_eq!(
        p.f.host
            .admit_session_recovery_preparation(
                &p.f.lease,
                p.metadata(1),
                digest("ordinary scope"),
                |_| Ok(())
            )
            .unwrap_err()
            .code,
        "review_round_required"
    );
    let metadata = p.metadata_success();
    assert!(
        crate::control::run_reference_from_receipt(&metadata)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        SessionRecoveryPreparationReference::from_receipt(&metadata).unwrap(),
        Some(p.metadata(1))
    );
    assert_eq!(
        ReviewRoundReference::from_receipt(&metadata).unwrap(),
        Some(p.round.clone())
    );
    let (input, resume, provenance) = p.resume(&metadata);
    let count = p.f.host.operations().unwrap().len();
    assert_eq!(
        p.f.host
            .admit_run_resumed_observed(
                &p.f.lease,
                admission(input.reference.clone(), Uuid::new_v4()),
                resume.clone(),
                |_| Ok(())
            )
            .unwrap_err()
            .code,
        "review_round_required"
    );
    let rejected =
        p.f.host
            .admit_round_run_resumed_observed(
                &p.f.lease,
                admission(input.reference.clone(), Uuid::new_v4()),
                resume.clone(),
                provenance.clone(),
                |_| {
                    Err(Error::new(
                        "access_denied",
                        "synthetic current authority revoked",
                    ))
                },
            )
            .unwrap_err();
    assert_eq!(rejected.code, "access_denied");
    assert_eq!(p.f.host.operations().unwrap().len(), count);
    let permit =
        p.f.host
            .admit_round_run_resumed_observed(
                &p.f.lease,
                input,
                resume.clone(),
                provenance.clone(),
                |_| Ok(()),
            )
            .unwrap();
    let receipt = p.f.host.operation(permit.operation_id).unwrap();
    assert_eq!(
        ReviewRoundReference::from_receipt(&receipt).unwrap(),
        Some(p.round.clone())
    );
    assert_eq!(
        ReviewRoundResumeReference::from_model_receipt(&receipt).unwrap(),
        Some(provenance.resume)
    );
    assert_eq!(
        RunResumeReference::from_receipt(&receipt).unwrap(),
        Some(resume)
    );
    let actual = crate::control::run_reference_from_receipt(&receipt)
        .unwrap()
        .unwrap();
    assert_eq!(
        (
            actual.attempt,
            actual.max_attempts,
            actual.max_turns,
            actual.max_review_rounds
        ),
        (3, 8, 4, 1)
    );
    assert_eq!(p.round.round_limit, 5);
    assert!(!receipt.expected.contains_key("continuation"));
    assert!(!receipt.expected.contains_key("review_round_preparation"));
    let mut orphan = receipt.clone();
    orphan.expected.remove("review_round");
    assert!(crate::control::run_reference_from_receipt(&orphan).is_err());
    assert!(RunResumeReference::from_receipt(&orphan).is_err());
    let mut orphan = metadata.clone();
    orphan.expected.remove("review_round");
    assert!(SessionRecoveryPreparationReference::from_receipt(&orphan).is_err());
    let mut mixed = receipt.clone();
    mixed
        .expected
        .insert("worker_preparation".into(), "{}".into());
    assert!(ReviewRoundReference::from_receipt(&mixed).is_err());
    p.f.host
        .retire_local(permit.operation_id, observation())
        .unwrap();
}
#[test]
fn paired_recovery_metadata_is_atomic_and_keeps_generic_readmission_closed() {
    let p = failed_pair();
    let barrier = Arc::new(Barrier::new(3));
    let outcomes = std::thread::scope(|scope| {
        let mut threads = vec![];
        for _ in 0..2 {
            let barrier = barrier.clone();
            let p = &p;
            threads.push(scope.spawn(move || {
                barrier.wait();
                p.f.host.admit_round_session_recovery_preparation(
                    &p.f.lease,
                    p.metadata(1),
                    p.provenance(),
                    digest("metadata scope"),
                    |_| Ok(()),
                )
            }));
        }
        barrier.wait();
        threads
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(outcomes.iter().filter(|r| r.is_ok()).count(), 1);
    let permit = outcomes.into_iter().find_map(Result::ok).unwrap();
    assert_eq!(p.f.host.snapshot().unwrap().active_operations.len(), 1);
    let failed_metadata = p.f.host.retire_local(permit.operation_id, json!({"metadata_recovery":"failed_no_effect","retryable":true,"local_exit_confirmed":true})).unwrap();
    // Deterministic synthetic journal timing, not production freshness.
    let mut future = failed_metadata.clone();
    future.updated_at = Utc::now() + chrono::Duration::seconds(60);
    p.f.host
        .receipts
        .directory
        .write(&format!("{}.json", future.operation_id), &future)
        .unwrap();
    assert_eq!(
        p.f.host
            .admit_round_session_recovery_preparation(
                &p.f.lease,
                p.metadata(2),
                p.provenance(),
                digest("metadata scope"),
                |_| Ok(())
            )
            .unwrap_err()
            .code,
        "continuation_wait"
    );
    p.f.host
        .receipts
        .directory
        .write(
            &format!("{}.json", failed_metadata.operation_id),
            &failed_metadata,
        )
        .unwrap();
    spacing();
    let permit =
        p.f.host
            .admit_round_session_recovery_preparation(
                &p.f.lease,
                p.metadata(2),
                p.provenance(),
                digest("metadata scope"),
                |_| Ok(()),
            )
            .unwrap();
    p.f.host.retire_local(permit.operation_id, json!({"metadata_recovery":"failed_no_effect","retryable":false,"local_exit_confirmed":true})).unwrap();
    assert_eq!(
        p.f.host
            .admit_round_session_recovery_preparation(
                &p.f.lease,
                p.metadata(3),
                p.provenance(),
                digest("metadata scope"),
                |_| Ok(())
            )
            .unwrap_err()
            .code,
        "reconciliation_required"
    );
}
#[test]
fn paired_recovery_rejects_changed_origin_preparation_and_unknown_terminal_without_writes() {
    let p = failed_pair();
    let count = p.f.host.operations().unwrap().len();
    let mut wrong = p.provenance();
    wrong.recovery.origin_preparation.receipt_digest = digest("other preparation");
    assert!(
        p.f.host
            .admit_round_session_recovery_preparation(
                &p.f.lease,
                p.metadata(1),
                wrong,
                digest("scope"),
                |_| panic!("invalid actual lineage reached callback")
            )
            .is_err()
    );
    let original = p.f.host.operation(p.failed.operation_id).unwrap();
    let mut unknown = original.clone();
    unknown.phase = ReceiptPhase::Sent;
    p.f.host
        .receipts
        .directory
        .write(&format!("{}.json", original.operation_id), &unknown)
        .unwrap();
    assert!(
        p.f.host
            .admit_round_session_recovery_preparation(
                &p.f.lease,
                p.metadata(1),
                p.provenance(),
                digest("scope"),
                |_| panic!("uncertain actual lineage reached callback")
            )
            .is_err()
    );
    p.f.host
        .receipts
        .directory
        .write(&format!("{}.json", original.operation_id), &original)
        .unwrap();
    let mut missing = p.provenance();
    missing.recovery.classification = TransientFailureCode::InternalServerError;
    assert!(
        p.f.host
            .admit_round_session_recovery_preparation(
                &p.f.lease,
                p.metadata(1),
                missing,
                digest("scope"),
                |_| panic!("false classification reached callback")
            )
            .is_err()
    );
    assert_eq!(p.f.host.operations().unwrap().len(), count);
}
#[test]
fn paired_resume_requires_actual_closed_metadata_and_nonincreasing_implementation_caps() {
    let p = failed_pair();
    let metadata = p.metadata_success();
    let count = p.f.host.operations().unwrap().len();
    let (mut input, resume, provenance) = p.resume(&metadata);
    input.reference.max_turns += 1;
    assert_eq!(
        p.f.host
            .admit_round_run_resumed_observed(&p.f.lease, input, resume, provenance, |_| panic!(
                "raised caps reached callback"
            ))
            .unwrap_err()
            .code,
        "attempts_exhausted"
    );
    let (input, resume, mut provenance) = p.resume(&metadata);
    provenance.resume.metadata.receipt_digest = digest("another metadata observation");
    assert!(
        p.f.host
            .admit_round_run_resumed_observed(&p.f.lease, input, resume, provenance, |_| panic!(
                "changed metadata reached callback"
            ))
            .is_err()
    );
    let mut changed = metadata.clone();
    changed.observed_result.as_mut().unwrap()["observation"]["metadata_recovery"] =
        json!("failed_no_effect");
    p.f.host
        .receipts
        .directory
        .write(&format!("{}.json", metadata.operation_id), &changed)
        .unwrap();
    let (input, resume, provenance) = p.resume(&changed);
    assert!(
        p.f.host
            .admit_round_run_resumed_observed(&p.f.lease, input, resume, provenance, |_| panic!(
                "unverified metadata reached callback"
            ))
            .is_err()
    );
    p.f.host
        .receipts
        .directory
        .write(&format!("{}.json", metadata.operation_id), &metadata)
        .unwrap();
    assert_eq!(p.f.host.operations().unwrap().len(), count);
}

#[test]
fn paired_control_repeated_resume_retains_original_and_actual_previous_bridge() {
    // Structural Control support only: the opaque repeated-resumed-failure
    // producer/native joined bridge is still a separate integration prerequisite.
    let mut p = failed_pair();
    let original = p.recovery.origin.clone();
    let metadata = p.metadata_success();
    let (input, resume, provenance) = p.resume(&metadata);
    let next_run = input.reference.clone();
    let prior_marker = provenance.resume.clone();
    let permit =
        p.f.host
            .admit_round_run_resumed_observed(&p.f.lease, input, resume, provenance, |_| Ok(()))
            .unwrap();
    let state = failed_session(&next_run, permit.operation_id);
    let failed = p.f.host.retire_local(permit.operation_id, json!({"outcome":"failed","local_exit_confirmed":true,"hook_error":null,"session":state})).unwrap();
    p.run = next_run;
    p.failed = failed.clone();
    p.recovery.predecessor = exact(&failed);
    p.recovery.terminal_state_digest = canonical_digest(1, &state).unwrap();
    p.recovery.protected_candidate_digest = digest("synthetic next protected candidate");
    p.recovery.previous_resume_digest = Some(canonical_digest(1, &prior_marker).unwrap());
    spacing();
    let mut missing = p.provenance();
    missing.recovery.previous_resume_digest = Some(digest("another bridge"));
    assert!(
        p.f.host
            .admit_round_session_recovery_preparation(
                &p.f.lease,
                p.metadata(1),
                missing,
                digest("scope"),
                |_| panic!("wrong prior bridge reached callback")
            )
            .is_err()
    );
    let metadata = p.metadata_success();
    let (input, resume, provenance) = p.resume(&metadata);
    assert_eq!(provenance.resume.recovery.origin, original);
    let permit =
        p.f.host
            .admit_round_run_resumed_observed(&p.f.lease, input, resume, provenance, |_| Ok(()))
            .unwrap();
    let actual = p.f.host.operation(permit.operation_id).unwrap();
    assert_eq!(
        crate::control::run_reference_from_receipt(&actual)
            .unwrap()
            .unwrap()
            .attempt,
        4
    );
    assert_eq!(
        ReviewRoundReference::from_receipt(&actual).unwrap(),
        Some(p.round)
    );
    p.f.host
        .retire_local(permit.operation_id, observation())
        .unwrap();
}
#[test]
fn paired_recovery_encoded_bound_precedes_current_callback_and_receipt_write() {
    let p = failed_pair();
    let before = p.f.host.operations().unwrap().len();
    let mut too_large = p.provenance();
    too_large.recovery.round_plan_digest = "x".repeat(16 * 1024);
    let error =
        p.f.host
            .admit_round_session_recovery_preparation(
                &p.f.lease,
                p.metadata(1),
                too_large,
                digest("scope"),
                |_| panic!("oversized marker reached callback"),
            )
            .unwrap_err();
    assert_eq!(error.code, "review_round_history_limit");
    assert_eq!(p.f.host.operations().unwrap().len(), before);
}

#[test]
fn paired_resume_preserves_implementation_scalar_floor_independently_of_round_ordinal() {
    // Actual Control effects; source/provider/session observations stay explicitly
    // synthetic. The retained scalar represents earlier consumed review work,
    // while this marker remains exact ordinal one in its current namespace.
    let p = failed_pair_with_scalar(3, 5);
    assert_eq!(p.round.ordinal, 1);
    let metadata = p.metadata_success();
    let (input, mut resume, provenance) = p.resume(&metadata);
    let count = p.f.host.operations().unwrap().len();
    for wrong in [1, 2, 4] {
        resume.review_rounds_consumed = wrong;
        assert!(
            p.f.host
                .admit_round_run_resumed_observed(
                    &p.f.lease,
                    admission(input.reference.clone(), Uuid::new_v4()),
                    resume.clone(),
                    provenance.clone(),
                    |_| Ok(())
                )
                .is_err()
        );
        assert_eq!(p.f.host.operations().unwrap().len(), count);
    }
    resume.review_rounds_consumed = 3;
    let permit =
        p.f.host
            .admit_round_run_resumed_observed(
                &p.f.lease,
                input,
                resume.clone(),
                provenance.clone(),
                |_| Ok(()),
            )
            .unwrap();
    let actual = p.f.host.operation(permit.operation_id).unwrap();
    assert_eq!(
        RunResumeReference::from_receipt(&actual).unwrap(),
        Some(resume)
    );
    assert_eq!(
        ReviewRoundReference::from_receipt(&actual)
            .unwrap()
            .unwrap()
            .ordinal,
        1
    );
    assert_eq!(
        ReviewRoundResumeReference::from_model_receipt(&actual).unwrap(),
        Some(provenance.resume)
    );
}

#[test]
fn completed_paired_resume_retains_real_original_metadata_closure_without_phantom_preparation() {
    // Real protected Control admissions/retirement, synthetic execution and
    // readback observations only. This reader does not create a delivery cap.
    let p = failed_pair();
    let metadata = p.metadata_success();
    let (input, resume, provenance) = p.resume(&metadata);
    let permit = p
        .f
        .host
        .admit_round_run_resumed_observed(&p.f.lease, input, resume, provenance.clone(), |_| Ok(()))
        .unwrap();
    let terminal =
        p.f.host
            .retire_local(permit.operation_id, observation())
            .unwrap();
    assert!(!terminal.expected.contains_key("review_round_preparation"));
    let refs = [
        exact(&p.failed),
        p.recovery.origin_preparation.clone(),
        exact(&terminal),
        exact(&metadata),
    ]
    .into_iter()
    .map(|r| (r.operation, r.receipt_digest))
    .collect::<BTreeMap<_, _>>();
    let reference = PairedResumeResultReference {
        schema_version: 1,
        round_reference_digest: canonical_digest(1, &p.round).unwrap(),
        round_plan_digest: p.recovery.round_plan_digest.clone(),
        origin: exact(&p.failed),
        origin_preparation: p.recovery.origin_preparation.clone(),
        terminal: exact(&terminal),
        resume_reference_digest: canonical_digest(1, &provenance.resume).unwrap(),
        history_digest: canonical_digest(1, &refs).unwrap(),
    };
    let records = p.f.host.operations().unwrap();
    let snapshot = p.f.host.snapshot().unwrap();
    recovery::verify_completed_paired_result(&snapshot, &p.round, &reference, &records).unwrap();
    for missing in refs.keys() {
        let reduced = records
            .iter()
            .filter(|r| r.operation_id != *missing)
            .cloned()
            .collect::<Vec<_>>();
        assert!(
            recovery::verify_completed_paired_result(&snapshot, &p.round, &reference, &reduced)
                .is_err()
        );
    }
    let mut changed = reference.clone();
    changed.origin_preparation = exact(&metadata);
    assert!(
        recovery::verify_completed_paired_result(&snapshot, &p.round, &changed, &records).is_err()
    );
    let mut changed = reference;
    changed.history_digest = digest("another closure");
    assert!(
        recovery::verify_completed_paired_result(&snapshot, &p.round, &changed, &records).is_err()
    );
    assert_eq!(p.f.host.operations().unwrap().len(), records.len());
}

/// Real protected records with synthetic observed execution/body facts. The
/// passed marker is data only; no LocalPassed or native grant cap is constructed.
fn retain_component_passed_validation(f: &mut Fixture) -> Vec<Uuid> {
    let worker = Uuid::new_v4();
    let mut run = f.implementation.clone();
    run.phase.phase = "child_local_validation".into();
    let preparation = PreparationReference {
        worker_operation: worker,
        workspace: run.workspace,
        account: run.account.clone(),
        phase: run.phase.clone(),
    };
    let permit = f
        .host
        .admit_preparation(
            &f.lease,
            PreparationAdmission {
                request_id: preparation.operation_id().unwrap(),
                work_key: run.phase.work.clone(),
                scope_digest: digest("synthetic validation preparation"),
                reference: preparation,
            },
            || Ok(()),
        )
        .unwrap();
    let preparation = f
        .host
        .retire_local(
            permit.operation_id,
            json!({"phase":"worker_preparation","result":"prepared","local_exit_confirmed":true}),
        )
        .unwrap();
    let permit = f
        .host
        .admit_run(&f.lease, admission(run, worker), || Ok(()))
        .unwrap();
    let validation = f
        .host
        .retire_local(permit.operation_id, observation())
        .unwrap();
    let owner = f
        .host
        .admit(
            &f.lease,
            Uuid::new_v4(),
            f.round.review_phase.work.clone(),
            digest("synthetic grant publication scope"),
            || Ok(()),
        )
        .unwrap();
    let (effect, ()) = f
        .host
        .begin_effect(
            MutationRequest {
                request_id: Uuid::new_v4(),
                runtime_id: f.lease.runtime_id,
                actor: Actor {
                    identity: format!("owner:{}", f.lease.owner_id),
                    roles: BTreeSet::new(),
                    principal: None,
                },
                kind: "linear_comment".into(),
                canonical_targets: vec![f.round.review_phase.work.clone()],
                expected: BTreeMap::new(),
                payload: json!({"fixture":"synthetic grant publication body; no native authority"}),
            },
            &owner,
            || Ok(()),
            |_| Ok(()),
        )
        .unwrap();
    let publication = f
        .host
        .complete_effect(
            effect,
            &owner,
            || Ok(()),
            json!({"fixture":"synthetic observed grant publication"}),
        )
        .unwrap();
    f.host
        .retire_local(owner.operation_id, json!({"local_exit_confirmed":true}))
        .unwrap();
    let references = [
        f.round.source.implementation.clone(),
        f.round.source.seal.clone(),
        exact(&validation),
        exact(&preparation),
        exact(&publication),
    ];
    f.round.passed_validation = Some(crate::control::PassedValidationReference {
        schema_version: 1,
        source: f.round.source.clone(),
        validation: exact(&validation),
        completed_validation_digest: digest("synthetic completed validation cap reference"),
        local_pass_digest: digest("synthetic LocalPassed reference"),
        retained_receipts: references
            .into_iter()
            .map(|r| (r.operation, r.receipt_digest))
            .collect(),
    });
    f.round.validate().unwrap();
    vec![
        validation.operation_id,
        preparation.operation_id,
        publication.operation_id,
    ]
}

#[test]
fn new_grant_initial_source_retains_older_resumed_pair_passed_references() {
    // The only new-grant Initial below is a data-reader input, not an admitted
    // review or a native grant. All referenced records are actual Control writes;
    // source, terminal and publication observations remain synthetic.
    let mut f = setup_with_implementation_limits(5, 8, 1);
    let retained = retain_component_passed_validation(&mut f);
    let p = failed_pair_from_fixture(f, 1);
    let metadata = p.metadata_success();
    let (input, resume, provenance) = p.resume(&metadata);
    let permit = p
        .f
        .host
        .admit_round_run_resumed_observed(&p.f.lease, input, resume, provenance.clone(), |_| Ok(()))
        .unwrap();
    let terminal =
        p.f.host
            .retire_local(permit.operation_id, observation())
            .unwrap();
    let refs = [
        exact(&p.failed),
        p.recovery.origin_preparation.clone(),
        exact(&terminal),
        exact(&metadata),
    ]
    .into_iter()
    .map(|r| (r.operation, r.receipt_digest))
    .collect::<BTreeMap<_, _>>();
    let reference = PairedResumeResultReference {
        schema_version: 1,
        round_reference_digest: canonical_digest(1, &p.round).unwrap(),
        round_plan_digest: p.recovery.round_plan_digest.clone(),
        origin: exact(&p.failed),
        origin_preparation: p.recovery.origin_preparation.clone(),
        terminal: exact(&terminal),
        resume_reference_digest: canonical_digest(1, &provenance.resume).unwrap(),
        history_digest: canonical_digest(1, &refs).unwrap(),
    };
    let seal = source_seal(&p.f.host, &p.f.lease, &p.round.review_phase.work, "c", "d");
    let mut next = p.f.round.clone();
    next.review_phase.retry_grant = Some(Uuid::new_v4());
    next.source.implementation = exact(&terminal);
    next.source.seal = exact(&seal);
    next.source.commit = "c".repeat(40);
    next.source.tree = "d".repeat(40);
    next.source.source_digest = digest("synthetic completed resumed source reference");
    next.source.paired_resume = Some(reference);
    next.passed_validation = None;
    next.round_id = next.expected_round_id().unwrap();
    next.validate().unwrap();
    let records = p.f.host.operations().unwrap();
    let snapshot = p.f.host.snapshot().unwrap();
    assert!(review_history(&next, &records).unwrap().is_empty());
    verify_source(&snapshot, &next, &records, false).unwrap();
    for operation in retained {
        let missing = records
            .iter()
            .filter(|r| r.operation_id != operation)
            .cloned()
            .collect::<Vec<_>>();
        assert!(verify_source(&snapshot, &next, &missing, false).is_err());
        let mut changed = records.clone();
        let receipt = changed
            .iter_mut()
            .find(|r| r.operation_id == operation)
            .unwrap();
        receipt.observed_result = Some(json!({"fixture":"substituted protected observation"}));
        assert!(verify_source(&snapshot, &next, &changed, false).is_err());
    }
    assert_eq!(
        canonical_digest(1, &p.f.host.operations().unwrap()).unwrap(),
        canonical_digest(1, &records).unwrap()
    );
}
