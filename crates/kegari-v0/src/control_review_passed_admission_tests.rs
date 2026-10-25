//! Real Control journal negative. The oversized provenance is deliberately data,
//! not a passed/source capability; no provider, model or native work is launched.
use super::*;
#[test]
fn oversized_round_cannot_persist_preparation_or_model_admission() {
    let f = setup();
    let mut round = f.round.clone();
    let validation = ReviewReceiptReference {
        operation: Uuid::new_v4(),
        receipt_digest: digest("untrusted data-only validation"),
    };
    let mut retained: BTreeMap<_, _> = [
        (
            round.source.implementation.operation,
            round.source.implementation.receipt_digest.clone(),
        ),
        (
            round.source.seal.operation,
            round.source.seal.receipt_digest.clone(),
        ),
        (validation.operation, validation.receipt_digest.clone()),
    ]
    .into();
    for id in 1000..1500 {
        retained.insert(Uuid::from_u128(id), digest("untrusted oversized reference"));
    }
    round.passed_validation = Some(crate::control::PassedValidationReference {
        schema_version: 1,
        source: round.source.clone(),
        validation,
        completed_validation_digest: digest("data only"),
        local_pass_digest: digest("data only"),
        retained_receipts: retained,
    });
    let count = f.host.operations().unwrap().len();
    let worker = Uuid::new_v4();
    let prep = PreparationReference {
        worker_operation: worker,
        workspace: round.workspace,
        account: round.account.clone(),
        phase: round.review_phase.clone(),
    };
    let error = f
        .host
        .admit_round_preparation_observed(
            &f.lease,
            PreparationAdmission {
                request_id: prep.operation_id().unwrap(),
                work_key: round.review_phase.work.clone(),
                scope_digest: digest("fixture"),
                reference: prep,
            },
            round.clone(),
            |_| panic!("oversized host shape must fail before opaque callback"),
        )
        .unwrap_err();
    assert_eq!(error.code, "review_round_history_limit");
    assert_eq!(f.host.operations().unwrap().len(), count);
    let mut run = f.implementation.clone();
    run.phase = round.review_phase.clone();
    let error = f
        .host
        .admit_round_run_observed(
            &f.lease,
            admission(run, worker),
            ReviewRoundRunProvenance {
                round,
                preparation: exact(
                    &f.host
                        .operation(f.round.source.implementation.operation)
                        .unwrap(),
                ),
                cause: ReviewRoundRunCause::Review,
            },
            |_| panic!("oversized host shape must fail before opaque callback"),
        )
        .unwrap_err();
    assert_eq!(error.code, "review_round_history_limit");
    assert_eq!(f.host.operations().unwrap().len(), count);
}
