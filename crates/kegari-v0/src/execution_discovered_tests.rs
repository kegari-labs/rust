//! Actual owned source/enrollment/executor components with an injected command
//! result. No test manufactures confined execution or live validation authority.
use super::*;
use crate::evidence::{Generation, WorkIdentity};
use crate::isolation::{BoundManagedExecution, IsolationPlan, PathGrant};
use crate::test_pinned_git as pinned_git;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

fn git_run(binary: &Path, cwd: &Path, arguments: &[&str]) {
    let output = kegari_process::group::OwnedGroup::spawn_std(
        std::process::Command::new(binary)
            .current_dir(cwd)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_AUTHOR_NAME", "Disposable Fixture Human")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
            .env("GIT_COMMITTER_NAME", "Disposable Fixture Human")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
            .args([
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "commit.gpgSign=false",
            ])
            .args(arguments)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped()),
    )
    .unwrap()
    .0
    .wait_with_output()
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
struct Fixture {
    _temp: tempfile::TempDir,
    git: Git,
    isolation: ValidatedIsolation,
    binding: ExecutionBinding,
    owner: OwnedDirectory,
}
impl Fixture {
    fn new() -> Self {
        Self::with_scope(false)
    }
    fn integrated() -> Self {
        Self::with_scope(true)
    }
    fn with_scope(integrated: bool) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let binary = pinned_git::prepare(&root);
        let operator = root.join("operator");
        std::fs::create_dir(&operator).unwrap();
        git_run(&binary, &operator, &["init", "-b", "main"]);
        std::fs::write(operator.join("source.txt"), b"actual admitted source\n").unwrap();
        git_run(&binary, &operator, &["add", "source.txt"]);
        git_run(
            &binary,
            &operator,
            &[
                "commit",
                "--signoff",
                "-m",
                "test: initialize discovered validation fixture",
            ],
        );
        let execution = root.join("execution");
        std::fs::create_dir(&execution).unwrap();
        let repository = execution.join("repository");
        git_run(
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
        let scratch = execution.join("scratch");
        std::fs::create_dir(&scratch).unwrap();
        let mut plan = IsolationPlan {
            schema_version: 1,
            role: if integrated {
                ExecutionRole::Validation
            } else {
                ExecutionRole::Child
            },
            execution_root: execution,
            repository: repository.clone(),
            cwd: repository.clone(),
            scratch,
            read_roots: vec![PathGrant {
                path: repository.clone(),
                directory: true,
            }],
            source_writes: vec![PathGrant {
                path: repository.join("source.txt"),
                directory: false,
            }],
            resource_writes: vec![],
            forbidden_roots: vec![operator.clone()],
            managed_execution: None,
        };
        if integrated {
            // Actual read-only bytes model the separately retained repositories
            // granted to an E2E worker. The vector digest below is component
            // data, not a reconstructed parent-vector capability.
            let candidate = root.join("other-candidate");
            std::fs::create_dir(&candidate).unwrap();
            std::fs::write(candidate.join("input.txt"), b"original integrated input\n").unwrap();
            plan.read_roots.push(PathGrant {
                path: candidate,
                directory: true,
            });
            plan.source_writes.clear();
        }
        plan.managed_execution = Some(BoundManagedExecution::automatic(&plan).unwrap());
        let isolation = plan.validate().unwrap();
        let git = Git::new(binary).unwrap();
        let digest = bytes_digest(b"component generation");
        let binding = ExecutionBinding {
            runtime_id: Uuid::new_v4(),
            operation_id: Uuid::new_v4(),
            work: WorkIdentity {
                linear_endpoint: "https://api.linear.app/graphql".into(),
                linear_workspace: Uuid::new_v4(),
                parent: Uuid::new_v4(),
                child: Some(Uuid::new_v4()),
                generation: Generation {
                    authorization: digest.clone(),
                    definition: digest.clone(),
                    correction: digest.clone(),
                    scope: digest,
                },
            },
            repository: RepositoryId {
                hostname: "github.com".into(),
                id: "101".into(),
            },
            revision: git.revision(&repository, "HEAD").unwrap(),
            candidate_vector_digest: integrated
                .then(|| bytes_digest(b"component candidate vector; not native authority")),
            isolation_digest: isolation.digest().into(),
        };
        let owner = OwnedDirectory::register(
            DirectoryIdentity {
                schema_version: 1,
                runtime_id: binding.runtime_id,
                directory_id: Uuid::new_v4(),
                repository: binding.repository.clone(),
                work: binding.work.clone(),
                repository_path: repository,
                operator_checkout: operator,
            },
            &isolation,
        )
        .unwrap();
        Self {
            _temp: temp,
            git,
            isolation,
            binding,
            owner,
        }
    }
    fn executor<'a>(&'a self, boundary: &'a dyn CommandBoundary) -> LocalExecutor<'a> {
        LocalExecutor {
            isolation: &self.isolation,
            git: &self.git,
            boundary,
            limits: ExecutionLimits::default(),
            ownership: &self.owner,
        }
    }
}
#[derive(Default)]
struct Boundary {
    calls: AtomicUsize,
    exit: i32,
    writes: Vec<(PathBuf, Vec<u8>)>,
    refuse_closure: bool,
}
struct Cancellation(Arc<AtomicBool>);
impl CommandCancellation for Cancellation {
    fn cancel(&self) -> Result<()> {
        Ok(())
    }
    fn exited(&self) -> Result<bool> {
        Ok(self.0.load(Ordering::SeqCst))
    }
}
struct Started {
    outcome: CommandOutcome,
    closed: Arc<AtomicBool>,
    refuse_closure: bool,
}
impl StartedCommand for Started {
    fn cancellation(&self) -> Arc<dyn CommandCancellation> {
        Arc::new(Cancellation(self.closed.clone()))
    }
    fn wait(self: Box<Self>) -> Pin<Box<dyn Future<Output = Result<CommandOutcome>> + Send>> {
        Box::pin(async move {
            self.closed.store(!self.refuse_closure, Ordering::SeqCst);
            Ok(self.outcome)
        })
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
        self.calls.fetch_add(1, Ordering::SeqCst);
        for (path, bytes) in &self.writes {
            std::fs::write(path, bytes).unwrap();
        }
        Ok(Box::new(Started {
            outcome: CommandOutcome {
                schema_version: 1,
                argv: argv.to_vec(),
                cwd: isolation.plan().cwd.clone(),
                isolation_digest: isolation.digest().into(),
                exit_code: Some(self.exit),
                termination_signal: None,
                timed_out: false,
                stdout: "actual injected stdout\n".into(),
                stderr: String::new(),
                output_truncated: false,
                elapsed_ms: 1,
            },
            closed: Arc::new(AtomicBool::new(false)),
            refuse_closure: self.refuse_closure,
        }))
    }
}
#[test]
fn validation_source_never_excludes_untracked_or_dirty_candidate_content() {
    let f = Fixture::new();
    let boundary = Boundary::default();
    let executor = f.executor(&boundary);
    let source = executor.validation_source(&f.binding, None).unwrap();
    assert_eq!(source.revision, f.binding.revision);
    std::fs::write(
        f.isolation.plan().repository.join("untracked-report.json"),
        b"{}",
    )
    .unwrap();
    assert!(executor.validation_source(&f.binding, None).is_err());
    std::fs::remove_file(f.isolation.plan().repository.join("untracked-report.json")).unwrap();
    std::fs::write(
        f.isolation.plan().repository.join("source.txt"),
        b"dirty source\n",
    )
    .unwrap();
    assert!(executor.validation_source(&f.binding, None).is_err());
    assert_eq!(boundary.calls.load(Ordering::SeqCst), 0);
}

fn requirement() -> crate::validation::ValidationRequirement {
    crate::validation::ValidationRequirement {
        id: "discovered-obligation".into(),
        statement: "Observe this fixture candidate".into(),
        rationale:
            "Component verifies generic invocation and observed transcript, not project semantics"
                .into(),
        argv: vec!["/usr/bin/true".into(), "literal $(not-a-shell)".into()],
        cwd: ".".into(),
        services: vec![],
        artifacts: vec![],
    }
}
fn enroll(
    f: &Fixture,
    executor: &LocalExecutor<'_>,
    requirement: crate::validation::ValidationRequirement,
) -> ValidationPlan {
    use crate::validation::{
        DiscoveryJournalReference, DiscoveryReadContent, DiscoveryReadReference,
        ValidationEnrollment, ValidationProposal,
    };
    let source = executor.validation_source(&f.binding, None).unwrap();
    let path = f.isolation.plan().repository.join("source.txt");
    let bytes = std::fs::read(&path).unwrap();
    let digest = bytes_digest(b"synthetic component instruction/context identity");
    // Retain an actual read and protected record. The fixed instruction/turn
    // labels are component data, not a manufactured native worker enrollment.
    let records = kegari_storage::PrivateDir::create(
        &f._temp
            .path()
            .canonicalize()
            .unwrap()
            .join(format!("read-{}", Uuid::new_v4())),
    )
    .unwrap();
    let event = serde_json::json!({"path":path,"digest":bytes_digest(&bytes),"bytes":bytes.len()});
    records.write("event-00000000.json", &event).unwrap();
    let reference = DiscoveryReadReference {
        schema_version: 1,
        operation_id: f.binding.operation_id,
        call_id: Uuid::new_v4(),
        turn: "component-turn".into(),
        instruction_digest: digest.clone(),
        correction_input_digest: digest.clone(),
        result_digest: canonical_digest(1, &event).unwrap(),
        journal: DiscoveryJournalReference {
            leaf: "event-00000000.json".into(),
            digest: canonical_digest(1, &event).unwrap(),
            bytes: serde_json::to_vec(&event).unwrap().len() as u64,
        },
        content: DiscoveryReadContent::Source {
            requested_path: "source.txt".into(),
            path,
            digest: bytes_digest(&bytes),
            bytes: bytes.len() as u64,
        },
    };
    ValidationPlan::enroll(
        ValidationEnrollment {
            original_binding: f.binding.clone(),
            binding: f.binding.clone(),
            correction: None,
            kind: if f.binding.candidate_vector_digest.is_some() {
                CheckKind::Integrated
            } else {
                CheckKind::Local
            },
            workflow_digest: bytes_digest(b"generic workflow; no named checks"),
            instruction_digest: digest.clone(),
            tool_contract_digest: digest.clone(),
            context_digest: digest,
            source_digest: source.digest().unwrap(),
        },
        ValidationProposal {
            previous: None,
            reason: "Initial component discovery from actual source".into(),
            supporting_reads: vec![reference.digest().unwrap()],
            requirements: vec![requirement],
        },
        vec![reference],
        None,
    )
    .unwrap()
}

#[tokio::test]
async fn stdout_only_discovered_check_retains_actual_transcript_plan_and_fixture_class() {
    let f = Fixture::new();
    std::fs::create_dir(f.isolation.plan().repository.join("nested")).unwrap();
    let boundary = Boundary::default();
    let executor = f.executor(&boundary);
    let mut requested = requirement();
    requested.cwd = "nested".into();
    let plan = enroll(&f, &executor, requested);
    let invocation = Uuid::new_v4();
    let mut journal = Vec::new();
    let prepared = executor
        .prepare_discovered_check(
            &f.binding,
            &plan,
            "discovered-obligation",
            invocation,
            &[],
            None,
            |event| {
                journal.push(serde_json::to_value(event).unwrap());
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(boundary.calls.load(Ordering::SeqCst), 0);
    let running = executor.start_check(prepared).unwrap();
    let CheckCompletion::Passed(passed) = executor
        .complete_check(running, |event| {
            journal.push(serde_json::to_value(event).unwrap());
            Ok(())
        })
        .await
        .unwrap()
    else {
        panic!("component command did not pass mechanical checks")
    };
    assert_eq!(passed.record().schema_version, 4);
    assert_eq!(passed.validation_plan().unwrap().digest(), plan.digest());
    assert_eq!(passed.record().before, passed.record().after);
    assert_eq!(passed.record().command.argv, requirement().argv);
    assert_eq!(
        passed.record().command.cwd,
        f.isolation.plan().repository.join("nested")
    );
    assert_ne!(
        passed.record().command.isolation_digest,
        f.binding.isolation_digest
    );
    assert_eq!(passed.record().artifacts.len(), 1);
    passed.record().validate_shape().unwrap();
    assert_eq!(
        passed.production_evidence().unwrap_err().code,
        "unqualified_fixture"
    );
    let provenance = passed.record().discovered.as_ref().unwrap();
    assert_eq!(provenance.invocation_id, invocation);
    let transcript: Value =
        serde_json::from_slice(&std::fs::read(&provenance.transcript).unwrap()).unwrap();
    assert_eq!(transcript["command"]["stdout"], "actual injected stdout\n");
    assert_eq!(transcript["plan_digest"], plan.digest());
    let protected = kegari_storage::PrivateDir::create(
        &f._temp.path().canonicalize().unwrap().join("retained"),
    )
    .unwrap();
    let retained = passed.retain_outputs(&f.isolation, &protected).unwrap();
    assert_eq!(retained.len(), 1);
    assert!(journal[0]["discovered"].is_object());
    assert_eq!(boundary.calls.load(Ordering::SeqCst), 1);
    let mut rewritten = passed.record().clone();
    rewritten.discovered.as_mut().unwrap().plan_digest = bytes_digest(b"different plan");
    assert!(rewritten.validate_shape().is_err());
    let mut rewritten = passed.record().clone();
    rewritten.schema_version = 3;
    assert!(rewritten.validate_shape().is_err());
}

#[test]
fn discovered_validation_refuses_source_exclusion_stale_plan_and_reused_transcript() {
    let f = Fixture::new();
    let boundary = Boundary::default();
    let executor = f.executor(&boundary);
    let mut source_artifact = requirement();
    source_artifact.artifacts = vec!["source.txt".into()];
    let plan = enroll(&f, &executor, source_artifact);
    assert_eq!(
        executor
            .prepare_discovered_check(
                &f.binding,
                &plan,
                "discovered-obligation",
                Uuid::new_v4(),
                &[],
                None,
                |_| Ok(())
            )
            .unwrap_err()
            .code,
        "artifact_conflict"
    );
    let plan = enroll(&f, &executor, requirement());
    std::fs::write(
        f.isolation.plan().repository.join("source.txt"),
        b"changed candidate\n",
    )
    .unwrap();
    assert!(
        executor
            .prepare_discovered_check(
                &f.binding,
                &plan,
                "discovered-obligation",
                Uuid::new_v4(),
                &[],
                None,
                |_| Ok(())
            )
            .is_err()
    );
    std::fs::write(
        f.isolation.plan().repository.join("source.txt"),
        b"actual admitted source\n",
    )
    .unwrap();
    let invocation = Uuid::new_v4();
    let transcript = f
        .isolation
        .plan()
        .scratch
        .join(format!("validation-transcript-{invocation}.json"));
    std::fs::write(&transcript, b"earlier bytes").unwrap();
    assert_eq!(
        executor
            .prepare_discovered_check(
                &f.binding,
                &plan,
                "discovered-obligation",
                invocation,
                &[],
                None,
                |_| Ok(())
            )
            .unwrap_err()
            .code,
        "artifact_conflict"
    );
    assert_eq!(std::fs::read(&transcript).unwrap(), b"earlier bytes");
    assert_eq!(boundary.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn discovered_preflight_retains_input_anchors_and_original_expired_cutoff() {
    let f = Fixture::new();
    let boundary = Boundary::default();
    let executor = f.executor(&boundary);
    let plan = enroll(&f, &executor, requirement());
    let prepared = executor
        .prepare_discovered_check(
            &f.binding,
            &plan,
            "discovered-obligation",
            Uuid::new_v4(),
            &[],
            None,
            |_| Ok(()),
        )
        .unwrap();
    std::fs::create_dir(f.isolation.plan().repository.join("new-directory")).unwrap();
    assert_eq!(
        executor.start_check(prepared).err().unwrap().code,
        "stale_command"
    );
    std::fs::remove_dir(f.isolation.plan().repository.join("new-directory")).unwrap();
    let mut prepared = executor
        .prepare_discovered_check(
            &f.binding,
            &plan,
            "discovered-obligation",
            Uuid::new_v4(),
            &[],
            None,
            |_| Ok(()),
        )
        .unwrap();
    // Negative-only resource cutoff injection; no public setter or authority is
    // added. The start path must not renew the retained original deadline.
    prepared.discovered.as_mut().unwrap().deadline = Instant::now();
    assert_eq!(
        executor.start_check(prepared).err().unwrap().code,
        "timeout"
    );
    assert_eq!(boundary.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn discovered_failure_retains_plan_but_unknown_closure_never_writes_transcript() {
    let f = Fixture::new();
    let boundary = Boundary {
        exit: 7,
        ..Default::default()
    };
    let executor = f.executor(&boundary);
    let plan = enroll(&f, &executor, requirement());
    let prepared = executor
        .prepare_discovered_check(
            &f.binding,
            &plan,
            "discovered-obligation",
            Uuid::new_v4(),
            &[],
            None,
            |_| Ok(()),
        )
        .unwrap();
    let running = executor.start_check(prepared).unwrap();
    let CheckCompletion::Failed(failed) =
        executor.complete_check(running, |_| Ok(())).await.unwrap()
    else {
        panic!("nonzero command cannot pass")
    };
    assert_eq!(failed.validation_plan().unwrap().digest(), plan.digest());
    assert_eq!(failed.record().command.exit_code, Some(7));
    assert_eq!(
        failed.production_failure().unwrap_err().code,
        "unqualified_fixture"
    );
    failed.record().validate_shape().unwrap();
    let unknown = Boundary {
        refuse_closure: true,
        ..Default::default()
    };
    let executor = f.executor(&unknown);
    let invocation = Uuid::new_v4();
    let prepared = executor
        .prepare_discovered_check(
            &f.binding,
            &plan,
            "discovered-obligation",
            invocation,
            &[],
            None,
            |_| Ok(()),
        )
        .unwrap();
    let running = executor.start_check(prepared).unwrap();
    let mut journal = Vec::new();
    assert_eq!(
        executor
            .complete_check(running, |event| {
                journal.push(serde_json::to_value(event).unwrap());
                Ok(())
            })
            .await
            .unwrap_err()
            .code,
        "operation_uncertain"
    );
    assert!(
        !f.isolation
            .plan()
            .scratch
            .join(format!("validation-transcript-{invocation}.json"))
            .exists()
    );
    assert_eq!(journal.len(), 1);
    assert_eq!(journal[0]["state"], "uncertain");
}

#[test]
fn discovered_services_bind_actual_observations_and_require_original_bundle_admission() {
    let f = Fixture::new();
    let boundary = Boundary::default();
    let executor = f.executor(&boundary);
    let mut requested = requirement();
    requested.services = vec![crate::workflow::CheckService {
        name: "owned-service".into(),
        argv: vec!["/usr/bin/true".into()],
        readiness: crate::workflow::ServiceReadiness {
            argv: vec!["/usr/bin/true".into(), "ready".into()],
            timeout_ms: 1000,
        },
        max_lifetime_ms: 5000,
    }];
    let plan = enroll(&f, &executor, requested);
    let prepare = || {
        executor
            .prepare_discovered_check(
                &f.binding,
                &plan,
                "discovered-obligation",
                Uuid::new_v4(),
                &[],
                None,
                |_| Ok(()),
            )
            .unwrap()
    };
    let prepared = prepare();
    let observed = &prepared
        .discovered
        .as_ref()
        .unwrap()
        .provenance
        .service_observations;
    assert_eq!(observed.len(), 1);
    assert_eq!(observed[0].name, "owned-service");
    assert_eq!(
        observed[0].readiness.requested_argv,
        vec!["/usr/bin/true", "ready"]
    );
    assert_eq!(
        executor.start_check(prepared).err().unwrap().code,
        "check_bundle_admission_required"
    );
    let prepared = prepare();
    let original_invocation = prepared
        .discovered
        .as_ref()
        .unwrap()
        .provenance
        .invocation_id;
    let bundle = executor
        .prepare_bundle(prepared, original_invocation)
        .unwrap();
    assert_eq!(bundle.plan().steps().len(), 3);
    assert_eq!(bundle.plan().effect_id(), original_invocation);
    assert_eq!(boundary.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn integrated_discovery_retains_actual_e2e_binding_and_transcript_without_native_promotion() {
    let f = Fixture::integrated();
    assert_eq!(f.isolation.plan().role, ExecutionRole::Validation);
    assert!(f.isolation.plan().source_writes.is_empty());
    let boundary = Boundary::default();
    let executor = f.executor(&boundary);
    let plan = enroll(&f, &executor, requirement());
    assert_eq!(plan.record().kind, CheckKind::Integrated);
    let invocation = Uuid::new_v4();
    let prepared = executor
        .prepare_discovered_check(
            &f.binding,
            &plan,
            "discovered-obligation",
            invocation,
            &[],
            None,
            |_| Ok(()),
        )
        .unwrap();
    assert_eq!(boundary.calls.load(Ordering::SeqCst), 0);
    let running = executor.start_check(prepared).unwrap();
    let CheckCompletion::Passed(passed) =
        executor.complete_check(running, |_| Ok(())).await.unwrap()
    else {
        panic!("injected integrated command did not pass its mechanical checks")
    };
    let record = passed.record();
    assert_eq!(record.kind, CheckKind::Integrated);
    assert_eq!(record.binding, f.binding);
    assert_eq!(record.before, record.after);
    assert_eq!(
        record.discovered.as_ref().unwrap().plan_digest,
        plan.digest()
    );
    assert_eq!(
        passed
            .validation_plan()
            .unwrap()
            .record()
            .binding
            .candidate_vector_digest,
        f.binding.candidate_vector_digest
    );
    assert!(
        record
            .artifacts
            .iter()
            .all(|a| a.candidate_vector_digest == f.binding.candidate_vector_digest)
    );
    record.validate_shape().unwrap();
    assert_eq!(
        passed.production_evidence().unwrap_err().code,
        "unqualified_fixture"
    );
    let transcript: Value = serde_json::from_slice(
        &std::fs::read(&record.discovered.as_ref().unwrap().transcript).unwrap(),
    )
    .unwrap();
    assert_eq!(transcript["command"]["stdout"], "actual injected stdout\n");
    assert_eq!(boundary.calls.load(Ordering::SeqCst), 1);
    let mut rewritten = record.clone();
    rewritten.kind = CheckKind::Local;
    assert!(rewritten.validate_shape().is_err());
    let mut rewritten = record.clone();
    rewritten.binding.candidate_vector_digest = Some(bytes_digest(b"substituted vector"));
    assert!(rewritten.validate_shape().is_err());
}

#[test]
fn integrated_preflight_refuses_vector_role_and_other_candidate_changes_before_dispatch() {
    let f = Fixture::integrated();
    let boundary = Boundary::default();
    let executor = f.executor(&boundary);
    let plan = enroll(&f, &executor, requirement());
    let mut changed = f.binding.clone();
    changed.candidate_vector_digest = Some(bytes_digest(b"substituted vector"));
    assert_eq!(
        executor
            .prepare_discovered_check(
                &changed,
                &plan,
                "discovered-obligation",
                Uuid::new_v4(),
                &[],
                None,
                |_| Ok(()),
            )
            .unwrap_err()
            .code,
        "validation_plan_binding"
    );
    let prepared = executor
        .prepare_discovered_check(
            &f.binding,
            &plan,
            "discovered-obligation",
            Uuid::new_v4(),
            &[],
            None,
            |_| Ok(()),
        )
        .unwrap();
    let candidate = f
        ._temp
        .path()
        .canonicalize()
        .unwrap()
        .join("other-candidate/input.txt");
    std::fs::write(candidate, b"changed other repository before launch\n").unwrap();
    assert!(executor.start_check(prepared).is_err());
    assert_eq!(boundary.calls.load(Ordering::SeqCst), 0);

    // A descriptive vector on an ordinary Child fixture does not supply the
    // actual integrated Validation-role boundary.
    let mut child = Fixture::new();
    child.binding.candidate_vector_digest = Some(bytes_digest(b"descriptive vector"));
    let executor = child.executor(&boundary);
    let plan = enroll(&child, &executor, requirement());
    assert_eq!(
        executor
            .prepare_discovered_check(
                &child.binding,
                &plan,
                "discovered-obligation",
                Uuid::new_v4(),
                &[],
                None,
                |_| Ok(()),
            )
            .unwrap_err()
            .code,
        "validation_plan_binding"
    );
    assert_eq!(boundary.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn integrated_completion_rechecks_other_candidate_bytes_before_transcript_or_capability() {
    let f = Fixture::integrated();
    let candidate = f
        ._temp
        .path()
        .canonicalize()
        .unwrap()
        .join("other-candidate/input.txt");
    let boundary = Boundary {
        // Injected boundary deliberately violates the read-only fixture scope;
        // actual completion must detect this, even with reported exit zero.
        writes: vec![(candidate, b"changed during injected execution\n".to_vec())],
        ..Default::default()
    };
    let executor = f.executor(&boundary);
    let plan = enroll(&f, &executor, requirement());
    let invocation = Uuid::new_v4();
    let prepared = executor
        .prepare_discovered_check(
            &f.binding,
            &plan,
            "discovered-obligation",
            invocation,
            &[],
            None,
            |_| Ok(()),
        )
        .unwrap();
    let running = executor.start_check(prepared).unwrap();
    let mut journal = Vec::new();
    assert!(
        executor
            .complete_check(running, |event| {
                journal.push(serde_json::to_value(event).unwrap());
                Ok(())
            })
            .await
            .is_err()
    );
    assert_eq!(boundary.calls.load(Ordering::SeqCst), 1);
    assert_eq!(journal.len(), 1);
    assert_eq!(journal[0]["state"], "uncertain");
    assert!(
        !f.isolation
            .plan()
            .scratch
            .join(format!("validation-transcript-{invocation}.json"))
            .exists()
    );
}

#[test]
fn generic_command_refuses_changed_original_resource_before_dispatch() {
    let f = Fixture::integrated();
    let boundary = Boundary::default();
    let executor = f.executor(&boundary);
    let request = CommandRequest {
        argv: requirement().argv,
        cwd: ".".into(),
    };
    let invocation = Uuid::new_v4();
    let mut journal = Vec::new();
    let prepared = executor
        .prepare_command(&f.binding, &request, invocation, None, |event| {
            journal.push(serde_json::to_value(event).unwrap());
            Ok(())
        })
        .unwrap();
    let candidate = f
        ._temp
        .path()
        .canonicalize()
        .unwrap()
        .join("other-candidate/input.txt");
    std::fs::write(
        candidate,
        b"changed resource after actual command preparation\n",
    )
    .unwrap();
    assert_eq!(
        executor.preflight_command(&prepared).unwrap_err().code,
        "identity_mismatch"
    );
    let error = executor
        .start_command(prepared, |event| {
            journal.push(serde_json::to_value(event).unwrap());
            Ok(())
        })
        .err()
        .expect("stale original resource must refuse without dispatch");
    assert_eq!(error.code, "identity_mismatch");
    assert_eq!(boundary.calls.load(Ordering::SeqCst), 0);
    assert_eq!(journal.len(), 2);
    assert_eq!(journal[0]["state"], "prepared");
    assert_eq!(journal[1]["state"], "not_started");
    assert_eq!(
        journal[1]["invocation_id"],
        serde_json::to_value(invocation).unwrap()
    );
}
