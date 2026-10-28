use kegari_digest::canonical_digest;
use kegari_types::error::Error;
use kegari_v0::{control::*, scheduler::PhaseKey, storage::PrivateDir};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use tempfile::TempDir;
use uuid::Uuid;

fn started() -> (TempDir, ControlHost, Lease) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap().join("state");
    let runtime_id = Uuid::new_v4();
    let host = ControlHost::setup(
        PrivateDir::create(&root).unwrap(),
        runtime_id,
        Uuid::new_v4(),
    )
    .unwrap();
    host.lifecycle(
        MutationRequest {
            request_id: Uuid::new_v4(),
            runtime_id,
            actor: Actor {
                identity: "fixture_operator".into(),
                roles: BTreeSet::from(["lifecycle".into()]),
                principal: None,
            },
            kind: "lifecycle".into(),
            canonical_targets: vec![runtime_id.to_string()],
            expected: BTreeMap::new(),
            payload: json!(null),
        },
        Lifecycle::Start,
        0,
    )
    .unwrap();
    let lease = Lease {
        runtime_id,
        owner_generation: 1,
        owner_id: Uuid::new_v4(),
    };
    host.attach_reconciled(lease.clone()).unwrap();
    (temp, host, lease)
}

fn reference() -> PreparationReference {
    PreparationReference {
        worker_operation: Uuid::new_v4(),
        workspace: Uuid::new_v4(),
        account: "fixture-account".into(),
        phase: PhaseKey {
            work: "canonical-work".into(),
            generation: canonical_digest(1, &"work-generation").unwrap(),
            phase: "implementation".into(),
            retry_grant: Some(Uuid::new_v4()),
        },
    }
}
fn admission(reference: &PreparationReference) -> PreparationAdmission {
    PreparationAdmission {
        request_id: reference.operation_id().unwrap(),
        work_key: reference.phase.work.clone(),
        scope_digest: canonical_digest(1, &"immutable-preparation-scope").unwrap(),
        reference: reference.clone(),
    }
}

#[test]
fn exact_preparation_lineage_survives_restart_without_becoming_a_run_or_releasing_its_hold() {
    let (temp, host, lease) = started();
    let reference = reference();
    let input = admission(&reference);
    let scope = input.scope_digest.clone();
    let permit = host.admit_preparation(&lease, input, || Ok(())).unwrap();
    let receipt = host.operation(permit.operation_id).unwrap();
    assert_eq!(
        PreparationReference::from_receipt(&receipt).unwrap(),
        Some(reference.clone())
    );
    assert!(RunReference::from_receipt(&receipt).unwrap().is_none());
    assert!(MetadataReference::from_receipt(&receipt).unwrap().is_none());
    assert_eq!(receipt.expected["scope"], scope);
    assert_eq!(receipt.phase, ReceiptPhase::Prepared);
    assert_eq!(
        host.snapshot()
            .unwrap()
            .active_operations
            .iter()
            .map(|active| active.operation_id)
            .collect::<Vec<_>>(),
        vec![permit.operation_id]
    );
    let digest = canonical_digest(1, &receipt).unwrap();
    drop(host);
    let root = PrivateDir::open(&temp.path().canonicalize().unwrap().join("state")).unwrap();
    let recovered = ControlHost::recover(root, lease.runtime_id).unwrap();
    let retained = recovered.operation(permit.operation_id).unwrap();
    assert_eq!(canonical_digest(1, &retained).unwrap(), digest);
    assert_eq!(
        PreparationReference::from_receipt(&retained).unwrap(),
        Some(reference)
    );
    assert_eq!(
        recovered.snapshot().unwrap().unresolved_operations[&permit.operation_id],
        vec![permit.work_key]
    );
    assert!(!recovered.snapshot().unwrap().admission_open);
}

#[test]
fn preparation_identity_and_current_gate_rejections_leave_no_admission() {
    let (_temp, host, lease) = started();
    let reference = reference();
    let mut wrong_id = admission(&reference);
    wrong_id.request_id = Uuid::new_v4();
    assert_eq!(
        host.admit_preparation(&lease, wrong_id, || panic!(
            "invalid input reached authority gate"
        ))
        .unwrap_err()
        .code,
        "scope_violation"
    );
    let mut wrong_work = admission(&reference);
    wrong_work.work_key = "another-work".into();
    assert_eq!(
        host.admit_preparation(&lease, wrong_work, || panic!(
            "invalid work reached authority gate"
        ))
        .unwrap_err()
        .code,
        "scope_violation"
    );
    assert_eq!(
        host.admit_preparation(&lease, admission(&reference), || Err(Error::new(
            "stale_result",
            "actual current authority changed"
        )))
        .unwrap_err()
        .code,
        "stale_result"
    );
    assert_eq!(
        host.operation(reference.operation_id().unwrap())
            .unwrap_err()
            .code,
        "record_missing"
    );
    assert!(host.snapshot().unwrap().active_operations.is_empty());

    let permit = host
        .admit_preparation(&lease, admission(&reference), || Ok(()))
        .unwrap();
    host.retire_local(
        permit.operation_id,
        json!({"fixture":"no process launched"}),
    )
    .unwrap();
    let original = canonical_digest(1, &host.operation(permit.operation_id).unwrap()).unwrap();
    let mut edited = reference.clone();
    edited.phase.retry_grant = Some(Uuid::new_v4());
    assert_eq!(edited.operation_id().unwrap(), permit.operation_id);
    assert_eq!(
        host.admit_preparation(&lease, admission(&edited), || Ok(()))
            .unwrap_err()
            .code,
        "request_conflict"
    );
    assert_eq!(
        canonical_digest(1, &host.operation(permit.operation_id).unwrap()).unwrap(),
        original
    );
}

#[test]
fn preparation_decoder_rejects_transplant_and_mixed_reference_tampering() {
    let (_temp, host, lease) = started();
    let reference = reference();
    let permit = host
        .admit_preparation(&lease, admission(&reference), || Ok(()))
        .unwrap();
    let receipt = host.operation(permit.operation_id).unwrap();
    for field in [
        "operation",
        "request",
        "target",
        "scope",
        "kind",
        "worker",
        "phase",
        "workspace",
        "account",
    ] {
        let mut edited = receipt.clone();
        let mut value = reference.clone();
        match field {
            "operation" => edited.operation_id = Uuid::new_v4(),
            "request" => edited.request_id = Uuid::new_v4(),
            "target" => edited.canonical_targets = vec!["other".into()],
            "scope" => {
                edited.expected.remove("scope");
            }
            "kind" => edited.kind = "linear_comment".into(),
            "worker" => value.worker_operation = Uuid::new_v4(),
            "phase" => value.phase.phase = "not a phase".into(),
            "workspace" => value.workspace = Uuid::nil(),
            "account" => value.account.clear(),
            _ => unreachable!(),
        }
        edited.expected.insert(
            "worker_preparation".into(),
            serde_json::to_string(&value).unwrap(),
        );
        assert!(
            PreparationReference::from_receipt(&edited).is_err(),
            "{field}"
        );
    }
    for key in [
        "run",
        "metadata",
        "continuation",
        "source_advanced",
        "release_resume",
    ] {
        let mut edited = receipt.clone();
        edited.expected.insert(key.into(), "{}".into());
        assert_eq!(
            PreparationReference::from_receipt(&edited)
                .unwrap_err()
                .code,
            "record_corrupt",
            "{key}"
        );
        if key == "run" {
            assert!(RunReference::from_receipt(&edited).is_err());
        }
        if key == "metadata" {
            assert!(MetadataReference::from_receipt(&edited).is_err());
        }
    }
    let mut legacy = receipt;
    legacy.expected.remove("worker_preparation");
    assert!(
        PreparationReference::from_receipt(&legacy)
            .unwrap()
            .is_none()
    );
}
