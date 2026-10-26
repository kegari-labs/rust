//! Explicitly approved component qualification only. The synthetic lease below
//! belongs to this disposable fixture; it does not exercise authenticated Control
//! admission or issue a reusable managed-profile qualification record.
use super::*;
use crate::{
    config::{ManagedExecutable, ManagedExecution, ManagedLifecycle},
    control::Lease,
    platform::BinaryIdentity,
    process_records::{ProcessJournal, RecoveryState, SpawnBinding},
    storage::PrivateDir,
};
use std::sync::Arc;
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires separately authorized native Seatbelt/owned-command qualification"]
async fn managed_foreground_native_exit_output_and_timeout_are_journaled() {
    let temporary = tempfile::tempdir().unwrap();
    // Keep native failure evidence and any uncertain ownership on panic/timeout.
    // Delete only after every actual group, journal and collector has closed.
    let root = temporary.keep().canonicalize().unwrap();
    eprintln!("native managed fixture evidence root: {}", root.display());
    let runs = root.join("runs");
    let repository = runs.join("repository");
    let scratch = runs.join("scratch");
    let protected = root.join("protected");
    for path in [&repository, &scratch, &protected] {
        std::fs::create_dir_all(path).unwrap();
    }
    let sentinel = protected.join("sentinel");
    std::fs::write(&sentinel, b"fixture-protected-value\n").unwrap();
    // Pin the actual shell directly: macOS /bin/sh is a host-selected wrapper.
    // This fixture does not qualify that wrapper or its mutable selector.
    let policy = ManagedExecution {
        lifecycle: ManagedLifecycle::ForegroundV1,
        executables: ["/bin/bash", "/bin/sleep"]
            .into_iter()
            .map(|path| {
                let identity = BinaryIdentity::inspect(Path::new(path), false).unwrap();
                ManagedExecutable {
                    path: identity.path.to_string_lossy().into_owned(),
                    digest: identity.digest,
                }
            })
            .collect(),
        read_roots: vec![],
        generated_executables: None,
    };
    let profile = BoundManagedExecution::resolve(&policy).unwrap();
    let isolation = IsolationPlan {
        schema_version: 1,
        role: ExecutionRole::Validation,
        execution_root: runs,
        cwd: repository.clone(),
        repository,
        scratch: scratch.clone(),
        read_roots: vec![],
        source_writes: vec![],
        resource_writes: vec![],
        forbidden_roots: vec![protected],
        executables: profile.executables(),
        managed_execution: Some(profile),
    }
    .validate()
    .unwrap();
    // Even this exact fixture cannot open the public execution entry.
    assert_eq!(
        repository_command_prerequisite(&isolation)
            .unwrap_err()
            .code,
        "managed_profile_unqualified"
    );
    std::fs::write(
        root.join("seatbelt.sb"),
        isolation.seatbelt_profile().unwrap(),
    )
    .unwrap();
    let runtime = Uuid::new_v4();
    let journal = Arc::new(
        ProcessJournal::new(
            PrivateDir::create(&root.join("processes")).unwrap(),
            runtime,
        )
        .unwrap(),
    );
    let lease = Lease {
        runtime_id: runtime,
        owner_id: Uuid::new_v4(),
        owner_generation: 1,
    };
    let cases = [
        (
            vec![
                "/bin/bash".to_owned(),
                "--noprofile".to_owned(),
                "--norc".to_owned(),
                "--posix".to_owned(),
                "-c".to_owned(),
                "printf 'managed fixture-secret\\n'; printf 'diagnostic\\n' >&2; printf 'allowed' > \"$HOME/result\"; if (read value < \"$1\") 2>/dev/null; then exit 91; fi; if (printf changed > \"$1\") 2>/dev/null; then exit 92; fi; exit 0".to_owned(),
                "fixture".to_owned(),
                sentinel.to_string_lossy().into_owned(),
            ],
            Duration::from_secs(2),
            Some(0),
            false,
        ),
        (
            vec!["/bin/bash".into(), "--noprofile".into(), "--norc".into(), "--posix".into(), "-c".into(), "exit 7".into()],
            Duration::from_secs(2),
            Some(7),
            false,
        ),
        (
            // Natural expiry bounds the helper even if signalling fails.
            vec!["/bin/sleep".into(), "5".into()],
            Duration::from_millis(100),
            None,
            true,
        ),
    ];
    for (index, (argv, timeout, expected_exit, timed_out)) in cases.into_iter().enumerate() {
        let operation_id = Uuid::new_v4();
        let launch = command_launch_identity(&isolation, &argv).unwrap();
        std::fs::write(
            root.join(format!("case-{index}-launch.json")),
            serde_json::to_vec_pretty(&serde_json::json!({
                "argv": argv, "cwd": launch.cwd, "launcher": launch.launcher,
                "target": launch.target, "argv_digest": launch.argv_digest,
                "isolation_digest": isolation.digest,
            }))
            .unwrap(),
        )
        .unwrap();
        let guard = journal
            .begin(SpawnBinding {
                operation_id,
                effect_id: Uuid::new_v4(),
                lease: Some(lease.clone()),
                control_owner: None,
                check_bundle: None,
                work_key: format!("native-managed-fixture-{index}"),
                scope_digest: isolation.digest.clone(),
                cwd: launch.cwd,
                executable: launch.launcher,
                observed_executables: vec![launch.target],
                argv_digest: launch.argv_digest,
                kind: "repository_command".into(),
            })
            .unwrap();
        let command = spawn_command_inner(
            &isolation,
            &argv,
            timeout,
            1024,
            &["fixture-secret".into()],
            Some(CommandJournal::Guard(Box::new(guard))),
        )
        .unwrap();
        let cancellation = command.cancellation();
        let waited = tokio::time::timeout(Duration::from_secs(9), command.wait()).await;
        std::fs::write(
            root.join(format!("case-{index}-wait.json")),
            serde_json::to_vec_pretty(&serde_json::json!({
                "wait": waited.as_ref().map_err(|_| "independent case timeout"),
                "cancellation_exited": cancellation.exited(),
                "processes": journal.observe(operation_id),
            }))
            .unwrap(),
        )
        .unwrap();
        let outcome = waited
            .expect("native component case exceeded its independent bound")
            .unwrap();
        assert_eq!(outcome.exit_code, expected_exit);
        assert_eq!(
            outcome.termination_signal,
            (index == 2).then_some(libc::SIGKILL)
        );
        assert_eq!(outcome.timed_out, timed_out);
        assert!(!outcome.output_truncated);
        assert!(cancellation.exited().unwrap());
        if index == 0 {
            assert_eq!(outcome.stdout, "managed [REDACTED]\n");
            assert_eq!(outcome.stderr, "diagnostic\n");
            assert_eq!(std::fs::read(scratch.join("result")).unwrap(), b"allowed");
            assert_eq!(
                std::fs::read(&sentinel).unwrap(),
                b"fixture-protected-value\n"
            );
        }
        let observed = journal.observe(operation_id).unwrap();
        assert_eq!(observed.len(), 1);
        assert_eq!(observed[0].state, RecoveryState::OwnedGroupExitRecorded);
        assert!(observed[0].owned_group_exited);
        assert!(!observed[0].all_descendants_exited);
    }
    std::fs::remove_dir_all(root).unwrap();
}
