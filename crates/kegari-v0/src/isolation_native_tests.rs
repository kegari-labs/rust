//! Explicitly approved component qualification only. The synthetic lease below
//! belongs to this disposable fixture; it does not exercise authenticated Control
//! admission or issue a reusable managed-profile qualification record.
use super::*;
use crate::{
    control::Lease,
    process_records::{ProcessJournal, RecoveryState, SpawnBinding},
};
use kegari_storage::PrivateDir;
use std::sync::Arc;
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires separately authorized native Seatbelt/owned-command qualification"]
async fn managed_foreground_native_exit_output_and_timeout_are_journaled() {
    let temporary = tempfile::tempdir().unwrap();
    // Retain bounded evidence on success as well as failure. The test harness's
    // one-line result is not a substitute for the actual per-case observations.
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
    let mut plan = IsolationPlan {
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
        managed_execution: None,
    };
    plan.managed_execution =
        Some(BoundManagedExecution::automatic(&plan, crate::BUILD_ID).unwrap());
    let isolation = plan.validate(crate::BUILD_ID).unwrap();
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
        let effect_id = Uuid::new_v4();
        let launch = command_launch_identity(&isolation, &argv).unwrap();
        std::fs::write(
            root.join(format!("case-{index}-launch.json")),
            serde_json::to_vec_pretty(&serde_json::json!({
                "operation_id": operation_id, "effect_id": effect_id,
                "argv": argv, "cwd": launch.cwd, "launcher": launch.launcher,
                "target": launch.target, "argv_digest": launch.argv_digest,
                "isolation_digest": isolation.digest(),
            }))
            .unwrap(),
        )
        .unwrap();
        let guarded = journal.begin(SpawnBinding {
            operation_id,
            effect_id,
            lease: Some(lease.clone()),
            control_owner: None,
            check_bundle: None,
            work_key: format!("native-managed-fixture-{index}"),
            scope_digest: isolation.digest().to_owned(),
            cwd: launch.cwd,
            executable: launch.launcher,
            observed_executables: vec![launch.target],
            argv_digest: launch.argv_digest,
            kind: "repository_command".into(),
        });
        std::fs::write(
            root.join(format!("case-{index}-registration.json")),
            serde_json::to_vec_pretty(&guarded.as_ref().map(|_| "registered")).unwrap(),
        )
        .unwrap();
        let guard = guarded.unwrap();
        let started = spawn_command_inner(
            &isolation,
            &argv,
            timeout,
            1024,
            &["fixture-secret".into()],
            Some(CommandJournal::Guard(Box::new(guard))),
        );
        // A diagnostic failure must not panic while the returned command is
        // still live. Defer its assertion until the original waiter settles or
        // the explicitly bounded cleanup observation has completed.
        let start_record = serde_json::to_vec_pretty(&started.as_ref().map(|_| "started"))
            .map_err(|error| error.to_string())
            .and_then(|bytes| {
                std::fs::write(root.join(format!("case-{index}-start.json")), bytes)
                    .map_err(|error| error.to_string())
            });
        let command = started.unwrap();
        let cancellation = command.cancellation();
        let mut waiter = Box::pin(command.wait());
        let waited = tokio::time::timeout(Duration::from_secs(9), &mut waiter).await;
        // A case observation timeout is always a failure. Keep its original
        // waiter alive for bounded cancellation/join observation before runtime
        // teardown; an expired cleanup observation still means unknown closure.
        let cleanup = if waited.is_err() {
            let requested = cancellation.cancel();
            let settled = tokio::time::timeout(Duration::from_secs(2), &mut waiter).await;
            Some(serde_json::json!({
                "cancellation_request": requested,
                "original_waiter": settled.as_ref().map_err(|_| "cleanup closure unknown"),
            }))
        } else {
            None
        };
        std::fs::write(
            root.join(format!("case-{index}-wait.json")),
            serde_json::to_vec_pretty(&serde_json::json!({
                "wait": waited.as_ref().map_err(|_| "independent case timeout"),
                "timeout_cleanup": cleanup,
                "cancellation_exited": cancellation.exited(),
                "processes": journal.observe(operation_id),
            }))
            .unwrap(),
        )
        .unwrap();
        start_record.unwrap();
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
    std::fs::write(
        root.join("completed.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "cases_passed": 3,
            "qualification_issued": false,
            "scope": "fixed disposable foreground component cases only",
        }))
        .unwrap(),
    )
    .unwrap();
}
