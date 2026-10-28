use super::*;
use crate::control::Lifecycle;
use crate::control::MutationRequest;
use crate::control::RunAdmission;
use kegari_control_api::identity::Actor;

#[test]
fn actual_model_request_observation_retains_pause_and_refuses_change_fence_or_missing_bytes() {
    // Real protected Control/admission component only. No worker registration,
    // native process, active snapshot or MatchedAtCollection value is invented.
    let root = tempfile::tempdir().unwrap();
    let directory =
        PrivateDir::create(&root.path().canonicalize().unwrap().join("control")).unwrap();
    let receipts = directory.child("receipts").unwrap();
    let runtime = Uuid::new_v4();
    let host = ControlHost::setup(directory, runtime, Uuid::new_v4()).unwrap();
    let command = |kind: &str| MutationRequest {
        request_id: Uuid::new_v4(),
        runtime_id: runtime,
        actor: Actor {
            identity: "component operator".into(),
            principal: None,
            roles: BTreeSet::from(["lifecycle".into()]),
        },
        kind: kind.into(),
        canonical_targets: vec![runtime.to_string()],
        expected: BTreeMap::new(),
        payload: Value::Null,
    };
    host.lifecycle(command("start"), Lifecycle::Start, 0)
        .unwrap();
    let lease = Lease {
        runtime_id: runtime,
        owner_generation: 1,
        owner_id: Uuid::new_v4(),
    };
    host.attach_reconciled(lease.clone()).unwrap();
    let budget = RunReference {
        workspace: Uuid::new_v4(),
        account: "component account".into(),
        normalized_state: None,
        phase: crate::scheduler::PhaseKey {
            work: "component independent PR".into(),
            generation: bytes_digest(b"generation"),
            phase: "independent_review".into(),
            retry_grant: None,
        },
        profile_digest: bytes_digest(b"profile"),
        instruction_digest: bytes_digest(b"instructions"),
        max_attempts: 2,
        max_turns: 3,
        max_review_rounds: 1,
        attempt: 1,
    };
    let permit = host
        .admit_run(
            &lease,
            RunAdmission {
                request_id: Uuid::new_v4(),
                work_key: budget.phase.work.clone(),
                scope_digest: bytes_digest(b"scope"),
                reference: budget.clone(),
            },
            || Ok(()),
        )
        .unwrap();
    let check = |permit: &Permit, budget: &RunReference| {
        host.validate_permit(permit, || {
            boundary_model_request(&host, permit, budget)?;
            Ok(())
        })
    };
    check(&permit, &budget).unwrap();
    let original = host.operation(permit.operation_id).unwrap();
    let mut changed_budget = budget.clone();
    changed_budget.max_turns += 1;
    assert_eq!(
        check(&permit, &changed_budget).unwrap_err().code,
        "worker_binding"
    );
    let leaf = format!("{}.json", permit.operation_id);
    let bytes = receipts.read_bytes(&leaf, 256 * 1024).unwrap();
    let mut changed = original.clone();
    changed.actor = "substituted owner".into();
    receipts.write(&leaf, &changed).unwrap();
    assert_eq!(check(&permit, &budget).unwrap_err().code, "worker_binding");
    receipts.write_bytes(&leaf, &bytes).unwrap();
    std::fs::remove_file(receipts.path().join(&leaf)).unwrap();
    assert_eq!(check(&permit, &budget).unwrap_err().code, "record_missing");
    receipts.write_bytes(&leaf, &bytes).unwrap();
    assert_eq!(
        canonical_digest(1, &host.operation(permit.operation_id).unwrap()).unwrap(),
        canonical_digest(1, &original).unwrap()
    );
    host.lifecycle(
        command("pause"),
        Lifecycle::Pause,
        host.snapshot().unwrap().intent.generation,
    )
    .unwrap();
    assert!(!host.snapshot().unwrap().admission_open);
    check(&permit, &budget).unwrap();
    host.fence(permit.operation_id).unwrap();
    let mut callback_ran = false;
    assert!(
        host.validate_permit(&permit, || {
            callback_ran = true;
            Ok(())
        })
        .is_err()
    );
    assert!(
        !callback_ran,
        "a fenced permit must never reach diagnostic collection"
    );
}
