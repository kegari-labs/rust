//! Actual disposable Git objects/commands and Control metadata admission;
//! the boot probe is synthetic and no native sandbox/worker completion is claimed.
use super::*;
use kegari_digest::bytes_digest;
use kegari_v0::{
    authority::IssueKey,
    evidence::{Generation, WorkIdentity},
    execution::{DirectoryIdentity, ExecutionBinding, OwnedDirectory},
    isolation::{ExecutionRole, IsolationPlan, ValidatedIsolation},
};
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};

struct ReuseFixture {
    _temp: tempfile::TempDir,
    git: Git,
    unscoped: Git,
    ownership: OwnedDirectory,
    binding: ExecutionBinding,
    boundary: ValidatedIsolation,
    authority: Arc<LocalProcessPermit>,
    services: HostProcessServices,
    source: PathBuf,
}
impl ReuseFixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let binary = pinned_git::prepare(&root);
        let operator = PrivateDir::create(&root.join("operator")).unwrap();
        git_run(&binary, operator.path(), &["init", "--initial-branch=main"]);
        fs::write(operator.path().join("file.txt"), b"original bytes\n").unwrap();
        git_run(&binary, operator.path(), &["add", "file.txt"]);
        git_run(
            &binary,
            operator.path(),
            &[
                "commit",
                "--signoff",
                "-m",
                "test: initialize owned reuse fixture",
            ],
        );
        let unscoped = Git::new(&binary).unwrap();
        let revision = unscoped.revision(operator.path(), "HEAD").unwrap();
        let execution = PrivateDir::create(&root.join("execution")).unwrap();
        let repository = RepositoryId {
            hostname: "github.com".into(),
            id: "42".into(),
        };
        let work = WorkIdentity {
            linear_endpoint: "https://linear.invalid/graphql".into(),
            linear_workspace: id(3),
            parent: id(31),
            child: Some(id(32)),
            generation: Generation {
                authorization: bytes_digest(b"authorization"),
                definition: bytes_digest(b"definition"),
                correction: bytes_digest(b"correction"),
                scope: bytes_digest(b"scope"),
            },
        };
        let identity = DirectoryIdentity {
            schema_version: 1,
            runtime_id: id(1),
            directory_id: id(34),
            repository: repository.clone(),
            work: work.clone(),
            repository_path: execution.path().join("repository"),
            operator_checkout: operator.path().into(),
        };
        let source = unscoped
            .prepare_execution_clone(operator.path(), execution.path(), &identity, &revision)
            .unwrap();
        let probe = Arc::new(process_probe::FixtureBootProbe::new());
        let current = probe.current().unwrap();
        let host = HostIdentity {
            host_id: "fixture".into(),
            boot_id: current.boot_id,
            uid: current.uid,
            user: "fixture".into(),
            home: root.clone(),
        };
        let (registration, control) = Registration::bootstrap_with_probe(
            &host,
            id(1),
            BinaryIdentity::inspect(&std::env::current_exe().unwrap(), false).unwrap(),
            Uuid::new_v4(),
            probe.clone(),
        )
        .unwrap();
        let mut config = config();
        config.runtime.execution_root = execution.path().to_string_lossy().into();
        config
            .workspaces
            .get_mut("example")
            .unwrap()
            .repositories
            .get_mut("42")
            .unwrap()
            .source = source.to_string_lossy().into();
        registration.complete_setup(&config).unwrap();
        let boundary = IsolationPlan {
            managed_execution: None,
            schema_version: 1,
            role: ExecutionRole::Validation,
            execution_root: execution.path().into(),
            repository: source.clone(),
            cwd: source.clone(),
            scratch: execution.child("scratch").unwrap().path().into(),
            read_roots: vec![],
            resource_writes: vec![],
            source_writes: vec![],
            forbidden_roots: vec![registration.root().path().into()],
        }
        .validate()
        .unwrap();
        let ownership = OwnedDirectory::register(identity, &boundary).unwrap();
        let binding = ExecutionBinding {
            runtime_id: id(1),
            operation_id: id(33),
            work: work.clone(),
            repository: repository.clone(),
            revision,
            candidate_vector_digest: None,
            isolation_digest: boundary.digest().into(),
        };
        let contract = RepositoryContract {
            configuration_name: "42".into(),
            repository,
            provider: RepositoryIdentity {
                id: 42,
                owner: "fixture".into(),
                name: "repo".into(),
                owner_id: 7,
                organization_id: None,
            },
            configuration: config.workspaces["example"].repositories["42"].clone(),
        };
        let processes = Arc::new(
            ProcessJournal::with_probe(
                registration.root().child("worker-processes").unwrap(),
                id(1),
                probe,
            )
            .unwrap(),
        );
        let services = HostProcessServices {
            control: Arc::new(control),
            registration: Arc::new(registration),
            configuration: Arc::new(ReadConfiguration(Mutex::new(config.clone()))),
            processes,
            executions: Arc::new(ExecutionRegistry::default()),
        };
        let authority = LocalProcessPermit::metadata(
            services.clone(),
            MetadataRequest {
                request_id: Uuid::new_v4(),
                work_key: IssueKey::new(
                    &work.linear_endpoint,
                    work.linear_workspace,
                    work.child.unwrap(),
                )
                .unwrap()
                .identity()
                .unwrap(),
                roots: vec![configured_host_read_root(&config, id(2), &contract).unwrap()],
                git_binary: BinaryIdentity::inspect(&binary, true).unwrap(),
            },
        )
        .unwrap();
        let git = unscoped
            .with_host_execution_scope(HostGitExecutionScope {
                limits: GitLimits::default(),
                authority: authority.clone(),
            })
            .unwrap();
        Self {
            _temp: temp,
            git,
            unscoped,
            ownership,
            binding,
            boundary,
            authority,
            services,
            source,
        }
    }
    fn verify(&self) -> Result<kegari_v0::git::VerifiedReusableSource> {
        self.git
            .verify_reusable_execution(&self.ownership, &self.binding, &self.boundary)
    }
}

#[test]
fn reusable_source_requires_fresh_host_scope_and_revalidates_without_processes() {
    let f = ReuseFixture::new();
    let original_index = fs::read(f.source.join(".git/index")).unwrap();
    assert_eq!(
        f.unscoped
            .verify_reusable_execution(&f.ownership, &f.binding, &f.boundary)
            .err()
            .unwrap()
            .code,
        "process_authority_required"
    );
    let proof = f.verify().unwrap();
    assert_eq!(proof.binding(), &f.binding);
    assert!(!proof.digest().is_empty());
    assert_eq!(
        fs::read(f.source.join(".git/index")).unwrap(),
        original_index
    );
    f.authority
        .finish(json!({"fixture":"actual bounded source metadata reads"}))
        .unwrap();
    let operation = f.authority.identity().operation_id;
    let count = f.services.processes.observe(operation).unwrap().len();
    assert!(count > 0);
    proof.revalidate_filesystem().unwrap();
    assert_eq!(
        f.services.processes.observe(operation).unwrap().len(),
        count
    );
    assert!(
        f.verify().is_err(),
        "finished host authority cannot mint another proof"
    );
    assert_eq!(
        f.services.processes.observe(operation).unwrap().len(),
        count
    );
    fs::write(f.source.join("file.txt"), b"modified bytes\n").unwrap();
    assert_eq!(
        proof.revalidate_filesystem().unwrap_err().code,
        "source_reuse_blocked"
    );
}

#[test]
fn reusable_source_rejects_hidden_edits_untracked_files_and_extra_directories() {
    for fault in ["hidden", "ignored", "directory", "mode", "hardlink"] {
        let f = ReuseFixture::new();
        let binary = f.unscoped.executable().to_path_buf();
        match fault {
            "hidden" => {
                git_run(
                    &binary,
                    &f.source,
                    &["update-index", "--assume-unchanged", "file.txt"],
                );
                fs::write(f.source.join("file.txt"), b"modified bytes\n").unwrap();
                assert!(
                    f.unscoped
                        .cleanup_read(&f.source, kegari_v0::git::CleanupRead::Status)
                        .unwrap()
                        .is_empty()
                );
            }
            "ignored" => {
                fs::create_dir_all(f.source.join(".git/info")).unwrap();
                fs::write(f.source.join(".git/info/exclude"), b"ignored.tmp\n").unwrap();
                fs::write(f.source.join("ignored.tmp"), b"unfinished output").unwrap();
            }
            "directory" => {
                fs::create_dir(f.source.join("untracked-empty")).unwrap();
            }
            "mode" => {
                fs::set_permissions(f.source.join("file.txt"), fs::Permissions::from_mode(0o755))
                    .unwrap()
            }
            _ => fs::hard_link(
                f.source.join("file.txt"),
                f._temp.path().join("outside-alias"),
            )
            .unwrap(),
        }
        assert!(f.verify().is_err(), "{fault}");
        assert!(
            f.source.join("file.txt").exists(),
            "verification preserves {fault}"
        );
        f.authority
            .finish(json!({"fixture":"source refused","fault":fault}))
            .unwrap();
    }
}

#[test]
fn reusable_source_binds_marker_index_source_inode_and_exact_original_revision() {
    for fault in ["source-inode", "index", "marker", "revision"] {
        let f = ReuseFixture::new();
        let proof = f.verify().unwrap();
        match fault {
            "source-inode" => {
                let source = f.source.join("file.txt");
                fs::rename(&source, f._temp.path().join("old-source")).unwrap();
                fs::write(&source, b"original bytes\n").unwrap();
            }
            "index" => {
                git_run(
                    f.unscoped.executable(),
                    &f.source,
                    &["update-index", "--assume-unchanged", "file.txt"],
                );
            }
            "marker" => {
                let mut marker: Value =
                    serde_json::from_slice(&fs::read(f.ownership.marker()).unwrap()).unwrap();
                marker["directory_id"] = json!(Uuid::new_v4());
                fs::write(f.ownership.marker(), serde_json::to_vec(&marker).unwrap()).unwrap();
            }
            _ => {
                fs::write(f.source.join("file.txt"), b"new committed source\n").unwrap();
                let binary = f.unscoped.executable();
                git_run(binary, &f.source, &["add", "file.txt"]);
                git_run(
                    binary,
                    &f.source,
                    &[
                        "commit",
                        "--signoff",
                        "-m",
                        "test: change retained fixture source",
                    ],
                );
                assert!(f.verify().is_err());
            }
        }
        assert!(proof.revalidate_filesystem().is_err(), "{fault}");
        f.authority
            .finish(json!({"fixture":"filesystem changed","fault":fault}))
            .unwrap();
    }
}
