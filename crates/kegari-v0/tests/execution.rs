//! Retained deterministic Git/source ownership checks. Obsolete repository runner,
//! command sandbox, hook and media-tool fixtures were removed with that harness.
//! Native Codex result and artifact coverage lives with the current worker path.
#[path = "support/pinned_git.rs"]
mod pinned_git;
use kegari_digest::bytes_digest;
use kegari_execution::{ExecutionRole, IsolationPlan, PathGrant, ValidatedIsolation};
use kegari_git::RepositoryId;
use kegari_v0::evidence::{Generation, WorkIdentity};
use kegari_v0::execution::*;
use kegari_v0::git::Git;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
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
    let output = kegari_process::group::OwnedGroup::spawn_std(
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
        let plan = IsolationPlan {
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
        let boundary = plan.validate(kegari_v0::BUILD_ID).unwrap();
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
}

fn source_metadata() -> kegari_git::SourceCommitMetadata {
    kegari_git::SourceCommitMetadata {
        identity: kegari_git::CommitIdentity {
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
    fs::write(root.join(".git/info/exclude"), "generated-output/\n").unwrap();
    fs::create_dir(root.join("generated-output")).unwrap();
    fs::write(
        root.join("generated-output/result.bin"),
        b"ignored build output",
    )
    .unwrap();
    let scope = kegari_git::WriteScope {
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
                &kegari_git::WriteScope {
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
    assert_eq!(
        fs::read(root.join("generated-output/result.bin")).unwrap(),
        b"ignored build output"
    );
    fs::write(root.join("untracked-result"), b"not an admitted output").unwrap();
    assert_eq!(
        f.git
            .verify_execution_tree(root, &next_binding.revision, &Default::default())
            .unwrap_err()
            .code,
        "dirty_execution"
    );
    fs::remove_file(root.join("untracked-result")).unwrap();
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
    let scope = kegari_git::WriteScope {
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
    let scope = kegari_git::WriteScope {
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
    assert!(
        f.git
            .commit_execution_changes(
                &f.owner,
                &f.binding,
                &f.boundary,
                &kegari_git::WriteScope {
                    paths: vec!["a.txt".into()]
                },
                source_metadata(),
            )
            .is_err()
    );
    assert_eq!(fs::read(&source).unwrap(), b"original\n");
}
