//! Owned local fixtures; provider responses/process ownership are explicitly
//! synthetic. These tests never access native Keychain or launchd.
#[path = "support/pinned_git.rs"]
mod pinned_git;
#[path = "support/process_probe.rs"]
mod process_probe;
use kegari_types::error::{Error, Result};
use kegari_v0::{
    control::*,
    digest::bytes_digest,
    evidence::{Generation, WorkIdentity},
    execution::{
        DirectoryIdentity, ExecutionBinding, ExecutionEvidenceClass, HookLedger, HookPhase,
        HookRecord, HookState, OwnedDirectory, SessionLifecycle,
    },
    git::{Git, RepositoryId},
    housekeeping::*,
    isolation::{ExecutionRole, IsolationPlan},
    platform::{ProcessIdentity, ProcessObservation, ProcessProbe},
    providers::*,
    storage::PrivateDir,
    workflow::Workflow,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    future::Future,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    pin::Pin,
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
};
use uuid::Uuid;
fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
fn git_path() -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|p| p.join("git"))
        .find(|p| p.is_file())
        .unwrap()
        .canonicalize()
        .unwrap()
}
fn git(path: &Path, args: &[&str]) -> String {
    let out = kegari_v0::process_group::OwnedGroup::spawn_std(
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
                "commit.gpgsign=false",
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
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().into()
}
fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir(to).unwrap();
    std::fs::set_permissions(to, std::fs::metadata(from).unwrap().permissions()).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dst = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &dst);
        } else {
            std::fs::copy(entry.path(), dst).unwrap();
        }
    }
}
struct Probe {
    live: AtomicBool,
}
impl ProcessProbe for Probe {
    fn current(&self) -> Result<ProcessIdentity> {
        Ok(ProcessIdentity {
            pid: std::process::id(),
            parent_pid: 1,
            process_group: 1,
            uid: unsafe { libc::geteuid() },
            boot_id: id(1),
            started_seconds: 1,
            started_microseconds: 1,
            executable: std::env::current_exe().unwrap(),
        })
    }
    fn observe(&self, _: &ProcessIdentity) -> Result<ProcessObservation> {
        Ok(if self.live.load(Ordering::SeqCst) {
            ProcessObservation::Running
        } else {
            ProcessObservation::Gone
        })
    }
    fn matching_executable(&self, _: &Path) -> Result<Vec<ProcessIdentity>> {
        Ok(if self.live.load(Ordering::SeqCst) {
            vec![self.current()?]
        } else {
            vec![]
        })
    }
}
struct Wire {
    commit: String,
    changed: AtomicBool,
    reads: AtomicUsize,
    pause_at: AtomicUsize,
    paused: tokio::sync::Notify,
    resume: tokio::sync::Notify,
}
impl HttpTransport for Wire {
    fn send(
        &self,
        request: HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Result<HttpResponse>> + Send + '_>> {
        Box::pin(async move {
            assert!(matches!(request.method, Method::Get));
            let value = match request.url.path() {
                "/user" => json!({"id":7,"login":"fixture","type":"User"}),
                "/repos/fixture/repo" => {
                    json!({"id":101,"name":"repo","owner":{"id":7,"login":"fixture","type":"User"},"default_branch":"main","private":true,"archived":false,"disabled":false,"allow_squash_merge":true,"permissions":{"pull":true,"push":true}})
                }
                "/repos/fixture/repo/git/ref/heads/main" => {
                    let count = self.reads.fetch_add(1, Ordering::SeqCst) + 1;
                    if self.pause_at.load(Ordering::SeqCst) == count {
                        self.paused.notify_one();
                        self.resume.notified().await;
                    }
                    json!({"ref":"refs/heads/main","object":{"type":"commit","sha":if self.changed.load(Ordering::SeqCst){"1111111111111111111111111111111111111111"}else{&self.commit}}})
                }
                other => panic!("unexpected fixture read {other}"),
            };
            Ok(HttpResponse {
                status: 200,
                headers: Default::default(),
                body: serde_json::to_vec(&value).unwrap(),
            })
        })
    }
}
struct NativeReadConfiguration(kegari_config::Config);
impl kegari_v0::app::ConfigurationControl for NativeReadConfiguration {
    fn authority_fences(&self) -> Result<kegari_v0::configuration::AuthorityFences> {
        Ok(Default::default())
    }
    fn snapshot(&self) -> Result<kegari_config::Config> {
        Ok(self.0.clone())
    }
    fn execute(
        &self,
        _: Actor,
        _: kegari_types::api::CommandRequest,
        _: Option<kegari_v0::secrets::Secret>,
        _: Receipt,
    ) -> kegari_v0::app::HostFuture<'_, Receipt> {
        Box::pin(async { Err(Error::new("fixture", "configuration mutation unexpected")) })
    }
    fn check(&self, _: Option<Uuid>) -> kegari_v0::app::HostFuture<'_, Value> {
        Box::pin(async { Ok(json!({})) })
    }
}
struct FixtureCleanupProcesses {
    git: Git,
}
impl CleanupProcesses for FixtureCleanupProcesses {
    fn read<'a>(
        &'a self,
        root: VerifiedCleanupReadRoot,
        query: kegari_v0::git::CleanupRead,
        cleanup: Option<Arc<VerifiedCleanupExecution>>,
    ) -> CleanupFuture<'a, String> {
        Box::pin(async move {
            root.revalidate_filesystem()?;
            if let Some(proof) = cleanup {
                assert_eq!(proof.plan().operation_id, root.operation_id());
            }
            self.git.cleanup_read(root.root(), query)
        })
    }
    fn before_remove<'a>(&'a self, _: CleanupHook<'a>) -> CleanupFuture<'a, ()> {
        Box::pin(async {
            Err(Error::new(
                "fixture",
                "native hooks are not exercised by the retention fixture",
            ))
        })
    }
}
struct Fixture {
    _tmp: tempfile::TempDir,
    root: PrivateDir,
    source: PathBuf,
    retained: PathBuf,
    git_binary: PathBuf,
    house: Arc<Housekeeping>,
    control: Arc<ControlHost>,
    wire: Arc<Wire>,
    client: GithubClient,
    probe: Arc<Probe>,
    processes: Option<Arc<kegari_v0::process_records::ProcessJournal>>,
}
impl Fixture {
    fn new() -> Self {
        Self::with_hook(false)
    }
    fn with_hook(hook: bool) -> Self {
        Self::build(hook, false)
    }
    fn build(hook: bool, native: bool) -> Self {
        Self::build_with_default(hook, native, false)
    }
    fn build_with_default(hook: bool, native: bool, installed_default: bool) -> Self {
        assert!(!installed_default || !hook);
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().canonicalize().unwrap();
        let pinned_git = pinned_git::prepare(&home);
        let native = if native {
            let probe = Arc::new(process_probe::FixtureBootProbe::new());
            let process = probe.current().unwrap();
            let host = kegari_v0::platform::HostIdentity {
                host_id: "fixture".into(),
                boot_id: process.boot_id,
                uid: process.uid,
                user: "fixture".into(),
                home: home.clone(),
            };
            let (registration, control) = kegari_v0::platform::Registration::bootstrap_with_probe(
                &host,
                id(2),
                kegari_v0::platform::BinaryIdentity::inspect(
                    &std::env::current_exe().unwrap(),
                    false,
                )
                .unwrap(),
                Uuid::new_v4(),
                probe.clone(),
            )
            .unwrap();
            let configuration:kegari_config::Config=serde_json::from_value(json!({"schema_version":1,"runtime":{"id":id(2),"host_id":"fixture","max_agents":1,"execution_root":home.join("execution")},"connections":{},"workspaces":{},"control":{"local_users":{unsafe{libc::geteuid()}.to_string():{"id":"fixture-human","linear_user_id":id(99),"roles":["inspect","recover","configure","lifecycle"]}}}})).unwrap();
            registration.complete_setup(&configuration).unwrap();
            let processes = Arc::new(
                kegari_v0::process_records::ProcessJournal::with_probe(
                    registration.root().child("worker-processes").unwrap(),
                    id(2),
                    probe,
                )
                .unwrap(),
            );
            Some(kegari_v0::host_process::HostProcessServices {
                control: Arc::new(control),
                registration: Arc::new(registration),
                configuration: Arc::new(NativeReadConfiguration(configuration)),
                processes,
                executions: Arc::new(kegari_v0::worker::ExecutionRegistry::default()),
            })
        } else {
            None
        };
        let root = if let Some(services) = &native {
            PrivateDir::open(services.registration.root().path()).unwrap()
        } else {
            PrivateDir::create(&home.join("control")).unwrap()
        };
        let execution = PrivateDir::create(&home.join("execution")).unwrap();
        let source = execution.child("repo").unwrap().path().to_owned();
        let scratch = execution.child("scratch").unwrap().path().to_owned();
        let operator = PrivateDir::create(&home.join("operator")).unwrap();
        git(&source, &["init", "--initial-branch=main"]);
        std::fs::write(source.join("file.txt"), "retained fixture\n").unwrap();
        git(&source, &["add", "file.txt"]);
        git(
            &source,
            &[
                "commit",
                "--signoff",
                "-m",
                "test: create owned retention fixture",
            ],
        );
        let commit = git(&source, &["rev-parse", "HEAD"]);
        let git_api = Git::new(&pinned_git).unwrap();
        let revision = git_api.revision(&source, "HEAD").unwrap();
        let retained = home.join("retained");
        copy_tree(&source, &retained);
        std::fs::set_permissions(&retained, std::fs::Permissions::from_mode(0o700)).unwrap();
        let digest = bytes_digest(b"fixture");
        let work = WorkIdentity {
            linear_endpoint: "https://linear.invalid/graphql".into(),
            linear_workspace: id(3),
            parent: id(4),
            child: Some(id(5)),
            generation: Generation {
                authorization: digest.clone(),
                definition: digest.clone(),
                correction: digest.clone(),
                scope: digest,
            },
        };
        let repository = RepositoryId {
            hostname: "github.invalid".into(),
            id: "101".into(),
        };
        let isolation = IsolationPlan {
            managed_execution: None,
            schema_version: 1,
            role: ExecutionRole::Validation,
            execution_root: execution.path().into(),
            repository: source.clone(),
            cwd: source.clone(),
            scratch,
            read_roots: vec![],
            resource_writes: vec![],
            source_writes: vec![],
            forbidden_roots: vec![root.path().into()],
        }
        .validate()
        .unwrap();
        let owned = OwnedDirectory::register(
            DirectoryIdentity {
                schema_version: 1,
                runtime_id: id(2),
                directory_id: id(6),
                repository: repository.clone(),
                work: work.clone(),
                repository_path: source.clone(),
                operator_checkout: operator.path().into(),
            },
            &isolation,
        )
        .unwrap();
        let binding = ExecutionBinding {
            runtime_id: id(2),
            operation_id: id(7),
            work,
            repository,
            revision,
            candidate_vector_digest: None,
            isolation_digest: isolation.digest().into(),
        };
        let workflow_path = home.join("WORKFLOW.md");
        std::fs::write(
            &workflow_path,
            if hook {
                "---\nhooks:\n  before_remove: 'true'\n---\nFixture context"
            } else {
                "Fixture context"
            },
        )
        .unwrap();
        let workflow = if installed_default {
            std::fs::remove_file(&workflow_path).unwrap();
            Workflow::installed_default().unwrap()
        } else {
            Workflow::load(&workflow_path).unwrap()
        };
        let control = if let Some(services) = &native {
            services.control.clone()
        } else {
            Arc::new(
                ControlHost::setup(PrivateDir::open(root.path()).unwrap(), id(2), id(8)).unwrap(),
            )
        };
        let probe = Arc::new(Probe {
            live: AtomicBool::new(false),
        });
        let house = Arc::new(
            Housekeeping::new(
                PrivateDir::open(root.path()).unwrap(),
                control.clone(),
                probe.clone(),
                pinned_git.clone(),
            )
            .unwrap(),
        );
        let adapter: Arc<dyn CleanupProcesses> = if let Some(services) = &native {
            Arc::new(NativeCleanupProcesses::new(services.clone(), &pinned_git).unwrap())
        } else {
            Arc::new(FixtureCleanupProcesses {
                git: Git::new(&pinned_git).unwrap(),
            })
        };
        house.install_processes(adapter.clone()).unwrap();
        assert_eq!(
            house.install_processes(adapter).unwrap_err().code,
            "authority_already_bound"
        );
        house
            .register_execution(&binding, &isolation, &owned, &workflow)
            .unwrap();
        if hook {
            house
                .record_hook_ledger(
                    id(7),
                    &kegari_v0::execution::HookLedger::new(&binding).unwrap(),
                )
                .unwrap();
        }
        let wire = Arc::new(Wire {
            commit: commit.clone(),
            changed: AtomicBool::new(false),
            reads: AtomicUsize::new(0),
            pause_at: AtomicUsize::new(0),
            paused: tokio::sync::Notify::new(),
            resume: tokio::sync::Notify::new(),
        });
        let client = GithubClient::with_transport(
            HttpPolicy::new(
                "https://github.invalid/",
                "7".into(),
                std::time::Duration::from_secs(2),
            )
            .unwrap(),
            kegari_config::GithubSelection {
                organizations: vec![],
                repositories: vec!["101".into()],
                pr_authors: vec![],
            },
            Credential::new("fixture".into()).unwrap(),
            Some(wire.clone()),
        )
        .unwrap();
        house
            .retain(
                id(7),
                RetentionLocation {
                    directory: retained.clone(),
                    repository: RepositoryIdentity {
                        id: 101,
                        owner: "fixture".into(),
                        name: "repo".into(),
                        owner_id: 7,
                        organization_id: None,
                    },
                    published_refs: vec![PublishedReference {
                        reference: "heads/main".into(),
                        commit,
                    }],
                },
            )
            .unwrap();
        control
            .mutate(
                request(id(7)),
                None,
                Some("recover"),
                || Ok(()),
                |_| Ok((BTreeMap::new(), json!({"finished":"fixture"}))),
            )
            .unwrap();
        Self {
            _tmp: tmp,
            root,
            source,
            retained,
            git_binary: pinned_git,
            house,
            control,
            wire,
            client,
            probe,
            processes: native.map(|services| services.processes),
        }
    }
}
fn actor() -> Actor {
    Actor {
        identity: "fixture-human".into(),
        principal: Some(Principal::Unix {
            uid: unsafe { libc::geteuid() },
        }),
        roles: ["recover".into(), "configure".into()].into(),
    }
}

struct PausedRegistrationProbe {
    original: Arc<Probe>,
    entered: mpsc::Sender<()>,
    resume: Mutex<mpsc::Receiver<()>>,
}
impl ProcessProbe for PausedRegistrationProbe {
    fn current(&self) -> Result<ProcessIdentity> {
        self.original.current()
    }
    fn observe(&self, process: &ProcessIdentity) -> Result<ProcessObservation> {
        self.entered.send(()).unwrap();
        self.resume
            .lock()
            .unwrap()
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        self.original.observe(process)
    }
    fn matching_executable(&self, path: &Path) -> Result<Vec<ProcessIdentity>> {
        self.original.matching_executable(path)
    }
}

#[test]
fn concurrent_retention_and_process_registration_preserve_both_facts() {
    let f = Fixture::new();
    let (entered, observed) = mpsc::channel();
    let (resume, paused) = mpsc::channel();
    let probe = Arc::new(PausedRegistrationProbe {
        original: f.probe.clone(),
        entered,
        resume: Mutex::new(paused),
    });
    let house = Arc::new(
        Housekeeping::new(
            PrivateDir::open(f.root.path()).unwrap(),
            f.control.clone(),
            probe.clone(),
            f.git_binary.clone(),
        )
        .unwrap(),
    );
    let process = probe.current().unwrap();
    f.probe.live.store(true, Ordering::SeqCst);
    let add_process = {
        let house = house.clone();
        let process = process.clone();
        std::thread::spawn(move || house.record_process(id(7), process))
    };
    observed
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    // The process updater has read the old record and is paused at the probe.
    // A concurrent retention write must wait, then preserve its process fact.
    let new_retention = f.retained.with_file_name("retained-new");
    copy_tree(&f.retained, &new_retention);
    let stored: Value = f
        .root
        .child("cleanup")
        .unwrap()
        .read(&format!("{}.json", id(7)))
        .unwrap();
    let mut location: RetentionLocation =
        serde_json::from_value(stored["retention"].clone()).unwrap();
    location.directory = new_retention.clone();
    let (starting, started) = mpsc::channel();
    let (finished, result) = mpsc::channel();
    let retain = std::thread::spawn(move || {
        starting.send(()).unwrap();
        finished.send(house.retain(id(7), location)).unwrap();
    });
    started.recv().unwrap();
    assert!(matches!(
        result.recv_timeout(std::time::Duration::from_millis(100)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    resume.send(()).unwrap();
    add_process.join().unwrap().unwrap();
    result
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap()
        .unwrap();
    retain.join().unwrap();
    let stored: Value = f
        .root
        .child("cleanup")
        .unwrap()
        .read(&format!("{}.json", id(7)))
        .unwrap();
    assert_eq!(
        stored["retention"]["directory"],
        new_retention.to_str().unwrap()
    );
    assert_eq!(stored["processes"], json!([process]));
    assert!(f.source.join("file.txt").exists());
}

fn hook_ledger_fixture(f: &Fixture) -> (PrivateDir, HookLedger, String) {
    let stored: Value = f
        .root
        .child("cleanup")
        .unwrap()
        .read(&format!("{}.json", id(7)))
        .unwrap();
    let binding: ExecutionBinding = serde_json::from_value(stored["binding"].clone()).unwrap();
    let mut ledger = HookLedger::new(&binding).unwrap();
    ledger.initialized = true;
    // Keep a deliberately stale clean copy to prove that it cannot override the
    // canonical worker ledger after its exact location is registered.
    f.house.record_hook_ledger(id(7), &ledger).unwrap();
    let records = f.root.child("worker-hooks").unwrap();
    records.write("hooks.json", &ledger).unwrap();
    f.house
        .register_hook_ledger_location(id(7), &records)
        .unwrap();
    (
        records,
        ledger,
        stored["workflow_digest"].as_str().unwrap().into(),
    )
}

#[tokio::test]
async fn canonical_hook_ledger_tracks_active_unknown_and_completed_transitions() {
    let f = Fixture::new();
    let (records, mut ledger, workflow_digest) = hook_ledger_fixture(&f);
    let initial = f.house.inspect(id(7), &f.client).await.unwrap();
    ledger.sessions.insert(id(77), SessionLifecycle::Launched);
    records.write("hooks.json", &ledger).unwrap();
    assert_eq!(
        f.house.inspect(id(7), &f.client).await.unwrap_err().code,
        "cleanup_blocked"
    );
    ledger.sessions.insert(id(77), SessionLifecycle::Ended);
    ledger.records.insert(
        "fixture-after-run".into(),
        HookRecord {
            phase: HookPhase::AfterRun,
            session: Some(id(77)),
            workflow_digest,
            script_digest: Some(bytes_digest(b"fixture only")),
            cleanup_plan: None,
            state: HookState::StartedUnknown,
            exit_code: None,
            timed_out: false,
            stdout: String::new(),
            stderr: String::new(),
            error_code: None,
            evidence_class: ExecutionEvidenceClass::DeterministicFixture,
        },
    );
    records.write("hooks.json", &ledger).unwrap();
    assert_eq!(
        f.house.inspect(id(7), &f.client).await.unwrap_err().code,
        "cleanup_blocked"
    );
    let record = ledger.records.get_mut("fixture-after-run").unwrap();
    record.state = HookState::Succeeded;
    record.exit_code = Some(0);
    records.write("hooks.json", &ledger).unwrap();
    let completed = f.house.inspect(id(7), &f.client).await.unwrap();
    assert_ne!(initial.token, completed.token);
    assert!(f.source.exists());
}

#[tokio::test]
async fn canonical_hook_location_never_falls_back_to_a_stale_copy() {
    for fault in [
        "missing-reference",
        "missing-file",
        "missing-directory",
        "replaced-directory",
    ] {
        let f = Fixture::new();
        let (records, ledger, _) = hook_ledger_fixture(&f);
        let plan = f.house.inspect(id(7), &f.client).await.unwrap();
        if fault == "missing-reference" {
            std::fs::remove_file(
                f.root
                    .child("cleanup")
                    .unwrap()
                    .path()
                    .join(format!("hook-location-{}.json", id(7))),
            )
            .unwrap();
        } else if fault == "missing-file" {
            std::fs::remove_file(records.path().join("hooks.json")).unwrap();
        } else {
            std::fs::rename(
                records.path(),
                records.path().with_file_name("original-worker-hooks"),
            )
            .unwrap();
            if fault == "replaced-directory" {
                PrivateDir::create(records.path())
                    .unwrap()
                    .write("hooks.json", &ledger)
                    .unwrap();
            }
        }
        assert!(f.house.inspect(id(7), &f.client).await.is_err(), "{fault}");
        assert!(
            f.house
                .clone()
                .apply(
                    request(id(640)),
                    id(7),
                    plan.token.clone(),
                    f.client.clone(),
                    Arc::new(authorize)
                )
                .await
                .is_err(),
            "{fault}"
        );
        assert!(f.source.join("file.txt").exists(), "{fault}");
        assert_eq!(
            f.control.operation(id(640)).unwrap_err().code,
            "record_missing"
        );
        if fault == "missing-reference" {
            let replacement = f.root.child("wrong-worker-hooks").unwrap();
            replacement.write("hooks.json", &ledger).unwrap();
            assert_eq!(
                f.house
                    .register_hook_ledger_location(id(7), &replacement)
                    .unwrap_err()
                    .code,
                "ownership_conflict"
            );
            assert_eq!(
                f.root
                    .child("cleanup")
                    .unwrap()
                    .read::<Value>(&format!("hook-location-{}.json", id(7)))
                    .unwrap_err()
                    .code,
                "record_missing"
            );
            f.house
                .register_hook_ledger_location(id(7), &records)
                .unwrap();
            assert_eq!(
                f.house.inspect(id(7), &f.client).await.unwrap().token,
                plan.token
            );
        }
    }
}

#[tokio::test]
async fn unlisted_family_members_and_foreign_aliases_block_cleanup() {
    for same_family in [true, false] {
        let f = Fixture::new();
        let plan = f.house.inspect(id(7), &f.client).await.unwrap();
        let storage = f.root.child("cleanup").unwrap();
        let mut extra: Value = storage.read(&format!("{}.json", id(7))).unwrap();
        extra["binding"]["operation_id"] = json!(id(99));
        if !same_family {
            extra["family_id"] = json!(id(98));
        }
        // Simulate interrupted enrollment or a corrupted foreign alias index;
        // no new directory authority or genuine family member is fabricated.
        storage.write(&format!("{}.json", id(99)), &extra).unwrap();
        let expected = if same_family {
            "record_corrupt"
        } else {
            "ownership_conflict"
        };
        assert_eq!(
            f.house.inspect(id(7), &f.client).await.unwrap_err().code,
            expected
        );
        assert_eq!(
            f.house
                .clone()
                .apply(
                    request(id(641)),
                    id(7),
                    plan.token,
                    f.client.clone(),
                    Arc::new(authorize)
                )
                .await
                .unwrap_err()
                .code,
            expected
        );
        assert!(f.source.join("file.txt").exists());
        assert_eq!(
            f.control.operation(id(641)).unwrap_err().code,
            "record_missing"
        );
    }
}

#[tokio::test]
async fn legacy_registration_mutation_after_final_read_preserves_source() {
    let f = Fixture::new();
    let storage = f.root.child("cleanup").unwrap();
    let leaf = format!("{}.json", id(7));
    let mut legacy: Value = storage.read(&leaf).unwrap();
    legacy["schema_version"] = json!(1);
    legacy.as_object_mut().unwrap().remove("family_id");
    storage.write(&leaf, &legacy).unwrap();
    std::fs::remove_file(storage.path().join(format!("family-{}.json", id(6)))).unwrap();
    let plan = f.house.inspect(id(7), &f.client).await.unwrap();
    assert!(plan.directory_family_digest.is_none());
    let changed = Arc::new(AtomicBool::new(false));
    let did_change = changed.clone();
    let wire = f.wire.clone();
    let registration_root = storage.path().to_path_buf();
    let current_actor = Arc::new(move |actor: &Actor| {
        authorize(actor)?;
        // The initial plan, apply preflight and final provider observation each
        // read main once. Mutate protected fixture data only after all three,
        // immediately before the final snapshot/unlink predicate.
        if wire.reads.load(Ordering::SeqCst) >= 3 && !did_change.swap(true, Ordering::SeqCst) {
            let records = PrivateDir::open(&registration_root)?;
            let mut current: Value = records.read(&leaf)?;
            current["retention"]["published_refs"][0]["commit"] =
                json!("2222222222222222222222222222222222222222");
            records.write(&leaf, &current)?;
        }
        Ok(())
    });
    let error = f
        .house
        .clone()
        .apply(
            request(id(642)),
            id(7),
            plan.token,
            f.client.clone(),
            current_actor,
        )
        .await
        .unwrap_err();
    assert!(changed.load(Ordering::SeqCst));
    assert_eq!(error.code, "stale_cleanup_plan");
    assert!(f.source.join("file.txt").exists());
    assert!(f.retained.join("file.txt").exists());
    assert_eq!(
        f.control.operation(id(642)).unwrap().phase,
        ReceiptPhase::Sent
    );
}

#[tokio::test]
async fn directory_family_rejects_a_second_unrelated_registration_without_changing_cleanup() {
    let f = Fixture::new();
    let storage = f.root.child("cleanup").unwrap();
    let stored: Value = storage.read(&format!("{}.json", id(7))).unwrap();
    assert_eq!(stored["schema_version"], 2);
    assert_eq!(stored["family_id"], id(6).to_string());
    let mut binding: ExecutionBinding = serde_json::from_value(stored["binding"].clone()).unwrap();
    let isolation = serde_json::from_value::<IsolationPlan>(stored["isolation"].clone())
        .unwrap()
        .validate()
        .unwrap();
    let owned = OwnedDirectory::reopen(
        Path::new(stored["marker"].as_str().unwrap()),
        stored["directory_digest"].as_str().unwrap(),
        &isolation,
    )
    .unwrap();
    let workflow = Workflow::load(Path::new(stored["workflow_path"].as_str().unwrap())).unwrap();
    let before = f.house.inspect(id(7), &f.client).await.unwrap();
    assert!(before.directory_family_digest.is_some());
    f.house
        .register_execution(&binding, &isolation, &owned, &workflow)
        .unwrap();
    binding.operation_id = id(99);
    assert_eq!(
        f.house
            .register_execution(&binding, &isolation, &owned, &workflow)
            .unwrap_err()
            .code,
        "ownership_conflict"
    );
    assert_eq!(
        storage
            .read::<Value>(&format!("{}.json", id(99)))
            .unwrap_err()
            .code,
        "record_missing"
    );
    assert_eq!(
        f.house.inspect(id(7), &f.client).await.unwrap().token,
        before.token
    );
    assert!(f.source.exists());
}

#[tokio::test]
async fn family_detects_replaced_directory_and_incomplete_members_before_any_removal() {
    for replaced in [true, false] {
        let f = Fixture::new();
        let before = f.house.inspect(id(7), &f.client).await.unwrap();
        if replaced {
            let previous = f.source.with_file_name("prior-repository");
            std::fs::rename(&f.source, &previous).unwrap();
            copy_tree(&previous, &f.source);
            assert!(previous.exists());
        } else {
            let storage = f.root.child("cleanup").unwrap();
            let name = format!("family-{}.json", id(6));
            let mut family: Value = storage.read(&name).unwrap();
            family["members"][id(99).to_string()] = json!({"registration":bytes_digest(b"missing"),"predecessor":id(7),"resume_reference":bytes_digest(b"missing")});
            storage.write(&name, &family).unwrap();
        }
        assert!(f.house.inspect(id(7), &f.client).await.is_err());
        assert!(
            f.house
                .clone()
                .apply(
                    request(Uuid::new_v4()),
                    id(7),
                    before.token,
                    f.client.clone(),
                    Arc::new(|_| Ok(()))
                )
                .await
                .is_err()
        );
        assert!(f.source.join("file.txt").exists());
        assert!(f.retained.join("file.txt").exists());
    }
}

#[tokio::test]
async fn new_family_cannot_drop_its_marker_but_legacy_single_registration_stays_readable() {
    let f = Fixture::new();
    let storage = f.root.child("cleanup").unwrap();
    let name = format!("{}.json", id(7));
    let mut stored: Value = storage.read(&name).unwrap();
    stored.as_object_mut().unwrap().remove("family_id");
    storage.write(&name, &stored).unwrap();
    assert_eq!(
        f.house.inspect(id(7), &f.client).await.unwrap_err().code,
        "record_corrupt"
    );
    stored["schema_version"] = json!(1);
    storage.write(&name, &stored).unwrap();
    assert_eq!(
        f.house.inspect(id(7), &f.client).await.unwrap_err().code,
        "record_corrupt"
    );
    // Reproduce the complete legacy shape in this disposable fixture. A real
    // family cannot become legacy merely by dropping its registration field.
    std::fs::remove_file(storage.path().join(format!("family-{}.json", id(6)))).unwrap();
    let plan = f.house.inspect(id(7), &f.client).await.unwrap();
    assert!(plan.directory_family_digest.is_none());
    assert!(
        serde_json::to_value(&plan)
            .unwrap()
            .get("directory_family_digest")
            .is_none()
    );
}
fn request(request_id: Uuid) -> MutationRequest {
    MutationRequest {
        request_id,
        runtime_id: id(2),
        actor: actor(),
        kind: "fixture".into(),
        canonical_targets: vec![id(7).to_string()],
        expected: BTreeMap::new(),
        payload: json!({}),
    }
}
fn authorize(actor: &Actor) -> Result<()> {
    if actor.identity == "fixture-human" {
        Ok(())
    } else {
        Err(Error::new("access_denied", "fixture role revoked"))
    }
}

#[tokio::test]
async fn explicit_cleanup_removes_only_verified_clone_and_retains_recoverable_bytes_and_receipts() {
    let f = Fixture::new();
    let plan = f.house.inspect(id(7), &f.client).await.unwrap();
    assert!(f.source.exists());
    let original = f.root.read_bytes("intent.json", 65536).unwrap();
    let req = request(id(9));
    let receipt = f
        .house
        .clone()
        .apply(
            req.clone(),
            id(7),
            plan.token.clone(),
            f.client.clone(),
            Arc::new(authorize),
        )
        .await
        .unwrap();
    assert_eq!(receipt.phase, ReceiptPhase::Acknowledged);
    assert!(!f.source.exists());
    assert_eq!(f.house.retained_directory(id(7)).unwrap(), f.retained);
    assert_eq!(
        std::fs::read_to_string(f.retained.join("file.txt")).unwrap(),
        "retained fixture\n"
    );
    assert_eq!(f.root.read_bytes("intent.json", 65536).unwrap(), original);
    assert!(f.control.operation(id(7)).is_ok());
    let replay = f
        .house
        .clone()
        .apply(
            req,
            id(7),
            plan.token,
            f.client.clone(),
            Arc::new(authorize),
        )
        .await
        .unwrap();
    assert_eq!(replay.operation_id, receipt.operation_id);
}

#[tokio::test]
async fn dirty_missing_retained_and_changed_remote_facts_preserve_the_source() {
    for fault in ["dirty", "retained", "provider"] {
        let f = Fixture::new();
        match fault {
            "dirty" => std::fs::write(f.source.join("file.txt"), "unfinished work").unwrap(),
            "retained" => std::fs::remove_file(f.retained.join("file.txt")).unwrap(),
            _ => f.wire.changed.store(true, Ordering::SeqCst),
        }
        assert!(f.house.inspect(id(7), &f.client).await.is_err(), "{fault}");
        assert!(f.source.exists());
    }
}

#[tokio::test]
async fn stale_plan_and_unknown_operations_never_delete() {
    let f = Fixture::new();
    let plan = f.house.inspect(id(7), &f.client).await.unwrap();
    std::fs::write(f.source.join("file.txt"), "changed").unwrap();
    assert!(
        f.house
            .clone()
            .apply(
                request(id(9)),
                id(7),
                plan.token,
                f.client.clone(),
                Arc::new(authorize)
            )
            .await
            .is_err()
    );
    assert!(f.source.exists());
    let g = Fixture::new();
    let _ = g.control.mutate(
        request(id(12)),
        None,
        Some("recover"),
        || Ok(()),
        |_| Err(Error::new("unknown", "fixture sent outcome unknown")),
    );
    assert_eq!(
        g.house.inspect(id(7), &g.client).await.unwrap_err().code,
        "cleanup_blocked"
    );
    assert!(g.source.exists());
}

#[tokio::test]
async fn live_recorded_process_preserves_execution() {
    let f = Fixture::new();
    f.probe.live.store(true, Ordering::SeqCst);
    f.house
        .record_process(id(7), f.probe.current().unwrap())
        .unwrap();
    assert_eq!(
        f.house.inspect(id(7), &f.client).await.unwrap_err().code,
        "cleanup_blocked"
    );
    f.probe.live.store(false, Ordering::SeqCst);
    assert!(f.house.inspect(id(7), &f.client).await.is_ok());
}

#[tokio::test]
async fn replacing_identical_bytes_invalidates_the_exact_inode_bound_cleanup_plan() {
    let f = Fixture::new();
    let plan = f.house.inspect(id(7), &f.client).await.unwrap();
    let temporary = f.source.join("replacement.tmp");
    std::fs::write(&temporary, b"retained fixture\n").unwrap();
    std::fs::rename(temporary, f.source.join("file.txt")).unwrap();
    assert_eq!(
        f.house
            .clone()
            .apply(
                request(id(9)),
                id(7),
                plan.token,
                f.client.clone(),
                Arc::new(authorize)
            )
            .await
            .unwrap_err()
            .code,
        "stale_cleanup_plan"
    );
    assert!(f.source.exists());
}

#[tokio::test]
async fn linked_files_and_revoked_operator_preserve_the_registered_directory() {
    let f = Fixture::new();
    let plan = f.house.inspect(id(7), &f.client).await.unwrap();
    assert_eq!(
        f.house
            .clone()
            .apply(
                request(id(9)),
                id(7),
                plan.token,
                f.client.clone(),
                Arc::new(|_| Err(Error::new(
                    "access_denied",
                    "fixture current principal revoked"
                )))
            )
            .await
            .unwrap_err()
            .code,
        "access_denied"
    );
    std::os::unix::fs::symlink(f.retained.join("file.txt"), f.source.join("linked")).unwrap();
    assert_eq!(
        f.house.inspect(id(7), &f.client).await.unwrap_err().code,
        "unsafe_storage"
    );
    assert!(f.source.exists());
}

#[tokio::test]
async fn cleanup_background_lineage_permits_only_its_own_owned_parent_command() {
    let f = Fixture::new();
    let plan = f.house.inspect(id(7), &f.client).await.unwrap();
    let args = json!({"operation":id(7),"apply":true,"plan":plan.token});
    let mut parent_request = request(id(15));
    parent_request.kind = "command/Cleanup".into();
    parent_request.payload = json!({"arguments":args,"secret_digest":null});
    let (parent, fresh) = f
        .control
        .queue_command(parent_request, "recover", || Ok(()))
        .unwrap();
    assert!(fresh);
    let mut effect = request(kegari_v0::app::command_effect_id(parent.request_id));
    effect.payload = args;
    let mut altered = effect.clone();
    altered.payload["plan"] = json!("changed");
    assert_eq!(
        f.house
            .clone()
            .apply_for_command(
                altered,
                id(7),
                plan.token.clone(),
                f.client.clone(),
                Arc::new(authorize),
                parent.clone()
            )
            .await
            .unwrap_err()
            .code,
        "request_conflict"
    );
    assert!(f.source.exists());
    let observed = f
        .house
        .clone()
        .apply_for_command(
            effect,
            id(7),
            plan.token,
            f.client.clone(),
            Arc::new(authorize),
            parent.clone(),
        )
        .await
        .unwrap();
    assert_eq!(
        observed.expected["parent_command_id"],
        parent.request_id.to_string()
    );
    assert!(!f.source.exists());
    f.control
        .complete_command(parent.request_id, Ok(json!({"receipt":observed})))
        .unwrap();
}

#[test]
fn diagnostic_rotation_redacts_secrets_and_failure_remains_visible() {
    let tmp = tempfile::tempdir().unwrap();
    let root = PrivateDir::create(&tmp.path().canonicalize().unwrap().join("logs")).unwrap();
    let sink = LogSink::with_limit(PrivateDir::open(root.path()).unwrap(), 2048).unwrap();
    sink.register_secret(&kegari_v0::secrets::Secret::new(b"sentinel-secret".to_vec()).unwrap())
        .unwrap();
    let mut event = LogEvent {
        severity: Severity::Info,
        runtime_id: id(2),
        owner_generation: Some(3),
        workspace_id: Some(id(4)),
        work: None,
        operation_id: Some(id(7)),
        session_id: None,
        revision: None,
        event: "fixture".into(),
        outcome: "observed".into(),
        reason: Some(format!("sentinel-secret {}", "x".repeat(2000))),
    };
    for _ in 0..35 {
        assert!(sink.emit(&event), "{:?}", sink.health());
    }
    let files = std::fs::read_dir(root.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|s| s == "jsonl"))
        .collect::<Vec<_>>();
    assert_eq!(files.len(), 10);
    for file in files {
        let bytes = std::fs::read_to_string(file).unwrap();
        assert!(!bytes.contains("sentinel-secret"));
        for line in bytes.lines() {
            let value: Value = serde_json::from_str(line).unwrap();
            assert_eq!(value["runtime_id"], id(2).to_string());
        }
    }
    event.reason = Some("Authorization: Bearer example-token".into());
    assert!(sink.emit(&event));
    assert!(
        !std::fs::read_to_string(root.path().join("events.jsonl"))
            .unwrap()
            .contains("example-token")
    );
    let bounded = LogSink::with_limit(PrivateDir::open(root.path()).unwrap(), 1024).unwrap();
    event.reason = Some("x".repeat(2000));
    assert!(!bounded.emit(&event));
    assert_eq!(
        bounded.health().last_error.as_deref(),
        Some("log_event_too_large")
    );
    assert!(
        std::fs::metadata(root.path().join("events.jsonl"))
            .unwrap()
            .len()
            <= 2048
    );
    event.reason = None;
    std::fs::remove_file(root.path().join("events.jsonl")).unwrap();
    std::os::unix::fs::symlink("/nonexistent", root.path().join("events.jsonl")).unwrap();
    assert!(!sink.emit(&event));
    assert_eq!(sink.health().failed_events, 1);
    assert!(sink.health().last_error.is_some());
}

struct HandoverFixture {
    _tmp: tempfile::TempDir,
    host: kegari_v0::platform::HostIdentity,
    root: PrivateDir,
    probe: Probe,
    replacement: kegari_v0::platform::BinaryIdentity,
    qualification: VerifiedRuntimeQualification,
    info: HandoverInfo,
    qualification_record: RuntimeQualification,
    key: kegari_v0::evidence::AttestationKey,
}
fn metadata_binary(
    path: &Path,
    info: &impl serde::Serialize,
) -> kegari_v0::platform::BinaryIdentity {
    let document = serde_json::to_string(info).unwrap();
    std::fs::write(
        path,
        format!(
            "#!/bin/sh\ncat <<'KEGARI_FIXTURE_METADATA'\n{document}\nKEGARI_FIXTURE_METADATA\n"
        ),
    )
    .unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o555)).unwrap();
    kegari_v0::platform::BinaryIdentity::inspect(path, true).unwrap()
}
impl HandoverFixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().canonicalize().unwrap();
        let root = PrivateDir::create(&home.join(".kegari")).unwrap();
        let uid = unsafe { libc::geteuid() };
        let host = kegari_v0::platform::HostIdentity {
            host_id: "fixture-host".into(),
            boot_id: id(1),
            uid,
            user: "fixture".into(),
            home: home.clone(),
        };
        let mut info = handover_info().unwrap();
        let previous = metadata_binary(&home.join("old-kegari"), &info);
        info.runtime_build = "reviewed-replacement-fixture-build".into();
        let replacement = metadata_binary(&home.join("new-kegari"), &info);
        let config:kegari_config::Config=serde_json::from_value(json!({"schema_version":1,"runtime":{"id":id(2),"host_id":"fixture-host","execution_root":home.join("runs"),"max_agents":2},"connections":{},"workspaces":{},"control":{"local_users":{uid.to_string():{"id":"fixture-human","linear_user_id":id(3),"roles":["configure","recover"]}}}})).unwrap();
        let control =
            ControlHost::setup(PrivateDir::open(root.path()).unwrap(), id(2), id(8)).unwrap();
        drop(control);
        root.write_bytes("config.toml", toml::to_string(&config).unwrap().as_bytes())
            .unwrap();
        root.write(
            "owner.json",
            &kegari_v0::platform::OwnerRecord {
                runtime_id: id(2),
                host_id: host.host_id.clone(),
                uid,
                root: root.path().into(),
                owner_generation: 1,
                binary: previous.clone(),
                runtime_build: kegari_v0::BUILD_ID.into(),
                setup_complete: true,
                config_revision: Some(kegari_v0::config::config_revision(&config).unwrap()),
                control_process: None,
                pending_owner: None,
                orchestration: None,
                updated_at: chrono::Utc::now(),
            },
        )
        .unwrap();
        root.write(
            "service-shutdown.json",
            &kegari_v0::platform::ServiceShutdown {
                schema_version: 1,
                runtime_id: id(2),
                stopped: true,
                observed_at: chrono::Utc::now(),
            },
        )
        .unwrap();
        let plan = kegari_v0::platform::service_plan(&host, previous).unwrap();
        std::fs::create_dir_all(plan.plist_path.parent().unwrap()).unwrap();
        std::fs::set_permissions(home.join("Library"), std::fs::Permissions::from_mode(0o700))
            .unwrap();
        std::fs::set_permissions(
            plan.plist_path.parent().unwrap(),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        std::fs::write(&plan.plist_path, &plan.plist).unwrap();
        std::fs::set_permissions(&plan.plist_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        root.write(
            "service.json",
            &kegari_v0::platform::InstalledService {
                schema_version: 1,
                runtime_build: kegari_v0::BUILD_ID.into(),
                plan,
                installed_at: chrono::Utc::now(),
            },
        )
        .unwrap();
        let artifact = |name: &str| {
            let path = home.join(format!("fixture-{name}.json"));
            let bytes=serde_json::to_vec(&json!({"evidence_class":"synthetic qualification-signature test fixture","section":name})).unwrap();
            std::fs::write(&path, &bytes).unwrap();
            QualificationArtifact {
                path,
                digest: bytes_digest(&bytes),
            }
        };
        let qualification_record = RuntimeQualification {
            schema_version: 1,
            runtime_id: id(2),
            binary_digest: replacement.digest.clone(),
            runtime_build: info.runtime_build.clone(),
            instruction_digest: kegari_v0::digest::canonical_digest(1, &info).unwrap(),
            reviewed_by: "fixture-human".into(),
            reviewed_at: chrono::Utc::now(),
            deterministic: artifact("deterministic"),
            actual_agents: artifact("agent"),
            integration: artifact("integration"),
            human_calibration: artifact("human"),
        };
        let key = kegari_v0::evidence::AttestationKey::new(
            format!("keychain:{}:attestation-v1", id(2)),
            vec![7; 32],
        )
        .unwrap();
        let signed = key
            .sign("runtime_qualification", &qualification_record)
            .unwrap();
        let qualification = verify_runtime_qualification(&signed, &key).unwrap();
        Self {
            _tmp: tmp,
            host,
            root,
            probe: Probe {
                live: AtomicBool::new(false),
            },
            replacement,
            qualification,
            info,
            qualification_record,
            key,
        }
    }
}

#[tokio::test]
async fn explicit_handover_preserves_intent_evidence_and_old_service_without_starting() {
    let f = HandoverFixture::new();
    let before = f.root.read_bytes("intent.json", 65536).unwrap();
    let plan = inspect_handover(&f.host, f.replacement.clone(), &f.qualification, &f.probe)
        .await
        .unwrap();
    let receipt = apply_handover(&f.host, &plan, &f.qualification, &f.probe, actor(), id(50))
        .await
        .unwrap();
    assert_eq!(receipt.phase, ReceiptPhase::Acknowledged);
    let owner: kegari_v0::platform::OwnerRecord = f.root.read("owner.json").unwrap();
    assert_eq!(owner.binary, f.replacement);
    assert_eq!(owner.owner_generation, 2);
    assert!(owner.control_process.is_none());
    assert!(owner.orchestration.is_none());
    assert_eq!(f.root.read_bytes("intent.json", 65536).unwrap(), before);
    let stopped: kegari_v0::platform::ServiceShutdown =
        f.root.read("service-shutdown.json").unwrap();
    assert!(stopped.stopped);
    let service: kegari_v0::platform::InstalledService = f.root.read("service.json").unwrap();
    assert_eq!(service.plan.binary, f.replacement);
    assert!(
        std::fs::read_to_string(&service.plan.plist_path)
            .unwrap()
            .contains("new-kegari")
    );
    assert!(
        f.root
            .child("handover-history")
            .unwrap()
            .read::<kegari_v0::platform::InstalledService>("service-1.json")
            .is_ok()
    );
    let replay = apply_handover(&f.host, &plan, &f.qualification, &f.probe, actor(), id(50))
        .await
        .unwrap();
    assert_eq!(replay.operation_id, receipt.operation_id);
}

#[tokio::test]
async fn unknown_effects_live_old_processes_and_changed_qualification_prevent_handover() {
    for fault in ["unknown", "live", "artifact"] {
        let f = HandoverFixture::new();
        let before = f.root.read_bytes("owner.json", 65536).unwrap();
        match fault {
            "unknown" => {
                let store = ReceiptStore::new(f.root.child("receipts").unwrap());
                store.prepare(&request(id(60)), 1).unwrap();
            }
            "live" => f.probe.live.store(true, Ordering::SeqCst),
            _ => std::fs::write(
                &f.qualification_record.actual_agents.path,
                "changed fixture evidence",
            )
            .unwrap(),
        }
        assert!(
            inspect_handover(&f.host, f.replacement.clone(), &f.qualification, &f.probe)
                .await
                .is_err(),
            "{fault}"
        );
        assert_eq!(f.root.read_bytes("owner.json", 65536).unwrap(), before);
    }
}

#[tokio::test]
async fn unsupported_replacement_format_preserves_old_state_and_qualification_cannot_be_forged() {
    let f = HandoverFixture::new();
    let mut info = f.info.clone();
    info.receipt_schema = 99;
    let binary = metadata_binary(&f.host.home.join("unsupported"), &info);
    let before = f.root.read_bytes("owner.json", 65536).unwrap();
    assert_eq!(
        inspect_handover(&f.host, binary, &f.qualification, &f.probe)
            .await
            .unwrap_err()
            .code,
        "incompatible_version"
    );
    assert_eq!(f.root.read_bytes("owner.json", 65536).unwrap(), before);
    let mut signed = f
        .key
        .sign("runtime_qualification", &f.qualification_record)
        .unwrap();
    signed.payload.reviewed_by = "forged reviewer".into();
    assert!(verify_runtime_qualification(&signed, &f.key).is_err());
}

#[tokio::test]
async fn replacement_must_explicitly_support_owned_process_and_managed_policy_records() {
    let f = HandoverFixture::new();
    let preserved = [
        "owner.json",
        "service.json",
        "intent.json",
        "service-shutdown.json",
    ]
    .into_iter()
    .map(|name| (name, f.root.read_bytes(name, 65536).unwrap()))
    .collect::<Vec<_>>();
    let store = ReceiptStore::new(f.root.child("receipts").unwrap());
    let receipts = serde_json::to_value(store.all().unwrap()).unwrap();
    for fault in [
        "old_process",
        "pre_bundle_process",
        "unknown_process",
        "missing_process",
        "old_metadata",
        "alpha3_metadata",
        "pre_default_workflow_metadata",
        "pre_optional_policy_metadata",
        "missing_repository_policy",
        "unknown_repository_policy",
        "missing_workflow_source",
        "unknown_workflow_source",
        "missing_managed_policy",
        "foreground_only_managed_policy",
        "pre_resource_managed_policy",
        "pre_generic_managed_policy",
        "obsolete_offline_cargo_managed_policy",
        "unknown_managed_policy",
    ] {
        let mut info = serde_json::to_value(&f.info).unwrap();
        match fault {
            "old_process" => info["process_schema"] = 1.into(),
            "pre_bundle_process" => info["process_schema"] = 2.into(),
            "unknown_process" => info["process_schema"] = 99.into(),
            "missing_process" => {
                info.as_object_mut().unwrap().remove("process_schema");
            }
            "alpha3_metadata" => info["schema_version"] = 2.into(),
            // Keep the current asset inventory: reader support must be checked
            // directly rather than inferred from the presence of an asset name.
            "pre_default_workflow_metadata" => info["schema_version"] = 3.into(),
            "pre_optional_policy_metadata" => info["schema_version"] = 4.into(),
            "missing_repository_policy" => {
                info.as_object_mut()
                    .unwrap()
                    .remove("repository_policy_schema");
            }
            "unknown_repository_policy" => info["repository_policy_schema"] = 99.into(),
            "missing_workflow_source" => {
                info.as_object_mut()
                    .unwrap()
                    .remove("workflow_source_schema");
            }
            "unknown_workflow_source" => info["workflow_source_schema"] = 99.into(),
            "missing_managed_policy" => {
                info.as_object_mut()
                    .unwrap()
                    .remove("managed_execution_schema");
            }
            "unknown_managed_policy" => info["managed_execution_schema"] = 99.into(),
            "foreground_only_managed_policy" => info["managed_execution_schema"] = 1.into(),
            "pre_resource_managed_policy" => info["managed_execution_schema"] = 2.into(),
            "pre_generic_managed_policy" => info["managed_execution_schema"] = 3.into(),
            "obsolete_offline_cargo_managed_policy" => info["managed_execution_schema"] = 4.into(),
            _ => info["schema_version"] = 1.into(),
        }
        let binary = metadata_binary(&f.host.home.join(fault), &info);
        assert_eq!(
            inspect_handover(&f.host, binary, &f.qualification, &f.probe)
                .await
                .unwrap_err()
                .code,
            "incompatible_version",
            "{fault}"
        );
        for (name, bytes) in &preserved {
            assert_eq!(
                &f.root.read_bytes(name, 65536).unwrap(),
                bytes,
                "{fault}: {name}"
            );
        }
        assert_eq!(
            serde_json::to_value(store.all().unwrap()).unwrap(),
            receipts,
            "{fault}"
        );
    }
}

#[tokio::test]
async fn blocked_provider_read_does_not_hold_control_and_force_prevents_deletion() {
    let f = Fixture::new();
    let plan = f.house.inspect(id(7), &f.client).await.unwrap();
    f.wire.pause_at.store(3, Ordering::SeqCst);
    let house = f.house.clone();
    let client = f.client.clone();
    let task = tokio::spawn(async move {
        house
            .apply(
                request(id(600)),
                id(7),
                plan.token,
                client,
                Arc::new(authorize),
            )
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), f.wire.paused.notified())
        .await
        .unwrap();
    assert_eq!(
        f.control.operation(id(600)).unwrap().phase,
        ReceiptPhase::Sent
    );
    let control = f.control.clone();
    let snapshot = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        tokio::task::spawn_blocking(move || control.snapshot()),
    )
    .await
    .unwrap()
    .unwrap()
    .unwrap();
    assert!(snapshot.pending_local_processes.contains(&id(600)));
    let control = f.control.clone();
    let mut force = request(id(601));
    force.actor.roles.insert("lifecycle".into());
    force.canonical_targets = vec![id(2).to_string()];
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        tokio::task::spawn_blocking(move || control.lifecycle(force, Lifecycle::ForceStop, 0)),
    )
    .await
    .unwrap()
    .unwrap()
    .unwrap();
    f.wire.resume.notify_one();
    assert!(task.await.unwrap().is_err());
    assert!(f.source.join("file.txt").is_file());
    assert!(f.retained.join("file.txt").is_file());
    assert_eq!(
        f.control.operation(id(600)).unwrap().phase,
        ReceiptPhase::Sent
    );
    assert!(
        f.control
            .snapshot()
            .unwrap()
            .fenced_operations
            .contains(&id(600))
    );
}

#[tokio::test]
async fn unavailable_native_containment_blocks_hook_before_cleanup_effect() {
    let f = Fixture::with_hook(true);
    let plan = f.house.inspect(id(7), &f.client).await.unwrap();
    let error = f
        .house
        .clone()
        .apply(
            request(id(602)),
            id(7),
            plan.token,
            f.client.clone(),
            Arc::new(authorize),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "process_tree_unverified");
    assert_eq!(
        f.control.operation(id(602)).unwrap_err().code,
        "record_missing"
    );
    assert!(
        f.control
            .snapshot()
            .unwrap()
            .pending_local_processes
            .is_empty()
    );
    assert!(f.source.exists());
}

#[tokio::test]
async fn native_cleanup_metadata_uses_control_journal_and_same_sealed_cleanup_effect() {
    let f = Fixture::build(false, true);
    let plan = f.house.inspect(id(7), &f.client).await.unwrap();
    let receipt = f
        .house
        .clone()
        .apply(
            request(id(603)),
            id(7),
            plan.token,
            f.client.clone(),
            Arc::new(authorize),
        )
        .await
        .unwrap();
    assert_eq!(receipt.phase, ReceiptPhase::Acknowledged);
    assert!(!f.source.exists());
    assert!(f.retained.join("file.txt").is_file());
    let journal = f.processes.unwrap();
    let records = journal.observe(id(603)).unwrap();
    assert!(!records.is_empty());
    assert!(records.iter().all(|record| {
        record.binding.lease.is_none()
            && record.binding.control_owner.as_ref().is_some_and(|owner| {
                owner.cleanup_receipt.as_ref() == Some(&receipt.payload_digest)
            })
    }));
    assert!(
        f.control
            .snapshot()
            .unwrap()
            .pending_local_processes
            .is_empty()
    );
}

#[tokio::test]
async fn cleanup_reloads_installed_workflow_origin_and_refuses_conflicting_or_missing_source() {
    let fixture = Fixture::build_with_default(false, false, true);
    let storage = fixture.root.child("cleanup").unwrap();
    let name = format!("{}.json", id(7));
    let original: Value = storage.read(&name).unwrap();
    assert!(original.get("workflow_path").is_none());
    assert!(original["workflow_embedded"].is_object());
    assert!(fixture.house.inspect(id(7), &fixture.client).await.is_ok());
    for fault in ["missing", "conflicting", "obsolete"] {
        let mut changed = original.clone();
        match fault {
            "missing" => {
                changed.as_object_mut().unwrap().remove("workflow_embedded");
            }
            "conflicting" => changed["workflow_path"] = json!(fixture.source.join("WORKFLOW.md")),
            _ => changed["workflow_embedded"]["version"] = json!(999),
        }
        storage.write(&name, &changed).unwrap();
        assert!(
            fixture.house.inspect(id(7), &fixture.client).await.is_err(),
            "{fault}"
        );
        assert!(fixture.source.is_dir());
        storage.write(&name, &original).unwrap();
    }
    assert!(fixture.house.inspect(id(7), &fixture.client).await.is_ok());
}
