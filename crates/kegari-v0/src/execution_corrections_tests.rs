//! Actual signed correction lineage + deterministic command boundary. The
//! resulting records deliberately remain ineligible as production check proof.
use super::*;
use crate::isolation::{IsolationPlan, PathGrant};
use std::{fs, os::unix::fs::PermissionsExt, process::Command, sync::Mutex};

fn run(binary: &Path, root: &Path, args: &[&str]) {
    let result = kegari_process::group::OwnedGroup::spawn_std(
        Command::new(binary)
            .current_dir(root)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_AUTHOR_NAME", "Disposable Fixture Human")
            .env("GIT_COMMITTER_NAME", "Disposable Fixture Human")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
            .args([
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "commit.gpgSign=false",
            ])
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped()),
    )
    .expect("spawn owned disposable fixture")
    .0
    .wait_with_output()
    .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}
struct Boundary {
    output: PathBuf,
    calls: Mutex<usize>,
}
struct Cancel;
impl CommandCancellation for Cancel {
    fn cancel(&self) -> Result<()> {
        Ok(())
    }
}
struct Started(CommandOutcome);
impl StartedCommand for Started {
    fn cancellation(&self) -> Arc<dyn CommandCancellation> {
        Arc::new(Cancel)
    }
    fn wait(self: Box<Self>) -> Pin<Box<dyn Future<Output = Result<CommandOutcome>> + Send>> {
        Box::pin(async move { Ok(self.0) })
    }
}
impl CommandBoundary for Boundary {
    fn evidence_class(&self) -> ExecutionEvidenceClass {
        ExecutionEvidenceClass::DeterministicFixture
    }
    fn start(
        &self,
        isolation: &ValidatedIsolation,
        argv: &[String],
        _: Duration,
        _: usize,
    ) -> Result<Box<dyn StartedCommand>> {
        let mut calls = self.calls.lock().unwrap();
        *calls += 1;
        fs::write(&self.output, format!("{{\"fixture_run\":{calls}}}")).unwrap();
        Ok(Box::new(Started(CommandOutcome {
            schema_version: 1,
            argv: argv.to_vec(),
            cwd: isolation.plan().cwd.clone(),
            isolation_digest: isolation.digest().into(),
            exit_code: Some(0),
            termination_signal: None,
            timed_out: false,
            stdout: String::new(),
            stderr: String::new(),
            output_truncated: false,
            elapsed_ms: 1,
        })))
    }
}
#[tokio::test]
async fn native_correction_check_keeps_original_marker_and_historical_evidence() {
    let (_correction_files, transition) = crate::corrections::test_transition_fixture().await;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let system_git = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|p| p.join("git"))
        .find(|p| p.is_file())
        .unwrap()
        .canonicalize()
        .unwrap();
    let tools = root.join("tools");
    fs::create_dir(&tools).unwrap();
    fs::set_permissions(&tools, fs::Permissions::from_mode(0o700)).unwrap();
    let binary = tools.join("git");
    fs::copy(system_git, &binary).unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o555)).unwrap();
    let operator = root.join("operator");
    fs::create_dir(&operator).unwrap();
    run(&binary, &operator, &["init", "-b", "main"]);
    fs::create_dir(operator.join("tests")).unwrap();
    fs::write(operator.join("tests/a.txt"), "original\n").unwrap();
    run(&binary, &operator, &["add", "tests/a.txt"]);
    run(
        &binary,
        &operator,
        &[
            "commit",
            "--signoff",
            "-m",
            "test: initialize disposable correction fixture",
        ],
    );
    let execution = root.join("execution");
    fs::create_dir(&execution).unwrap();
    let repository = execution.join("repository");
    run(
        &binary,
        &execution,
        &[
            "clone",
            "--local",
            "--no-hardlinks",
            operator.to_str().unwrap(),
            repository.to_str().unwrap(),
        ],
    );
    fs::set_permissions(&repository, fs::Permissions::from_mode(0o700)).unwrap();
    let scratch = execution.join("scratch");
    fs::create_dir(&scratch).unwrap();
    let mut plan = IsolationPlan {
        managed_execution: None,
        schema_version: 1,
        role: ExecutionRole::Child,
        execution_root: execution,
        repository: repository.clone(),
        cwd: repository.clone(),
        scratch: scratch.clone(),
        read_roots: vec![PathGrant {
            path: repository.clone(),
            directory: true,
        }],
        source_writes: vec![PathGrant {
            path: repository.join("tests"),
            directory: true,
        }],
        resource_writes: vec![],
        forbidden_roots: vec![operator.clone()],
    };
    plan.managed_execution =
        Some(crate::isolation::BoundManagedExecution::automatic(&plan).unwrap());
    let isolation = plan.validate().unwrap();
    let git = Git::new(binary).unwrap();
    let original = ExecutionBinding {
        runtime_id: Uuid::from_u128(1),
        operation_id: Uuid::from_u128(903),
        work: transition.original_work().clone(),
        repository: RepositoryId {
            hostname: "github.com".into(),
            id: transition
                .original_authority()
                .child_scope()
                .unwrap()
                .write_repository
                .clone()
                .unwrap(),
        },
        revision: git.revision(&repository, "HEAD").unwrap(),
        candidate_vector_digest: None,
        isolation_digest: isolation.digest().into(),
    };
    let ownership = OwnedDirectory::register(
        DirectoryIdentity {
            schema_version: 1,
            runtime_id: original.runtime_id,
            directory_id: Uuid::from_u128(904),
            repository: original.repository.clone(),
            work: original.work.clone(),
            repository_path: repository,
            operator_checkout: operator,
        },
        &isolation,
    )
    .unwrap();
    let marker = fs::read(ownership.marker()).unwrap();
    let effective =
        EffectiveExecutionBinding::new(&original, &ownership, &isolation, transition.clone())
            .unwrap();
    assert_eq!(effective.original(), &original);
    assert_eq!(effective.binding().work, *transition.effective_work());
    let workflow = Workflow::parse(&format!("---\nchecks:\n  local:\n    unit:\n      argv: ['/usr/bin/true']\n      result_artifact: '{}'\n---\nExecute the named check\n",scratch.join("unit.json").display()),root.join("WORKFLOW.md")).unwrap();
    let runner = Boundary {
        output: scratch.join("unit.json"),
        calls: Mutex::new(0),
    };
    let executor = LocalExecutor {
        isolation: &isolation,
        git: &git,
        boundary: &runner,
        limits: ExecutionLimits::default(),
        ownership: &ownership,
    };
    let historical = executor
        .run_check(
            CheckInvocation {
                binding: &original,
                workflow: &workflow,
                name: "unit",
                kind: CheckKind::Local,
                additional_outputs: &[],
                previous_outputs: &[],
            },
            |_| Ok(()),
        )
        .await
        .unwrap();
    let retained = historical
        .retain_outputs(
            &isolation,
            &kegari_storage::PrivateDir::create(&root.join("retained")).unwrap(),
        )
        .unwrap();
    let invocation = || CheckInvocation {
        binding: effective.binding(),
        workflow: &workflow,
        name: "unit",
        kind: CheckKind::Local,
        additional_outputs: &[],
        previous_outputs: &retained,
    };
    assert_eq!(
        executor
            .prepare_check(invocation(), |_| Ok(()))
            .unwrap_err()
            .code,
        "execution_ownership"
    );
    let mut journal = Vec::new();
    let prepared = executor
        .prepare_effective_check(invocation(), &effective, |entry| {
            journal.push(serde_json::to_value(entry).unwrap());
            Ok(())
        })
        .unwrap();
    let running = executor.start_check(prepared).unwrap();
    let CheckCompletion::Passed(corrected) = executor
        .complete_check(running, |entry| {
            journal.push(serde_json::to_value(entry).unwrap());
            Ok(())
        })
        .await
        .unwrap()
    else {
        panic!("fixture check failed")
    };
    assert_eq!(corrected.record().schema_version, 2);
    assert_eq!(
        corrected.record().binding.work,
        *transition.effective_work()
    );
    assert_eq!(
        corrected.record().correction.as_ref().unwrap(),
        &effective.provenance().unwrap()
    );
    assert_eq!(
        corrected.effective_binding().unwrap().transition().digest(),
        transition.digest()
    );
    assert_eq!(historical.record().schema_version, 1);
    assert_eq!(historical.record().binding.work, original.work);
    assert!(historical.record().correction.is_none());
    assert_eq!(fs::read(ownership.marker()).unwrap(), marker);
    assert_eq!(ownership.identity().work, original.work);
    assert_eq!(*runner.calls.lock().unwrap(), 2);
    assert_eq!(journal.len(), 2);
    assert_eq!(
        corrected.production_evidence().unwrap_err().code,
        "unqualified_fixture"
    );
    let mut changed = effective.binding().clone();
    changed.revision.tree = "a".repeat(40);
    assert!(
        executor
            .prepare_effective_check(
                CheckInvocation {
                    binding: &changed,
                    ..invocation()
                },
                &effective,
                |_| Ok(())
            )
            .is_err()
    );
    // Generic preparation consumes the same actual signed transition. This is
    // a no-dispatch component assertion, not a corrected-worker/native launch.
    let request = CommandRequest {
        argv: vec!["/usr/bin/true".into()],
        cwd: ".".into(),
    };
    assert!(
        executor
            .prepare_command(
                &original,
                &request,
                Uuid::new_v4(),
                Some(&effective),
                |_| Ok(())
            )
            .is_err()
    );
    assert!(
        executor
            .prepare_command(effective.binding(), &request, Uuid::new_v4(), None, |_| Ok(
                ()
            ))
            .is_err()
    );
    let mut command_events = Vec::new();
    let prepared = executor
        .prepare_command(
            effective.binding(),
            &request,
            Uuid::new_v4(),
            Some(&effective),
            |entry| {
                command_events.push(entry.clone());
                Ok(())
            },
        )
        .unwrap();
    executor.preflight_command(&prepared).unwrap();
    executor
        .refuse_command(
            prepared,
            Error::new("fixture_no_dispatch", "component ends before launch"),
            |entry| {
                command_events.push(entry.clone());
                Ok(())
            },
        )
        .unwrap();
    let CommandJournalEntry::Prepared {
        preparation,
        operation_id,
        ..
    } = &command_events[0]
    else {
        panic!("expected actual preparation")
    };
    assert_eq!(*operation_id, original.operation_id);
    assert_eq!(preparation.original_binding, original);
    assert_eq!(&preparation.binding, effective.binding());
    assert_eq!(
        preparation.correction.as_ref(),
        Some(&effective.provenance().unwrap())
    );
    assert!(matches!(
        command_events[1],
        CommandJournalEntry::NotStarted { .. }
    ));
    assert_eq!(*runner.calls.lock().unwrap(), 2);
    assert_eq!(fs::read(ownership.marker()).unwrap(), marker);
    assert!(
        EffectiveExecutionBinding::new(effective.binding(), &ownership, &isolation, transition)
            .is_err()
    );
}
