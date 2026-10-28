//! Actual scoped Git and family APIs; terminal metadata is explicitly synthetic.
//! No provider session, ConfinedExecution or runtime joined cache is created.
use crate::authority::IssueKey;
use crate::control::Lifecycle;
use crate::control::MutationRequest;
use crate::control::Receipt;
use crate::delivery::RepositoryContract;
use crate::git::Git;
use crate::git::GitLimits;
use crate::git::HostGitExecutionScope;
use crate::git::RepositoryId;
use crate::git::configured_host_read_root;
use crate::host_process::HostProcessServices;
use crate::host_process::LocalProcessPermit;
use crate::host_process::MetadataRequest;
use crate::platform::Registration;
use crate::process_records::ProcessJournal;
use crate::runtime_host::ConfigurationControl;
use crate::runtime_host::HostFuture;
use crate::test_process_probe as process_probe;
use crate::worker::ExecutionRegistry;
use crate::{
    control::{Lease, RunAdmission, RunResumeReference},
    evidence::{Generation, WorkIdentity},
    execution::{
        DirectoryIdentity, ExecutionBinding, ExecutionEvidenceClass, HookLedger, HookPhase,
        HookRecord, HookState, OwnedDirectory,
    },
    housekeeping::Housekeeping,
    isolation::{ExecutionRole, IsolationPlan, ValidatedIsolation},
    workflow::Workflow,
};
use kegari_config::Config;
use kegari_control_api::identity::Actor;
use kegari_digest::{bytes_digest, canonical_digest};
use kegari_platform::{BinaryIdentity, HostIdentity, ProcessProbe};
use kegari_providers::RepositoryIdentity;
use kegari_storage::PrivateDir;
use kegari_types::api::CommandRequest;
use kegari_types::error::{Error, Result};
use kegari_types::run::RunReference;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::Path,
    process::Command,
    sync::{Arc, Mutex},
};
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};
use uuid::Uuid;
fn id(value: u128) -> Uuid {
    Uuid::from_u128(value)
}
fn config() -> Config {
    let profile =
        json!({"tool":"codex","model":"fixture-model","reasoning":"high","speed":"standard"});
    let proof = json!({"workflow_ref":"fixture/repo/.github/workflows/caller.yml@refs/heads/trusted","workflow_sha":"1111111111111111111111111111111111111111","job_workflow_ref":"fixture/trusted/.github/workflows/proof.yml@1111111111111111111111111111111111111111","job_workflow_sha":"1111111111111111111111111111111111111111","event_name":"workflow_dispatch"});
    let exact = json!({"workflow_id":3,"workflow_revision":"1111111111111111111111111111111111111111","source_input":"source_sha","operation_input":"operation_id","observed_source_output":"source_sha","proof":proof});
    serde_json::from_value(json!({"schema_version":1,"runtime":{"id":id(1),"host_id":"fixture","max_agents":4},"connections":{
        "linear":{"kind":"linear","account_id":id(5),"endpoint":"https://linear.invalid/graphql","credential_ref":"keychain:linear"},
        "github":{"kind":"github","account_id":"7","endpoint":"https://github.invalid/","credential_ref":"keychain:github"},
        "codex":{"kind":"codex","account_id":"codex-fixture","auth_home":"/private/tmp/fixture-auth","max_agents":2}
    },"workspaces":{"example":{"id":id(2),"linear_workspace_id":id(3),"linear_connection":"linear","github_connection":"github","codex_connection":"codex","workflow":"/private/tmp/WORKFLOW.md",
        "linear":{"teams":"all","projects":"all","users":"all","authorization_label_id":id(4),"states":{id(10).to_string():{"prepared":id(11),"running":id(12),"blocked":id(13),"child_done":id(14),"parent_accepted":id(15),"cancelled":id(16)}}},
        "github":{"organizations":[],"repositories":["42"],"pr_authors":["7"]},"actors":{"authorizers":[id(5)],"correctors":[id(5)],"acceptors":[id(5)]},"profiles":{"coordination":profile,"independent_review":profile},
        "routing_labels":{"tool":{id(20).to_string():"codex"},"model":{id(21).to_string():"fixture-model"},"reasoning":{id(22).to_string():"high"},"speed":{id(23).to_string():"standard"}},"max_agents":2,"release":{"mode":"manual","spacing_seconds":0},
        "repositories":{"42":{"source":"/private/tmp/source","local_checks":["local"],"integrated_checks":["e2e"],"ci":[{"workflow_id":1,"required_checks":["suite"],"proof":proof}],"production_ci":[{"workflow_id":2,"required_checks":["suite"],"proof":proof}],"no_change_validation":exact,"deployment":"none"}}
    }},"control":{"local_users":{"501":{"id":"operator","linear_user_id":id(5),"roles":["inspect","configure","lifecycle","correct","accept","release","recover"]}}}})).unwrap()
}
struct ReadConfiguration(Mutex<Config>);
impl ConfigurationControl for ReadConfiguration {
    fn authority_fences(&self) -> Result<crate::configuration::AuthorityFences> {
        Ok(Default::default())
    }
    fn snapshot(&self) -> Result<Config> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn execute(
        &self,
        _: Actor,
        _: CommandRequest,
        _: Option<kegari_secrets::Secret>,
        _: Receipt,
    ) -> HostFuture<'_, Receipt> {
        Box::pin(async { Err(Error::new("fixture", "unexpected configuration mutation")) })
    }
    fn check(&self, _: Option<Uuid>) -> HostFuture<'_, Value> {
        Box::pin(async { Ok(json!({"fixture":"metadata_only"})) })
    }
}
fn git_run(binary: &Path, root: &Path, args: &[&str]) {
    let output = kegari_process::group::OwnedGroup::spawn_std(
        Command::new(binary)
            .current_dir(root)
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
fn pinned_git(root: &Path) -> PathBuf {
    let source = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|directory| directory.join("git"))
        .find(|path| path.is_file())
        .expect("fixture Git is required")
        .canonicalize()
        .unwrap();
    let directory = root.canonicalize().unwrap().join("fixture-pinned-git");
    fs::create_dir_all(&directory).unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    let target = directory.join("git");
    let expected = fs::read(&source).unwrap();
    if target.exists() {
        assert_eq!(
            fs::read(&target).unwrap(),
            expected,
            "fixture tool bytes changed"
        );
    } else {
        use std::io::Write;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .unwrap();
        output.write_all(&expected).unwrap();
        output.sync_all().unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o555)).unwrap();
    }
    let identity = kegari_platform::BinaryIdentity::inspect(&target, false).unwrap();
    assert_eq!(identity.digest, kegari_digest::bytes_digest(&expected));
    target
}

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
    probe: Arc<dyn ProcessProbe>,
}
impl ReuseFixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let binary = pinned_git(&root);
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
                probe.clone(),
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
            probe,
        }
    }
    fn verify(&self) -> Result<crate::git::VerifiedReusableSource> {
        self.git
            .verify_reusable_execution(&self.ownership, &self.binding, &self.boundary)
    }
}

struct FamilyFixture {
    source: ReuseFixture,
    housekeeping: Housekeeping,
    workflow: Workflow,
    lease: Lease,
    reference: RunReference,
    predecessor: Receipt,
    reuse: crate::housekeeping::VerifiedDirectoryReuse,
    source_proof: crate::git::VerifiedReusableSource,
}
impl FamilyFixture {
    fn new() -> Self {
        let source = ReuseFixture::new();
        let workflow_path = source._temp.path().join("WORKFLOW.md");
        fs::write(
            &workflow_path,
            "---\nschema_version: 1\n---\nFixture execution workflow.\n",
        )
        .unwrap();
        let workflow = Workflow::load(&workflow_path).unwrap();
        let housekeeping = Housekeeping::new(
            PrivateDir::open(source.services.registration.root().path()).unwrap(),
            source.services.control.clone(),
            source.probe.clone(),
            source.unscoped.binary.path.clone(),
        )
        .unwrap();
        let proof = source.verify().unwrap();
        source
            .authority
            .finish(json!({"fixture":"source verified"}))
            .unwrap();
        let control = &source.services.control;
        control
            .lifecycle(
                MutationRequest {
                    request_id: Uuid::new_v4(),
                    runtime_id: id(1),
                    actor: Actor {
                        identity: "fixture".into(),
                        roles: ["lifecycle".into()].into(),
                        principal: None,
                    },
                    kind: "lifecycle".into(),
                    canonical_targets: vec![id(1).to_string()],
                    expected: BTreeMap::new(),
                    payload: Value::Null,
                },
                Lifecycle::Start,
                0,
            )
            .unwrap();
        let lease = Lease {
            runtime_id: id(1),
            owner_generation: 1,
            owner_id: Uuid::new_v4(),
        };
        control.attach_reconciled(lease.clone()).unwrap();
        let work = source.authority.identity().work_key.clone();
        let reference = RunReference {
            workspace: id(2),
            account: "fixture-account".into(),
            normalized_state: None,
            phase: kegari_types::run::PhaseKey {
                work: work.clone(),
                generation: canonical_digest(1, &source.binding.work.generation).unwrap(),
                phase: "implementation".into(),
                retry_grant: None,
            },
            profile_digest: bytes_digest(b"fixture profile"),
            instruction_digest: bytes_digest(b"fixture instructions"),
            max_attempts: 4,
            max_turns: 8,
            max_review_rounds: 3,
            attempt: 1,
        };
        control
            .admit_run(
                &lease,
                RunAdmission {
                    request_id: source.binding.operation_id,
                    work_key: work,
                    scope_digest: bytes_digest(b"fixture scope"),
                    reference: reference.clone(),
                },
                || Ok(()),
            )
            .unwrap();
        housekeeping
            .register_execution(
                &source.binding,
                &source.boundary,
                &source.ownership,
                &workflow,
            )
            .unwrap();
        let mut creation = HookLedger::new(&source.binding).unwrap();
        creation.initialized = true;
        creation.records.insert(
            "AfterCreate:directory".into(),
            HookRecord {
                phase: HookPhase::AfterCreate,
                session: None,
                workflow_digest: workflow.digest.clone(),
                script_digest: None,
                cleanup_plan: None,
                state: HookState::Skipped,
                exit_code: None,
                timed_out: false,
                stdout: String::new(),
                stderr: String::new(),
                error_code: None,
                evidence_class: ExecutionEvidenceClass::DeterministicFixture,
            },
        );
        housekeeping
            .record_hook_ledger(source.binding.operation_id, &creation)
            .unwrap();
        // Actual Control transitions with deliberately synthetic terminal facts.
        // This is not a native completion or provider recovery capability.
        let predecessor = control
            .retire_local(source.binding.operation_id, failed_observation())
            .unwrap();
        let reuse = housekeeping
            .prepare_directory_reuse(&proof, &source.boundary, &source.ownership, &workflow)
            .unwrap();
        Self {
            source,
            housekeeping,
            workflow,
            lease,
            reference,
            predecessor,
            reuse,
            source_proof: proof,
        }
    }
    fn input(&self, operation: Uuid) -> RunAdmission {
        let mut reference = self.reference.clone();
        reference.attempt = 2;
        RunAdmission {
            request_id: operation,
            work_key: reference.phase.work.clone(),
            scope_digest: bytes_digest(b"fixture resumed scope"),
            reference,
        }
    }
    fn resume(&self) -> RunResumeReference {
        RunResumeReference {
            predecessor: self.predecessor.operation_id,
            predecessor_receipt_digest: canonical_digest(1, &self.predecessor).unwrap(),
            recovery_digest: bytes_digest(b"synthetic component recovery"),
            instruction_compatibility_digest: bytes_digest(b"synthetic assembly compatibility"),
            directory_reuse_digest: self.reuse.digest().into(),
            review_rounds_consumed: 0,
        }
    }
    fn binding(&self, operation: Uuid) -> ExecutionBinding {
        let mut binding = self.source.binding.clone();
        binding.operation_id = operation;
        binding
    }
}
fn failed_observation() -> Value {
    json!({"outcome":"failed","local_exit_confirmed":true,"hook_error":null,"session":{"terminal":"failed","input_required":false,"turn_id":null,"intervention":null,"unresolved_turn":null}})
}
fn spacing() {
    std::thread::sleep(std::time::Duration::from_millis(1050));
}

#[test]
fn exact_typed_resume_enrolls_one_family_and_preserves_original_creation() {
    let f = FamilyFixture::new();
    spacing();
    let original_marker = fs::read(f.source.ownership.marker()).unwrap();
    let cleanup = f
        .source
        .services
        .registration
        .root()
        .child("cleanup")
        .unwrap();
    let original_ledger: HookLedger = cleanup
        .read(&format!("hooks-{}.json", f.predecessor.operation_id))
        .unwrap();
    let operation = Uuid::new_v4();
    let permit = f
        .source
        .services
        .control
        .admit_run_resumed_observed(&f.lease, f.input(operation), f.resume(), |_| {
            f.reuse.revalidate()
        })
        .unwrap();
    let binding = f.binding(operation);
    let enrolled = f
        .housekeeping
        .register_reused_execution(
            &permit,
            &f.reuse,
            &binding,
            &f.source.boundary,
            &f.source.ownership,
            &f.workflow,
        )
        .unwrap();
    assert_eq!(enrolled.binding(), &binding);
    enrolled.revalidate().unwrap();
    assert_eq!(
        fs::read(f.source.ownership.marker()).unwrap(),
        original_marker
    );
    let repeated = f
        .housekeeping
        .register_reused_execution(
            &permit,
            &f.reuse,
            &binding,
            &f.source.boundary,
            &f.source.ownership,
            &f.workflow,
        )
        .unwrap();
    assert_eq!(repeated.digest(), enrolled.digest());
    let inherited = HookLedger::for_reused_execution(&enrolled, &f.workflow).unwrap();
    assert!(inherited.initialized && inherited.inherited_creation.is_some());
    assert!(inherited.records.is_empty() && inherited.sessions.is_empty());
    assert_eq!(
        inherited.binding_digest,
        canonical_digest(1, &binding).unwrap()
    );
    f.housekeeping
        .record_hook_ledger(operation, &inherited)
        .unwrap();
    assert!(
        f.source
            .services
            .control
            .admit_run_resumed_observed(&f.lease, f.input(Uuid::new_v4()), f.resume(), |_| Ok(()))
            .is_err()
    );
    let root = f
        .source
        .services
        .registration
        .root()
        .child("cleanup")
        .unwrap();
    let family: Value = root
        .read(&format!(
            "family-{}.json",
            f.source.ownership.identity().directory_id
        ))
        .unwrap();
    assert_eq!(family["members"].as_object().unwrap().len(), 2);
    assert!(
        family["members"]
            .get(f.predecessor.operation_id.to_string())
            .is_some()
    );
    assert!(family["members"].get(operation.to_string()).is_some());
    let retained_ledger: HookLedger = cleanup
        .read(&format!("hooks-{}.json", f.predecessor.operation_id))
        .unwrap();
    assert_eq!(
        canonical_digest(1, &original_ledger).unwrap(),
        canonical_digest(1, &retained_ledger).unwrap()
    );
    for stale in [false, true] {
        let mut invalid = original_ledger.clone();
        let creation = invalid.records.get_mut("AfterCreate:directory").unwrap();
        if stale {
            creation.workflow_digest = bytes_digest(b"different workflow");
        } else {
            creation.state = HookState::StartedUnknown;
        }
        f.housekeeping
            .record_hook_ledger(f.predecessor.operation_id, &invalid)
            .unwrap();
        assert_eq!(
            HookLedger::for_reused_execution(&enrolled, &f.workflow)
                .unwrap_err()
                .code,
            "hook_gate"
        );
    }
    f.housekeeping
        .record_hook_ledger(f.predecessor.operation_id, &original_ledger)
        .unwrap();
    fs::write(f.source.source.join("file.txt"), b"modified bytes\n").unwrap();
    assert!(enrolled.revalidate().is_err());
    assert!(
        f.housekeeping
            .register_reused_execution(
                &permit,
                &f.reuse,
                &binding,
                &f.source.boundary,
                &f.source.ownership,
                &f.workflow
            )
            .is_err()
    );
    // A new registration must not hide the live successor from the old proof.
    assert!(f.reuse.revalidate().is_err());
}

#[test]
fn ordinary_admission_and_changed_files_cannot_enroll_a_directory() {
    for ordinary in [true, false] {
        let f = FamilyFixture::new();
        spacing();
        let operation = Uuid::new_v4();
        let control = &f.source.services.control;
        let permit = if ordinary {
            control
                .admit_run(&f.lease, f.input(operation), || Ok(()))
                .unwrap()
        } else {
            control
                .admit_run_resumed_observed(&f.lease, f.input(operation), f.resume(), |_| {
                    f.reuse.revalidate()
                })
                .unwrap()
        };
        if !ordinary {
            fs::write(f.source.source.join("file.txt"), b"modified bytes\n").unwrap();
        }
        assert!(
            f.housekeeping
                .register_reused_execution(
                    &permit,
                    &f.reuse,
                    &f.binding(operation),
                    &f.source.boundary,
                    &f.source.ownership,
                    &f.workflow
                )
                .is_err()
        );
        let root = f
            .source
            .services
            .registration
            .root()
            .child("cleanup")
            .unwrap();
        assert_eq!(
            root.read::<Value>(&format!("{operation}.json"))
                .unwrap_err()
                .code,
            "record_missing"
        );
        let family: Value = root
            .read(&format!(
                "family-{}.json",
                f.source.ownership.identity().directory_id
            ))
            .unwrap();
        assert_eq!(family["members"].as_object().unwrap().len(), 1);
        assert!(f.source.source.exists());
    }
}

struct PublishedFixture {
    commit: String,
}
impl kegari_providers::HttpTransport for PublishedFixture {
    fn send<'a>(
        &'a self,
        request: kegari_providers::HttpRequest,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<kegari_providers::HttpResponse>> + Send + 'a>,
    > {
        Box::pin(async move {
            assert_eq!(request.method, kegari_providers::Method::Get);
            let path = request.url.path().to_owned();
            let body = match path.as_str() {
                "/user" => json!({"id":7,"login":"fixture","type":"User"}),
                "/repositories/42" | "/repos/fixture/repo" => {
                    json!({"id":42,"name":"repo","owner":{"id":7,"login":"fixture","type":"User"},"default_branch":"main","private":true,"archived":false,"disabled":false,"allow_squash_merge":true,"permissions":{"pull":true,"push":true}})
                }
                "/repos/fixture/repo/git/ref/heads/main" => {
                    json!({"ref":"refs/heads/main","object":{"type":"commit","sha":self.commit}})
                }
                _ => panic!("unexpected fixture read {path}"),
            };
            Ok(kegari_providers::HttpResponse {
                status: 200,
                headers: Default::default(),
                body: serde_json::to_vec(&body).unwrap(),
            })
        })
    }
}
fn copy_retained_tree(source: &Path, destination: &Path) {
    fs::create_dir(destination).unwrap();
    fs::set_permissions(destination, fs::metadata(source).unwrap().permissions()).unwrap();
    for item in fs::read_dir(source).unwrap() {
        let item = item.unwrap();
        let target = destination.join(item.file_name());
        if item.file_type().unwrap().is_dir() {
            copy_retained_tree(&item.path(), &target)
        } else {
            fs::copy(item.path(), target).unwrap();
        }
    }
}
#[tokio::test]
async fn enrolled_family_cleanup_preserves_copy_and_tombstones_both_operations() {
    let mut f = FamilyFixture::new();
    spacing();
    f.housekeeping
        .install_processes(Arc::new(
            crate::housekeeping::NativeCleanupProcesses::for_housekeeping(
                f.source.services.clone(),
                &f.housekeeping,
            )
            .unwrap(),
        ))
        .unwrap();
    let retained = f
        .source
        ._temp
        .path()
        .canonicalize()
        .unwrap()
        .join("retained");
    copy_retained_tree(&f.source.source, &retained);
    let repository = RepositoryIdentity {
        id: 42,
        owner: "fixture".into(),
        name: "repo".into(),
        owner_id: 7,
        organization_id: None,
    };
    f.housekeeping
        .retain(
            f.predecessor.operation_id,
            crate::housekeeping::RetentionLocation {
                directory: retained.clone(),
                repository,
                published_refs: vec![crate::housekeeping::PublishedReference {
                    reference: "heads/main".into(),
                    commit: f.source.binding.revision.commit.clone(),
                }],
            },
        )
        .unwrap();
    let client = kegari_providers::GithubClient::with_transport(
        kegari_providers::HttpPolicy::new(
            "https://github.invalid/",
            "7".into(),
            std::time::Duration::from_secs(2),
        )
        .unwrap(),
        kegari_config::GithubSelection {
            organizations: vec![],
            repositories: vec!["42".into()],
            pr_authors: vec![],
        },
        kegari_providers::Credential::new("fixture".into()).unwrap(),
        Some(Arc::new(PublishedFixture {
            commit: f.source.binding.revision.commit.clone(),
        })),
    )
    .unwrap();
    let old_plan = f
        .housekeeping
        .inspect(f.predecessor.operation_id, &client)
        .await
        .unwrap();
    f.reuse = f
        .housekeeping
        .prepare_directory_reuse(
            &f.source_proof,
            &f.source.boundary,
            &f.source.ownership,
            &f.workflow,
        )
        .unwrap();
    let operation = Uuid::new_v4();
    let permit = f
        .source
        .services
        .control
        .admit_run_resumed_observed(&f.lease, f.input(operation), f.resume(), |_| {
            f.reuse.revalidate()
        })
        .unwrap();
    let enrolled = f
        .housekeeping
        .register_reused_execution(
            &permit,
            &f.reuse,
            &f.binding(operation),
            &f.source.boundary,
            &f.source.ownership,
            &f.workflow,
        )
        .unwrap();
    let ledger = HookLedger::for_reused_execution(&enrolled, &f.workflow).unwrap();
    f.housekeeping
        .record_hook_ledger(operation, &ledger)
        .unwrap();
    let house = Arc::new(f.housekeeping.clone());
    let denied_id = Uuid::new_v4();
    let denied = cleanup_request(denied_id, f.predecessor.operation_id);
    assert!(
        house
            .clone()
            .apply(
                denied.clone(),
                f.predecessor.operation_id,
                old_plan.token.clone(),
                client.clone(),
                Arc::new(|actor| actor.require("recover"))
            )
            .await
            .is_err()
    );
    assert_eq!(
        f.source
            .services
            .control
            .operation(denied_id)
            .unwrap_err()
            .code,
        "record_missing"
    );
    f.source
        .services
        .control
        .retire_local(operation, failed_observation())
        .unwrap();
    assert_eq!(
        house
            .clone()
            .apply(
                denied,
                f.predecessor.operation_id,
                old_plan.token.clone(),
                client.clone(),
                Arc::new(|actor| actor.require("recover"))
            )
            .await
            .unwrap_err()
            .code,
        "stale_cleanup_plan"
    );
    assert_eq!(
        f.source
            .services
            .control
            .operation(denied_id)
            .unwrap_err()
            .code,
        "record_missing"
    );
    let plan = f
        .housekeeping
        .inspect(f.predecessor.operation_id, &client)
        .await
        .unwrap();
    let cleanup_id = Uuid::new_v4();
    let request = cleanup_request(cleanup_id, f.predecessor.operation_id);
    let result = house
        .apply(
            request,
            f.predecessor.operation_id,
            plan.token,
            client,
            Arc::new(|actor| actor.require("recover")),
        )
        .await
        .unwrap();
    assert_eq!(result.phase, crate::control::ReceiptPhase::Acknowledged);
    assert!(!f.source.source.exists());
    assert_eq!(
        fs::read(retained.join("file.txt")).unwrap(),
        b"original bytes\n"
    );
    let cleanup = f
        .source
        .services
        .registration
        .root()
        .child("cleanup")
        .unwrap();
    for member in [f.predecessor.operation_id, operation] {
        let tombstone: Value = cleanup.read(&format!("retained-{member}.json")).unwrap();
        assert_eq!(tombstone["removed"], true);
        assert_eq!(tombstone["cleanup_request"], cleanup_id.to_string());
        assert_eq!(tombstone["retained"], retained.to_string_lossy().as_ref());
    }
}

fn cleanup_request(request_id: Uuid, operation: Uuid) -> MutationRequest {
    MutationRequest {
        request_id,
        runtime_id: id(1),
        actor: Actor {
            identity: "fixture-human".into(),
            roles: ["recover".into()].into(),
            principal: Some(kegari_control_api::identity::Principal::Unix {
                uid: unsafe { libc::geteuid() },
            }),
        },
        kind: "cleanup".into(),
        canonical_targets: vec![],
        expected: BTreeMap::new(),
        payload: json!({"operation":operation}),
    }
}

fn startup_fixture() -> (
    FamilyFixture,
    crate::control::Permit,
    crate::worker_run::ResumeStartupSource,
) {
    let f = FamilyFixture::new();
    spacing();
    let operation = Uuid::new_v4();
    let permit = f
        .source
        .services
        .control
        .admit_run_resumed_observed(&f.lease, f.input(operation), f.resume(), |_| {
            f.reuse.revalidate()
        })
        .unwrap();
    let enrolled = f
        .housekeeping
        .register_reused_execution(
            &permit,
            &f.reuse,
            &f.binding(operation),
            &f.source.boundary,
            &f.source.ownership,
            &f.workflow,
        )
        .unwrap();
    (
        f,
        permit,
        crate::worker_run::ResumeStartupSource::new(enrolled),
    )
}
fn protocol_request(permit: &crate::control::Permit, method: &str) -> MutationRequest {
    MutationRequest {
        request_id: Uuid::new_v4(),
        runtime_id: permit.lease.runtime_id,
        actor: Actor {
            identity: format!("owner:{}", permit.lease.owner_id),
            principal: None,
            roles: Default::default(),
        },
        kind: format!("codex/{method}"),
        canonical_targets: vec![format!("codex-session:{}", permit.operation_id)],
        expected: BTreeMap::new(),
        payload: json!({"jsonrpc":"2.0","id":1,"method":method,"params":{"threadId":"synthetic-component-thread"}}),
    }
}
fn send_component_frame(
    f: &FamilyFixture,
    permit: &crate::control::Permit,
    source: &crate::worker_run::ResumeStartupSource,
    method: &str,
    frames: &mut usize,
) {
    // Observe only the shared Control/source dispatch component. This closure
    // is a deterministic frame sink, not an app-server or joined worker.
    let effect = crate::worker_run::dispatch_protocol_effect(
        &f.source.services.control,
        permit,
        protocol_request(permit, method),
        Some(source),
        || Ok(()),
        &mut || {
            *frames += 1;
            Ok(())
        },
    )
    .unwrap();
    f.source
        .services
        .control
        .complete_effect(
            effect,
            permit,
            || Ok(()),
            json!({"component_fixture":"frame recorded"}),
        )
        .unwrap();
}
fn reject_changed_startup(
    f: &FamilyFixture,
    permit: &crate::control::Permit,
    source: &crate::worker_run::ResumeStartupSource,
    method: &str,
    frames: &mut usize,
) {
    let request = protocol_request(permit, method);
    let request_id = request.request_id;
    let prior_frames = *frames;
    let error = crate::worker_run::dispatch_protocol_effect(
        &f.source.services.control,
        permit,
        request,
        Some(source),
        || Ok(()),
        &mut || {
            *frames += 1;
            Ok(())
        },
    )
    .err()
    .unwrap();
    assert_eq!(error.code, "source_reuse_blocked");
    assert_eq!(*frames, prior_frames);
    // Control retains a known-not-started observation, never a Sent receipt or
    // unresolved provider effect. The original admission/marker remain intact.
    let recorded = f.source.services.control.operation(request_id).unwrap();
    assert_eq!(recorded.phase, crate::control::ReceiptPhase::Reconciled);
    assert_eq!(recorded.observed_result.unwrap()["effect"], "not_started");
    assert!(recorded.remote_ids.is_empty());
    assert!(
        !f.source
            .services
            .control
            .snapshot()
            .unwrap()
            .unresolved_operations
            .contains_key(&request_id)
    );
    assert!(f.source.ownership.marker().exists());
}
#[test]
fn resumed_source_edit_before_resume_cannot_transmit_a_frame() {
    let (f, permit, source) = startup_fixture();
    fs::write(f.source.source.join("file.txt"), b"modified bytes\n").unwrap();
    let mut frames = 0;
    reject_changed_startup(&f, &permit, &source, "thread/resume", &mut frames);
}
#[test]
fn resumed_source_edit_after_resume_cannot_start_first_turn() {
    let (f, permit, source) = startup_fixture();
    let mut frames = 0;
    send_component_frame(&f, &permit, &source, "thread/resume", &mut frames);
    assert_eq!(frames, 1);
    fs::write(f.source.source.join("file.txt"), b"modified bytes\n").unwrap();
    reject_changed_startup(&f, &permit, &source, "turn/start", &mut frames);
}
#[test]
fn transmitted_first_turn_releases_only_the_startup_source_constraint() {
    let (f, permit, source) = startup_fixture();
    let mut frames = 0;
    send_component_frame(&f, &permit, &source, "thread/resume", &mut frames);
    send_component_frame(&f, &permit, &source, "turn/start", &mut frames);
    fs::write(f.source.source.join("file.txt"), b"modified bytes\n").unwrap();
    send_component_frame(&f, &permit, &source, "turn/start", &mut frames);
    assert_eq!(frames, 3);
    let request = protocol_request(&permit, "turn/start");
    f.source
        .services
        .control
        .fence(permit.operation_id)
        .unwrap();
    assert!(
        crate::worker_run::dispatch_protocol_effect(
            &f.source.services.control,
            &permit,
            request,
            Some(&source),
            || Ok(()),
            &mut || {
                frames += 1;
                Ok(())
            },
        )
        .is_err()
    );
    assert_eq!(
        frames, 3,
        "source release must not bypass the actual permit gate"
    );
}

#[test]
fn retained_execution_metadata_uses_protected_family_without_repointing_configured_source() {
    let f = FamilyFixture::new();
    let mut config = f.source.services.configuration.snapshot().unwrap();
    let workspace = config.workspaces.get_mut("example").unwrap();
    workspace.repositories.get_mut("42").unwrap().source = f
        .source
        .ownership
        .identity()
        .operator_checkout
        .to_string_lossy()
        .into_owned();
    workspace.workflow = Some(
        f.workflow
            .path
            .as_ref()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
    );
    let read_root = || {
        f.housekeeping.retained_execution_read_root(
            &config,
            &f.source.binding,
            &f.source.boundary,
            &f.source.ownership,
            &f.workflow,
        )
    };
    let root = read_root().unwrap();
    assert_eq!(root.root(), f.source.source);
    assert_ne!(
        Path::new(&config.workspaces["example"].repositories["42"].source),
        root.root()
    );
    let mut services = f.source.services.clone();
    services.configuration = Arc::new(ReadConfiguration(Mutex::new(config.clone())));
    let authority = LocalProcessPermit::metadata(
        services,
        MetadataRequest {
            request_id: Uuid::new_v4(),
            work_key: f.reference.phase.work.clone(),
            roots: vec![root.clone()],
            git_binary: BinaryIdentity::inspect(f.source.unscoped.executable(), true).unwrap(),
        },
    )
    .unwrap();
    let git = f
        .source
        .unscoped
        .with_host_execution_scope(HostGitExecutionScope {
            limits: GitLimits::default(),
            authority: authority.clone(),
        })
        .unwrap();
    let actual_source = git
        .verify_reusable_execution(&f.source.ownership, &f.source.binding, &f.source.boundary)
        .unwrap();
    assert_eq!(actual_source.binding(), &f.source.binding);
    authority
        .finish(json!({"fixture":"actual retained metadata complete"}))
        .unwrap();
    actual_source.revalidate_filesystem().unwrap();

    // An operator-checkout replacement or raw binding change does not acquire
    // authority to read some other directory through this retained registration.
    let mut wrong_source = config.clone();
    wrong_source
        .workspaces
        .get_mut("example")
        .unwrap()
        .repositories
        .get_mut("42")
        .unwrap()
        .source = f.source.source.to_string_lossy().into_owned();
    assert_eq!(
        f.housekeeping
            .retained_execution_read_root(
                &wrong_source,
                &f.source.binding,
                &f.source.boundary,
                &f.source.ownership,
                &f.workflow,
            )
            .unwrap_err()
            .code,
        "resume_incompatible"
    );
    let mut wrong_binding = f.source.binding.clone();
    wrong_binding.operation_id = Uuid::new_v4();
    assert!(
        f.housekeeping
            .retained_execution_read_root(
                &config,
                &wrong_binding,
                &f.source.boundary,
                &f.source.ownership,
                &f.workflow,
            )
            .is_err()
    );

    let operation = Uuid::new_v4();
    f.source
        .services
        .control
        .admit_run(&f.lease, f.input(operation), || Ok(()))
        .unwrap();
    assert_eq!(read_root().unwrap_err().code, "resume_incompatible");
    f.source
        .services
        .control
        .retire_local(operation, failed_observation())
        .unwrap();
    read_root().unwrap();

    // A previously issued read root still fences changed protected ownership
    // bytes at each actual metadata process start.
    let marker = f.source.ownership.marker();
    let bytes = fs::read(marker).unwrap();
    fs::write(marker, b"changed ownership").unwrap();
    assert_eq!(
        root.revalidate_filesystem().unwrap_err().code,
        "execution_ownership"
    );
    assert!(read_root().is_err());
    fs::write(marker, bytes).unwrap();
}
