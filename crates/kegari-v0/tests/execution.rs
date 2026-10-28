//! Deterministic host/boundary fixtures. These deliberately do not establish
//! live sandbox or media qualification; no provider/network actions occur.
#[path = "support/pinned_git.rs"]
mod pinned_git;
use kegari_types::error::{Error, Result};
use kegari_v0::digest::bytes_digest;
use kegari_v0::evidence::{CheckKind, Generation, WorkIdentity};
use kegari_v0::execution::*;
use kegari_v0::git::{Git, RepositoryId};
use kegari_v0::isolation::{
    BoundManagedExecution, CommandOutcome, ExecutionRole, IsolationPlan, PathGrant,
    ValidatedIsolation,
};
use kegari_v0::workflow::Workflow;
use serde_json::json;
use std::collections::VecDeque;
use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Command;
use std::sync::Mutex;
use std::time::Duration;
use tempfile::TempDir;
use uuid::Uuid;

fn git_path() -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|p| p.join("git"))
        .find(|p| p.is_file())
        .unwrap()
        .canonicalize()
        .unwrap()
}
fn git_run(path: &Path, args: &[&str]) {
    let output = kegari_v0::process_group::OwnedGroup::spawn_std(
        Command::new(git_path())
            .current_dir(path)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_AUTHOR_NAME", "Disposable Fixture Human")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.test")
            .env("GIT_COMMITTER_NAME", "Disposable Fixture Human")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.test")
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
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
struct Fixture {
    _temp: TempDir,
    source: PathBuf,
    root: PathBuf,
    git: Git,
    boundary: ValidatedIsolation,
    binding: ExecutionBinding,
    owner: OwnedDirectory,
}
impl Fixture {
    fn new(role: ExecutionRole) -> Self {
        let temp = TempDir::new().unwrap();
        let temp_root = temp.path().canonicalize().unwrap();
        let source = temp_root.join("shared-checkout");
        fs::create_dir(&source).unwrap();
        git_run(&source, &["init", "-b", "main"]);
        fs::write(source.join("a.txt"), "original\n").unwrap();
        git_run(&source, &["add", "a.txt"]);
        git_run(
            &source,
            &[
                "commit",
                "--signoff",
                "-m",
                "test: initialize disposable execution source",
            ],
        );
        let root = temp_root.join("executions");
        fs::create_dir(&root).unwrap();
        let repo = root.join("repository");
        git_run(
            &root,
            &[
                "clone",
                "--local",
                "--no-hardlinks",
                source.to_str().unwrap(),
                repo.to_str().unwrap(),
            ],
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&repo, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let scratch = root.join("scratch");
        fs::create_dir(&scratch).unwrap();
        let executable = matches!(
            role,
            ExecutionRole::Child | ExecutionRole::Validation | ExecutionRole::Review
        );
        let mut plan = IsolationPlan {
            managed_execution: None,
            schema_version: 1,
            role,
            execution_root: root.clone(),
            repository: repo.clone(),
            cwd: repo.clone(),
            scratch,
            read_roots: vec![PathGrant {
                path: repo.clone(),
                directory: true,
            }],
            resource_writes: vec![],
            source_writes: if role == ExecutionRole::Child {
                vec![PathGrant {
                    path: repo.join("a.txt"),
                    directory: false,
                }]
            } else {
                vec![]
            },
            forbidden_roots: vec![source.clone()],
        };
        if executable {
            plan.managed_execution = Some(BoundManagedExecution::automatic(&plan).unwrap());
        }
        let boundary = plan.validate().unwrap();
        let git = Git::new(pinned_git::prepare(temp.path())).unwrap();
        let work = WorkIdentity {
            linear_endpoint: "https://api.linear.app/graphql".into(),
            linear_workspace: Uuid::new_v4(),
            parent: Uuid::new_v4(),
            child: Some(Uuid::new_v4()),
            generation: Generation {
                authorization: bytes_digest(b"authority"),
                definition: bytes_digest(b"definition"),
                correction: bytes_digest(b"correction"),
                scope: bytes_digest(b"scope"),
            },
        };
        let binding = ExecutionBinding {
            runtime_id: Uuid::new_v4(),
            operation_id: Uuid::new_v4(),
            work: work.clone(),
            repository: RepositoryId {
                hostname: "github.com".into(),
                id: "101".into(),
            },
            revision: git.revision(&repo, "HEAD").unwrap(),
            candidate_vector_digest: Some(bytes_digest(b"candidate-vector")),
            isolation_digest: boundary.digest().into(),
        };
        let owner = OwnedDirectory::register(
            DirectoryIdentity {
                schema_version: 1,
                runtime_id: binding.runtime_id,
                directory_id: Uuid::new_v4(),
                repository: binding.repository.clone(),
                work,
                repository_path: boundary.plan().repository.clone(),
                operator_checkout: source.canonicalize().unwrap(),
            },
            &boundary,
        )
        .unwrap();
        Self {
            _temp: temp,
            source,
            root,
            git,
            boundary,
            binding,
            owner,
        }
    }
    fn executor<'a>(&'a self, runner: &'a FakeBoundary) -> LocalExecutor<'a> {
        LocalExecutor {
            isolation: &self.boundary,
            git: &self.git,
            boundary: runner,
            limits: ExecutionLimits::default(),
            ownership: &self.owner,
        }
    }
    fn with_content_resource(role: ExecutionRole) -> (Self, PathBuf) {
        let mut fixture = Self::new(role);
        let resource = fixture
            .boundary
            .plan()
            .execution_root
            .parent()
            .unwrap()
            .join("tool-support");
        fs::create_dir(&resource).unwrap();
        let library = resource.join("library.dylib");
        fs::write(&library, b"fixture library; never loaded\n").unwrap();
        let mut plan = fixture.boundary.plan().clone();
        plan.read_roots.push(PathGrant {
            path: resource.clone(),
            directory: true,
        });
        plan.managed_execution = Some(BoundManagedExecution::automatic(&plan).unwrap());
        fixture.boundary = plan.validate().unwrap();
        fixture.binding.isolation_digest = fixture.boundary.digest().into();
        // Reuse the actual exclusive marker, work and repository identity. Only
        // the fixture's newly declared isolation binding changes.
        fixture.owner = OwnedDirectory::reopen(
            fixture.owner.marker(),
            fixture.owner.digest(),
            &fixture.boundary,
        )
        .unwrap();
        (fixture, library)
    }
    fn workflow(&self, hooks: &str) -> Workflow {
        Workflow::parse(&format!("---\nhooks:\n{hooks}\nchecks:\n  local:\n    unit:\n      argv: ['/usr/bin/true']\n      result_artifact: '{}'\n  integrated:\n    e2e:\n      argv: ['/usr/bin/true']\n      result_artifact: '{}'\n---\nPerform the scoped work.\n", self.boundary.plan().scratch.join("unit.json").display(), self.boundary.plan().scratch.join("e2e.json").display()), self.source.join("WORKFLOW.md")).unwrap()
    }
}

fn source_metadata() -> kegari_v0::git::SourceCommitMetadata {
    kegari_v0::git::SourceCommitMetadata {
        identity: kegari_v0::git::CommitIdentity {
            name: "Fixture Human".into(),
            email: "fixture@example.test".into(),
        },
        timestamp: chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
        message:
            "fix: implement fixture child\n\nSigned-off-by: Fixture Human <fixture@example.test>\n"
                .into(),
    }
}

#[test]
fn trusted_source_commit_preserves_binary_mode_and_shared_checkout() {
    let f = Fixture::new(ExecutionRole::Child);
    let root = &f.boundary.plan().repository;
    let scope = kegari_v0::git::WriteScope {
        paths: vec!["a.txt".into()],
    };
    assert!(
        f.git
            .commit_execution_changes(&f.owner, &f.binding, &f.boundary, &scope, source_metadata())
            .unwrap()
            .is_none()
    );
    let bytes = vec![0u8, 1, 255, 0, 10, 13];
    fs::write(root.join("a.txt"), &bytes).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(root.join("a.txt"), fs::Permissions::from_mode(0o755)).unwrap();
    }
    let committed = f
        .git
        .commit_execution_changes(&f.owner, &f.binding, &f.boundary, &scope, source_metadata())
        .unwrap()
        .unwrap();
    assert_eq!(committed.base(), &f.binding.revision);
    assert_eq!(f.git.revision(root, "HEAD").unwrap(), *committed.revision());
    assert_eq!(committed.entries().len(), 1);
    assert_eq!(committed.entries()[0].mode, "100755");
    assert_eq!(committed.blobs().values().next().unwrap(), &bytes);
    let recovered = f
        .git
        .inspect_committed_source(
            root,
            committed.base(),
            committed.revision(),
            &scope,
            source_metadata(),
        )
        .unwrap();
    assert_eq!(recovered.entries(), committed.entries());
    assert_eq!(recovered.blobs(), committed.blobs());
    let mut changed_metadata = source_metadata();
    changed_metadata
        .message
        .insert_str(0, "unreviewed message\n");
    assert_eq!(
        f.git
            .inspect_committed_source(
                root,
                committed.base(),
                committed.revision(),
                &scope,
                changed_metadata
            )
            .err()
            .unwrap()
            .code,
        "provenance_mismatch"
    );
    assert_eq!(
        f.git
            .inspect_committed_source(
                root,
                committed.base(),
                committed.revision(),
                &kegari_v0::git::WriteScope {
                    paths: vec!["unrelated/".into()]
                },
                source_metadata()
            )
            .err()
            .unwrap()
            .code,
        "scope_violation"
    );
    assert_eq!(fs::read(root.join("a.txt")).unwrap(), bytes);
    assert_eq!(
        fs::read_to_string(f.source.join("a.txt")).unwrap(),
        "original\n"
    );
    assert_eq!(
        f.git.revision(&f.source, "HEAD").unwrap(),
        f.binding.revision
    );
    let mut next_binding = f.binding.clone();
    next_binding.revision = committed.revision().clone();
    f.git
        .verify_execution_tree(root, &next_binding.revision, &Default::default())
        .unwrap();
    let old = f.git.commit_execution_changes(
        &f.owner,
        &f.binding,
        &f.boundary,
        &scope,
        source_metadata(),
    );
    assert_eq!(old.err().unwrap().code, "stale_revision");
}

#[test]
fn trusted_source_commit_rejects_unscoped_or_staged_bytes_before_changing_head() {
    let f = Fixture::new(ExecutionRole::Child);
    let root = &f.boundary.plan().repository;
    let scope = kegari_v0::git::WriteScope {
        paths: vec!["a.txt".into()],
    };
    fs::write(root.join("unscoped"), "unapproved").unwrap();
    assert_eq!(
        f.git
            .commit_execution_changes(&f.owner, &f.binding, &f.boundary, &scope, source_metadata())
            .err()
            .unwrap()
            .code,
        "scope_violation"
    );
    assert_eq!(f.git.revision(root, "HEAD").unwrap(), f.binding.revision);
    fs::remove_file(root.join("unscoped")).unwrap();
    fs::write(root.join("a.txt"), "staged content").unwrap();
    git_run(root, &["add", "a.txt"]);
    let index = fs::read(root.join(".git/index")).unwrap();
    fs::write(root.join("a.txt"), "later content").unwrap();
    assert_eq!(
        f.git
            .commit_execution_changes(&f.owner, &f.binding, &f.boundary, &scope, source_metadata())
            .err()
            .unwrap()
            .code,
        "dirty_index"
    );
    assert_eq!(f.git.revision(root, "HEAD").unwrap(), f.binding.revision);
    assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
    assert_eq!(
        fs::read_to_string(root.join("a.txt")).unwrap(),
        "later content"
    );
}

#[test]
fn trusted_source_commit_preserves_deletion_symlink_and_denies_configured_filters() {
    let scope = kegari_v0::git::WriteScope {
        paths: vec!["a.txt".into()],
    };
    let deleted = Fixture::new(ExecutionRole::Child);
    fs::remove_file(deleted.boundary.plan().repository.join("a.txt")).unwrap();
    let committed = deleted
        .git
        .commit_execution_changes(
            &deleted.owner,
            &deleted.binding,
            &deleted.boundary,
            &scope,
            source_metadata(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(committed.entries()[0].object, None);
    assert!(committed.blobs().is_empty());
    assert!(!deleted.boundary.plan().repository.join("a.txt").exists());
    #[cfg(unix)]
    {
        let linked = Fixture::new(ExecutionRole::Child);
        let path = linked.boundary.plan().repository.join("a.txt");
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink("relative-target", &path).unwrap();
        let committed = linked
            .git
            .commit_execution_changes(
                &linked.owner,
                &linked.binding,
                &linked.boundary,
                &scope,
                source_metadata(),
            )
            .unwrap()
            .unwrap();
        assert_eq!(committed.entries()[0].mode, "120000");
        assert_eq!(
            committed.blobs().values().next().unwrap(),
            b"relative-target"
        );
    }
    let filtered = Fixture::new(ExecutionRole::Child);
    let root = &filtered.boundary.plan().repository;
    git_run(
        root,
        &[
            "config",
            "filter.fixture.clean",
            "unavailable-fixture-filter",
        ],
    );
    fs::write(root.join("a.txt"), "changed").unwrap();
    assert_eq!(
        filtered
            .git
            .commit_execution_changes(
                &filtered.owner,
                &filtered.binding,
                &filtered.boundary,
                &scope,
                source_metadata()
            )
            .err()
            .unwrap()
            .code,
        "unsupported_repository"
    );
    assert_eq!(
        filtered.git.revision(root, "HEAD").unwrap(),
        filtered.binding.revision
    );
}
#[derive(Default)]
struct Step {
    writes: Vec<(PathBuf, Vec<u8>)>,
    stdout: String,
    exit: Option<i32>,
    timeout: bool,
    error: bool,
}
impl Step {
    fn success() -> Self {
        Self {
            exit: Some(0),
            ..Self::default()
        }
    }
}
#[derive(Default)]
struct FakeBoundary {
    steps: Mutex<VecDeque<Step>>,
    calls: Mutex<Vec<Vec<String>>>,
    required_journal: Option<PathBuf>,
}
impl FakeBoundary {
    fn new(steps: Vec<Step>) -> Self {
        Self {
            steps: Mutex::new(steps.into()),
            ..Self::default()
        }
    }
}
struct FakeCancellation;
impl CommandCancellation for FakeCancellation {
    fn cancel(&self) -> Result<()> {
        Ok(())
    }
}
struct FakeCommand(Result<CommandOutcome>);
impl StartedCommand for FakeCommand {
    fn cancellation(&self) -> std::sync::Arc<dyn CommandCancellation> {
        std::sync::Arc::new(FakeCancellation)
    }
    fn wait(self: Box<Self>) -> Pin<Box<dyn Future<Output = Result<CommandOutcome>> + Send>> {
        Box::pin(async move { self.0 })
    }
}
impl CommandBoundary for FakeBoundary {
    fn evidence_class(&self) -> ExecutionEvidenceClass {
        ExecutionEvidenceClass::DeterministicFixture
    }
    fn start(
        &self,
        isolation: &ValidatedIsolation,
        argv: &[String],
        _timeout: Duration,
        _output_limit: usize,
    ) -> Result<Box<dyn StartedCommand>> {
        if let Some(path) = &self.required_journal {
            let saved = fs::read_to_string(path).unwrap();
            assert!(saved.contains("started_unknown") || saved.contains("\"started\""));
        }
        self.calls.lock().unwrap().push(argv.to_vec());
        let step = self
            .steps
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected command");
        for (path, bytes) in step.writes {
            fs::write(path, bytes).unwrap();
        }
        if step.error {
            return Ok(Box::new(FakeCommand(Err(Error::new(
                "fixture_transport",
                "simulated interruption",
            )))));
        }
        Ok(Box::new(FakeCommand(Ok(CommandOutcome {
            schema_version: 1,
            argv: argv.to_vec(),
            cwd: isolation.plan().cwd.clone(),
            isolation_digest: isolation.digest().into(),
            exit_code: step.exit,
            termination_signal: None,
            timed_out: step.timeout,
            stdout: step.stdout,
            stderr: String::new(),
            output_truncated: false,
            elapsed_ms: 1,
        }))))
    }
}
fn hook<'a>(
    f: &'a Fixture,
    w: &'a Workflow,
    phase: HookPhase,
    session: Option<Uuid>,
) -> HookInvocation<'a> {
    HookInvocation {
        binding: &f.binding,
        workflow: w,
        phase,
        session,
        eligible_cleanup_plan: None,
    }
}

#[tokio::test]
async fn hooks_persist_before_dispatch_and_obey_actual_session_lifecycle() {
    let f = Fixture::new(ExecutionRole::Child);
    let w=f.workflow("  after_create: 'printf setup'\n  before_run: 'printf prepare'\n  after_run: 'printf cleanup'\n  timeout_ms: 1000");
    let journal = f.root.join("hooks.json");
    let mut runner = FakeBoundary::new(vec![
        Step::success(),
        Step::success(),
        Step {
            exit: Some(1),
            ..Step::default()
        },
    ]);
    runner.required_journal = Some(journal.clone());
    let executor = f.executor(&runner);
    let mut ledger = HookLedger::new(&f.binding).unwrap();
    let mut persist = |l: &HookLedger| {
        fs::write(&journal, serde_json::to_vec(l).unwrap()).unwrap();
        Ok(())
    };
    assert!(
        executor
            .run_hook(
                &mut ledger,
                hook(&f, &w, HookPhase::AfterCreate, None),
                &mut persist
            )
            .await
            .unwrap()
            .allows_next_phase
    );
    let session = Uuid::new_v4();
    assert!(
        executor
            .run_hook(
                &mut ledger,
                hook(&f, &w, HookPhase::AfterRun, Some(session)),
                &mut persist
            )
            .await
            .is_err()
    );
    executor
        .run_hook(
            &mut ledger,
            hook(&f, &w, HookPhase::BeforeRun, Some(session)),
            &mut persist,
        )
        .await
        .unwrap();
    ledger.mark_launched(session, &mut persist).unwrap();
    ledger.mark_ended(session, &mut persist).unwrap();
    let after = executor
        .run_hook(
            &mut ledger,
            hook(&f, &w, HookPhase::AfterRun, Some(session)),
            &mut persist,
        )
        .await
        .unwrap();
    assert!(after.diagnostic_only);
    assert!(!after.allows_next_phase);
    assert_eq!(
        executor
            .run_hook(
                &mut ledger,
                hook(&f, &w, HookPhase::AfterRun, Some(session)),
                &mut persist
            )
            .await
            .unwrap_err()
            .code,
        "reconciliation_required"
    );
    assert_eq!(runner.calls.lock().unwrap().len(), 3);
    assert_eq!(
        fs::read_to_string(f.source.join("a.txt")).unwrap(),
        "original\n"
    );
}

#[tokio::test]
async fn hook_timeout_retains_unknown_effect_and_before_remove_needs_explicit_eligibility() {
    let f = Fixture::new(ExecutionRole::Child);
    let w = f.workflow("  after_create: 'run setup'\n  before_remove: 'run cleanup'");
    let runner = FakeBoundary::new(vec![Step {
        timeout: true,
        ..Step::default()
    }]);
    let executor = f.executor(&runner);
    let mut ledger = HookLedger::new(&f.binding).unwrap();
    let result = executor
        .run_hook(
            &mut ledger,
            hook(&f, &w, HookPhase::AfterCreate, None),
            |_| Ok(()),
        )
        .await
        .unwrap();
    assert_eq!(result.state, HookState::StartedUnknown);
    assert!(!ledger.initialized);
    assert!(
        executor
            .run_hook(
                &mut ledger,
                hook(&f, &w, HookPhase::BeforeRemove, None),
                |_| Ok(())
            )
            .await
            .is_err()
    );
    let cleanup = bytes_digest(b"eligible-cleanup");
    let mut request = hook(&f, &w, HookPhase::BeforeRemove, None);
    request.eligible_cleanup_plan = Some(&cleanup);
    assert!(
        executor
            .run_hook(&mut ledger, request, |_| Ok(()))
            .await
            .is_err()
    );
    assert!(f.boundary.plan().repository.exists());
}

#[tokio::test]
async fn journal_failure_prevents_hook_and_check_execution() {
    let f = Fixture::new(ExecutionRole::Child);
    let w = f.workflow("  after_create: 'printf setup'");
    let runner = FakeBoundary::default();
    let executor = f.executor(&runner);
    let fail = || Error::new("disk_full", "fixture persistence failure");
    let mut ledger = HookLedger::new(&f.binding).unwrap();
    assert!(
        executor
            .run_hook(
                &mut ledger,
                hook(&f, &w, HookPhase::AfterCreate, None),
                |_| Err(fail())
            )
            .await
            .is_err()
    );
    assert!(
        executor
            .run_check(
                CheckInvocation {
                    binding: &f.binding,
                    workflow: &w,
                    name: "unit",
                    kind: CheckKind::Local,
                    additional_outputs: &[],
                    previous_outputs: &[]
                },
                |_| Err(fail())
            )
            .await
            .is_err()
    );
    assert!(runner.calls.lock().unwrap().is_empty());
    assert!(ledger.records.is_empty());
}

#[tokio::test]
async fn split_hook_persists_unknown_before_start_and_skipped_hooks_have_no_process() {
    let f = Fixture::new(ExecutionRole::Child);
    let w = f.workflow("  after_create: 'prepare fixture'");
    let runner = FakeBoundary::new(vec![Step::success()]);
    let executor = f.executor(&runner);
    let mut ledger = HookLedger::new(&f.binding).unwrap();
    let prepared = executor
        .prepare_hook(
            &mut ledger,
            hook(&f, &w, HookPhase::AfterCreate, None),
            |_| Ok(()),
        )
        .unwrap();
    assert!(prepared.requires_execution());
    assert!(runner.calls.lock().unwrap().is_empty());
    assert_eq!(
        ledger.records.values().next().unwrap().state,
        HookState::StartedUnknown
    );
    assert!(!ledger.initialized);
    assert_eq!(
        executor
            .prepare_hook(
                &mut ledger,
                hook(&f, &w, HookPhase::AfterCreate, None),
                |_| Ok(())
            )
            .unwrap_err()
            .code,
        "reconciliation_required"
    );
    let running = executor.start_hook(prepared).unwrap();
    assert!(running.cancellation().is_some());
    assert!(!ledger.initialized);
    let result = executor
        .complete_hook(&mut ledger, running, |_| Ok(()))
        .await
        .unwrap();
    assert_eq!(result.state, HookState::Succeeded);
    assert!(ledger.initialized);

    let prepared = executor
        .prepare_hook(
            &mut ledger,
            hook(&f, &w, HookPhase::BeforeRun, Some(Uuid::new_v4())),
            |_| Ok(()),
        )
        .unwrap();
    assert!(!prepared.requires_execution());
    let running = executor.start_hook(prepared).unwrap();
    assert!(running.cancellation().is_none());
    assert_eq!(
        executor
            .complete_hook(&mut ledger, running, |_| Ok(()))
            .await
            .unwrap()
            .state,
        HookState::Skipped
    );
    assert_eq!(runner.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn changed_managed_resource_after_check_prepare_persists_uncertain_evidence() {
    let (f, library) = Fixture::with_content_resource(ExecutionRole::Validation);
    let workflow = f.workflow("  timeout_ms: 1000");
    let output = f.boundary.plan().scratch.join("unit.json");
    let journal = f.root.join("resource-check.json");
    let mut runner = FakeBoundary::new(vec![Step {
        writes: vec![
            (output.clone(), b"successful command artifact".to_vec()),
            (library.clone(), b"changed library during command".to_vec()),
        ],
        ..Step::success()
    }]);
    runner.required_journal = Some(journal.clone());
    let executor = f.executor(&runner);
    let invocation = || CheckInvocation {
        binding: &f.binding,
        workflow: &workflow,
        name: "unit",
        kind: CheckKind::Local,
        additional_outputs: &[],
        previous_outputs: &[],
    };
    let mut entries = Vec::new();
    let prepared = executor
        .prepare_check(invocation(), |entry| {
            entries.push(serde_json::to_value(entry).unwrap());
            fs::write(&journal, serde_json::to_vec(&entries).unwrap()).unwrap();
            Ok(())
        })
        .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["state"], "started");
    assert!(runner.calls.lock().unwrap().is_empty());
    f.boundary.revalidate_resource_contents().unwrap();

    let running = executor.start_check(prepared).unwrap();
    assert_eq!(runner.calls.lock().unwrap().len(), 1);
    assert_eq!(fs::read(&output).unwrap(), b"successful command artifact");
    let error = executor
        .complete_check(running, |entry| {
            entries.push(serde_json::to_value(entry).unwrap());
            fs::write(&journal, serde_json::to_vec(&entries).unwrap()).unwrap();
            Ok(())
        })
        .await
        .unwrap_err();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[1]["state"], "uncertain");
    assert_eq!(entries[1]["operation_id"], json!(f.binding.operation_id));
    assert_eq!(entries[1]["error_code"], error.code);
    assert_eq!(
        error.code,
        f.boundary.revalidate_resource_contents().unwrap_err().code
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&fs::read(&journal).unwrap()).unwrap(),
        json!(entries)
    );
    assert!(!entries.iter().any(|entry| entry["state"] == "finished"));
    assert_eq!(
        fs::read(&library).unwrap(),
        b"changed library during command"
    );

    // The same drift also refuses a fresh preparation before another dispatch
    // or Started record. No new evidence can adopt the changed resource.
    assert!(
        executor
            .prepare_check(invocation(), |_| {
                panic!("resource drift must be refused before another Started record")
            })
            .is_err()
    );
    assert_eq!(runner.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn changed_managed_resource_during_hook_retains_started_unknown() {
    let (f, library) = Fixture::with_content_resource(ExecutionRole::Child);
    let workflow = f.workflow("  after_create: 'prepare fixture'\n  timeout_ms: 1000");
    let journal = f.root.join("resource-hook.json");
    let mut runner = FakeBoundary::new(vec![Step {
        writes: vec![(library, b"changed hook library contents".to_vec())],
        ..Step::success()
    }]);
    runner.required_journal = Some(journal.clone());
    let executor = f.executor(&runner);
    let mut ledger = HookLedger::new(&f.binding).unwrap();
    let prepared = executor
        .prepare_hook(
            &mut ledger,
            hook(&f, &workflow, HookPhase::AfterCreate, None),
            |state| {
                fs::write(&journal, serde_json::to_vec(state).unwrap()).unwrap();
                Ok(())
            },
        )
        .unwrap();
    assert!(prepared.requires_execution());
    assert!(!ledger.initialized);
    assert_eq!(
        ledger.records.values().next().unwrap().state,
        HookState::StartedUnknown
    );
    assert!(runner.calls.lock().unwrap().is_empty());
    f.boundary.revalidate_resource_contents().unwrap();
    let running = executor.start_hook(prepared).unwrap();
    let result = executor
        .complete_hook(&mut ledger, running, |state| {
            fs::write(&journal, serde_json::to_vec(state).unwrap()).unwrap();
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(runner.calls.lock().unwrap().len(), 1);
    assert_eq!(result.state, HookState::StartedUnknown);
    assert!(!result.allows_next_phase);
    assert!(!ledger.initialized);
    let retained: HookLedger = serde_json::from_slice(&fs::read(&journal).unwrap()).unwrap();
    assert!(!retained.initialized);
    assert_eq!(retained.records.len(), 1);
    let record = retained.records.values().next().unwrap();
    assert_eq!(record.state, HookState::StartedUnknown);
    assert_eq!(record.exit_code, Some(0));
    let resource_error = f.boundary.revalidate_resource_contents().unwrap_err();
    assert_eq!(
        record.error_code.as_deref(),
        Some(resource_error.code.as_str())
    );
}

#[tokio::test]
async fn named_checks_capture_fresh_artifacts_and_refuse_stale_unrun_or_changed_source() {
    let f = Fixture::new(ExecutionRole::Validation);
    let w = f.workflow("  timeout_ms: 1000");
    let output = f.boundary.plan().scratch.join("unit.json");
    let runner = FakeBoundary::new(vec![
        Step {
            writes: vec![(output.clone(), b"real fixture check output".to_vec())],
            ..Step::success()
        },
        Step::success(),
    ]);
    let executor = f.executor(&runner);
    let mut entries = vec![];
    let verified = executor
        .run_check(
            CheckInvocation {
                binding: &f.binding,
                workflow: &w,
                name: "unit",
                kind: CheckKind::Local,
                additional_outputs: &[],
                previous_outputs: &[],
            },
            |e| {
                entries.push(serde_json::to_value(e).unwrap());
                Ok(())
            },
        )
        .await
        .unwrap();
    assert_eq!(entries[0]["state"], "started");
    assert_eq!(entries[1]["state"], "finished");
    assert_eq!(
        verified.record().artifacts[0].digest,
        bytes_digest(b"real fixture check output")
    );
    assert!(verified.production_evidence().is_err());
    let retention = kegari_v0::storage::PrivateDir::create(
        &f._temp
            .path()
            .canonicalize()
            .unwrap()
            .join("retained-evidence"),
    )
    .unwrap();
    let retained = verified.retain_outputs(&f.boundary, &retention).unwrap();
    assert_eq!(
        executor
            .run_check(
                CheckInvocation {
                    binding: &f.binding,
                    workflow: &w,
                    name: "unit",
                    kind: CheckKind::Local,
                    additional_outputs: &[],
                    previous_outputs: &retained
                },
                |_| Ok(())
            )
            .await
            .unwrap_err()
            .code,
        "validation_failed"
    );
    let changed = FakeBoundary::new(vec![Step {
        writes: vec![
            (output, b"new output".to_vec()),
            (
                f.boundary.plan().repository.join("a.txt"),
                b"unreviewed change".to_vec(),
            ),
        ],
        ..Step::success()
    }]);
    let executor = f.executor(&changed);
    assert_eq!(
        executor
            .run_check(
                CheckInvocation {
                    binding: &f.binding,
                    workflow: &w,
                    name: "unit",
                    kind: CheckKind::Local,
                    additional_outputs: &[],
                    previous_outputs: &retained
                },
                |_| Ok(())
            )
            .await
            .unwrap_err()
            .code,
        "dirty_execution"
    );
    assert_eq!(
        fs::read(&retained[0].retained_paths()[0]).unwrap(),
        b"real fixture check output"
    );
}

#[tokio::test]
async fn failed_check_artifacts_are_retained_before_a_corrected_rerun() {
    let f = Fixture::new(ExecutionRole::Validation);
    let workflow = f.workflow("  timeout_ms: 1000");
    let output = f.boundary.plan().scratch.join("unit.json");
    let runner = FakeBoundary::new(vec![
        Step {
            writes: vec![(output.clone(), b"failed result".to_vec())],
            exit: Some(1),
            ..Step::default()
        },
        Step {
            writes: vec![(output, b"corrected result".to_vec())],
            ..Step::success()
        },
    ]);
    let executor = f.executor(&runner);
    let completion = executor
        .run_check_complete(
            CheckInvocation {
                binding: &f.binding,
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
    let CheckCompletion::Failed(failed) = completion else {
        panic!("expected real failed execution receipt")
    };
    assert_eq!(
        failed.production_failure().unwrap_err().code,
        "unqualified_fixture"
    );
    assert!(!failed.record().successful);
    let directory = kegari_v0::storage::PrivateDir::create(
        &f._temp
            .path()
            .canonicalize()
            .unwrap()
            .join("failed-artifacts"),
    )
    .unwrap();
    let retained = failed.retain_outputs(&f.boundary, &directory).unwrap();
    let corrected = executor
        .run_check(
            CheckInvocation {
                binding: &f.binding,
                workflow: &workflow,
                name: "unit",
                kind: CheckKind::Local,
                additional_outputs: &[],
                previous_outputs: &retained,
            },
            |_| Ok(()),
        )
        .await
        .unwrap();
    assert!(corrected.record().successful);
    assert_eq!(
        fs::read(&retained[0].retained_paths()[0]).unwrap(),
        b"failed result"
    );
}

#[tokio::test]
async fn split_check_preparation_has_no_process_effect_and_output_races_block_start() {
    let f = Fixture::new(ExecutionRole::Validation);
    let workflow = f.workflow("  timeout_ms: 1000");
    let output = f.boundary.plan().scratch.join("unit.json");
    let runner = FakeBoundary::new(vec![Step {
        writes: vec![(output.clone(), b"executed".to_vec())],
        ..Step::success()
    }]);
    let executor = f.executor(&runner);
    let prepared = executor
        .prepare_check(
            CheckInvocation {
                binding: &f.binding,
                workflow: &workflow,
                name: "unit",
                kind: CheckKind::Local,
                additional_outputs: &[],
                previous_outputs: &[],
            },
            |_| Ok(()),
        )
        .unwrap();
    assert!(runner.calls.lock().unwrap().is_empty());
    assert!(!output.exists());
    let running = executor.start_check(prepared).unwrap();
    assert_eq!(runner.calls.lock().unwrap().len(), 1);
    let completed = executor.complete_check(running, |_| Ok(())).await.unwrap();
    assert!(matches!(completed, CheckCompletion::Passed(_)));

    let another = Fixture::new(ExecutionRole::Validation);
    let workflow = another.workflow("  timeout_ms: 1000");
    let runner = FakeBoundary::new(vec![]);
    let executor = another.executor(&runner);
    let prepared = executor
        .prepare_check(
            CheckInvocation {
                binding: &another.binding,
                workflow: &workflow,
                name: "unit",
                kind: CheckKind::Local,
                additional_outputs: &[],
                previous_outputs: &[],
            },
            |_| Ok(()),
        )
        .unwrap();
    fs::write(
        another.boundary.plan().scratch.join("unit.json"),
        b"unrelated preexisting bytes",
    )
    .unwrap();
    assert!(executor.start_check(prepared).is_err());
    assert!(runner.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn noexec_roles_and_unregistered_directory_cannot_run_or_write() {
    for role in [
        ExecutionRole::Coordination,
        ExecutionRole::Review,
        ExecutionRole::IndependentReview,
    ] {
        let f = Fixture::new(role);
        let w = f.workflow("  before_run: 'anything'");
        let runner = FakeBoundary::default();
        let executor = f.executor(&runner);
        assert_eq!(
            executor
                .run_check(
                    CheckInvocation {
                        binding: &f.binding,
                        workflow: &w,
                        name: "unit",
                        kind: CheckKind::Local,
                        additional_outputs: &[],
                        previous_outputs: &[]
                    },
                    |_| Ok(())
                )
                .await
                .unwrap_err()
                .code,
            "scope_violation"
        );
        assert!(runner.calls.lock().unwrap().is_empty());
    }
}

#[test]
fn source_tool_preserves_scope_and_conflicts_without_invoking_git_inside_mutation() {
    let f = Fixture::new(ExecutionRole::Child);
    let runner = FakeBoundary::default();
    // Source mutation holds Control's gate. Git-backed revision observation is
    // not part of the exact file CAS, and must not launch/wait under that gate.
    let missing_git = f.root.join("unavailable-git");
    fs::write(&missing_git, b"fixture bytes; never executed").unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&missing_git, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let unavailable = Git::from_pinned_executable(&missing_git).unwrap();
    fs::remove_file(missing_git).unwrap();
    let executor = LocalExecutor {
        git: &unavailable,
        ..f.executor(&runner)
    };
    let receipt = executor
        .write_source(
            &f.binding,
            SourceWrite {
                relative_path: "a.txt",
                expected_digest: Some(&bytes_digest(b"original\n")),
                bytes: b"child implementation\n",
                executable: false,
            },
            |_| Ok(()),
        )
        .unwrap();
    assert!(receipt.completed);
    assert_eq!(
        fs::read_to_string(f.source.join("a.txt")).unwrap(),
        "original\n"
    );
    assert_eq!(
        fs::read_to_string(f.boundary.plan().repository.join("a.txt")).unwrap(),
        "child implementation\n"
    );
    assert_eq!(
        executor
            .write_source(
                &f.binding,
                SourceWrite {
                    relative_path: "a.txt",
                    expected_digest: Some(&bytes_digest(b"original\n")),
                    bytes: b"lost update",
                    executable: false
                },
                |_| Ok(())
            )
            .unwrap_err()
            .code,
        "source_conflict"
    );
    for path in ["../escape", ".git/config", "ungranted.txt"] {
        assert!(
            executor
                .write_source(
                    &f.binding,
                    SourceWrite {
                        relative_path: path,
                        expected_digest: None,
                        bytes: b"no",
                        executable: false
                    },
                    |_| Ok(())
                )
                .is_err()
        );
    }
}

fn mp4_header() -> Vec<u8> {
    b"\0\0\0\x18ftypisom\0\0\0\0isomavc1".to_vec()
}
fn probe() -> String {
    json!({"streams":[{"codec_type":"video","codec_name":"h264","width":1280,"height":720}],"format":{"format_name":"mov,mp4,m4a,3gp,3g2,mj2","duration":"3.0"}}).to_string()
}
#[tokio::test]
async fn video_origin_is_executed_candidate_output_and_probe_decode_are_required() {
    let f = Fixture::new(ExecutionRole::Validation);
    let w = f.workflow("  timeout_ms: 1000");
    let video = f.boundary.plan().scratch.join("recording.mp4");
    let output = f.boundary.plan().scratch.join("e2e.json");
    let runner = FakeBoundary::new(vec![
        Step {
            writes: vec![
                (video.clone(), mp4_header()),
                (output, b"observed E2E assertions".to_vec()),
            ],
            ..Step::success()
        },
        Step {
            stdout: probe(),
            ..Step::success()
        },
        Step::success(),
    ]);
    let executor = f.executor(&runner);
    let checked = executor
        .run_check(
            CheckInvocation {
                binding: &f.binding,
                workflow: &w,
                name: "e2e",
                kind: CheckKind::Integrated,
                additional_outputs: std::slice::from_ref(&video),
                previous_outputs: &[],
            },
            |_| Ok(()),
        )
        .await
        .unwrap();
    assert!(
        checked
            .capture_origin(&f.boundary.plan().scratch.join("unrelated.mp4"))
            .is_err()
    );
    let origin = checked.capture_origin(&video).unwrap();
    let tools = MediaTools {
        ffprobe: PathBuf::from("/usr/bin/true"),
        ffmpeg: PathBuf::from("/usr/bin/true"),
    };
    let verified = executor
        .validate_video(&origin, &tools, None, |_| Ok(()))
        .await
        .unwrap();
    assert_eq!(verified.record().output.digest, bytes_digest(&mp4_header()));
    assert_eq!(verified.record().duration_seconds, 3.0);
    assert!(verified.production_artifact().is_err());
    assert_eq!(runner.calls.lock().unwrap().len(), 3);
    fs::write(&video, b"unrelated MP4 content").unwrap();
    assert_eq!(
        executor
            .validate_video(&origin, &tools, None, |_| Ok(()))
            .await
            .unwrap_err()
            .code,
        "video_provenance"
    );
}

#[tokio::test]
async fn dirty_source_cannot_be_labeled_with_a_clean_head_revision() {
    let f = Fixture::new(ExecutionRole::Child);
    let w = f.workflow("  timeout_ms: 1000");
    let runner = FakeBoundary::default();
    fs::write(
        f.boundary.plan().repository.join("a.txt"),
        b"uncommitted worker change",
    )
    .unwrap();
    assert_eq!(
        f.executor(&runner)
            .run_check(
                CheckInvocation {
                    binding: &f.binding,
                    workflow: &w,
                    name: "unit",
                    kind: CheckKind::Local,
                    additional_outputs: &[],
                    previous_outputs: &[]
                },
                |_| Ok(())
            )
            .await
            .unwrap_err()
            .code,
        "dirty_execution"
    );
    assert!(runner.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn cleanup_hook_failure_preserves_directory_and_requires_a_real_cleanup_plan() {
    let f = Fixture::new(ExecutionRole::Child);
    let w = f.workflow("  before_remove: 'exit 1'");
    let runner = FakeBoundary::new(vec![
        Step {
            exit: Some(1),
            ..Step::default()
        },
        Step::success(),
    ]);
    let executor = f.executor(&runner);
    let mut ledger = HookLedger::new(&f.binding).unwrap();
    executor
        .run_hook(
            &mut ledger,
            hook(&f, &w, HookPhase::AfterCreate, None),
            |_| Ok(()),
        )
        .await
        .unwrap();
    assert!(
        executor
            .run_hook(
                &mut ledger,
                hook(&f, &w, HookPhase::BeforeRemove, None),
                |_| Ok(())
            )
            .await
            .is_err()
    );
    let plan = bytes_digest(b"revalidated explicit cleanup");
    let mut request = hook(&f, &w, HookPhase::BeforeRemove, None);
    request.eligible_cleanup_plan = Some(&plan);
    let result = executor
        .run_hook(&mut ledger, request, |_| Ok(()))
        .await
        .unwrap();
    assert_eq!(result.state, HookState::Failed);
    assert!(!result.allows_next_phase);
    assert!(!result.diagnostic_only);
    assert!(f.boundary.plan().repository.join("a.txt").exists());
    let mut repeated = hook(&f, &w, HookPhase::BeforeRemove, None);
    repeated.eligible_cleanup_plan = Some(&plan);
    assert_eq!(
        executor
            .run_hook(&mut ledger, repeated, |_| Ok(()))
            .await
            .unwrap_err()
            .code,
        "reconciliation_required"
    );
    let refreshed = bytes_digest(b"new explicit cleanup operation, freshly revalidated");
    let mut retry = hook(&f, &w, HookPhase::BeforeRemove, None);
    retry.eligible_cleanup_plan = Some(&refreshed);
    let result = executor
        .run_hook(&mut ledger, retry, |_| Ok(()))
        .await
        .unwrap();
    assert!(result.allows_next_phase);
    assert_eq!(
        ledger
            .records
            .values()
            .filter(|r| r.phase == HookPhase::BeforeRemove)
            .count(),
        2
    );
    assert!(
        ledger
            .records
            .values()
            .any(|r| r.state == HookState::Failed)
    );
}

#[tokio::test]
async fn invalid_codec_duration_or_decode_cannot_produce_video_evidence() {
    let f = Fixture::new(ExecutionRole::Validation);
    let w = f.workflow("  timeout_ms: 1000");
    let video = f.boundary.plan().scratch.join("video.mp4");
    let capture = FakeBoundary::new(vec![Step {
        writes: vec![
            (video.clone(), mp4_header()),
            (
                f.boundary.plan().scratch.join("e2e.json"),
                b"observed".to_vec(),
            ),
        ],
        ..Step::success()
    }]);
    let checked = f
        .executor(&capture)
        .run_check(
            CheckInvocation {
                binding: &f.binding,
                workflow: &w,
                name: "e2e",
                kind: CheckKind::Integrated,
                additional_outputs: std::slice::from_ref(&video),
                previous_outputs: &[],
            },
            |_| Ok(()),
        )
        .await
        .unwrap();
    let origin = checked.capture_origin(&video).unwrap();
    let tools = MediaTools {
        ffprobe: PathBuf::from("/usr/bin/true"),
        ffmpeg: PathBuf::from("/usr/bin/true"),
    };
    for invalid in [
        probe().replace("h264", "vp9"),
        probe().replace("3.0", "NaN"),
        probe().replace("3.0", "0.0"),
        probe().replace("3.0", "999999"),
    ] {
        let runner = FakeBoundary::new(vec![Step {
            stdout: invalid,
            ..Step::success()
        }]);
        assert_eq!(
            f.executor(&runner)
                .validate_video(&origin, &tools, None, |_| Ok(()))
                .await
                .unwrap_err()
                .code,
            "media_invalid"
        );
    }
    let decode = FakeBoundary::new(vec![
        Step {
            stdout: probe(),
            ..Step::success()
        },
        Step {
            exit: Some(1),
            ..Step::default()
        },
    ]);
    assert_eq!(
        f.executor(&decode)
            .validate_video(&origin, &tools, None, |_| Ok(()))
            .await
            .unwrap_err()
            .code,
        "media_invalid"
    );
    let changed_during_decode = FakeBoundary::new(vec![
        Step {
            stdout: probe(),
            ..Step::success()
        },
        Step {
            writes: vec![(
                f.boundary.plan().repository.join("a.txt"),
                b"changed concurrently".to_vec(),
            )],
            ..Step::success()
        },
    ]);
    let mut journal = Vec::new();
    assert_eq!(
        f.executor(&changed_during_decode)
            .validate_video(&origin, &tools, None, |entry| {
                journal.push(serde_json::to_value(entry).unwrap());
                Ok(())
            })
            .await
            .unwrap_err()
            .code,
        "dirty_execution"
    );
    assert_eq!(journal.len(), 1);
    assert_eq!(journal[0]["state"], "started");
}

#[tokio::test]
async fn transcode_uses_only_complete_captured_flow_and_preserves_original_artifact() {
    let f = Fixture::new(ExecutionRole::Validation);
    let w = f.workflow("  timeout_ms: 1000");
    let raw = f.boundary.plan().scratch.join("raw.webm");
    let converted = f.boundary.plan().scratch.join("review.mp4");
    let runner = FakeBoundary::new(vec![
        Step {
            writes: vec![
                (raw.clone(), b"recorded raw flow fixture".to_vec()),
                (
                    f.boundary.plan().scratch.join("e2e.json"),
                    b"observed".to_vec(),
                ),
            ],
            ..Step::success()
        },
        Step {
            writes: vec![(converted.clone(), mp4_header())],
            ..Step::success()
        },
        Step {
            stdout: probe(),
            ..Step::success()
        },
        Step::success(),
    ]);
    let executor = f.executor(&runner);
    let check = executor
        .run_check(
            CheckInvocation {
                binding: &f.binding,
                workflow: &w,
                name: "e2e",
                kind: CheckKind::Integrated,
                additional_outputs: std::slice::from_ref(&raw),
                previous_outputs: &[],
            },
            |_| Ok(()),
        )
        .await
        .unwrap();
    let origin = check.capture_origin(&raw).unwrap();
    let tools = MediaTools {
        ffprobe: PathBuf::from("/usr/bin/true"),
        ffmpeg: PathBuf::from("/usr/bin/true"),
    };
    let mut journal = vec![];
    let validated = executor
        .validate_video(&origin, &tools, Some(&converted), |e| {
            journal.push(serde_json::to_value(e).unwrap());
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(journal[0]["state"], "started");
    assert_eq!(journal[1]["state"], "finished");
    assert!(validated.record().conversion_receipt_digest.is_some());
    assert_eq!(fs::read(&raw).unwrap(), b"recorded raw flow fixture");
    let calls = runner.calls.lock().unwrap();
    let conversion = &calls[1];
    assert!(conversion.iter().any(|a| a == "libx264"));
    assert!(conversion.iter().any(|a| a == "aac"));
    assert!(conversion.iter().any(|a| a == "-n"));
    assert!(
        !conversion
            .iter()
            .any(|a| ["-t", "-ss", "lavfi", "testsrc"].contains(&a.as_str()))
    );
}

#[test]
fn retained_directory_identity_and_hardlinked_source_cannot_be_adopted() {
    let f = Fixture::new(ExecutionRole::Child);
    let marker = fs::read_dir(&f.root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("owned-execution-")
        })
        .unwrap();
    assert!(
        OwnedDirectory::reopen(
            &marker,
            &bytes_digest(b"different-owned-record"),
            &f.boundary
        )
        .is_err()
    );
    let source = f.boundary.plan().repository.join("a.txt");
    fs::hard_link(&source, f.root.join("external-alias")).unwrap();
    let runner = FakeBoundary::default();
    assert!(
        f.executor(&runner)
            .write_source(
                &f.binding,
                SourceWrite {
                    relative_path: "a.txt",
                    expected_digest: Some(&bytes_digest(b"original\n")),
                    bytes: b"no",
                    executable: false
                },
                |_| Ok(())
            )
            .is_err()
    );
    assert_eq!(fs::read(&source).unwrap(), b"original\n");
}

// Generic-command components use the actual LocalExecutor, owned source and
// immutable binding. Their injected outcome/closure is explicitly synthetic.
struct GenericBoundary {
    inner: FakeBoundary,
    closed: bool,
    waited: std::sync::Arc<std::sync::atomic::AtomicBool>,
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    remaining: Mutex<Vec<Duration>>,
}
impl GenericBoundary {
    fn new(step: Step) -> Self {
        Self {
            inner: FakeBoundary::new(vec![step]),
            closed: true,
            waited: Default::default(),
            cancelled: Default::default(),
            remaining: Default::default(),
        }
    }
}
struct GenericCancellation {
    closed: bool,
    waited: std::sync::Arc<std::sync::atomic::AtomicBool>,
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
impl CommandCancellation for GenericCancellation {
    fn cancel(&self) -> Result<()> {
        self.cancelled
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
    fn exited(&self) -> Result<bool> {
        Ok(self.closed && self.waited.load(std::sync::atomic::Ordering::SeqCst))
    }
}
struct GenericStarted {
    inner: Box<dyn StartedCommand>,
    cancellation: std::sync::Arc<GenericCancellation>,
}
impl StartedCommand for GenericStarted {
    fn cancellation(&self) -> std::sync::Arc<dyn CommandCancellation> {
        self.cancellation.clone()
    }
    fn wait(self: Box<Self>) -> Pin<Box<dyn Future<Output = Result<CommandOutcome>> + Send>> {
        Box::pin(async move {
            let result = self.inner.wait().await;
            self.cancellation
                .waited
                .store(true, std::sync::atomic::Ordering::SeqCst);
            result
        })
    }
}
impl CommandBoundary for GenericBoundary {
    fn evidence_class(&self) -> ExecutionEvidenceClass {
        ExecutionEvidenceClass::DeterministicFixture
    }
    fn start(
        &self,
        isolation: &ValidatedIsolation,
        argv: &[String],
        timeout: Duration,
        output_limit: usize,
    ) -> Result<Box<dyn StartedCommand>> {
        self.remaining.lock().unwrap().push(timeout);
        let inner = self.inner.start(isolation, argv, timeout, output_limit)?;
        Ok(Box::new(GenericStarted {
            inner,
            cancellation: std::sync::Arc::new(GenericCancellation {
                closed: self.closed,
                waited: self.waited.clone(),
                cancelled: self.cancelled.clone(),
            }),
        }))
    }
}
fn generic_executor<'a>(f: &'a Fixture, boundary: &'a dyn CommandBoundary) -> LocalExecutor<'a> {
    LocalExecutor {
        isolation: &f.boundary,
        git: &f.git,
        boundary,
        limits: ExecutionLimits::default(),
        ownership: &f.owner,
    }
}
fn generic_request() -> CommandRequest {
    CommandRequest {
        argv: vec!["/usr/bin/true".into(), "literal $(not-a-shell)".into()],
        cwd: ".".into(),
    }
}

#[tokio::test]
async fn generic_command_observes_dirty_authorized_source_without_issuing_check_evidence() {
    let f = Fixture::new(ExecutionRole::Child);
    let source = f.boundary.plan().repository.join("a.txt");
    fs::write(&source, b"dirty candidate\n").unwrap();
    let boundary = GenericBoundary::new(Step {
        writes: vec![(source.clone(), b"implemented candidate\n".to_vec())],
        ..Step::success()
    });
    let executor = generic_executor(&f, &boundary);
    let invocation = Uuid::new_v4();
    let mut events = Vec::new();
    let prepared = executor
        .prepare_command(&f.binding, &generic_request(), invocation, None, |e| {
            events.push(e.clone());
            Ok(())
        })
        .unwrap();
    let running = executor
        .start_command(prepared, |e| {
            events.push(e.clone());
            Ok(())
        })
        .unwrap();
    assert!(!running.cancellation().exited().unwrap());
    let record = executor
        .complete_command(running, |e| {
            events.push(e.clone());
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(record.preparation.invocation_id, invocation);
    assert_eq!(
        record.preparation.evidence_class,
        ExecutionEvidenceClass::DeterministicFixture
    );
    assert_eq!(
        record.preparation.before.files["a.txt"].digest,
        bytes_digest(b"dirty candidate\n")
    );
    assert_eq!(
        record.after.files["a.txt"].digest,
        bytes_digest(b"implemented candidate\n")
    );
    assert_eq!(record.after.revision, f.binding.revision);
    assert_eq!(record.outcome.argv, generic_request().argv);
    assert_eq!(
        record.preparation_digest,
        record.preparation.digest().unwrap()
    );
    assert!(matches!(
        events.as_slice(),
        [
            CommandJournalEntry::Prepared { .. },
            CommandJournalEntry::DispatchRequested { .. },
            CommandJournalEntry::Started { .. },
            CommandJournalEntry::Completed { .. }
        ]
    ));
    assert_eq!(fs::read(f.source.join("a.txt")).unwrap(), b"original\n");
    assert!(
        boundary.remaining.lock().unwrap()[0]
            < Duration::from_millis(executor.limits.command_timeout_ms)
    );
}

#[tokio::test]
async fn generic_parent_command_preserves_literal_argv_cwd_and_nonzero_or_timeout_facts() {
    for (exit, timeout) in [(Some(7), false), (None, true)] {
        let f = Fixture::new(ExecutionRole::Validation);
        fs::create_dir(f.boundary.plan().repository.join("nested")).unwrap();
        let boundary = GenericBoundary::new(Step {
            exit,
            timeout,
            stdout: "observed output".into(),
            ..Step::default()
        });
        let executor = generic_executor(&f, &boundary);
        let mut request = generic_request();
        request.cwd = "nested".into();
        let prepared = executor
            .prepare_command(&f.binding, &request, Uuid::new_v4(), None, |_| Ok(()))
            .unwrap();
        let running = executor.start_command(prepared, |_| Ok(())).unwrap();
        let record = executor
            .complete_command(running, |_| Ok(()))
            .await
            .unwrap();
        assert_eq!(record.outcome.argv, request.argv);
        assert_eq!(
            record.outcome.cwd,
            f.boundary.plan().repository.join("nested")
        );
        assert_eq!(record.outcome.exit_code, exit);
        assert_eq!(record.outcome.timed_out, timeout);
        assert_eq!(record.preparation.before_digest, record.after_digest);
    }
}

#[test]
fn generic_prepared_source_addition_refuses_before_dispatch_and_records_known_no_start() {
    let f = Fixture::new(ExecutionRole::Child);
    let boundary = GenericBoundary::new(Step::success());
    let executor = generic_executor(&f, &boundary);
    let prepared = executor
        .prepare_command(&f.binding, &generic_request(), Uuid::new_v4(), None, |_| {
            Ok(())
        })
        .unwrap();
    fs::create_dir(f.boundary.plan().repository.join("new-directory")).unwrap();
    let mut events = Vec::new();
    let error = executor
        .start_command(prepared, |e| {
            events.push(e.clone());
            Ok(())
        })
        .err()
        .unwrap();
    assert_eq!(error.code, "stale_command");
    assert!(boundary.inner.calls.lock().unwrap().is_empty());
    assert!(matches!(
        events.as_slice(),
        [CommandJournalEntry::NotStarted { .. }]
    ));
}

#[test]
fn generic_native_prerequisite_and_cwd_escape_never_request_dispatch() {
    let f = Fixture::new(ExecutionRole::Child);
    let executor = generic_executor(&f, &SandboxBoundary);
    let prepared = executor
        .prepare_command(&f.binding, &generic_request(), Uuid::new_v4(), None, |_| {
            Ok(())
        })
        .unwrap();
    let error = executor.preflight_command(&prepared).unwrap_err();
    assert_eq!(error.code, "process_journal_required");
    let mut events = Vec::new();
    executor
        .refuse_command(prepared, error, |e| {
            events.push(e.clone());
            Ok(())
        })
        .unwrap();
    assert!(matches!(
        events.as_slice(),
        [CommandJournalEntry::NotStarted { .. }]
    ));
    let mut request = generic_request();
    request.cwd = f.source.to_string_lossy().into_owned();
    assert!(
        executor
            .prepare_command(&f.binding, &request, Uuid::new_v4(), None, |_| panic!(
                "no preparation for escaped cwd"
            ))
            .is_err()
    );
}

#[tokio::test]
async fn generic_started_persistence_failure_keeps_owner_and_actual_outcome_until_waited() {
    let f = Fixture::new(ExecutionRole::Child);
    let boundary = GenericBoundary::new(Step::success());
    let executor = generic_executor(&f, &boundary);
    let prepared = executor
        .prepare_command(&f.binding, &generic_request(), Uuid::new_v4(), None, |_| {
            Ok(())
        })
        .unwrap();
    let running = executor
        .start_command(prepared, |e| {
            if matches!(e, CommandJournalEntry::Started { .. }) {
                Err(Error::new(
                    "fixture_persistence",
                    "injected started write refusal",
                ))
            } else {
                Ok(())
            }
        })
        .unwrap();
    assert!(boundary.cancelled.load(std::sync::atomic::Ordering::SeqCst));
    assert!(!boundary.waited.load(std::sync::atomic::Ordering::SeqCst));
    let mut events = Vec::new();
    let error = executor
        .complete_command(running, |e| {
            events.push(e.clone());
            Ok(())
        })
        .await
        .unwrap_err();
    assert_eq!(error.code, "fixture_persistence");
    assert!(boundary.waited.load(std::sync::atomic::Ordering::SeqCst));
    assert!(matches!(
        events.as_slice(),
        [CommandJournalEntry::Uncertain {
            outcome: Some(CommandOutcome {
                exit_code: Some(0),
                ..
            }),
            ..
        }]
    ));
}

#[tokio::test]
async fn generic_unknown_closure_and_changed_read_resource_cannot_complete() {
    for changed_resource in [false, true] {
        let (f, resource) = Fixture::with_content_resource(ExecutionRole::Child);
        let mut boundary = GenericBoundary::new(Step {
            writes: if changed_resource {
                vec![(resource, b"changed immutable resource\n".to_vec())]
            } else {
                vec![]
            },
            ..Step::success()
        });
        boundary.closed = changed_resource;
        let executor = generic_executor(&f, &boundary);
        let prepared = executor
            .prepare_command(&f.binding, &generic_request(), Uuid::new_v4(), None, |_| {
                Ok(())
            })
            .unwrap();
        let running = executor.start_command(prepared, |_| Ok(())).unwrap();
        let mut events = Vec::new();
        assert!(
            executor
                .complete_command(running, |e| {
                    events.push(e.clone());
                    Ok(())
                })
                .await
                .is_err()
        );
        assert!(matches!(
            events.as_slice(),
            [CommandJournalEntry::Uncertain {
                outcome: Some(_),
                ..
            }]
        ));
    }
}
