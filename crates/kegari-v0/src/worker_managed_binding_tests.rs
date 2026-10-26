//! Actual file-backed declaration checks only; no process or worker is launched.
use super::*;
use crate::config::{
    ManagedExecutable, ManagedExecution, ManagedGeneratedExecutables, ManagedLifecycle,
    ManagedReadRoot,
};
use crate::isolation::{BoundManagedExecution, IsolationPlan, PathGrant};
use std::os::unix::fs::PermissionsExt;

fn bound_fixture() -> (tempfile::TempDir, ManagedExecution, ValidatedIsolation) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let execution = root.join("execution");
    let repository = execution.join("repository");
    let scratch = execution.join("scratch");
    let tool_reads = root.join("tool-support");
    let protected = root.join("control");
    for path in [&repository, &scratch, &tool_reads, &protected] {
        std::fs::create_dir_all(path).unwrap();
    }
    let tool = root.join("fixture-tool");
    let bytes = b"component tool bytes; never executed\n";
    std::fs::write(&tool, bytes).unwrap();
    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o700)).unwrap();
    let policy = ManagedExecution {
        lifecycle: ManagedLifecycle::ForegroundV1,
        executables: vec![ManagedExecutable {
            path: tool.to_str().unwrap().into(),
            digest: bytes_digest(bytes),
        }],
        read_roots: vec![ManagedReadRoot {
            path: tool_reads.to_str().unwrap().into(),
            directory: true,
        }],
        generated_executables: None,
    };
    let bound = BoundManagedExecution::resolve(&policy).unwrap();
    let isolation = IsolationPlan {
        schema_version: 1,
        role: ExecutionRole::Child,
        execution_root: execution,
        repository: repository.clone(),
        cwd: repository.clone(),
        scratch,
        read_roots: vec![PathGrant {
            path: repository,
            directory: true,
        }],
        source_writes: vec![],
        resource_writes: vec![],
        forbidden_roots: vec![protected],
        executables: bound.executables(),
        managed_execution: Some(bound),
    }
    .validate()
    .unwrap();
    (temp, policy, isolation)
}

#[test]
fn configured_profile_rejects_omission_substitution_and_unconfigured_use() {
    let (_temp, policy, isolation) = bound_fixture();
    verify_managed_policy(&isolation, Some(&policy)).unwrap();
    // The public legacy constructor uses this same None expectation.
    assert_eq!(
        verify_managed_policy(&isolation, None).unwrap_err().code,
        "worker_binding"
    );
    let mut omitted = isolation.plan().clone();
    omitted.managed_execution = None;
    let omitted = omitted.validate().unwrap();
    assert_eq!(
        verify_managed_policy(&omitted, Some(&policy))
            .unwrap_err()
            .code,
        "worker_binding"
    );
    verify_managed_policy(&omitted, None).unwrap();
    let mut changed = policy.clone();
    changed.generated_executables = Some(ManagedGeneratedExecutables::ScratchBuildV1);
    assert_eq!(
        verify_managed_policy(&isolation, Some(&changed))
            .unwrap_err()
            .code,
        "worker_binding"
    );
    assert!(
        !isolation
            .plan()
            .read_roots
            .iter()
            .any(|grant| grant.path == Path::new(&policy.read_roots[0].path))
    );
    assert_eq!(
        crate::isolation::repository_command_prerequisite(&isolation)
            .unwrap_err()
            .code,
        "managed_profile_unqualified"
    );
}

#[test]
fn configured_profile_revalidates_original_tool_bytes() {
    let (_temp, policy, isolation) = bound_fixture();
    verify_managed_policy(&isolation, Some(&policy)).unwrap();
    std::fs::write(&policy.executables[0].path, b"substituted tool bytes\n").unwrap();
    assert_eq!(
        verify_managed_policy(&isolation, Some(&policy))
            .unwrap_err()
            .code,
        "identity_mismatch"
    );
}
