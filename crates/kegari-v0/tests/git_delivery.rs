#[path = "support/pinned_git.rs"]
mod pinned_git;
#[path = "support/process_probe.rs"]
mod process_probe;
// Deterministic fixtures use real disposable Git repositories. Provider facts
// below are explicitly simulated complete snapshots, not live qualification.
use chrono::{Duration, Utc};
use kegari_digest::{bytes_digest, canonical_digest};
use kegari_secrets::{AttestationKey, Signed};
use kegari_types::error::{Error, Result};
use kegari_v0::evidence::*;
use kegari_v0::git::*;
use kegari_v0::release::*;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;
use uuid::Uuid;

fn git_binary() -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|p| p.join("git"))
        .find(|p| p.is_file())
        .expect("Git is required")
}

fn git_run(path: &Path, args: &[&str]) -> String {
    let output = kegari_process::group::OwnedGroup::spawn_std(
        Command::new(git_binary())
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_AUTHOR_NAME", "Disposable Fixture Human")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.test")
            .env("GIT_COMMITTER_NAME", "Disposable Fixture Human")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.test")
            .current_dir(path)
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
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .trim_end()
        .to_string()
}

fn commit(path: &Path, message: &str) -> String {
    git_run(path, &["add", "-A"]);
    git_run(path, &["commit", "--signoff", "-m", message]);
    git_run(path, &["rev-parse", "HEAD"])
}

fn private_dir(path: &Path) {
    fs::create_dir(path).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
}

#[test]
fn scoped_git_metadata_is_journaled_registered_and_fenced_before_each_spawn() {
    use kegari_storage::PrivateDir;
    use kegari_v0::{
        control::{Lease, Permit},
        process_records::{ProcessJournal, RecoveryState},
        worker::ExecutionRegistry,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let fixture = Fixture::new(false);
    let git = Git::from_pinned_executable(fixture.git.executable()).unwrap();
    assert!(git.version().is_empty());
    assert_eq!(
        git.revision(&fixture.source, "HEAD").unwrap_err().code,
        "unsupported_git_version"
    );
    let runtime = Uuid::new_v4();
    let operation = Uuid::new_v4();
    let records = Arc::new(
        ProcessJournal::with_probe(
            PrivateDir::create(
                &fixture
                    ._root
                    .path()
                    .canonicalize()
                    .unwrap()
                    .join("processes"),
            )
            .unwrap(),
            runtime,
            Arc::new(process_probe::FixtureBootProbe::new()),
        )
        .unwrap(),
    );
    let executions = Arc::new(ExecutionRegistry::default());
    let permitted = Arc::new(AtomicBool::new(true));
    let current = permitted.clone();
    let scoped = git
        .with_execution_scope(GitExecutionScope {
            limits: GitLimits::default(),
            start_gate: Arc::new(move |start| {
                if !current.load(Ordering::SeqCst) {
                    return Err(Error::new("stale_generation", "fixture fence"));
                }
                start()
            }),
            processes: records.clone(),
            permit: Permit {
                operation_id: operation,
                lease: Lease {
                    runtime_id: runtime,
                    owner_generation: 1,
                    owner_id: Uuid::new_v4(),
                },
                control_generation: 1,
                work_key: d("scoped fixture"),
                scope_digest: d("scope"),
                registered_at: Utc::now(),
            },
            executions: executions.clone(),
        })
        .unwrap();
    assert_eq!(scoped.version(), SUPPORTED_GIT_VERSION);
    assert_eq!(
        scoped.revision(&fixture.source, "HEAD").unwrap().commit,
        fixture.squash
    );
    assert!(executions.active_operations().unwrap().is_empty());
    let observed = records.observe(operation).unwrap();
    assert!(observed.len() >= 3);
    assert!(observed.iter().all(
        |record| record.state == RecoveryState::OwnedGroupExitRecorded
            && record.owned_group_exited
            && !record.all_descendants_exited
    ));
    permitted.store(false, Ordering::SeqCst);
    assert_eq!(
        scoped.revision(&fixture.source, "HEAD").unwrap_err().code,
        "stale_generation"
    );
    assert_eq!(records.observe(operation).unwrap().len(), observed.len());
    assert_eq!(
        git_run(&fixture.source, &["rev-parse", "HEAD"]),
        fixture.squash
    );
}

#[tokio::test]
async fn owned_git_sequence_fences_every_subprocess_and_preserves_fixture_checkout() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    let fixture = Fixture::new(false);
    let source = fixture.source.clone();
    let original = git_run(&source, &["rev-parse", "HEAD"]);
    let current = Arc::new(AtomicBool::new(true));
    let starts = Arc::new(AtomicUsize::new(0));
    let gate_current = current.clone();
    let gate_starts = starts.clone();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (continue_tx, continue_rx) = std::sync::mpsc::channel();
    let running = fixture
        .git
        .start_operation(
            GitLimits::default(),
            Arc::new(move |launch| {
                if !gate_current.load(Ordering::SeqCst) {
                    return Err(Error::new("stale_generation", "fixture durable fence"));
                }
                gate_starts.fetch_add(1, Ordering::SeqCst);
                launch()
            }),
            move |git| {
                let first = git.revision(&source, "HEAD")?;
                ready_tx.send(first).unwrap();
                continue_rx.recv().unwrap();
                git.revision(&source, "HEAD")
            },
        )
        .unwrap();
    assert_eq!(
        ready_rx
            .recv_timeout(std::time::Duration::from_secs(3))
            .unwrap()
            .commit,
        original
    );
    let completed_starts = starts.load(Ordering::SeqCst);
    current.store(false, Ordering::SeqCst);
    continue_tx.send(()).unwrap();
    assert_eq!(running.wait().await.unwrap_err().code, "stale_generation");
    assert_eq!(starts.load(Ordering::SeqCst), completed_starts);
    assert_eq!(git_run(&fixture.source, &["rev-parse", "HEAD"]), original);
    assert!(git_run(&fixture.source, &["status", "--porcelain"]).is_empty());
}

#[tokio::test]
async fn owned_git_output_bound_and_cancellation_prevent_following_commands() {
    use std::sync::Arc;
    let fixture = Fixture::new(false);
    let source = fixture.source.clone();
    let base = fixture.base.clone();
    let head = fixture.head.clone();
    let running = fixture
        .git
        .start_operation(
            GitLimits {
                output_bytes: 16,
                ..GitLimits::default()
            },
            Arc::new(|launch| launch()),
            move |git| git.patch(&source, &base, &head),
        )
        .unwrap();
    assert_eq!(running.wait().await.unwrap_err().code, "output_limit");

    let source = fixture.source.clone();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (continue_tx, continue_rx) = std::sync::mpsc::channel();
    let running = fixture
        .git
        .start_operation(
            GitLimits::default(),
            Arc::new(|launch| launch()),
            move |git| {
                ready_tx.send(()).unwrap();
                continue_rx.recv().unwrap();
                git.revision(&source, "HEAD")
            },
        )
        .unwrap();
    ready_rx
        .recv_timeout(std::time::Duration::from_secs(3))
        .unwrap();
    running.cancellation().cancel().unwrap();
    continue_tx.send(()).unwrap();
    assert_eq!(running.wait().await.unwrap_err().code, "cancelled");
}

#[tokio::test]
async fn execution_clone_is_exact_independent_and_reuses_dirty_work_without_reset() {
    use std::sync::Arc;
    let fixture = Fixture::new(false);
    fs::write(
        fixture.source.join("operator-untracked"),
        "preserve operator work",
    )
    .unwrap();
    fs::write(fixture.source.join("a.txt"), "operator staged bytes").unwrap();
    git_run(&fixture.source, &["add", "a.txt"]);
    fs::write(fixture.source.join("a.txt"), "operator working bytes").unwrap();
    let index = fs::read(fixture.source.join(".git/index")).unwrap();
    let path = fixture.runs.canonicalize().unwrap().join("repository");
    let identity = kegari_v0::execution::DirectoryIdentity {
        schema_version: 1,
        runtime_id: Uuid::new_v4(),
        directory_id: Uuid::new_v4(),
        repository: fixture.repository.clone(),
        work: work(fixture.parent, Some(fixture.child)),
        repository_path: path.clone(),
        operator_checkout: fixture.source.canonicalize().unwrap(),
    };
    let revision = fixture
        .git
        .exact_revision(&fixture.source, &fixture.base)
        .unwrap();
    let source = fixture.source.clone();
    let root = fixture.runs.clone();
    let owned_identity = identity.clone();
    let owned_revision = revision.clone();
    let running = fixture
        .git
        .start_operation(
            GitLimits::default(),
            Arc::new(|launch| launch()),
            move |git| {
                git.prepare_execution_clone(&source, &root, &owned_identity, &owned_revision)
            },
        )
        .unwrap();
    assert_eq!(running.wait().await.unwrap(), path);
    assert_eq!(fixture.git.revision(&path, "HEAD").unwrap(), revision);
    assert!(!path.join("operator-untracked").exists());
    assert!(git_run(&path, &["status", "--porcelain"]).is_empty());
    fs::write(path.join("a.txt"), "retained staged edit").unwrap();
    git_run(&path, &["add", "a.txt"]);
    fs::write(path.join("a.txt"), "retained later edit").unwrap();
    let retained_index = fs::read(path.join(".git/index")).unwrap();
    fixture
        .git
        .prepare_execution_clone(&fixture.source, &fixture.runs, &identity, &revision)
        .unwrap();
    assert_eq!(fs::read(path.join(".git/index")).unwrap(), retained_index);
    assert_eq!(
        fs::read_to_string(path.join("a.txt")).unwrap(),
        "retained later edit"
    );
    assert_eq!(fs::read(fixture.source.join(".git/index")).unwrap(), index);
    assert_eq!(
        fs::read_to_string(fixture.source.join("a.txt")).unwrap(),
        "operator working bytes"
    );
    let mut wrong = identity.clone();
    wrong.work.generation.definition = bytes_digest(b"different generation");
    assert_eq!(
        fixture
            .git
            .prepare_execution_clone(&fixture.source, &fixture.runs, &wrong, &revision)
            .unwrap_err()
            .code,
        "reconciliation_required"
    );
    let other = fixture
        .git
        .exact_revision(&fixture.source, &fixture.head)
        .unwrap();
    assert_eq!(
        fixture
            .git
            .prepare_execution_clone(&fixture.source, &fixture.runs, &identity, &other)
            .unwrap_err()
            .code,
        "reconciliation_required"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let relative = format!("objects/{}/{}", &fixture.base[..2], &fixture.base[2..]);
        assert_ne!(
            fs::metadata(path.join(".git").join(&relative))
                .unwrap()
                .ino(),
            fs::metadata(fixture.source.join(".git").join(relative))
                .unwrap()
                .ino()
        );
    }
}

#[test]
fn independent_review_clone_binds_provider_pr_without_fabricated_parent_authority() {
    let fixture = Fixture::new(false);
    let head = fixture
        .git
        .exact_revision(&fixture.source, &fixture.head)
        .unwrap();
    let base = fixture
        .git
        .exact_revision(&fixture.source, &fixture.base)
        .unwrap();
    let root = fixture.runs.canonicalize().unwrap();
    let identity = IndependentCloneIdentity {
        schema_version: 1,
        runtime_id: Uuid::new_v4(),
        workspace_id: Uuid::new_v4(),
        directory_id: Uuid::new_v4(),
        repository: fixture.repository.clone(),
        pull_request: 9,
        review_identity: canonical_digest(
            1,
            &serde_json::json!({"repository":101,"pr":9,"head":head.commit,"base":base.commit}),
        )
        .unwrap(),
        head,
        base,
        repository_path: root.join("review"),
        operator_checkout: fixture.source.canonicalize().unwrap(),
    };
    assert!(
        fixture
            .git
            .observe_independent_review_clone(&root, &identity)
            .is_err()
    );
    assert!(!identity.repository_path.exists());
    assert!(
        !root
            .join(format!("prepared-clone-{}.json", identity.directory_id))
            .exists()
    );
    let path = fixture
        .git
        .prepare_independent_review_clone(&fixture.source, &root, &identity)
        .unwrap();
    assert_eq!(fixture.git.revision(&path, "HEAD").unwrap(), identity.head);
    let index = fs::read(path.join(".git/index")).unwrap();
    fs::write(
        path.join("retained-untracked-note"),
        "preserve review scratch\n",
    )
    .unwrap();
    let observed = fixture
        .git
        .observe_independent_review_clone(&root, &identity)
        .unwrap();
    assert_eq!(observed.identity(), &identity);
    assert_eq!(observed.root(), root.as_path());
    observed.revalidate_filesystem().unwrap();
    assert_eq!(fs::read(path.join(".git/index")).unwrap(), index);
    assert_eq!(
        fs::read_to_string(path.join("retained-untracked-note")).unwrap(),
        "preserve review scratch\n"
    );
    let record: serde_json::Value = serde_json::from_slice(
        &fs::read(root.join(format!("prepared-clone-{}.json", identity.directory_id))).unwrap(),
    )
    .unwrap();
    assert_eq!(record["identity"]["kind"], "independent_review");
    assert!(record["identity"].get("work").is_none());
    assert!(record["identity"].get("parent").is_none());
    let mut wrong = identity.clone();
    wrong.pull_request += 1;
    assert_eq!(
        fixture
            .git
            .prepare_independent_review_clone(&fixture.source, &root, &wrong)
            .unwrap_err()
            .code,
        "execution_ownership"
    );
    assert_eq!(fixture.git.revision(&path, "HEAD").unwrap(), identity.head);
    let marker = root.join(format!("prepared-clone-{}.json", identity.directory_id));
    let mut tampered = record;
    tampered["prepared"] = serde_json::json!(false);
    fs::write(marker, serde_json::to_vec(&tampered).unwrap()).unwrap();
    assert_eq!(
        observed.revalidate_filesystem().unwrap_err().code,
        "execution_ownership"
    );
}

struct Fixture {
    _root: TempDir,
    source: PathBuf,
    runs: PathBuf,
    git: Git,
    repository: RepositoryId,
    parent: Uuid,
    child: Uuid,
    production: String,
    base: String,
    head: String,
    squash: String,
}

impl Fixture {
    fn new(dependent_on_b: bool) -> Self {
        let root = TempDir::new().unwrap();
        let source = root.path().join("operator-checkout");
        let runs = root.path().join("owned-runs");
        private_dir(&source);
        private_dir(&runs);
        git_run(&source, &["init", "-b", "main"]);
        fs::write(source.join("a.txt"), "first\nsecond\nthird\nfourth\n").unwrap();
        let production = commit(&source, "feat: establish production baseline");
        git_run(&source, &["switch", "-c", "staging"]);
        fs::write(
            source.join("unapproved-b.txt"),
            "B must never enter A's release\n",
        )
        .unwrap();
        if dependent_on_b {
            fs::write(
                source.join("a.txt"),
                "first\nB-only context\nthird\nfourth\n",
            )
            .unwrap();
        }
        let base = commit(&source, "feat: stage unapproved parent B");
        git_run(&source, &["switch", "-c", "CHILD-1-Full-Parent-A-Title"]);
        fs::write(
            source.join("a.txt"),
            if dependent_on_b {
                "first\nB-only context\nthird\nA fourth\n"
            } else {
                "first\nsecond\nthird\nA fourth\n"
            },
        )
        .unwrap();
        let head = commit(&source, "feat: implement parent A");
        git_run(&source, &["switch", "staging"]);
        git_run(
            &source,
            &["merge", "--squash", "CHILD-1-Full-Parent-A-Title"],
        );
        git_run(
            &source,
            &[
                "commit",
                "--signoff",
                "-m",
                "feat: squash parent A into staging",
            ],
        );
        let squash = git_run(&source, &["rev-parse", "HEAD"]);
        let owned_git = Git::new(pinned_git::prepare(root.path())).unwrap();
        Self {
            _root: root,
            source,
            runs,
            git: owned_git,
            repository: RepositoryId {
                hostname: "github.com".into(),
                id: "101".into(),
            },
            parent: Uuid::new_v4(),
            child: Uuid::new_v4(),
            production,
            base,
            head,
            squash,
        }
    }

    fn delta(&self) -> DeltaArtifact {
        self.git
            .capture_delta(
                &self.source,
                CaptureDelta {
                    repository: self.repository.clone(),
                    parent_id: self.parent,
                    child_id: self.child,
                    work_generation: &d("definition"),
                    correction_generation: &d("correction"),
                    scope: WriteScope {
                        paths: vec!["a.txt".into()],
                    },
                    base: &self.base,
                    head: &self.head,
                    staging_parent: &self.base,
                    staging_commit: &self.squash,
                },
            )
            .unwrap()
    }

    fn candidate(&self, operation: Uuid, deltas: &[DeltaArtifact]) -> Result<Candidate> {
        self.git.assemble_candidate(
            &self.source,
            &self.runs,
            CandidateRequest {
                operation_id: operation,
                runtime_id: Uuid::new_v4(),
                repository: self.repository.clone(),
                parent_id: self.parent,
                work_generation: &d("definition"),
                correction_generation: &d("correction"),
                production_commit: &self.production,
                deltas,
                child_dependencies: &BTreeMap::new(),
                satisfied_children: &BTreeSet::new(),
                production_integrations: &[],
                identity: CommitIdentity {
                    name: "Disposable Fixture Human".into(),
                    email: "fixture@example.test".into(),
                },
            },
        )
    }
}

fn d(value: &str) -> String {
    canonical_digest(1, &value).unwrap()
}

#[test]
fn v0_t16_real_candidate_excludes_staged_b_and_preserves_operator_partial_staging() {
    let f = Fixture::new(false);
    let delta = f.delta();
    fs::write(f.source.join("a.txt"), "operator staged change\n").unwrap();
    git_run(&f.source, &["add", "a.txt"]);
    fs::write(f.source.join("a.txt"), "operator unstaged change\n").unwrap();
    fs::write(
        f.source.join("private-note.txt"),
        "unfinished operator work\n",
    )
    .unwrap();
    let status_before = git_run(&f.source, &["status", "--porcelain=v1"]);
    let staged_before = git_run(&f.source, &["diff", "--cached", "--binary"]);
    let candidate = f.candidate(Uuid::new_v4(), &[delta]).unwrap();
    assert_eq!(
        fs::read_to_string(candidate.path().join("a.txt")).unwrap(),
        "first\nsecond\nthird\nA fourth\n"
    );
    assert!(!candidate.path().join("unapproved-b.txt").exists());
    assert!(!candidate.path().join("private-note.txt").exists());
    assert_eq!(
        git_run(&f.source, &["status", "--porcelain=v1"]),
        status_before
    );
    assert_eq!(
        git_run(&f.source, &["diff", "--cached", "--binary"]),
        staged_before
    );
    assert_eq!(
        fs::read_to_string(f.source.join("a.txt")).unwrap(),
        "operator unstaged change\n"
    );
    assert_eq!(
        git_run(candidate.path(), &["rev-parse", "HEAD^"]),
        f.production
    );
    assert!(
        git_run(candidate.path(), &["log", "-1", "--format=%B"])
            .contains("Signed-off-by: Disposable Fixture Human <fixture@example.test>")
    );
    let inspected = f
        .git
        .inspect_candidate(&f.runs, candidate.manifest.operation_id)
        .unwrap();
    assert_eq!(inspected.manifest, candidate.manifest);
}

#[test]
fn v0_t16_missing_b_context_blocks_without_fuzz_or_import_and_preserves_partial_candidate() {
    let f = Fixture::new(true);
    let operation = Uuid::new_v4();
    let error = f.candidate(operation, &[f.delta()]).unwrap_err();
    assert_eq!(error.code, "isolated_patch_conflict");
    let path = f.runs.join(format!("candidate-{operation}"));
    assert!(path.join(".git/kegari-candidate.json").exists());
    assert!(!path.join("unapproved-b.txt").exists());
    assert_eq!(
        fs::read_to_string(path.join("a.txt")).unwrap(),
        "first\nsecond\nthird\nfourth\n"
    );
    assert_eq!(
        f.git
            .inspect_candidate(&f.runs, operation)
            .unwrap_err()
            .code,
        "unfinished_candidate"
    );
    assert_eq!(
        f.candidate(operation, &[f.delta()]).unwrap_err().code,
        "ownership_conflict"
    );
}

#[test]
fn review_identity_is_available_before_squash_and_excludes_future_commit_metadata() {
    let f = Fixture::new(false);
    let proposed = f
        .git
        .propose_delta(
            &f.source,
            ProposeDelta {
                repository: f.repository.clone(),
                parent_id: f.parent,
                child_id: f.child,
                work_generation: &d("definition"),
                correction_generation: &d("correction"),
                scope: WriteScope {
                    paths: vec!["a.txt".into()],
                },
                base: &f.base,
                head: &f.head,
            },
        )
        .unwrap();
    let review_before_merge = proposed.review_digest().unwrap();
    git_run(
        &f.source,
        &["switch", "-c", "second-staging-fixture", &f.base],
    );
    git_run(&f.source, &["merge", "--squash", &f.head]);
    let second_squash = commit(&f.source, "feat: observe second squash metadata");
    let staged = f
        .git
        .capture_delta(
            &f.source,
            CaptureDelta {
                repository: f.repository.clone(),
                parent_id: f.parent,
                child_id: f.child,
                work_generation: &d("definition"),
                correction_generation: &d("correction"),
                scope: WriteScope {
                    paths: vec!["a.txt".into()],
                },
                base: &f.base,
                head: &f.head,
                staging_parent: &f.base,
                staging_commit: &second_squash,
            },
        )
        .unwrap();
    assert_eq!(
        review_before_merge,
        staged.manifest.review_digest().unwrap()
    );
    assert_eq!(proposed.patch(), staged.patch);
    assert_eq!(
        review_before_merge,
        f.delta().manifest.review_digest().unwrap()
    );
    assert_ne!(
        staged.manifest.digest().unwrap(),
        f.delta().manifest.digest().unwrap()
    );
    let mut changed = staged.manifest;
    changed.scope.paths.push("another-child.txt".into());
    assert_ne!(review_before_merge, changed.review_digest().unwrap());
}

#[test]
fn v0_t15_squash_delta_scope_and_exact_artifact_are_revalidated() {
    let f = Fixture::new(false);
    let mut delta = f.delta();
    delta.patch.extend_from_slice(b"forged patch data\n");
    assert_eq!(
        f.git.verify_delta(&f.source, &delta).unwrap_err().code,
        "provenance_mismatch"
    );
    let result = f.git.capture_delta(
        &f.source,
        CaptureDelta {
            repository: f.repository.clone(),
            parent_id: f.parent,
            child_id: f.child,
            work_generation: &d("definition"),
            correction_generation: &d("correction"),
            scope: WriteScope {
                paths: vec!["elsewhere/".into()],
            },
            base: &f.base,
            head: &f.head,
            staging_parent: &f.base,
            staging_commit: &f.squash,
        },
    );
    assert_eq!(result.unwrap_err().code, "scope_violation");
    fs::write(f.source.join("a.txt"), "unreviewed squash content\n").unwrap();
    let wrong = commit(&f.source, "fix: change staging after review");
    let result = f.git.capture_delta(
        &f.source,
        CaptureDelta {
            repository: f.repository.clone(),
            parent_id: f.parent,
            child_id: f.child,
            work_generation: &d("definition"),
            correction_generation: &d("correction"),
            scope: WriteScope {
                paths: vec!["a.txt".into()],
            },
            base: &f.base,
            head: &f.head,
            staging_parent: &f.base,
            staging_commit: &wrong,
        },
    );
    assert_eq!(result.unwrap_err().code, "provenance_mismatch");
}

#[test]
fn full_unicode_branch_title_is_preserved_and_length_is_never_truncated() {
    let fixture = TempDir::new().unwrap();
    let git = Git::new(pinned_git::prepare(fixture.path())).unwrap();
    let branch = git
        .branch_name(
            "CHILD-12",
            "Full Ångström / 日本語 Title @{Draft} .. Final.lock",
            255,
        )
        .unwrap();
    assert_eq!(
        branch.name,
        "CHILD-12-Full-Ångström-日本語-Title-@-Draft}-Final-lock"
    );
    assert_eq!(
        git.branch_name("CHILD-12", "Long complete parent title", 15)
            .unwrap_err()
            .code,
        "invalid_branch_name"
    );
    assert!(title_suffix(".. / \n ~").is_err());
    assert!(git.branch_name("../escaped", "Parent", 255).is_err());
}

#[test]
fn v0_t17_binary_modes_and_symlink_targets_survive_exact_patch_application() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let f = Fixture::new(false);
    git_run(&f.source, &["switch", "-c", "BINARY-child", &f.production]);
    fs::write(f.source.join("asset.bin"), [0, 255, 0, 1, 2, 3]).unwrap();
    fs::write(f.source.join("run.sh"), "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(f.source.join("run.sh"), fs::Permissions::from_mode(0o755)).unwrap();
    symlink("a.txt", f.source.join("link")).unwrap();
    let head = commit(&f.source, "feat: add binary executable and link");
    git_run(
        &f.source,
        &["switch", "-c", "binary-staging", &f.production],
    );
    git_run(&f.source, &["merge", "--squash", "BINARY-child"]);
    git_run(
        &f.source,
        &["commit", "--signoff", "-m", "feat: squash binary delta"],
    );
    let squash = git_run(&f.source, &["rev-parse", "HEAD"]);
    let delta = f
        .git
        .capture_delta(
            &f.source,
            CaptureDelta {
                repository: f.repository.clone(),
                parent_id: f.parent,
                child_id: f.child,
                work_generation: &d("definition"),
                correction_generation: &d("correction"),
                scope: WriteScope {
                    paths: vec!["asset.bin".into(), "run.sh".into(), "link".into()],
                },
                base: &f.production,
                head: &head,
                staging_parent: &f.production,
                staging_commit: &squash,
            },
        )
        .unwrap();
    assert!(String::from_utf8_lossy(&delta.patch).contains("GIT binary patch"));
    let candidate = f.candidate(Uuid::new_v4(), &[delta]).unwrap();
    assert_eq!(
        fs::read(candidate.path().join("asset.bin")).unwrap(),
        [0, 255, 0, 1, 2, 3]
    );
    assert_ne!(
        fs::metadata(candidate.path().join("run.sh"))
            .unwrap()
            .permissions()
            .mode()
            & 0o111,
        0
    );
    assert_eq!(
        fs::read_link(candidate.path().join("link")).unwrap(),
        PathBuf::from("a.txt")
    );
}

fn work(parent: Uuid, child: Option<Uuid>) -> WorkIdentity {
    WorkIdentity {
        linear_endpoint: "https://api.linear.app/graphql".into(),
        linear_workspace: Uuid::from_u128(42),
        parent,
        child,
        generation: Generation {
            authorization: d("authorization"),
            definition: d("definition"),
            correction: d("correction"),
            scope: d("scope"),
        },
    }
}

fn publication<T: serde::Serialize>(payload: &T, provider: &str) -> Publication {
    Publication {
        provider: provider.into(),
        record_id: "observed-record-123".into(),
        writer_account_id: "selected-user".into(),
        payload_digest: canonical_digest(1, payload).unwrap(),
        observed_at: Utc::now(),
        url: "https://example.test/evidence/123".into(),
    }
}

fn check_requirements() -> Vec<CheckRequirement> {
    vec![
        CheckRequirement {
            id: "local-full".into(),
            kind: CheckKind::Local,
            workflow_id: None,
            command: vec!["./test.sh".into()],
        },
        CheckRequirement {
            id: "ci-full".into(),
            kind: CheckKind::HostedCi,
            workflow_id: Some(99),
            command: vec![],
        },
    ]
}

fn checks(w: &WorkIdentity, revision: &RevisionBinding) -> CheckSnapshot {
    let now = Utc::now();
    CheckSnapshot {
        pagination_complete: true,
        observed_at: now,
        runs: check_requirements()
            .into_iter()
            .map(|r| CheckRun {
                id: r.id,
                kind: r.kind.clone(),
                operation_id: Uuid::new_v4(),
                work_digest: w.digest().unwrap(),
                revision: revision.clone(),
                workflow_id: r.workflow_id,
                run_id: r.workflow_id.map(|_| 123),
                attempt: 1,
                runner: if r.kind == CheckKind::Local {
                    Runner::LocalSandbox
                } else {
                    Runner::GitHubHosted
                },
                command: r.command,
                environment_identity: "fixture-environment-v1".into(),
                started_at: Some(now - Duration::seconds(3)),
                completed_at: Some(now - Duration::seconds(1)),
                conclusion: Conclusion::Success,
                artifacts: vec![artifact_bytes("fixture://actual-output", b"exit 0")],
            })
            .collect(),
    }
}

fn review(
    w: &WorkIdentity,
    revision: &RevisionBinding,
    delta: &str,
    pr: Option<u64>,
    key: &AttestationKey,
) -> (Signed<ReviewAttestation>, Publication, Option<ReviewStatus>) {
    let signed = key
        .sign(
            "codex_review",
            &ReviewAttestation {
                schema_version: 1,
                runtime_id: Uuid::new_v4(),
                owner_generation: 1,
                operation_id: Uuid::new_v4(),
                work: w.clone(),
                revision: revision.clone(),
                pull_request: pr,
                delta_digest: delta.into(),
                verdict: Verdict::Pass,
                unresolved_findings: vec![],
                supporting_links: vec!["https://example.test/actual-review".into()],
                review: ReviewKind::Agent {
                    session_id: "fixture-thread-fixture-turn".into(),
                    instruction_manifest: d("instructions"),
                },
            },
        )
        .unwrap();
    let record = publication(&signed, if pr.is_some() { "github" } else { "linear" });
    let status = pr.map(|_| ReviewStatus {
        repository: revision.repository.clone(),
        head: revision.head.clone(),
        context: REVIEW_CONTEXT.into(),
        state: "success".into(),
        writer_account_id: "selected-user".into(),
        attestation_record_id: record.record_id.clone(),
    });
    (signed, record, status)
}

#[test]
fn v0_t15_forged_review_status_and_stale_head_do_not_pass() {
    let f = Fixture::new(false);
    let delta = f.delta();
    let w = work(f.parent, Some(f.child));
    let revision = RevisionBinding {
        repository: f.repository.clone(),
        head: f.head.clone(),
        base: f.base.clone(),
        tested_commit: f.head.clone(),
        tested_tree: delta.manifest.validated_head.tree.clone(),
    };
    let key = AttestationKey::new("key-v1".into(), vec![7; 32]).unwrap();
    let (mut signed, mut record, status) = review(
        &w,
        &revision,
        &delta.manifest.digest().unwrap(),
        Some(1),
        &key,
    );
    verify_review(
        &signed,
        &record,
        status.as_ref(),
        &key,
        ReviewRequirements {
            work: &w,
            revision: &revision,
            delta_digest: &delta.manifest.digest().unwrap(),
            writer: "selected-user",
        },
    )
    .unwrap();
    signed.payload.revision.head = f.production.clone();
    record.payload_digest = signed.digest().unwrap();
    assert_eq!(
        verify_review(
            &signed,
            &record,
            status.as_ref(),
            &key,
            ReviewRequirements {
                work: &w,
                revision: &revision,
                delta_digest: &delta.manifest.digest().unwrap(),
                writer: "selected-user"
            }
        )
        .unwrap_err()
        .code,
        "invalid_attestation"
    );
    let (signed, record, mut status) = review(
        &w,
        &revision,
        &delta.manifest.digest().unwrap(),
        Some(1),
        &key,
    );
    status.as_mut().unwrap().writer_account_id = "untrusted-writer".into();
    assert_eq!(
        verify_review(
            &signed,
            &record,
            status.as_ref(),
            &key,
            ReviewRequirements {
                work: &w,
                revision: &revision,
                delta_digest: &delta.manifest.digest().unwrap(),
                writer: "selected-user"
            }
        )
        .unwrap_err()
        .code,
        "review_gate"
    );
    assert!(
        verify_review(
            &signed,
            &record,
            None,
            &key,
            ReviewRequirements {
                work: &w,
                revision: &revision,
                delta_digest: &delta.manifest.digest().unwrap(),
                writer: "selected-user"
            }
        )
        .is_err()
    );
}

#[test]
fn local_success_cannot_bypass_missing_skipped_stale_or_self_hosted_ci() {
    let f = Fixture::new(false);
    let revision = RevisionBinding {
        repository: f.repository.clone(),
        head: f.head.clone(),
        base: f.base.clone(),
        tested_commit: f.head.clone(),
        tested_tree: f.delta().manifest.validated_head.tree,
    };
    let w = work(f.parent, Some(f.child));
    let original = checks(&w, &revision);
    verify_checks(&check_requirements(), &w, &revision, &original).unwrap();
    for conclusion in [
        Conclusion::Skipped,
        Conclusion::Neutral,
        Conclusion::Failure,
        Conclusion::Cancelled,
        Conclusion::Pending,
        Conclusion::Unknown,
    ] {
        let mut snapshot = original.clone();
        snapshot.runs[1].conclusion = conclusion;
        assert_eq!(
            verify_checks(&check_requirements(), &w, &revision, &snapshot)
                .unwrap_err()
                .code,
            "validation_gate"
        );
    }
    let mut snapshot = original.clone();
    snapshot.runs[1].runner = Runner::SelfHosted;
    assert!(verify_checks(&check_requirements(), &w, &revision, &snapshot).is_err());
    let mut snapshot = original.clone();
    snapshot.runs[1].revision.base = f.production.clone();
    assert!(verify_checks(&check_requirements(), &w, &revision, &snapshot).is_err());
    let mut snapshot = original.clone();
    snapshot.pagination_complete = false;
    assert_eq!(
        verify_checks(&check_requirements(), &w, &revision, &snapshot)
            .unwrap_err()
            .code,
        "provider_pagination"
    );
    let mut snapshot = original;
    snapshot.runs.pop();
    assert!(verify_checks(&check_requirements(), &w, &revision, &snapshot).is_err());
}

fn synthetic_plan(parent: Uuid, repository: &RepositoryId, suffix: char) -> ReleasePlan {
    let base = Revision {
        commit: "1".repeat(40),
        tree: "2".repeat(40),
    };
    let candidate = Revision {
        commit: "3".repeat(40),
        tree: "4".repeat(40),
    };
    ReleasePlan {
        schema_version: 1,
        runtime_id: Uuid::new_v4(),
        operation_id: Uuid::new_v4(),
        parent_id: parent,
        linear_workspace: Uuid::from_u128(42),
        generation: work(parent, None).generation,
        evidence_token: d("evidence"),
        acceptance_digest: d("acceptance"),
        carried_acceptance: None,
        resources: BTreeSet::from([ResourceKey::Repository {
            repository: repository.clone(),
        }]),
        dependencies: vec![],
        spacing_seconds: 0,
        deployment_free: BTreeSet::from([repository.clone()]),
        steps: vec![ReleaseStep {
            id: format!("merge-{suffix}"),
            depends_on: BTreeSet::new(),
            kind: StepKind::Merge {
                repository: repository.clone(),
                pull_request: 1,
                production_base: base,
                candidate,
                selected_delta_digests: vec![d("delta")],
            },
        }],
    }
}

fn restored_attempt(plan: ReleasePlan) -> ReleaseAttempt {
    ReleaseAttempt {
        steps: plan
            .steps
            .iter()
            .map(|s| (s.id.clone(), StepState::NotStarted))
            .collect(),
        plan,
        admitted_at: Utc::now(),
        control_generation: 1,
        owner_generation: 1,
        step_history: BTreeMap::new(),
        cancellations: BTreeMap::new(),
        hold_reason: None,
        abandoned: false,
        resolved: false,
        assigned_successor: None,
        recovery_records: vec![],
    }
}

fn persist(path: &Path, attempt: &ReleaseAttempt) -> Result<()> {
    let bytes = serde_json::to_vec(attempt).unwrap();
    fs::write(path, &bytes).map_err(|e| Error::io("fixture persisted release receipt", e))
}

#[test]
fn v0_t09_sent_unknown_is_durable_before_dispatch_and_never_blindly_replayed() {
    let repo = RepositoryId {
        hostname: "github.com".into(),
        id: "101".into(),
    };
    let plan = synthetic_plan(Uuid::new_v4(), &repo, 'a');
    let mut attempt = restored_attempt(plan);
    let generation = attempt.plan.generation.clone();
    let evidence = attempt.plan.evidence_token.clone();
    let revisions = BTreeMap::from([(
        repo,
        Revision {
            commit: "1".repeat(40),
            tree: "2".repeat(40),
        },
    )]);
    let facts = FreshReleaseFacts {
        generation: &generation,
        evidence_token: &evidence,
        owner_generation: 1,
        authority_current: true,
        pending_correction: false,
        provider_reads_complete: true,
        resource_lock_owned: true,
        credentials_usable: true,
        repository_policy_current: true,
        force_stopped: false,
        current_revisions: &revisions,
    };
    let dir = TempDir::new().unwrap();
    let receipt = dir.path().join("release.json");
    let payload = attempt
        .prepare_step("merge-a", &facts, |a| persist(&receipt, a))
        .unwrap();
    let dispatch = attempt
        .mark_sent(&payload, &facts, |a| persist(&receipt, a))
        .unwrap();
    let recovered: ReleaseAttempt = serde_json::from_slice(&fs::read(&receipt).unwrap()).unwrap();
    assert!(matches!(
        recovered.steps["merge-a"],
        StepState::SentUnknown { .. }
    ));
    assert_eq!(dispatch.payload().kind, payload.kind);
    assert_eq!(
        attempt
            .mark_sent(&payload, &facts, |_| Ok(()))
            .unwrap_err()
            .code,
        "operation_uncertain"
    );
    assert!(attempt.prepare_step("merge-a", &facts, |_| Ok(())).is_err());
    attempt
        .observe_success(
            "merge-a",
            ObservedEffect::Merge {
                repository: revisions.keys().next().unwrap().clone(),
                pull_request: 1,
                commit: Revision {
                    commit: "5".repeat(40),
                    tree: "4".repeat(40),
                },
                parent_commit: "1".repeat(40),
            },
            Utc::now(),
            |a| persist(&receipt, a),
        )
        .unwrap();
    assert!(attempt.completed());
    let mut queue = ReleaseQueue::default();
    queue.record_success(&attempt).unwrap();
}

#[test]
fn v0_t20_correction_fences_remaining_step_and_persistence_failure_grants_no_dispatch() {
    let repo = RepositoryId {
        hostname: "github.com".into(),
        id: "101".into(),
    };
    let mut attempt = restored_attempt(synthetic_plan(Uuid::new_v4(), &repo, 'a'));
    let generation = attempt.plan.generation.clone();
    let evidence = attempt.plan.evidence_token.clone();
    let revisions = BTreeMap::from([(
        repo,
        Revision {
            commit: "1".repeat(40),
            tree: "2".repeat(40),
        },
    )]);
    let mut facts = FreshReleaseFacts {
        generation: &generation,
        evidence_token: &evidence,
        owner_generation: 1,
        authority_current: true,
        pending_correction: false,
        provider_reads_complete: true,
        resource_lock_owned: true,
        credentials_usable: true,
        repository_policy_current: true,
        force_stopped: false,
        current_revisions: &revisions,
    };
    let error = attempt
        .prepare_step("merge-a", &facts, |_| {
            Err(Error::new("disk_full", "fixture write failed"))
        })
        .unwrap_err();
    assert_eq!(error.code, "disk_full");
    assert_eq!(attempt.steps["merge-a"], StepState::NotStarted);
    let payload = attempt.prepare_step("merge-a", &facts, |_| Ok(())).unwrap();
    facts.pending_correction = true;
    assert_eq!(
        attempt
            .mark_sent(&payload, &facts, |_| Ok(()))
            .unwrap_err()
            .code,
        "release_fenced"
    );
    assert!(matches!(
        attempt.steps["merge-a"],
        StepState::Prepared { .. }
    ));
}

#[test]
fn v0_t20_human_rerun_cannot_be_cancelled_as_owned() {
    let owned = OwnedRun {
        repository: RepositoryId {
            hostname: "github.com".into(),
            id: "101".into(),
        },
        run_id: 10,
        attempt: 1,
        operation_id: Uuid::new_v4(),
        actor_account_id: "selected-user".into(),
        origin_digest: d("owned-dispatch"),
    };
    let mut rerun = owned.clone();
    rerun.attempt = 2;
    assert!(owned.may_cancel(&owned));
    assert!(!owned.may_cancel(&rerun));
}

#[test]
fn v0_t30_partial_hold_blocks_overlap_but_not_disjoint_release_and_resolve_never_claims_success() {
    let repo = RepositoryId {
        hostname: "github.com".into(),
        id: "101".into(),
    };
    let unrelated = RepositoryId {
        hostname: "github.com".into(),
        id: "202".into(),
    };
    let mut attempt = restored_attempt(synthetic_plan(Uuid::new_v4(), &repo, 'a'));
    attempt.steps.insert(
        "merge-a".into(),
        StepState::Failed {
            reason: "provider terminal failure".into(),
            remote_id: Some("remote-1".into()),
            observed_at: Utc::now(),
        },
    );
    attempt.hold_reason = Some("partial failure".into());
    let mut queue = ReleaseQueue::default();
    queue.restore_hold(&attempt).unwrap();
    assert_eq!(
        queue
            .eligible(&synthetic_plan(Uuid::new_v4(), &repo, 'b'), Utc::now())
            .unwrap_err()
            .code,
        "release_hold"
    );
    queue
        .eligible(&synthetic_plan(Uuid::new_v4(), &unrelated, 'c'), Utc::now())
        .unwrap();
    let baseline = ReconciledBaseline {
        provider_reads_complete: true,
        coherent: true,
        observed_at: Utc::now(),
        observations: vec![BaselineObservation {
            resource: ResourceKey::Repository { repository: repo },
            production_commit: "1".repeat(40),
            deployed_commit: None,
            artifact_digest: None,
            provider_record: "observed-github-ref".into(),
        }],
    };
    attempt
        .resolve(
            "authenticated-human",
            "retain known failed history",
            &baseline,
            Utc::now(),
            |_| Ok(()),
        )
        .unwrap();
    assert!(attempt.resolved);
    assert!(!attempt.completed());
    assert!(queue.record_success(&attempt).is_err());
}

#[test]
fn v0_t11_accepted_staging_or_generation_prose_is_not_gate_evidence() {
    // HMAC authenticates canonical bytes and purpose, not a same-named status or
    // a text claim. This also guards cross-purpose acceptance/review substitution.
    let key = AttestationKey::new("key-v1".into(), vec![9; 32]).unwrap();
    let signed = key
        .sign("codex_review", &serde_json::json!({"a":2,"z":1}))
        .unwrap();
    key.verify("codex_review", &signed).unwrap();
    assert!(key.verify("human_acceptance", &signed).is_err());
    assert_ne!(bytes_digest(b"same patch"), bytes_digest(b"same patch\n"));
}

#[test]
fn conflicting_recovery_preserves_all_existing_resource_ownership() {
    let first = RepositoryId {
        hostname: "github.com".into(),
        id: "101".into(),
    };
    let second = RepositoryId {
        hostname: "github.com".into(),
        id: "202".into(),
    };
    let first_plan = synthetic_plan(Uuid::new_v4(), &first, 'a');
    let second_plan = synthetic_plan(Uuid::new_v4(), &second, 'b');
    let mut queue = ReleaseQueue::default();
    queue.reserve(&first_plan, Utc::now()).unwrap();
    queue.reserve(&second_plan, Utc::now()).unwrap();
    let mut changed = first_plan.clone();
    changed.resources.extend(second_plan.resources.clone());
    changed
        .deployment_free
        .extend(second_plan.deployment_free.clone());
    changed.steps.extend(second_plan.steps.clone());
    let recovered = restored_attempt(changed);
    assert_eq!(
        queue.reconcile_attempt(&recovered).unwrap_err().code,
        "ownership_conflict"
    );
    assert!(queue.owns_all(&first_plan));
    assert!(queue.owns_all(&second_plan));
    assert_eq!(
        queue
            .eligible(&synthetic_plan(Uuid::new_v4(), &first, 'c'), Utc::now())
            .unwrap_err()
            .code,
        "release_locked"
    );
}

struct GateFixture {
    _git_fixtures: Vec<Fixture>,
    key: AttestationKey,
    parent: VerifiedParent,
    children: Vec<VerifiedChild>,
    production: Vec<VerifiedProduction>,
    acceptance: VerifiedAcceptance,
    plan: ReleasePlan,
}

fn protection(revision: &RevisionBinding, branch: &str) -> RepositoryProtection {
    RepositoryProtection {
        repository: revision.repository.clone(),
        branch: branch.into(),
        observed_base: revision.base.clone(),
        required_contexts: BTreeSet::from([REVIEW_CONTEXT.into(), "unit".into(), "e2e".into()]),
        strict_current_base: true,
        squash_allowed: true,
        extra_approvals_satisfied: true,
        code_owner_reviews_satisfied: true,
        admin_bypass: false,
        observed_at: Utc::now(),
    }
}

fn verified_child(
    w: WorkIdentity,
    revision: RevisionBinding,
    outcome: ChildOutcome,
    key: &AttestationKey,
) -> (VerifiedChild, VerifiedReview) {
    let criteria = BTreeSet::from(["prepared-outcome".into()]);
    let (delta_digest, pr) = match &outcome {
        ChildOutcome::Changed {
            pull_request,
            delta,
        } => (delta.review_digest().unwrap(), Some(*pull_request)),
        ChildOutcome::Replacement { delta, .. } => (delta.review_digest().unwrap(), None),
        ChildOutcome::NoChange { reason, .. } => (
            no_change_digest(&w, &revision, &criteria, reason).unwrap(),
            None,
        ),
    };
    let (signed, record, status) = review(&w, &revision, &delta_digest, pr, key);
    let review = verify_review(
        &signed,
        &record,
        status.as_ref(),
        key,
        ReviewRequirements {
            work: &w,
            revision: &revision,
            delta_digest: &delta_digest,
            writer: "selected-user",
        },
    )
    .unwrap();
    let evidence = ChildEvidence {
        validation_selected: None,
        schema_version: 1,
        work: w.clone(),
        revision: revision.clone(),
        fulfilled_criteria: criteria.clone(),
        outcome,
        checks: checks(&w, &revision),
        review_digest: review.digest().into(),
        results_for_dependents: vec![artifact_bytes("fixture://child-result", b"observed result")],
    };
    let observed = publication(&evidence, "linear");
    let requirements = ChildRequirements {
        work: &w,
        criteria: &criteria,
        revision: &revision,
        checks: &check_requirements(),
        required_contexts: &BTreeSet::from([REVIEW_CONTEXT.into()]),
    };
    let child = verify_child(
        &evidence,
        requirements,
        &review,
        pr.map(|_| protection(&revision, "staging")).as_ref(),
        &observed,
        "selected-user",
    )
    .unwrap();
    (child, review)
}

fn verify_fixture_parent(
    manifest: &ParentEvidence,
    children: &[VerifiedChild],
) -> Result<VerifiedParent> {
    verify_fixture_parent_with_integrations(manifest, children, &[])
}

fn verify_fixture_parent_with_integrations(
    manifest: &ParentEvidence,
    children: &[VerifiedChild],
    integrations: &[VerifiedProductionIntegration],
) -> Result<VerifiedParent> {
    verify_parent(
        manifest,
        ParentRequirements {
            work: &manifest.work,
            required_children: &manifest.required_children,
            e2e_child: manifest.e2e_child,
            criteria: &BTreeSet::from(["prepared-outcome".into()]),
            integrated_commands: &BTreeMap::from([("integrated".into(), vec!["./e2e.sh".into()])]),
            test_resources: &BTreeSet::new(),
            production_targets: &manifest.production_targets,
            visual: false,
            required_contributions: &manifest.required_contributions,
            production_integrations: integrations,
        },
        children,
        &publication(manifest, "linear"),
        "selected-user",
    )
}

impl GateFixture {
    fn new(repo_count: usize) -> Self {
        let key = AttestationKey::new("fixture-key-v1".into(), vec![17; 32]).unwrap();
        let parent_id = Uuid::new_v4();
        let parent_work = work(parent_id, None);
        let mut fixtures = vec![];
        let mut children = vec![];
        let mut reviews = vec![];
        let mut candidates = vec![];
        let mut baselines = vec![];
        for index in 0..repo_count {
            let mut fixture = Fixture::new(false);
            fixture.parent = parent_id;
            fixture.repository.id = (100 + index).to_string();
            let delta = fixture.delta();
            let revision = RevisionBinding {
                repository: fixture.repository.clone(),
                head: fixture.head.clone(),
                base: fixture.base.clone(),
                tested_commit: fixture.head.clone(),
                tested_tree: delta.manifest.validated_head.tree.clone(),
            };
            let (child, review) = verified_child(
                work(parent_id, Some(fixture.child)),
                revision,
                ChildOutcome::Changed {
                    pull_request: 100 + index as u64,
                    delta: Box::new(delta.manifest.clone()),
                },
                &key,
            );
            let candidate = fixture.candidate(Uuid::new_v4(), &[delta]).unwrap();
            children.push(child);
            reviews.push(review);
            baselines.push(ProductionBaseline {
                repository: fixture.repository.clone(),
                production: candidate.manifest.production.clone(),
                deployed_revision: None,
                target_revisions: BTreeMap::new(),
                deployed_artifact: None,
                target_artifacts: BTreeMap::new(),
                deployment_free: true,
                successor_recovery_operation: None,
            });
            candidates.push(candidate.manifest);
            fixtures.push(fixture);
        }
        let e2e_child = Uuid::new_v4();
        let candidate = &candidates[0];
        let result = candidate.result.as_ref().unwrap();
        let revision = RevisionBinding {
            repository: candidate.repository.clone(),
            head: result.commit.clone(),
            base: candidate.production.commit.clone(),
            tested_commit: result.commit.clone(),
            tested_tree: result.tree.clone(),
        };
        let (e2e, _) = verified_child(
            work(parent_id, Some(e2e_child)),
            revision,
            ChildOutcome::NoChange {
                reason: "read-only E2E validation fulfilled its prepared responsibility".into(),
                change_gates_inapplicable: BTreeSet::from([
                    "branch".into(),
                    "pull_request".into(),
                    "staging_merge".into(),
                ]),
                exact_sha_validation: "observed exact SHA workflow".into(),
            },
            &key,
        );
        children.push(e2e);
        let mut manifest = ParentEvidence {
            archived_integrated_validation: None,
            integrated_validation: None,
            schema_version: 1,
            work: parent_work.clone(),
            required_children: children
                .iter()
                .map(|c| c.evidence().work.child.unwrap())
                .collect(),
            e2e_child,
            child_results: children
                .iter()
                .map(|c| (c.evidence().work.child.unwrap(), c.digest().into()))
                .collect(),
            required_contributions: children
                .iter()
                .filter_map(|c| match &c.evidence().outcome {
                    ChildOutcome::Changed { delta, .. }
                    | ChildOutcome::Replacement { delta, .. } => {
                        Some((delta.child_id, delta.original_contributions().unwrap()))
                    }
                    ChildOutcome::NoChange { .. } => None,
                })
                .collect(),
            executive_summary:
                "Prepared outcome delivered by scoped children; isolated E2E checks passed.".into(),
            touched_components: vec!["a.txt".into()],
            fulfilled_criteria: BTreeSet::from(["prepared-outcome".into()]),
            caveats: vec!["deterministic provider fixture".into()],
            go: true,
            baselines,
            candidates,
            dependency_snapshot_digest: d("no-parent-dependencies"),
            production_targets: BTreeSet::new(),
            integrated_results: vec![],
            visual: false,
            video: None,
            publication_operation: Uuid::new_v4(),
            created_at: Utc::now(),
            supersedes: None,
        };
        manifest.integrated_results.push(IntegratedRun {
            id: "integrated".into(),
            operation_id: Uuid::new_v4(),
            e2e_child,
            candidate_vector_digest: manifest.candidate_vector_digest().unwrap(),
            command: vec!["./e2e.sh".into()],
            test_resources: BTreeSet::new(),
            started_at: manifest.created_at - Duration::seconds(2),
            completed_at: manifest.created_at - Duration::seconds(1),
            conclusion: Conclusion::Success,
            after_staging_integration: true,
            artifacts: vec![artifact_bytes(
                "fixture://integrated-output",
                b"observed integration result",
            )],
        });
        let parent = verify_fixture_parent(&manifest, &children).unwrap();
        let acceptors = BTreeSet::from(["human-linear-user".into()]);
        let signed = prepare_acceptance(
            &parent,
            AcceptanceRequest {
                supplied_token: parent.token(),
                current_work: &parent_work,
                pending_correction: false,
                actor: AcceptanceActor {
                    control_actor: "os-user-501",
                    linear_actor: "human-linear-user",
                    provider_writer: "selected-user",
                    has_accept_role: true,
                    permitted_acceptors: &acceptors,
                },
                runtime_id: Uuid::new_v4(),
                operation_id: Uuid::new_v4(),
                now: Utc::now(),
            },
            &key,
        )
        .unwrap();
        let acceptance = verify_acceptance(
            &signed,
            &publication(&signed, "linear"),
            &parent,
            &parent_work,
            false,
            &acceptors,
            &key,
        )
        .unwrap();
        let mut production = vec![];
        let mut steps = vec![];
        for (index, candidate) in parent.manifest().candidates.iter().enumerate() {
            let result = candidate.result.as_ref().unwrap();
            let revision = RevisionBinding {
                repository: candidate.repository.clone(),
                head: result.commit.clone(),
                base: candidate.production.commit.clone(),
                tested_commit: result.commit.clone(),
                tested_tree: result.tree.clone(),
            };
            let derived = derive_review(
                &parent,
                candidate,
                revision.clone(),
                200 + index as u64,
                &reviews,
                HostOperation {
                    runtime_id: Uuid::new_v4(),
                    owner_generation: 1,
                    operation_id: Uuid::new_v4(),
                },
            )
            .unwrap();
            let signed = key.sign("codex_review", &derived).unwrap();
            let published = publication(&signed, "github");
            let status = ReviewStatus {
                repository: revision.repository.clone(),
                head: revision.head.clone(),
                context: REVIEW_CONTEXT.into(),
                state: "success".into(),
                writer_account_id: "selected-user".into(),
                attestation_record_id: published.record_id.clone(),
            };
            let verified = verify_review(
                &signed,
                &published,
                Some(&status),
                &key,
                ReviewRequirements {
                    work: &parent_work,
                    revision: &revision,
                    delta_digest: &derived.delta_digest,
                    writer: "selected-user",
                },
            )
            .unwrap();
            let requirements = vec![
                CheckRequirement {
                    id: "unit".into(),
                    kind: CheckKind::ProductionCi,
                    workflow_id: Some(400),
                    command: vec![],
                },
                CheckRequirement {
                    id: "e2e".into(),
                    kind: CheckKind::ProductionCi,
                    workflow_id: Some(401),
                    command: vec![],
                },
            ];
            let mut snapshot = checks(&parent_work, &revision);
            snapshot.runs = requirements
                .iter()
                .map(|r| {
                    let mut run = snapshot.runs[1].clone();
                    run.id = r.id.clone();
                    run.kind = CheckKind::ProductionCi;
                    run.workflow_id = r.workflow_id;
                    run
                })
                .collect();
            production.push(
                verify_production(
                    ProductionRequirements {
                        parent: &parent,
                        revision: &revision,
                        checks: &requirements,
                        required_contexts: &BTreeSet::from([
                            "unit".into(),
                            "e2e".into(),
                            REVIEW_CONTEXT.into(),
                        ]),
                    },
                    &snapshot,
                    &verified,
                    &protection(&revision, "main"),
                )
                .unwrap(),
            );
            steps.push(ReleaseStep {
                id: format!("merge-{index}"),
                depends_on: BTreeSet::new(),
                kind: StepKind::Merge {
                    repository: candidate.repository.clone(),
                    pull_request: 200 + index as u64,
                    production_base: candidate.production.clone(),
                    candidate: result.clone(),
                    selected_delta_digests: candidate.ordered_delta_digests.clone(),
                },
            });
        }
        let plan = ReleasePlan {
            schema_version: 1,
            runtime_id: Uuid::new_v4(),
            operation_id: Uuid::new_v4(),
            parent_id,
            linear_workspace: parent_work.linear_workspace,
            generation: parent_work.generation,
            evidence_token: parent.token().into(),
            acceptance_digest: acceptance.digest().into(),
            carried_acceptance: None,
            resources: parent
                .manifest()
                .candidates
                .iter()
                .map(|c| ResourceKey::Repository {
                    repository: c.repository.clone(),
                })
                .collect(),
            dependencies: vec![],
            spacing_seconds: 0,
            deployment_free: parent
                .manifest()
                .candidates
                .iter()
                .map(|c| c.repository.clone())
                .collect(),
            steps,
        };
        Self {
            _git_fixtures: fixtures,
            key,
            parent,
            children,
            production,
            acceptance,
            plan,
        }
    }

    fn facts(&self) -> AdmissionFacts {
        AdmissionFacts {
            running: true,
            included: true,
            authorized: true,
            no_pending_correction: true,
            preparation_complete: true,
            provider_reads_complete: true,
            repository_policy_usable: true,
            credentials_usable: true,
            manual_mode: true,
            explicit_manual_request: Some(Uuid::new_v4()),
            actor_may_release: true,
            owner_generation: 1,
            control_generation: 1,
            required_step_ids: self.plan.steps.iter().map(|s| s.id.clone()).collect(),
        }
    }

    fn evidence(&self) -> DeliveryEvidence<'_> {
        DeliveryEvidence {
            parent: &self.parent,
            acceptance: &self.acceptance,
            carried: None,
            production: &self.production,
        }
    }
}

#[test]
fn manual_release_readiness_is_read_only_and_does_not_invent_a_command() {
    let f = GateFixture::new(1);
    let queue = ReleaseQueue::default();
    let mut facts = f.facts();
    facts.explicit_manual_request = None;
    facts.actor_may_release = false;
    let proof =
        verify_release_eligibility(&f.plan, f.evidence(), facts, &queue, Utc::now()).unwrap();
    assert!(proof.awaiting_manual_command());
    assert_eq!(proof.evidence_token(), f.parent.token());
    assert_eq!(proof.plan_digest(), f.plan.digest().unwrap());
    assert!(!queue.owns_all(&f.plan));
    let mut facts = f.facts();
    facts.explicit_manual_request = None;
    facts.actor_may_release = false;
    assert_eq!(
        ReleaseAttempt::admit(f.plan.clone(), f.evidence(), facts, Utc::now())
            .unwrap_err()
            .code,
        "release_authorization_required"
    );
    let mut facts = f.facts();
    facts.running = false;
    assert!(verify_release_eligibility(&f.plan, f.evidence(), facts, &queue, Utc::now()).is_err());
    let mut held = ReleaseQueue::default();
    let mut overlapping = f.plan.clone();
    overlapping.operation_id = Uuid::new_v4();
    held.reserve(&overlapping, Utc::now()).unwrap();
    assert!(
        verify_release_eligibility(&f.plan, f.evidence(), f.facts(), &held, Utc::now()).is_err()
    );
}

#[test]
fn review_regression_release_cannot_drop_an_accepted_repository_or_required_step() {
    let f = GateFixture::new(2);
    ReleaseAttempt::admit(f.plan.clone(), f.evidence(), f.facts(), Utc::now()).unwrap();
    let mut partial = f.plan.clone();
    let removed = partial.steps.pop().unwrap();
    let repo = removed.kind.repository().clone();
    partial.resources.remove(&ResourceKey::Repository {
        repository: repo.clone(),
    });
    partial.deployment_free.remove(&repo);
    assert_eq!(
        ReleaseAttempt::admit(partial.clone(), f.evidence(), f.facts(), Utc::now())
            .unwrap_err()
            .code,
        "release_incomplete"
    );
    let mut bad_contract = f.facts();
    bad_contract.required_step_ids = partial.steps.iter().map(|s| s.id.clone()).collect();
    assert_eq!(
        ReleaseAttempt::admit(partial, f.evidence(), bad_contract, Utc::now())
            .unwrap_err()
            .code,
        "scope_violation"
    );
    let mut missing_tests = f.evidence();
    missing_tests.production = &f.production[..1];
    assert!(ReleaseAttempt::admit(f.plan.clone(), missing_tests, f.facts(), Utc::now()).is_err());
}

#[test]
fn review_regression_parent_cannot_add_an_unaccounted_patch_or_omit_nochange_e2e_child() {
    let f = GateFixture::new(1);
    let mut forged = f.parent.manifest().clone();
    forged.candidates[0]
        .ordered_delta_digests
        .push(d("unapproved-extra-delta"));
    forged.candidates[0]
        .ordered_patch_digests
        .push(bytes_digest(b"unapproved extra code"));
    assert_eq!(
        verify_fixture_parent(&forged, &f.children)
            .unwrap_err()
            .code,
        "candidate_provenance"
    );
    assert_eq!(
        verify_fixture_parent(f.parent.manifest(), &f.children[..1])
            .unwrap_err()
            .code,
        "parent_incomplete"
    );
}

#[test]
fn review_regression_public_carry_record_is_not_a_verified_carry_capability() {
    let f = GateFixture::new(1);
    let mut manifest = f.parent.manifest().clone();
    manifest.created_at = Utc::now();
    manifest.caveats.push("new evidence publication".into());
    let newer = verify_fixture_parent(&manifest, &f.children).unwrap();
    let mut plan = f.plan.clone();
    plan.evidence_token = newer.token().into();
    plan.carried_acceptance = Some(CarriedAcceptance {
        schema_version: 1,
        original_reviewed_token: f.parent.token().into(),
        original_acceptance_digest: f.acceptance.digest().into(),
        current_evidence_token: newer.token().into(),
        unchanged_patch_proof: d("fabricated-proof"),
        production_validation_digest: d("fabricated-tests"),
        created_at: Utc::now(),
    });
    let evidence = DeliveryEvidence {
        parent: &newer,
        acceptance: &f.acceptance,
        carried: None,
        production: &f.production,
    };
    assert_eq!(
        ReleaseAttempt::admit(plan, evidence, f.facts(), Utc::now())
            .unwrap_err()
            .code,
        "stale_result"
    );
    assert!(
        carry_acceptance(
            &f.parent,
            &newer,
            &f.acceptance,
            &artifact_bytes("fixture://known-released-baseline", b"proof"),
            &[],
            Utc::now()
        )
        .is_err()
    );
}

#[test]
fn review_regression_resume_retains_failed_remote_step_history() {
    let f = GateFixture::new(1);
    let mut attempt =
        ReleaseAttempt::admit(f.plan.clone(), f.evidence(), f.facts(), Utc::now()).unwrap();
    let failed = StepState::Failed {
        reason: "observed provider terminal failure".into(),
        remote_id: Some("remote-action-123".into()),
        observed_at: Utc::now(),
    };
    attempt.steps.insert("merge-0".into(), failed.clone());
    attempt.hold_reason = Some("known failure".into());
    attempt
        .resume(
            "authenticated-operator",
            "provider prerequisite repaired",
            &f.parent,
            true,
            Utc::now(),
            |_| Ok(()),
        )
        .unwrap();
    assert_eq!(attempt.steps["merge-0"], StepState::NotStarted);
    assert!(attempt.step_history["merge-0"].contains(&failed));
    let persisted = serde_json::to_vec(&attempt).unwrap();
    let recovered: ReleaseAttempt = serde_json::from_slice(&persisted).unwrap();
    assert!(recovered.step_history["merge-0"].contains(&failed));
}

#[test]
fn v0_t19_stale_token_pending_correction_and_wrong_acceptor_cannot_approve() {
    let f = GateFixture::new(1);
    let acceptors = BTreeSet::from(["human-linear-user".into()]);
    for (token, pending, actor) in [
        ("stale", false, "human-linear-user"),
        (f.parent.token(), true, "human-linear-user"),
        (f.parent.token(), false, "not-allowed"),
    ] {
        assert!(
            prepare_acceptance(
                &f.parent,
                AcceptanceRequest {
                    supplied_token: token,
                    current_work: &f.parent.manifest().work,
                    pending_correction: pending,
                    actor: AcceptanceActor {
                        control_actor: "os-user-501",
                        linear_actor: actor,
                        provider_writer: "selected-user",
                        has_accept_role: true,
                        permitted_acceptors: &acceptors
                    },
                    runtime_id: Uuid::new_v4(),
                    operation_id: Uuid::new_v4(),
                    now: Utc::now()
                },
                &f.key
            )
            .is_err()
        );
    }
    let mut facts = f.facts();
    facts.running = false;
    assert_eq!(
        ReleaseAttempt::admit(f.plan.clone(), f.evidence(), facts, Utc::now())
            .unwrap_err()
            .code,
        "release_ineligible"
    );
}

fn replacement_fixture(
    f: &Fixture,
    key: &AttestationKey,
) -> (DeltaArtifact, VerifiedChild, VerifiedReview) {
    let original = f.delta();
    let staged_revision = f.git.exact_revision(&f.source, &f.squash).unwrap();
    let staging_binding = RevisionBinding {
        repository: f.repository.clone(),
        head: f.squash.clone(),
        base: f.squash.clone(),
        tested_commit: f.squash.clone(),
        tested_tree: staged_revision.tree,
    };
    let (staged, _) = verified_child(
        work(f.parent, Some(f.child)),
        staging_binding,
        ChildOutcome::NoChange {
            reason: "staging already contains the required A behavior; no extra merge is necessary"
                .into(),
            change_gates_inapplicable: BTreeSet::from([
                "branch".into(),
                "pull_request".into(),
                "staging_merge".into(),
            ]),
            exact_sha_validation: "complete staging exact-SHA workflow".into(),
        },
        key,
    );
    git_run(
        &f.source,
        &["switch", "-c", "A-reviewed-replacement", &f.production],
    );
    fs::write(f.source.join("a.txt"), "first\nsecond\nthird\nA fourth\n").unwrap();
    let replacement_head = commit(
        &f.source,
        "fix: produce scoped A replacement against production",
    );
    let replacement = f
        .git
        .capture_replacement(
            &f.source,
            CaptureReplacement {
                repository: f.repository.clone(),
                parent_id: f.parent,
                child_id: f.child,
                work_generation: &d("definition"),
                correction_generation: &d("correction"),
                scope: original.manifest.scope.clone(),
                production_base: &f.production,
                head: &replacement_head,
                staging_revision: &f.squash,
                staging_result_digest: staged.digest(),
                originals: &[original],
                reason: "the original patch requires unapproved B context",
            },
        )
        .unwrap();
    let binding = RevisionBinding {
        repository: f.repository.clone(),
        head: replacement_head.clone(),
        base: f.production.clone(),
        tested_commit: replacement_head,
        tested_tree: replacement.manifest.validated_head.tree.clone(),
    };
    let w = work(f.parent, Some(f.child));
    let (signed, published, status) = review(
        &w,
        &binding,
        &replacement.manifest.review_digest().unwrap(),
        None,
        key,
    );
    let reviewed = verify_review(
        &signed,
        &published,
        status.as_ref(),
        key,
        ReviewRequirements {
            work: &w,
            revision: &binding,
            delta_digest: &replacement.manifest.review_digest().unwrap(),
            writer: "selected-user",
        },
    )
    .unwrap();
    let evidence = ChildEvidence {
        validation_selected: None,
        schema_version: 1,
        work: w.clone(),
        revision: binding.clone(),
        fulfilled_criteria: BTreeSet::from(["prepared-outcome".into()]),
        outcome: ChildOutcome::Replacement {
            related_pull_request: 1,
            delta: Box::new(replacement.manifest.clone()),
        },
        checks: checks(&w, &binding),
        review_digest: reviewed.digest().into(),
        results_for_dependents: vec![artifact_bytes(
            "fixture://replacement-result",
            b"isolated replacement and staged result both validated",
        )],
    };
    let verify = |record: &ChildEvidence, staged: &VerifiedChild| {
        verify_replacement_child(
            record,
            ChildRequirements {
                work: &w,
                revision: &binding,
                criteria: &BTreeSet::from(["prepared-outcome".into()]),
                checks: &check_requirements(),
                required_contexts: &BTreeSet::from([REVIEW_CONTEXT.into()]),
            },
            &reviewed,
            ReplacementGates {
                staged_result: staged,
                related_pull_request: 1,
                repository: &f.repository,
                github_publication: &publication(record, "github"),
                github_writer: "selected-user",
            },
            &publication(record, "linear"),
            "selected-user",
        )
    };
    let child = verify(&evidence, &staged).unwrap();
    let mut failed = evidence.clone();
    failed.checks.runs[1].conclusion = Conclusion::Failure;
    assert_eq!(
        verify(&failed, &staged).unwrap_err().code,
        "validation_gate"
    );
    assert_eq!(
        verify_child(
            &evidence,
            ChildRequirements {
                work: &w,
                revision: &binding,
                criteria: &evidence.fulfilled_criteria,
                checks: &check_requirements(),
                required_contexts: &BTreeSet::new(),
            },
            &reviewed,
            None,
            &publication(&evidence, "linear"),
            "selected-user"
        )
        .unwrap_err()
        .code,
        "replacement_gate"
    );
    (replacement, child, reviewed)
}

fn parent_fixture_for_candidates(
    candidates: Vec<CandidateManifest>,
    mut children: Vec<VerifiedChild>,
    key: &AttestationKey,
) -> (ParentEvidence, Vec<VerifiedChild>) {
    let first = &candidates[0];
    let parent_id = first.parent_id;
    let e2e_id = Uuid::new_v4();
    let result = first.result.as_ref().unwrap();
    let (e2e, _) = verified_child(
        work(parent_id, Some(e2e_id)),
        RevisionBinding {
            repository: first.repository.clone(),
            head: result.commit.clone(),
            base: first.production.commit.clone(),
            tested_commit: result.commit.clone(),
            tested_tree: result.tree.clone(),
        },
        ChildOutcome::NoChange {
            reason: "E2E validated the complete exact isolated vector".into(),
            change_gates_inapplicable: BTreeSet::from([
                "branch".into(),
                "pull_request".into(),
                "staging_merge".into(),
            ]),
            exact_sha_validation: "complete candidate exact-SHA validation".into(),
        },
        key,
    );
    children.push(e2e);
    let now = Utc::now();
    let mut manifest = ParentEvidence {
        archived_integrated_validation: None,
        integrated_validation: None,
        schema_version: 1,
        work: work(parent_id, None),
        required_children: children
            .iter()
            .map(|c| c.evidence().work.child.unwrap())
            .collect(),
        e2e_child: e2e_id,
        child_results: children
            .iter()
            .map(|c| (c.evidence().work.child.unwrap(), c.digest().into()))
            .collect(),
        required_contributions: children
            .iter()
            .filter_map(|c| match &c.evidence().outcome {
                ChildOutcome::Changed { delta, .. } | ChildOutcome::Replacement { delta, .. } => {
                    Some((delta.child_id, delta.original_contributions().unwrap()))
                }
                ChildOutcome::NoChange { .. } => None,
            })
            .collect(),
        executive_summary: "Candidate fulfills the parent and complete required child inventory"
            .into(),
        touched_components: vec!["a.txt".into()],
        fulfilled_criteria: BTreeSet::from(["prepared-outcome".into()]),
        caveats: vec![],
        go: true,
        baselines: candidates
            .iter()
            .map(|c| ProductionBaseline {
                repository: c.repository.clone(),
                production: c.production.clone(),
                deployed_revision: None,
                target_revisions: BTreeMap::new(),
                deployed_artifact: None,
                target_artifacts: BTreeMap::new(),
                deployment_free: true,
                successor_recovery_operation: None,
            })
            .collect(),
        candidates,
        dependency_snapshot_digest: d("dependencies"),
        production_targets: BTreeSet::new(),
        integrated_results: vec![],
        visual: false,
        video: None,
        publication_operation: Uuid::new_v4(),
        created_at: now,
        supersedes: None,
    };
    manifest.integrated_results.push(IntegratedRun {
        id: "integrated".into(),
        operation_id: Uuid::new_v4(),
        e2e_child: e2e_id,
        candidate_vector_digest: manifest.candidate_vector_digest().unwrap(),
        command: vec!["./e2e.sh".into()],
        test_resources: BTreeSet::new(),
        started_at: now - Duration::seconds(2),
        completed_at: now - Duration::seconds(1),
        conclusion: Conclusion::Success,
        after_staging_integration: true,
        artifacts: vec![artifact_bytes(
            "fixture://integrated-run",
            b"actual fixture result",
        )],
    });
    (manifest, children)
}

#[test]
fn v0_t28_reviewed_replacement_isolated_from_b_retains_history_and_requires_complete_mapping() {
    let f = Fixture::new(true);
    let original = f.delta();
    assert_eq!(
        f.candidate(Uuid::new_v4(), std::slice::from_ref(&original))
            .unwrap_err()
            .code,
        "isolated_patch_conflict"
    );
    let key = AttestationKey::new("replacement-fixture-key".into(), vec![23; 32]).unwrap();
    let (replacement, child, source_review) = replacement_fixture(&f, &key);
    f.git.verify_delta(&f.source, &original).unwrap();
    assert_eq!(
        replacement.manifest.original_contributions().unwrap(),
        BTreeSet::from([original.manifest.digest().unwrap()])
    );
    let candidate = f
        .candidate(Uuid::new_v4(), std::slice::from_ref(&replacement))
        .unwrap();
    assert!(!candidate.path().join("unapproved-b.txt").exists());
    assert_eq!(
        fs::read_to_string(candidate.path().join("a.txt")).unwrap(),
        "first\nsecond\nthird\nA fourth\n"
    );
    assert_eq!(
        git_run(&f.source, &["show", &format!("{}:a.txt", f.squash)]),
        "first\nB-only context\nthird\nA fourth"
    );
    let (manifest, children) =
        parent_fixture_for_candidates(vec![candidate.manifest.clone()], vec![child], &key);
    let parent = verify_fixture_parent(&manifest, &children).unwrap();
    let result = candidate.manifest.result.as_ref().unwrap();
    derive_review(
        &parent,
        &candidate.manifest,
        RevisionBinding {
            repository: f.repository.clone(),
            head: result.commit.clone(),
            base: f.production.clone(),
            tested_commit: result.commit.clone(),
            tested_tree: result.tree.clone(),
        },
        2,
        &[source_review],
        HostOperation {
            runtime_id: Uuid::new_v4(),
            operation_id: Uuid::new_v4(),
            owner_generation: 1,
        },
    )
    .unwrap();
    let mut missing = manifest.clone();
    missing
        .required_contributions
        .get_mut(&f.child)
        .unwrap()
        .insert(d("omitted-child-contribution"));
    assert_eq!(
        verify_fixture_parent(&missing, &children).unwrap_err().code,
        "candidate_provenance"
    );
    assert_ne!(
        original.manifest.digest().unwrap(),
        replacement.manifest.digest().unwrap()
    );
    let mut forged = replacement.clone();
    forged
        .manifest
        .replacement
        .as_mut()
        .unwrap()
        .originals
        .clear();
    assert_eq!(
        f.git.verify_delta(&f.source, &forged).unwrap_err().code,
        "replacement_mapping"
    );
    let mut cross_child = original.clone();
    cross_child.manifest.child_id = Uuid::new_v4();
    assert_eq!(
        f.git
            .capture_replacement(
                &f.source,
                CaptureReplacement {
                    repository: f.repository.clone(),
                    parent_id: f.parent,
                    child_id: f.child,
                    work_generation: &d("definition"),
                    correction_generation: &d("correction"),
                    scope: original.manifest.scope.clone(),
                    production_base: &f.production,
                    head: &replacement.manifest.validated_head.commit,
                    staging_revision: &f.squash,
                    staging_result_digest: &d("stage"),
                    originals: &[cross_child],
                    reason: "must not rewrite another child's contribution",
                }
            )
            .unwrap_err()
            .code,
        "scope_violation"
    );
}

fn merge_fixture_candidate(f: &Fixture, candidate: &Candidate) -> String {
    git_run(
        &f.source,
        &[
            "fetch",
            "--no-tags",
            candidate.path().to_str().unwrap(),
            &candidate.manifest.result.as_ref().unwrap().commit,
        ],
    );
    git_run(&f.source, &["switch", "main"]);
    git_run(&f.source, &["merge", "--squash", "FETCH_HEAD"]);
    commit(
        &f.source,
        "feat: observe approved isolated production squash",
    )
}

fn accepted_candidate_fixture(f: &Fixture, candidate: &Candidate) -> VerifiedAcceptedCandidate {
    let key = AttestationKey::new("accepted-candidate-fixture".into(), vec![41; 32]).unwrap();
    let delta = f.delta();
    let (child, _) = verified_child(
        work(f.parent, Some(f.child)),
        RevisionBinding {
            repository: f.repository.clone(),
            head: f.head.clone(),
            base: f.base.clone(),
            tested_commit: f.head.clone(),
            tested_tree: delta.manifest.validated_head.tree.clone(),
        },
        ChildOutcome::Changed {
            pull_request: 1,
            delta: Box::new(delta.manifest),
        },
        &key,
    );
    let (manifest, children) =
        parent_fixture_for_candidates(vec![candidate.manifest.clone()], vec![child], &key);
    let parent = verify_fixture_parent(&manifest, &children).unwrap();
    let acceptance = accept_fixture_parent(&parent, &key);
    verify_accepted_candidate(&parent, &acceptance, None, &candidate.manifest).unwrap()
}

fn candidate_with_integrations(
    f: &Fixture,
    main: &str,
    deltas: &[DeltaArtifact],
    integrations: &[VerifiedProductionIntegration],
) -> Result<Candidate> {
    f.git.assemble_candidate(
        &f.source,
        &f.runs,
        CandidateRequest {
            operation_id: Uuid::new_v4(),
            runtime_id: Uuid::new_v4(),
            repository: f.repository.clone(),
            parent_id: f.parent,
            work_generation: &d("definition"),
            correction_generation: &d("correction"),
            production_commit: main,
            deltas,
            child_dependencies: &BTreeMap::new(),
            satisfied_children: &BTreeSet::new(),
            production_integrations: integrations,
            identity: CommitIdentity {
                name: "Disposable Fixture Human".into(),
                email: "fixture@example.test".into(),
            },
        },
    )
}

#[test]
fn v0_t21_partial_production_merge_is_accounted_without_replaying_or_manufacturing_commit() {
    let f = Fixture::new(false);
    let delta = f.delta();
    let candidate = f
        .candidate(Uuid::new_v4(), std::slice::from_ref(&delta))
        .unwrap();
    let merged = merge_fixture_candidate(&f, &candidate);
    let authority = accepted_candidate_fixture(&f, &candidate);
    let proof = f
        .git
        .observe_production_integration(
            &f.source,
            ObserveProductionIntegration {
                release_operation: Uuid::new_v4(),
                pull_request: 4,
                candidate: &candidate,
                authority: &authority,
                merged_commit: &merged,
                current_main: &merged,
            },
        )
        .unwrap();
    assert_eq!(
        candidate_with_integrations(
            &f,
            &merged,
            std::slice::from_ref(&delta),
            std::slice::from_ref(&proof)
        )
        .unwrap_err()
        .code,
        "duplicate_delta"
    );
    let recovered =
        candidate_with_integrations(&f, &merged, &[], std::slice::from_ref(&proof)).unwrap();
    assert_eq!(
        recovered.manifest.result,
        Some(recovered.manifest.production.clone())
    );
    assert!(recovered.manifest.ordered_delta_digests.is_empty());
    assert_eq!(git_run(recovered.path(), &["rev-parse", "HEAD"]), merged);
    f.git
        .inspect_candidate(&f.runs, recovered.manifest.operation_id)
        .unwrap();
    let key = AttestationKey::new("partial-fixture-key".into(), vec![29; 32]).unwrap();
    let binding = RevisionBinding {
        repository: f.repository.clone(),
        head: f.head.clone(),
        base: f.base.clone(),
        tested_commit: f.head.clone(),
        tested_tree: delta.manifest.validated_head.tree.clone(),
    };
    let (child, _) = verified_child(
        work(f.parent, Some(f.child)),
        binding,
        ChildOutcome::Changed {
            pull_request: 1,
            delta: Box::new(delta.manifest.clone()),
        },
        &key,
    );
    let (manifest, children) =
        parent_fixture_for_candidates(vec![recovered.manifest], vec![child], &key);
    let parent =
        verify_fixture_parent_with_integrations(&manifest, &children, std::slice::from_ref(&proof))
            .unwrap();
    let production = &parent.manifest().candidates[0].production;
    let production_revision = RevisionBinding {
        repository: f.repository.clone(),
        head: production.commit.clone(),
        base: production.commit.clone(),
        tested_commit: production.commit.clone(),
        tested_tree: production.tree.clone(),
    };
    let required: Vec<_> = [("unit", 400), ("integration", 401)]
        .into_iter()
        .map(|(name, workflow)| CheckRequirement {
            id: name.into(),
            kind: CheckKind::ProductionCi,
            workflow_id: Some(workflow),
            command: vec![],
        })
        .collect();
    let mut snapshot = checks(&parent.manifest().work, &production_revision);
    snapshot.runs = required
        .iter()
        .map(|requirement| {
            let mut run = snapshot.runs[1].clone();
            run.id = requirement.id.clone();
            run.kind = CheckKind::ProductionCi;
            run.workflow_id = requirement.workflow_id;
            run
        })
        .collect();
    let validated = verify_existing_production(
        &parent,
        &production_revision,
        &required,
        &snapshot,
        std::slice::from_ref(&proof),
    )
    .unwrap();
    assert!(validated.is_existing_production());
    assert_eq!(validated.revision(), &production_revision);
    assert_eq!(
        verify_existing_production(&parent, &production_revision, &required, &snapshot, &[])
            .unwrap_err()
            .code,
        "production_validation"
    );
    let mut missing = snapshot.clone();
    missing.runs.pop();
    assert_eq!(
        verify_existing_production(
            &parent,
            &production_revision,
            &required,
            &missing,
            std::slice::from_ref(&proof)
        )
        .unwrap_err()
        .code,
        "validation_gate"
    );
    assert_eq!(
        verify_fixture_parent(&manifest, &children)
            .unwrap_err()
            .code,
        "production_provenance"
    );
    let mut forged = proof.record().clone();
    forged
        .selected_contributions
        .values_mut()
        .next()
        .unwrap()
        .insert(d("forged-extra-effect"));
    assert_eq!(
        f.git
            .reverify_production_integration(&f.source, &forged, &candidate, &authority, &merged)
            .unwrap_err()
            .code,
        "production_provenance"
    );
    assert!(
        f.git
            .reverify_production_integration(
                &f.source,
                proof.record(),
                &candidate,
                &authority,
                &f.production
            )
            .is_err()
    );
}

fn accept_fixture_parent(parent: &VerifiedParent, key: &AttestationKey) -> VerifiedAcceptance {
    let acceptors = BTreeSet::from(["human-linear-user".into()]);
    let signed = prepare_acceptance(
        parent,
        AcceptanceRequest {
            supplied_token: parent.token(),
            current_work: &parent.manifest().work,
            pending_correction: false,
            actor: AcceptanceActor {
                control_actor: "os-user-501",
                linear_actor: "human-linear-user",
                provider_writer: "selected-user",
                has_accept_role: true,
                permitted_acceptors: &acceptors,
            },
            runtime_id: Uuid::new_v4(),
            operation_id: Uuid::new_v4(),
            now: Utc::now(),
        },
        key,
    )
    .unwrap();
    verify_acceptance(
        &signed,
        &publication(&signed, "linear"),
        parent,
        &parent.manifest().work,
        false,
        &acceptors,
        key,
    )
    .unwrap()
}

#[test]
fn v0_t21_no_release_required_needs_accepted_complete_production_outcome_and_satisfies_dependencies()
 {
    let f = Fixture::new(false);
    let key = AttestationKey::new("no-release-fixture-key".into(), vec![31; 32]).unwrap();
    let unchanged = candidate_with_integrations(&f, &f.production, &[], &[]).unwrap();
    assert_eq!(
        git_run(unchanged.path(), &["rev-parse", "HEAD"]),
        f.production
    );
    let revision = unchanged.manifest.production.clone();
    let (child, _) = verified_child(
        work(f.parent, Some(f.child)),
        RevisionBinding {
            repository: f.repository.clone(),
            head: revision.commit.clone(),
            base: revision.commit.clone(),
            tested_commit: revision.commit.clone(),
            tested_tree: revision.tree,
        },
        ChildOutcome::NoChange {
            reason: "requested behavior already exists in the production baseline".into(),
            change_gates_inapplicable: BTreeSet::from([
                "branch".into(),
                "pull_request".into(),
                "staging_merge".into(),
            ]),
            exact_sha_validation: "complete production baseline validation".into(),
        },
        &key,
    );
    let (manifest, children) =
        parent_fixture_for_candidates(vec![unchanged.manifest], vec![child], &key);
    let parent = verify_fixture_parent(&manifest, &children).unwrap();
    let acceptance = accept_fixture_parent(&parent, &key);
    let record = NoReleaseRequired {
        schema_version: 1,
        runtime_id: Uuid::new_v4(),
        operation_id: Uuid::new_v4(),
        work: parent.manifest().work.clone(),
        evidence_token: parent.token().into(),
        acceptance_digest: acceptance.digest().into(),
        baselines: parent.manifest().baselines.clone(),
        targets: vec![],
        recorded_at: Utc::now(),
    };
    let no_targets = BTreeMap::new();
    let requirements = || NoReleaseRequirements {
        current_work: &manifest.work,
        pending_correction: false,
        targets: &no_targets,
    };
    let signed =
        prepare_no_release_required(record.clone(), &parent, &acceptance, requirements(), &key)
            .unwrap();
    let verified = verify_no_release_required(
        &signed,
        &parent,
        &acceptance,
        requirements(),
        &publication(&signed, "linear"),
        "selected-user",
        &key,
    )
    .unwrap();
    let mut queue = ReleaseQueue::default();
    let outcome = queue.record_no_release_required(&verified).unwrap();
    assert!(outcome.no_release_required);
    let mut dependent = synthetic_plan(Uuid::new_v4(), &f.repository, 'd');
    dependent.spacing_seconds = 86400;
    dependent.dependencies.push(DependencyRequirement {
        parent_id: f.parent,
        generation: outcome.generation.clone(),
        outcome_digest: outcome.outcome_digest,
    });
    queue.eligible(&dependent, Utc::now()).unwrap();
    dependent.dependencies[0].generation.correction = d("new-corrected-required-outcome");
    assert_eq!(
        queue.eligible(&dependent, Utc::now()).unwrap_err().code,
        "release_dependency"
    );
    let mut changed = record;
    changed.evidence_token = d("a-different-result");
    assert_eq!(
        prepare_no_release_required(changed, &parent, &acceptance, requirements(), &key)
            .unwrap_err()
            .code,
        "no_release_gate"
    );
    assert_eq!(
        prepare_no_release_required(
            signed.payload.clone(),
            &parent,
            &acceptance,
            NoReleaseRequirements {
                current_work: &manifest.work,
                pending_correction: true,
                targets: &BTreeMap::new()
            },
            &key
        )
        .unwrap_err()
        .code,
        "no_release_gate"
    );
    let mut locked = ReleaseQueue::default();
    locked
        .reserve(
            &synthetic_plan(Uuid::new_v4(), &f.repository, 'l'),
            Utc::now(),
        )
        .unwrap();
    assert_eq!(
        locked
            .record_no_release_required(&verified)
            .unwrap_err()
            .code,
        "release_hold"
    );
}

#[test]
fn no_release_preserves_distinct_target_artifacts_and_rejects_unaccounted_inventory() {
    let f = Fixture::new(false);
    let key = AttestationKey::new("target-map-fixture-key".into(), vec![32; 32]).unwrap();
    let unchanged = candidate_with_integrations(&f, &f.production, &[], &[]).unwrap();
    let (mut manifest, children) =
        parent_fixture_for_candidates(vec![unchanged.manifest], vec![], &key);
    manifest.production_targets = BTreeSet::from(["web".into(), "jobs".into()]);
    let baseline = &mut manifest.baselines[0];
    baseline.deployment_free = false;
    baseline.deployed_revision = Some(baseline.production.commit.clone());
    baseline.target_artifacts = BTreeMap::from([
        ("web".into(), d("web artifact")),
        ("jobs".into(), d("jobs artifact")),
    ]);
    assert!(baseline.deployed_artifact.is_none());
    baseline.validate().unwrap();
    let mut inconsistent = baseline.clone();
    inconsistent.deployed_artifact = Some(d("web artifact"));
    assert_eq!(
        inconsistent.validate().unwrap_err().code,
        "production_baseline"
    );
    let configured: BTreeMap<_, _> = manifest
        .production_targets
        .iter()
        .map(|target| (target.clone(), ("github".into(), f.repository.clone())))
        .collect();
    let check = |manifest: &ParentEvidence| {
        let mut manifest = manifest.clone();
        manifest.integrated_results[0].candidate_vector_digest =
            manifest.candidate_vector_digest().unwrap();
        let parent = verify_parent(
            &manifest,
            ParentRequirements {
                work: &manifest.work,
                required_children: &manifest.required_children,
                e2e_child: manifest.e2e_child,
                criteria: &manifest.fulfilled_criteria,
                integrated_commands: &BTreeMap::from([(
                    "integrated".into(),
                    vec!["./e2e.sh".into()],
                )]),
                test_resources: &BTreeSet::new(),
                production_targets: &manifest.production_targets,
                visual: false,
                required_contributions: &manifest.required_contributions,
                production_integrations: &[],
            },
            &children,
            &publication(&manifest, "linear"),
            "selected-user",
        )
        .unwrap();
        let acceptance = accept_fixture_parent(&parent, &key);
        let record = NoReleaseRequired {
            schema_version: 1,
            runtime_id: Uuid::new_v4(),
            operation_id: Uuid::new_v4(),
            work: manifest.work.clone(),
            evidence_token: parent.token().into(),
            acceptance_digest: acceptance.digest().into(),
            baselines: manifest.baselines.clone(),
            targets: configured
                .keys()
                .map(|target| ProductionTargetBaseline {
                    provider: "github".into(),
                    target: target.clone(),
                    repository: f.repository.clone(),
                    source: manifest.baselines[0].production.clone(),
                    artifact_digest: d(if target == "web" {
                        "web artifact"
                    } else {
                        "jobs artifact"
                    }),
                    provider_record: format!("fixture-deployment-{target}"),
                    observed_at: Utc::now(),
                })
                .collect(),
            recorded_at: Utc::now(),
        };
        prepare_no_release_required(
            record,
            &parent,
            &acceptance,
            NoReleaseRequirements {
                current_work: &manifest.work,
                pending_correction: false,
                targets: &configured,
            },
            &key,
        )
    };
    check(&manifest).unwrap();
    let mut extra = manifest.clone();
    extra.baselines[0]
        .target_artifacts
        .insert("unconfigured".into(), d("extra"));
    assert_eq!(check(&extra).unwrap_err().code, "no_release_gate");
    let mut missing = manifest.clone();
    missing.baselines[0].target_artifacts.remove("jobs");
    assert_eq!(check(&missing).unwrap_err().code, "no_release_gate");
    let mut swapped = manifest;
    swapped.baselines[0]
        .target_artifacts
        .insert("jobs".into(), d("web artifact"));
    assert_eq!(check(&swapped).unwrap_err().code, "no_release_gate");
}

#[test]
fn v0_t28_changed_selected_replacement_requires_new_acceptance_even_if_result_bytes_match() {
    let gates = GateFixture::new(1);
    let f = &gates._git_fixtures[0];
    let (replacement, child, _) = replacement_fixture(f, &gates.key);
    let candidate = f.candidate(Uuid::new_v4(), &[replacement]).unwrap();
    let mut manifest = gates.parent.manifest().clone();
    manifest.candidates = vec![candidate.manifest];
    let mut children = gates.children.clone();
    let child_index = children
        .iter()
        .position(|c| c.evidence().work.child == Some(f.child))
        .unwrap();
    children[child_index] = child;
    manifest.child_results = children
        .iter()
        .map(|c| (c.evidence().work.child.unwrap(), c.digest().into()))
        .collect();
    manifest.created_at = Utc::now();
    manifest.integrated_results[0].candidate_vector_digest =
        manifest.candidate_vector_digest().unwrap();
    let newer = verify_fixture_parent(&manifest, &children).unwrap();
    assert_eq!(
        gates.parent.manifest().candidates[0].ordered_patch_digests,
        newer.manifest().candidates[0].ordered_patch_digests
    );
    assert_ne!(
        gates.parent.manifest().candidates[0].ordered_delta_digests,
        newer.manifest().candidates[0].ordered_delta_digests
    );
    assert_eq!(
        carry_acceptance(
            &gates.parent,
            &newer,
            &gates.acceptance,
            &artifact_bytes("fixture://known-baseline", b"known"),
            &[],
            Utc::now()
        )
        .unwrap_err()
        .code,
        "acceptance_required"
    );
    let renewed = accept_fixture_parent(&newer, &gates.key);
    assert_eq!(renewed.acceptance().reviewed_token, newer.token());
}

#[test]
fn v0_t30_partial_a_delivered_b_merged_recovery_has_no_replayed_contribution_or_false_delivery() {
    let key = AttestationKey::new("partial-vector-fixture-key".into(), vec![37; 32]).unwrap();
    let parent_id = Uuid::new_v4();
    let mut a = Fixture::new(false);
    a.parent = parent_id;
    a.repository.id = "repo-a".into();
    let mut b = Fixture::new(false);
    b.parent = parent_id;
    b.repository.id = "repo-b".into();
    let mut candidates = vec![];
    let mut children = vec![];
    let mut proofs = vec![];
    let release_operation = Uuid::new_v4();
    for (index, f) in [&a, &b].into_iter().enumerate() {
        let delta = f.delta();
        let binding = RevisionBinding {
            repository: f.repository.clone(),
            head: f.head.clone(),
            base: f.base.clone(),
            tested_commit: f.head.clone(),
            tested_tree: delta.manifest.validated_head.tree.clone(),
        };
        let (child, _) = verified_child(
            work(parent_id, Some(f.child)),
            binding,
            ChildOutcome::Changed {
                pull_request: 1,
                delta: Box::new(delta.manifest.clone()),
            },
            &key,
        );
        children.push(child);
        let selected = f.candidate(Uuid::new_v4(), &[delta]).unwrap();
        let merged = merge_fixture_candidate(f, &selected);
        let authority = accepted_candidate_fixture(f, &selected);
        let proof = f
            .git
            .observe_production_integration(
                &f.source,
                ObserveProductionIntegration {
                    release_operation,
                    pull_request: 10 + index as u64,
                    candidate: &selected,
                    authority: &authority,
                    merged_commit: &merged,
                    current_main: &merged,
                },
            )
            .unwrap();
        let recovered =
            candidate_with_integrations(f, &merged, &[], std::slice::from_ref(&proof)).unwrap();
        assert!(recovered.manifest.ordered_delta_digests.is_empty());
        assert_eq!(git_run(recovered.path(), &["rev-parse", "HEAD"]), merged);
        assert_eq!(
            fs::read_to_string(recovered.path().join("a.txt")).unwrap(),
            "first\nsecond\nthird\nA fourth\n"
        );
        candidates.push(recovered.manifest);
        proofs.push(proof);
    }
    let (mut manifest, children) = parent_fixture_for_candidates(candidates, children, &key);
    // A is delivered. B's exact merge is known, but its deployment is still the
    // previous baseline. The recovery vector must preserve that distinction.
    for baseline in &mut manifest.baselines {
        baseline.deployment_free = false;
        baseline.deployed_revision = Some(if baseline.repository == a.repository {
            baseline.production.commit.clone()
        } else {
            b.production.clone()
        });
        baseline.deployed_artifact = Some(d(if baseline.repository == a.repository {
            "delivered-a"
        } else {
            "previous-b"
        }));
        let target = if baseline.repository == a.repository {
            "target-a"
        } else {
            "target-b"
        };
        baseline
            .target_revisions
            .insert(target.into(), baseline.deployed_revision.clone().unwrap());
        baseline
            .target_artifacts
            .insert(target.into(), baseline.deployed_artifact.clone().unwrap());
        baseline.successor_recovery_operation = Some(release_operation);
    }
    manifest.production_targets = BTreeSet::from(["target-a".into(), "target-b".into()]);
    manifest.integrated_results[0].candidate_vector_digest =
        manifest.candidate_vector_digest().unwrap();
    let parent = verify_parent(
        &manifest,
        ParentRequirements {
            work: &manifest.work,
            required_children: &manifest.required_children,
            e2e_child: manifest.e2e_child,
            criteria: &manifest.fulfilled_criteria,
            integrated_commands: &BTreeMap::from([("integrated".into(), vec!["./e2e.sh".into()])]),
            test_resources: &BTreeSet::new(),
            production_targets: &manifest.production_targets,
            visual: false,
            required_contributions: &manifest.required_contributions,
            production_integrations: &proofs,
        },
        &children,
        &publication(&manifest, "linear"),
        "selected-user",
    )
    .unwrap();
    let acceptance = accept_fixture_parent(&parent, &key);
    let targets = BTreeMap::from([
        ("target-a".into(), ("github".into(), a.repository.clone())),
        ("target-b".into(), ("github".into(), b.repository.clone())),
    ]);
    // Real owned Git integration proofs establish that neither repository needs
    // another merge. Hosted checks below are explicit deterministic facts.
    let required = vec![CheckRequirement {
        id: "production-ci".into(),
        kind: CheckKind::ProductionCi,
        workflow_id: Some(400),
        command: vec![],
    }];
    let mut production = Vec::new();
    for candidate in &manifest.candidates {
        let source = &candidate.production;
        let revision = RevisionBinding {
            repository: candidate.repository.clone(),
            head: source.commit.clone(),
            base: source.commit.clone(),
            tested_commit: source.commit.clone(),
            tested_tree: source.tree.clone(),
        };
        let mut snapshot = checks(&manifest.work, &revision);
        let mut run = snapshot.runs[1].clone();
        run.id = "production-ci".into();
        run.kind = CheckKind::ProductionCi;
        run.workflow_id = Some(400);
        snapshot.runs = vec![run];
        production.push(
            verify_existing_production(&parent, &revision, &required, &snapshot, &proofs).unwrap(),
        );
    }
    let source_a = manifest
        .baselines
        .iter()
        .find(|baseline| baseline.repository == a.repository)
        .unwrap()
        .production
        .clone();
    let source_b = manifest
        .baselines
        .iter()
        .find(|baseline| baseline.repository == b.repository)
        .unwrap()
        .production
        .clone();
    let target_a = ResourceKey::Target {
        provider: "github".into(),
        id: "target-a".into(),
    };
    let target_b = ResourceKey::Target {
        provider: "github".into(),
        id: "target-b".into(),
    };
    let plan = ReleasePlan {
        schema_version: 1,
        runtime_id: Uuid::new_v4(),
        operation_id: Uuid::new_v4(),
        parent_id,
        linear_workspace: manifest.work.linear_workspace,
        generation: manifest.work.generation.clone(),
        evidence_token: parent.token().into(),
        acceptance_digest: acceptance.digest().into(),
        carried_acceptance: None,
        resources: BTreeSet::from([
            ResourceKey::Repository {
                repository: a.repository.clone(),
            },
            ResourceKey::Repository {
                repository: b.repository.clone(),
            },
            target_a.clone(),
            target_b.clone(),
        ]),
        dependencies: vec![],
        spacing_seconds: 0,
        deployment_free: BTreeSet::new(),
        steps: vec![
            ReleaseStep {
                id: "verify-a".into(),
                depends_on: BTreeSet::new(),
                kind: StepKind::VerifyExisting {
                    repository: a.repository.clone(),
                    target: target_a.clone(),
                    source: source_a.clone(),
                    artifact_digest: d("delivered-a"),
                },
            },
            ReleaseStep {
                id: "deploy-b".into(),
                depends_on: BTreeSet::from(["verify-a".into()]),
                kind: StepKind::DeployExisting {
                    repository: b.repository.clone(),
                    target: target_b.clone(),
                    workflow_id: 400,
                    workflow_revision: "a".repeat(40),
                    dispatch_ref: "heads/deploy".into(),
                    source_input: "source".into(),
                    source: source_b.clone(),
                    expected_artifact: None,
                },
            },
            ReleaseStep {
                id: "verify-b".into(),
                depends_on: BTreeSet::from(["deploy-b".into()]),
                kind: StepKind::VerifyDeployed {
                    repository: b.repository.clone(),
                    target: target_b.clone(),
                    source: source_b.clone(),
                    deployment_step: "deploy-b".into(),
                    expected_artifact: None,
                },
            },
        ],
    };
    let admission = AdmissionFacts {
        running: true,
        included: true,
        authorized: true,
        no_pending_correction: true,
        preparation_complete: true,
        provider_reads_complete: true,
        repository_policy_usable: true,
        credentials_usable: true,
        manual_mode: true,
        explicit_manual_request: Some(Uuid::new_v4()),
        actor_may_release: true,
        owner_generation: 1,
        control_generation: 1,
        required_step_ids: plan.steps.iter().map(|step| step.id.clone()).collect(),
    };
    let mut attempt = ReleaseAttempt::admit(
        plan.clone(),
        DeliveryEvidence {
            parent: &parent,
            acceptance: &acceptance,
            carried: None,
            production: &production,
        },
        admission,
        Utc::now(),
    )
    .unwrap();
    assert!(
        plan.steps
            .iter()
            .all(|step| !matches!(step.kind, StepKind::Merge { .. }))
    );
    let revisions = BTreeMap::from([
        (a.repository.clone(), source_a.clone()),
        (b.repository.clone(), source_b.clone()),
    ]);
    let facts = FreshReleaseFacts {
        generation: &plan.generation,
        evidence_token: &plan.evidence_token,
        owner_generation: 1,
        authority_current: true,
        pending_correction: false,
        provider_reads_complete: true,
        resource_lock_owned: true,
        credentials_usable: true,
        repository_policy_current: true,
        force_stopped: false,
        current_revisions: &revisions,
    };
    for (step, effect) in [
        (
            "verify-a",
            ObservedEffect::Verification {
                repository: a.repository.clone(),
                target: target_a,
                source_commit: source_a.commit,
                tree: source_a.tree,
                artifact_digest: d("delivered-a"),
                evidence_digest: d("native-a-proof"),
            },
        ),
        (
            "deploy-b",
            ObservedEffect::Deployment {
                repository: b.repository.clone(),
                target: target_b.clone(),
                source_commit: source_b.commit.clone(),
                tree: source_b.tree.clone(),
                artifact_digest: d("delivered-b"),
                deployment_id: "actual-fixture-deployment-b".into(),
                run: OwnedRun {
                    repository: b.repository.clone(),
                    operation_id: plan.operation_id,
                    run_id: 550,
                    attempt: 1,
                    actor_account_id: "selected-user".into(),
                    origin_digest: d("fixture-owned-target-run"),
                },
            },
        ),
        (
            "verify-b",
            ObservedEffect::Verification {
                repository: b.repository.clone(),
                target: target_b,
                source_commit: source_b.commit,
                tree: source_b.tree,
                artifact_digest: d("delivered-b"),
                evidence_digest: d("native-b-proof"),
            },
        ),
    ] {
        let payload = attempt.prepare_step(step, &facts, |_| Ok(())).unwrap();
        assert!(payload.exact_source_commit.is_some());
        attempt.mark_sent(&payload, &facts, |_| Ok(())).unwrap();
        attempt
            .observe_success(step, effect, Utc::now(), |_| Ok(()))
            .unwrap();
    }
    assert!(attempt.completed());
    let record = NoReleaseRequired {
        schema_version: 1,
        runtime_id: Uuid::new_v4(),
        operation_id: Uuid::new_v4(),
        work: manifest.work.clone(),
        evidence_token: parent.token().into(),
        acceptance_digest: acceptance.digest().into(),
        baselines: manifest.baselines.clone(),
        targets: vec![ProductionTargetBaseline {
            provider: "github".into(),
            target: "target-a".into(),
            repository: a.repository,
            source: manifest.baselines[0].production.clone(),
            artifact_digest: d("delivered-a"),
            provider_record: "observed-deployment-a".into(),
            observed_at: Utc::now(),
        }],
        recorded_at: Utc::now(),
    };
    assert_eq!(
        prepare_no_release_required(
            record,
            &parent,
            &acceptance,
            NoReleaseRequirements {
                current_work: &manifest.work,
                pending_correction: false,
                targets: &targets
            },
            &key
        )
        .unwrap_err()
        .code,
        "no_release_gate"
    );
}

#[test]
fn v0_t21_later_correction_selects_only_new_delta_and_preserves_earlier_production_history() {
    let f = Fixture::new(false);
    let original = f.delta();
    let old_candidate = f
        .candidate(Uuid::new_v4(), std::slice::from_ref(&original))
        .unwrap();
    let merged = merge_fixture_candidate(&f, &old_candidate);
    let authority = accepted_candidate_fixture(&f, &old_candidate);
    let proof = f
        .git
        .observe_production_integration(
            &f.source,
            ObserveProductionIntegration {
                release_operation: Uuid::new_v4(),
                pull_request: 7,
                candidate: &old_candidate,
                authority: &authority,
                merged_commit: &merged,
                current_main: &merged,
            },
        )
        .unwrap();
    git_run(
        &f.source,
        &["switch", "-c", "A-authorized-correction", &f.squash],
    );
    fs::write(
        f.source.join("a.txt"),
        "first\nsecond\nthird\nA corrected fourth\n",
    )
    .unwrap();
    let corrected_head = commit(
        &f.source,
        "fix: implement authorized correction in original child",
    );
    git_run(&f.source, &["switch", "staging"]);
    git_run(&f.source, &["merge", "--squash", "A-authorized-correction"]);
    let corrected_squash = commit(
        &f.source,
        "fix: squash reviewed child correction into staging",
    );
    let delta = f
        .git
        .capture_delta(
            &f.source,
            CaptureDelta {
                repository: f.repository.clone(),
                parent_id: f.parent,
                child_id: f.child,
                work_generation: &d("definition"),
                correction_generation: &d("corrected-generation"),
                scope: original.manifest.scope.clone(),
                base: &f.squash,
                head: &corrected_head,
                staging_parent: &f.squash,
                staging_commit: &corrected_squash,
            },
        )
        .unwrap();
    let request = CandidateRequest {
        operation_id: Uuid::new_v4(),
        runtime_id: Uuid::new_v4(),
        repository: f.repository.clone(),
        parent_id: f.parent,
        work_generation: &d("definition"),
        correction_generation: &d("corrected-generation"),
        production_commit: &merged,
        deltas: std::slice::from_ref(&delta),
        child_dependencies: &BTreeMap::new(),
        satisfied_children: &BTreeSet::new(),
        production_integrations: std::slice::from_ref(&proof),
        identity: CommitIdentity {
            name: "Disposable Fixture Human".into(),
            email: "fixture@example.test".into(),
        },
    };
    let corrected = f
        .git
        .assemble_candidate(&f.source, &f.runs, request)
        .unwrap();
    assert_eq!(
        corrected.manifest.ordered_delta_digests,
        vec![delta.manifest.digest().unwrap()]
    );
    assert_eq!(
        corrected.manifest.production_integrations[0].selected_delta_digests,
        vec![original.manifest.digest().unwrap()]
    );
    assert_eq!(git_run(corrected.path(), &["rev-parse", "HEAD^"]), merged);
    assert_eq!(
        fs::read_to_string(corrected.path().join("a.txt")).unwrap(),
        "first\nsecond\nthird\nA corrected fourth\n"
    );
    assert!(!corrected.path().join("unapproved-b.txt").exists());
    assert_eq!(
        git_run(&f.source, &["show", &format!("{merged}:a.txt")]),
        "first\nsecond\nthird\nA fourth"
    );
    f.git.verify_delta(&f.source, &original).unwrap();
}

#[test]
fn recovery_cannot_reassign_contributions_by_rewriting_both_retained_metadata_and_receipt() {
    let f = Fixture::new(false);
    let candidate = f.candidate(Uuid::new_v4(), &[f.delta()]).unwrap();
    let authority = accepted_candidate_fixture(&f, &candidate);
    let merged = merge_fixture_candidate(&f, &candidate);
    let integration = f
        .git
        .observe_production_integration(
            &f.source,
            ObserveProductionIntegration {
                release_operation: Uuid::new_v4(),
                pull_request: 8,
                candidate: &candidate,
                authority: &authority,
                merged_commit: &merged,
                current_main: &merged,
            },
        )
        .unwrap();
    let mut forged_manifest = candidate.manifest.clone();
    forged_manifest
        .selected_contributions
        .values_mut()
        .next()
        .unwrap()
        .insert(d("another-required-outcome-never-produced"));
    fs::write(
        candidate.path().join(".git/kegari-candidate.json"),
        serde_json::to_vec(&forged_manifest).unwrap(),
    )
    .unwrap();
    let inspected = f
        .git
        .inspect_candidate(&f.runs, candidate.manifest.operation_id)
        .unwrap();
    assert_eq!(inspected.manifest.result, candidate.manifest.result);
    let mut forged_receipt = integration.record().clone();
    forged_receipt.selected_contributions = forged_manifest.selected_contributions;
    forged_receipt.source_candidate_digest = canonical_digest(1, &inspected.manifest).unwrap();
    assert_eq!(
        f.git
            .reverify_production_integration(
                &f.source,
                &forged_receipt,
                &inspected,
                &authority,
                &merged
            )
            .unwrap_err()
            .code,
        "production_provenance"
    );
}

#[test]
fn canonical_publications_reject_quotes_trailing_data_duplicates_and_wrong_domains() {
    let key = AttestationKey::new("fixture-key".into(), vec![17; 32]).unwrap();
    let value = serde_json::json!({"schema_version":1,"exact":"record"});
    let signed = key.sign("fixture_record", &value).unwrap();
    let body = encode_signed_publication("fixture_record", &signed).unwrap();
    assert_eq!(
        decode_signed_publication::<serde_json::Value>(&body, "fixture_record", &key).unwrap(),
        signed
    );
    for bad in [
        format!("Example: {body}"),
        format!("{body}\n"),
        format!("{body}{body}"),
        body.replacen("\"signature\":", "\"unexpected\":1,\"signature\":", 1),
        body.replacen(
            "\"schema_version\":1",
            "\"schema_version\":1,\"schema_version\":1",
            1,
        ),
    ] {
        assert!(
            decode_signed_publication::<serde_json::Value>(&bad, "fixture_record", &key).is_err()
        );
    }
    assert!(
        decode_signed_publication::<serde_json::Value>(&body, "human_acceptance", &key).is_err()
    );
    let other = AttestationKey::new("fixture-key".into(), vec![18; 32]).unwrap();
    assert!(
        decode_signed_publication::<serde_json::Value>(&body, "fixture_record", &other).is_err()
    );
}

#[test]
fn mixed_release_verifies_unchanged_target_without_inventing_merge_or_deploy() {
    let mut f = GateFixture::new(1);
    let mut unchanged = Fixture::new(false);
    unchanged.parent = f.parent.manifest().work.parent;
    unchanged.repository.id = "unchanged-target-repository".into();
    let candidate = unchanged.candidate(Uuid::new_v4(), &[]).unwrap();
    let source = candidate.manifest.production.clone();
    let original_main = git_run(&unchanged.source, &["rev-parse", "main"]);
    let target = ResourceKey::Target {
        provider: "github".into(),
        id: "already-serving".into(),
    };
    let artifact = d("actual-fixture-existing-artifact");
    let mut manifest = f.parent.manifest().clone();
    manifest.candidates.push(candidate.manifest.clone());
    manifest.baselines.push(ProductionBaseline {
        repository: unchanged.repository.clone(),
        production: source.clone(),
        deployed_revision: Some(source.commit.clone()),
        target_revisions: BTreeMap::new(),
        deployed_artifact: Some(artifact.clone()),
        target_artifacts: BTreeMap::from([("already-serving".into(), artifact.clone())]),
        deployment_free: false,
        successor_recovery_operation: None,
    });
    manifest.production_targets.insert("already-serving".into());
    manifest.integrated_results[0].candidate_vector_digest =
        manifest.candidate_vector_digest().unwrap();
    f.parent = verify_fixture_parent(&manifest, &f.children).unwrap();
    f.acceptance = accept_fixture_parent(&f.parent, &f.key);
    let revision = RevisionBinding {
        repository: unchanged.repository.clone(),
        head: source.commit.clone(),
        base: source.commit.clone(),
        tested_commit: source.commit.clone(),
        tested_tree: source.tree.clone(),
    };
    let requirements = vec![CheckRequirement {
        id: "existing-production".into(),
        kind: CheckKind::ProductionCi,
        workflow_id: Some(400),
        command: vec![],
    }];
    let mut snapshot = checks(&manifest.work, &revision);
    let mut run = snapshot.runs[1].clone();
    run.id = requirements[0].id.clone();
    run.kind = CheckKind::ProductionCi;
    run.workflow_id = Some(400);
    snapshot.runs = vec![run];
    f.production.push(
        verify_existing_production(&f.parent, &revision, &requirements, &snapshot, &[]).unwrap(),
    );
    f.plan.evidence_token = f.parent.token().into();
    f.plan.acceptance_digest = f.acceptance.digest().into();
    f.plan.resources.insert(ResourceKey::Repository {
        repository: unchanged.repository.clone(),
    });
    f.plan.resources.insert(target.clone());
    f.plan.steps.push(ReleaseStep {
        id: "verify-existing".into(),
        depends_on: BTreeSet::new(),
        kind: StepKind::VerifyExisting {
            repository: unchanged.repository.clone(),
            target: target.clone(),
            source: source.clone(),
            artifact_digest: artifact.clone(),
        },
    });
    let mut attempt =
        ReleaseAttempt::admit(f.plan.clone(), f.evidence(), f.facts(), Utc::now()).unwrap();
    let current = BTreeMap::from([(unchanged.repository.clone(), source.clone())]);
    let facts = FreshReleaseFacts {
        generation: &f.plan.generation,
        evidence_token: f.parent.token(),
        owner_generation: 1,
        authority_current: true,
        pending_correction: false,
        provider_reads_complete: true,
        resource_lock_owned: true,
        credentials_usable: true,
        repository_policy_current: true,
        force_stopped: false,
        current_revisions: &current,
    };
    let payload = attempt
        .prepare_step("verify-existing", &facts, |_| Ok(()))
        .unwrap();
    assert_eq!(
        payload.exact_source_commit.as_deref(),
        Some(source.commit.as_str())
    );
    attempt.mark_sent(&payload, &facts, |_| Ok(())).unwrap();
    let effect = ObservedEffect::Verification {
        repository: unchanged.repository.clone(),
        target: target.clone(),
        source_commit: source.commit.clone(),
        tree: source.tree.clone(),
        artifact_digest: artifact.clone(),
        evidence_digest: d("current authenticated target fixture"),
    };
    let mut wrong = effect.clone();
    if let ObservedEffect::Verification {
        artifact_digest, ..
    } = &mut wrong
    {
        *artifact_digest = d("different serving artifact");
    }
    assert_eq!(
        attempt
            .observe_success("verify-existing", wrong, Utc::now(), |_| Ok(()))
            .unwrap_err()
            .code,
        "release_provenance"
    );
    attempt
        .observe_success("verify-existing", effect, Utc::now(), |_| Ok(()))
        .unwrap();
    assert!(!attempt.completed());
    let mut changed_artifact = f.plan.clone();
    if let StepKind::VerifyExisting {
        artifact_digest, ..
    } = &mut changed_artifact.steps.last_mut().unwrap().kind
    {
        *artifact_digest = d("unaccepted artifact");
    }
    assert_eq!(
        ReleaseAttempt::admit(changed_artifact, f.evidence(), f.facts(), Utc::now())
            .unwrap_err()
            .code,
        "scope_violation"
    );
    let mut omitted = f.plan.clone();
    omitted.steps.pop();
    omitted.resources.remove(&target);
    omitted.resources.remove(&ResourceKey::Repository {
        repository: unchanged.repository.clone(),
    });
    let mut incomplete_facts = f.facts();
    incomplete_facts.required_step_ids = omitted.steps.iter().map(|step| step.id.clone()).collect();
    assert_eq!(
        ReleaseAttempt::admit(omitted, f.evidence(), incomplete_facts, Utc::now())
            .unwrap_err()
            .code,
        "scope_violation"
    );
    assert_eq!(
        git_run(&unchanged.source, &["rev-parse", "main"]),
        original_main
    );
    assert_eq!(
        git_run(candidate.path(), &["rev-parse", "HEAD"]),
        source.commit
    );
}
