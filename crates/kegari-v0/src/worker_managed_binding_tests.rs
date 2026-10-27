//! Actual file-backed automatic-domain checks only; no process or worker is launched.
use super::*;
use crate::isolation::{BoundManagedExecution, IsolationPlan, PathGrant};

fn bound_fixture() -> (tempfile::TempDir, ValidatedIsolation) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let execution = root.join("execution");
    let repository = execution.join("repository");
    let scratch = execution.join("scratch");
    let resource = root.join("admitted-resource");
    let protected = root.join("control");
    for path in [&repository, &scratch, &resource, &protected] {
        std::fs::create_dir_all(path).unwrap();
    }
    std::fs::write(resource.join("input"), b"actual admitted fixture input\n").unwrap();
    let mut plan = IsolationPlan {
        schema_version: 1,
        role: ExecutionRole::Child,
        execution_root: execution,
        repository: repository.clone(),
        cwd: repository.clone(),
        scratch,
        read_roots: vec![
            PathGrant {
                path: repository,
                directory: true,
            },
            PathGrant {
                path: resource,
                directory: true,
            },
        ],
        source_writes: vec![],
        resource_writes: vec![],
        forbidden_roots: vec![protected],
        managed_execution: None,
    };
    plan.managed_execution = Some(BoundManagedExecution::automatic(&plan).unwrap());
    (temp, plan.validate().unwrap())
}

#[test]
fn automatic_domain_rejects_omission_and_nonexecuting_contract_use() {
    let (_temp, isolation) = bound_fixture();
    verify_command_domain(&isolation, true).unwrap();
    // Both constructors require the actual domain for execution; supplying
    // one to a nonexecuting contract remains a mismatch.
    assert_eq!(
        verify_command_domain(&isolation, false).unwrap_err().code,
        "worker_binding"
    );
    let mut omitted = isolation.plan().clone();
    omitted.managed_execution = None;
    let omitted = omitted.validate().unwrap();
    assert_eq!(
        verify_command_domain(&omitted, true).unwrap_err().code,
        "worker_binding"
    );
    verify_command_domain(&omitted, false).unwrap();
    assert_eq!(
        isolation
            .plan()
            .managed_execution
            .as_ref()
            .unwrap()
            .policy_digest(),
        BoundManagedExecution::automatic_policy_digest().unwrap()
    );
}

#[test]
fn automatic_domain_rejects_other_owned_roots_and_nonexecuting_roles() {
    let (_first, original) = bound_fixture();
    let (_second, other) = bound_fixture();
    let mut substituted = original.plan().clone();
    substituted.managed_execution = other.plan().managed_execution.clone();
    // A self-consistent domain for another actual directory cannot bind this plan.
    assert!(substituted.validate().is_err());
    for role in [
        ExecutionRole::Coordination,
        ExecutionRole::IndependentReview,
    ] {
        let mut changed = original.plan().clone();
        changed.role = role;
        assert!(changed.clone().validate().is_err());
        changed.managed_execution = None;
        assert!(BoundManagedExecution::automatic(&changed).is_err());
    }
}

#[test]
fn automatic_domain_revalidates_original_admitted_resource_identity() {
    let (_temp, isolation) = bound_fixture();
    verify_command_domain(&isolation, true).unwrap();
    let resource = &isolation.plan().read_roots[1].path;
    let retained = resource.with_file_name("retained-original-resource");
    std::fs::rename(resource, &retained).unwrap();
    std::fs::create_dir(resource).unwrap();
    std::fs::write(resource.join("input"), b"actual admitted fixture input\n").unwrap();
    // Same pathname and bytes do not replace the original observed directory.
    assert!(verify_command_domain(&isolation, true).is_err());
    assert!(retained.join("input").is_file());
}
