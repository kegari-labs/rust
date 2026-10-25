//! Owned local fixtures; provider responses/process ownership are explicitly
//! synthetic. These tests never access native Keychain or launchd.
use kegari_v0::{
    control::*,
    digest::bytes_digest,
    error::{Error, Result},
    evidence::{Generation, WorkIdentity},
    execution::{DirectoryIdentity, ExecutionBinding, OwnedDirectory},
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
        Arc,
        atomic::{AtomicBool, Ordering},
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
    let out = Command::new(git_path())
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
        .output()
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
struct Fixture {
    _tmp: tempfile::TempDir,
    root: PrivateDir,
    source: PathBuf,
    retained: PathBuf,
    house: Arc<Housekeeping>,
    control: Arc<ControlHost>,
    wire: Arc<Wire>,
    client: GithubClient,
    probe: Arc<Probe>,
}
impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().canonicalize().unwrap();
        let root = PrivateDir::create(&home.join("control")).unwrap();
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
        let git_api = Git::new(git_path()).unwrap();
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
            executables: vec![git_path()],
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
        std::fs::write(&workflow_path, "Fixture context").unwrap();
        let workflow = Workflow::load(&workflow_path).unwrap();
        let control = Arc::new(
            ControlHost::setup(PrivateDir::open(root.path()).unwrap(), id(2), id(8)).unwrap(),
        );
        let probe = Arc::new(Probe {
            live: AtomicBool::new(false),
        });
        let house = Arc::new(
            Housekeeping::new(
                PrivateDir::open(root.path()).unwrap(),
                control.clone(),
                probe.clone(),
                git_path(),
            )
            .unwrap(),
        );
        house
            .register_execution(&binding, &isolation, &owned, &workflow)
            .unwrap();
        let wire = Arc::new(Wire {
            commit: commit.clone(),
            changed: AtomicBool::new(false),
        });
        let client = GithubClient::with_transport(
            HttpPolicy::new(
                "https://github.invalid/",
                "7".into(),
                std::time::Duration::from_secs(2),
            )
            .unwrap(),
            kegari_v0::config::GithubSelection {
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
            house,
            control,
            wire,
            client,
            probe,
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
fn metadata_binary(path: &Path, info: &HandoverInfo) -> kegari_v0::platform::BinaryIdentity {
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
        let config:kegari_v0::config::Config=serde_json::from_value(json!({"schema_version":1,"runtime":{"id":id(2),"host_id":"fixture-host","execution_root":home.join("runs"),"max_agents":2},"connections":{},"workspaces":{},"control":{"local_users":{uid.to_string():{"id":"fixture-human","linear_user_id":id(3),"roles":["configure","recover"]}}}})).unwrap();
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
                config_revision: Some(config.revision().unwrap()),
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
