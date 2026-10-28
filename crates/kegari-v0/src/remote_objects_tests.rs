//! Actual disposable Git protocol/import fixtures; injected provider wire and
//! boot identity do not constitute native enforcement or live qualification.
use super::*;
use crate::test_process_probe as probe;
use crate::{
    app::ConfigurationControl,
    platform::ProcessProbe,
    providers::{Credential, HttpPolicy, HttpRequest, HttpResponse, HttpTransport},
    reconcile::{DependencyOutcomes, GatePurpose},
};
use std::{
    io::Write,
    os::unix::fs::PermissionsExt,
    process::{Command, Stdio},
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
fn fixture_git(binary: &Path, cwd: &Path, args: &[&str], input: Option<&[u8]>) -> Vec<u8> {
    let (mut child, _group) = crate::process_group::OwnedGroup::spawn_std(
        Command::new(binary)
            .current_dir(cwd)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_PROTOCOL", "version=2")
            .env("GIT_AUTHOR_NAME", "Disposable Fixture")
            .env("GIT_COMMITTER_NAME", "Disposable Fixture")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
            .env("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z")
            .env("GIT_COMMITTER_DATE", "2026-01-01T00:00:00Z")
            .args([
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "commit.gpgSign=false",
            ])
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )
    .unwrap();
    if let Some(input) = input {
        child.stdin.take().unwrap().write_all(input).unwrap();
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{:?}: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}
struct GitFixture {
    root: PathBuf,
    binary: PathBuf,
    revision: Revision,
    parent: String,
    advertisement: Vec<u8>,
    response: Vec<u8>,
}
fn git_fixture(temp: &Path) -> GitFixture {
    use std::os::unix::fs::PermissionsExt;
    let binary = temp.join("tools/git");
    let root = temp.join("source");
    fixture_git(&binary, &root, &["init", "--initial-branch=main"], None);
    fs::write(root.join("base.txt"), "base\n").unwrap();
    fixture_git(&binary, &root, &["add", "."], None);
    fixture_git(&binary, &root, &["commit", "-m", "fixture: base"], None);
    let parent = String::from_utf8(fixture_git(&binary, &root, &["rev-parse", "HEAD"], None))
        .unwrap()
        .trim()
        .into();
    fs::write(root.join("binary.dat"), (0..=255u8).collect::<Vec<_>>()).unwrap();
    fs::write(root.join("run.sh"), "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(root.join("run.sh"), fs::Permissions::from_mode(0o755)).unwrap();
    std::os::unix::fs::symlink("base.txt", root.join("link")).unwrap();
    fs::write(
        root.join("AGENTS.md"),
        "Only this fetched revision's instructions apply.\n",
    )
    .unwrap();
    fixture_git(&binary, &root, &["add", "."], None);
    fixture_git(
        &binary,
        &root,
        &["commit", "-m", "fixture: remote source"],
        None,
    );
    let handle = Git::new(&binary).unwrap();
    let revision = handle.revision(&root, "HEAD").unwrap();
    let advertisement = fixture_git(
        &binary,
        &root,
        &[
            "upload-pack",
            "--stateless-rpc",
            "--advertise-refs",
            root.to_str().unwrap(),
        ],
        None,
    );
    let request = pack::fetch_request(&BTreeSet::from([revision.commit.clone()])).unwrap();
    let response = fixture_git(
        &binary,
        &root,
        &["upload-pack", "--stateless-rpc", root.to_str().unwrap()],
        Some(&request),
    );
    fs::write(root.join("base.txt"), "uncommitted operator work\n").unwrap();
    fs::write(root.join("untracked.txt"), "preserve\n").unwrap();
    GitFixture {
        root,
        binary,
        revision,
        parent,
        advertisement,
        response,
    }
}
struct Wire {
    fixture: GitFixture,
    fetches: AtomicUsize,
    requests: AtomicUsize,
    move_after_fetch: AtomicBool,
    force: Mutex<Option<(Arc<ControlHost>, Uuid)>>,
    response_override: Mutex<Option<Vec<u8>>>,
    expected_wants: Mutex<Option<BTreeSet<String>>>,
    api_values: Mutex<BTreeMap<String, serde_json::Value>>,
}
impl HttpTransport for Wire {
    fn send(
        &self,
        request: HttpRequest,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<HttpResponse>> + Send + '_>>
    {
        Box::pin(async move {
            self.requests.fetch_add(1, Ordering::SeqCst);
            let host = request.url.host_str().unwrap();
            let path = request.url.path();
            let mut headers = reqwest::header::HeaderMap::new();
            let body = if host == "github.com" {
                assert!(request.headers["authorization"].is_sensitive());
                assert_eq!(request.headers["git-protocol"], "version=2");
                if path == "/fixture/repo.git/info/refs" {
                    assert_eq!(request.url.query(), Some("service=git-upload-pack"));
                    headers.insert(
                        "content-type",
                        reqwest::header::HeaderValue::from_static(
                            "application/x-git-upload-pack-advertisement",
                        ),
                    );
                    self.fixture.advertisement.clone()
                } else {
                    assert_eq!(path, "/fixture/repo.git/git-upload-pack");
                    assert_eq!(
                        request.body,
                        pack::fetch_request(
                            &self
                                .expected_wants
                                .lock()
                                .unwrap()
                                .clone()
                                .unwrap_or_else(|| BTreeSet::from([self
                                    .fixture
                                    .revision
                                    .commit
                                    .clone()]))
                        )?
                    );
                    self.fetches.fetch_add(1, Ordering::SeqCst);
                    if let Some((control, op)) = self.force.lock().unwrap().take() {
                        control.fence(op)?;
                    }
                    headers.insert(
                        "content-type",
                        reqwest::header::HeaderValue::from_static(
                            "application/x-git-upload-pack-result",
                        ),
                    );
                    self.response_override
                        .lock()
                        .unwrap()
                        .clone()
                        .unwrap_or_else(|| self.fixture.response.clone())
                }
            } else {
                assert_eq!(host, "api.github.com");
                let override_value = self.api_values.lock().unwrap().get(path).cloned();
                let value = if let Some(value) = override_value {
                    value
                } else if path == "/user" {
                    json!({"id":7,"login":"fixture-user","type":"User"})
                } else if path == "/repos/fixture/repo" {
                    json!({"id":42,"name":"repo","owner":{"id":9,"login":"fixture","type":"User"},"default_branch":"main","private":true,"archived":false,"disabled":false,"allow_squash_merge":true,"permissions":{"push":true}})
                } else if path == "/repos/fixture/repo/git/ref/heads/staging" {
                    json!({"ref":"refs/heads/staging","object":{"type":"commit","sha":if self.move_after_fetch.load(Ordering::SeqCst)&&self.fetches.load(Ordering::SeqCst)>0 {&self.fixture.parent}else{&self.fixture.revision.commit}}})
                } else if path
                    == format!(
                        "/repos/fixture/repo/git/commits/{}",
                        self.fixture.revision.commit
                    )
                {
                    json!({"sha":self.fixture.revision.commit,"tree":{"sha":self.fixture.revision.tree},"parents":[{"sha":self.fixture.parent}]})
                } else {
                    panic!("unexpected fixture route {path}")
                };
                serde_json::to_vec(&value).unwrap()
            };
            Ok(HttpResponse {
                status: 200,
                headers,
                body,
            })
        })
    }
}
struct Fixture {
    _temp: tempfile::TempDir,
    runtime: Arc<crate::runtime::RuntimeCoordinator>,
    acquisition: ObjectAcquisition,
    request: AuthorizedObjectRequest,
    wire: Arc<Wire>,
}
async fn fixture() -> Fixture {
    fixture_with_delivery_policy(true).await
}
async fn fixture_with_delivery_policy(declared: bool) -> Fixture {
    let (temp, built, _delivery, _, _) =
        crate::runtime_delivery::tests::fixture_authorization_with_configuration(true, |config| {
            if !declared {
                let repository = config
                    .workspaces
                    .get_mut("example")
                    .unwrap()
                    .repositories
                    .get_mut("42")
                    .unwrap();
                repository.ci = None;
                repository.production_ci = None;
                repository.no_change_validation = None;
                repository.deployment = None;
            }
        })
        .await;
    let root = temp.path().canonicalize().unwrap();
    let git = git_fixture(&root);
    let wire = Arc::new(Wire {
        fixture: git,
        fetches: AtomicUsize::new(0),
        requests: AtomicUsize::new(0),
        move_after_fetch: AtomicBool::new(false),
        force: Mutex::new(None),
        response_override: Mutex::new(None),
        expected_wants: Mutex::new(None),
        api_values: Mutex::new(BTreeMap::new()),
    });
    let config = built.configuration.snapshot().unwrap();
    let workspace = &config.workspaces["example"];
    let github = GithubClient::with_transport(
        HttpPolicy::new(
            "https://api.github.com/",
            "7".into(),
            std::time::Duration::from_secs(10),
        )
        .unwrap(),
        workspace.github.clone(),
        Credential::new("synthetic-token-not-live".into()).unwrap(),
        Some(wire.clone()),
    )
    .unwrap();
    let contract = RepositoryContract {
        configuration_name: "42".into(),
        repository: RepositoryId {
            hostname: "github.com".into(),
            id: "42".into(),
        },
        provider: crate::providers::RepositoryIdentity {
            id: 42,
            owner: "fixture".into(),
            name: "repo".into(),
            owner_id: 9,
            organization_id: None,
        },
        configuration: workspace.repositories["42"].clone(),
    };
    let projection = built.runtime.observations.snapshot().unwrap();
    let parent = projection
        .parent(Uuid::from_u128(2), Uuid::from_u128(100))
        .unwrap();
    let gate = parent
        .fresh_gate(
            Some(Uuid::from_u128(101)),
            GatePurpose::NewOperation,
            &DependencyOutcomes::default(),
            chrono::Utc::now(),
            chrono::Duration::seconds(30),
        )
        .unwrap();
    let lease = crate::runtime_delivery::tests::start_control(&built.runtime);
    let permit = built
        .runtime
        .control
        .admit(
            &lease,
            Uuid::new_v4(),
            gate.work_key().unwrap(),
            canonical_digest(1, &"fixture owned preparation").unwrap(),
            || Ok(()),
        )
        .unwrap();
    let request = AuthorizedObjectRequest::for_worker(
        &contract,
        wire.fixture.revision.clone(),
        &permit,
        &WorkerAuthority::Authorized(gate.clone()),
    )
    .unwrap();
    let probe = Arc::new(probe::FixtureBootProbe::new());
    probe.current().unwrap();
    let processes = Arc::new(
        ProcessJournal::with_probe(
            PrivateDir::create(&root.join("acquisition-processes")).unwrap(),
            lease.runtime_id,
            probe,
        )
        .unwrap(),
    );
    let current_gate = gate.clone();
    let acquisition = ObjectAcquisition::new(AcquisitionServices {
        control: built.runtime.control.clone(),
        github,
        git: Git::from_pinned_executable(&wire.fixture.binary).unwrap(),
        records: PrivateDir::create(&root.join("objects"))
            .unwrap()
            .child("object-sources")
            .unwrap(),
        key: built.runtime.services.key.clone(),
        processes,
        executions: built.runtime.executions.clone(),
        current: Arc::new(move |r| {
            if r.authority_digest() != current_gate.digest()? {
                return Err(Error::new("stale_generation", "fixture authority changed"));
            }
            Ok(())
        }),
        limits: ObjectLimits::default(),
    })
    .unwrap();
    Fixture {
        _temp: temp,
        runtime: built.runtime,
        acquisition,
        request,
        wire,
    }
}
#[tokio::test]
async fn unresolved_delivery_policy_still_acquires_exact_scoped_source_and_owned_instructions() {
    // Omission happens before configuration, authorization, and the real Control
    // permit are constructed. No authenticated state is rewritten for this case.
    let fixture = fixture_with_delivery_policy(false).await;
    let configuration = fixture.runtime.configuration.snapshot().unwrap();
    let repository = &configuration.workspaces["example"].repositories["42"];
    configuration.validate().unwrap();
    assert_eq!(
        repository.staging_ci().unwrap_err().code,
        "delivery_policy_unresolved"
    );
    assert_eq!(
        repository.production_ci_policy().unwrap_err().code,
        "delivery_policy_unresolved"
    );
    assert_eq!(
        repository.no_change_workflow().unwrap_err().code,
        "delivery_policy_unresolved"
    );
    assert_eq!(
        repository.deployment_policy().unwrap_err().code,
        "delivery_policy_unresolved"
    );
    let original_revision = crate::config::config_revision(&configuration).unwrap();
    let original_contract = fixture.request.contract_digest().unwrap();
    let source = fixture
        .acquisition
        .acquire(fixture.request.clone())
        .await
        .unwrap();
    assert_eq!(source.revision(), &fixture.wire.fixture.revision);
    let git = Git::new(&fixture.wire.fixture.binary).unwrap();
    let root = fixture._temp.path().canonicalize().unwrap();
    let execution = PrivateDir::create(&root.join("discovery-execution")).unwrap();
    let projection = fixture.runtime.observations.snapshot().unwrap();
    let parent = projection
        .parent(Uuid::from_u128(2), Uuid::from_u128(100))
        .unwrap();
    let mut work = parent.work.clone().unwrap();
    work.child = Some(Uuid::from_u128(101));
    let identity = crate::execution::DirectoryIdentity {
        schema_version: 1,
        runtime_id: fixture.request.permit.lease.runtime_id,
        directory_id: Uuid::new_v4(),
        repository: source.repository().clone(),
        work,
        repository_path: execution.path().join("repository"),
        operator_checkout: fixture.wire.fixture.root.clone(),
    };
    let prepared = git
        .prepare_acquired_execution_clone(&source, execution.path(), &identity)
        .unwrap();
    assert_eq!(
        fs::read_to_string(prepared.join("AGENTS.md")).unwrap(),
        "Only this fetched revision's instructions apply.\n"
    );
    assert_eq!(
        fs::read_to_string(fixture.wire.fixture.root.join("base.txt")).unwrap(),
        "uncommitted operator work\n"
    );
    assert_eq!(
        fixture.request.contract_digest().unwrap(),
        original_contract
    );
    assert_eq!(
        crate::config::config_revision(&fixture.runtime.configuration.snapshot().unwrap()).unwrap(),
        original_revision
    );
    assert!(
        fixture
            .runtime
            .current_delivery(&parent.key)
            .unwrap()
            .verified_parent()
            .is_none()
    );
    assert!(
        fixture
            .runtime
            .current_delivery(&parent.key)
            .unwrap()
            .verified_acceptance()
            .is_none()
    );
    // The injected wire accepts only identity/ref/object reads and upload-pack;
    // these local source-import records do not constitute hosted or model work.
    let effect = fixture
        .runtime
        .control
        .operation(fixture.request.effect_id().unwrap())
        .unwrap();
    assert_eq!(effect.phase, ReceiptPhase::Acknowledged);
    assert!(
        fixture
            .runtime
            .executions
            .active_operations()
            .unwrap()
            .is_empty()
    );
    assert_eq!(fixture.wire.fetches.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn actual_protocol_import_preserves_dirty_operator_source_and_reuses_only_verified_cache() {
    let fixture = fixture().await;
    let index = fs::read(fixture.wire.fixture.root.join(".git/index")).unwrap();
    let config = fs::read(fixture.wire.fixture.root.join(".git/config")).unwrap();
    let source = fixture
        .acquisition
        .acquire(fixture.request.clone())
        .await
        .unwrap();
    let git = Git::new(&fixture.wire.fixture.binary).unwrap();
    source.verify_for_clone(&git).unwrap();
    assert_eq!(
        git.exact_revision(source.root(), &fixture.wire.fixture.revision.commit)
            .unwrap(),
        fixture.wire.fixture.revision
    );
    assert!(
        git.is_ancestor(
            source.root(),
            &fixture.wire.fixture.parent,
            &fixture.wire.fixture.revision.commit
        )
        .unwrap()
    );
    assert_eq!(
        fixture_git(
            &fixture.wire.fixture.binary,
            source.root(),
            &["show", &format!("{}:AGENTS.md", source.revision().commit)],
            None
        ),
        b"Only this fetched revision's instructions apply.\n"
    );
    assert_eq!(
        fs::read(fixture.wire.fixture.root.join(".git/index")).unwrap(),
        index
    );
    assert_eq!(
        fs::read(fixture.wire.fixture.root.join(".git/config")).unwrap(),
        config
    );
    assert_eq!(
        fs::read_to_string(fixture.wire.fixture.root.join("base.txt")).unwrap(),
        "uncommitted operator work\n"
    );
    assert!(fixture.wire.fixture.root.join("untracked.txt").is_file());
    let again = fixture
        .acquisition
        .acquire(fixture.request.clone())
        .await
        .unwrap();
    assert_eq!(again.digest().unwrap(), source.digest().unwrap());
    assert_eq!(fixture.wire.fetches.load(Ordering::SeqCst), 1);
    let records = fixture
        .acquisition
        .services
        .processes
        .observe(fixture.request.permit.operation_id)
        .unwrap();
    assert!(!records.is_empty());
    assert!(records.iter().all(|r| matches!(
        r.state,
        crate::process_records::RecoveryState::OwnedGroupExitRecorded
    )));
    assert!(
        fixture
            .runtime
            .executions
            .active_operations()
            .unwrap()
            .is_empty()
    );
    fs::write(
        source.root().join("config"),
        "[core]\nrepositoryformatversion = 0\n[include]\npath = /tmp/forbidden\n",
    )
    .unwrap();
    assert_eq!(
        fixture
            .acquisition
            .observe_cached(&fixture.request)
            .await
            .err()
            .unwrap()
            .code,
        "ownership_conflict"
    );
}
#[tokio::test]
async fn moved_ref_or_force_after_read_never_imports_or_spawns() {
    for force in [false, true] {
        let fixture = fixture().await;
        if force {
            *fixture.wire.force.lock().unwrap() = Some((
                fixture.runtime.control.clone(),
                fixture.request.permit.operation_id,
            ));
        } else {
            fixture.wire.move_after_fetch.store(true, Ordering::SeqCst);
        }
        let error = fixture
            .acquisition
            .acquire(fixture.request.clone())
            .await
            .err()
            .unwrap();
        assert!(matches!(
            error.code.as_str(),
            "stale_generation" | "access_denied"
        ));
        assert!(
            fixture
                .acquisition
                .services
                .processes
                .operations()
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            fs::read_dir(fixture.acquisition.services.records.path())
                .unwrap()
                .count(),
            0
        );
    }
}
#[tokio::test]
async fn stopped_or_wrong_scope_request_cannot_create_object_storage() {
    let fixture = fixture().await;
    fixture
        .runtime
        .control
        .fence(fixture.request.permit.operation_id)
        .unwrap();
    assert_eq!(
        fixture
            .acquisition
            .acquire(fixture.request.clone())
            .await
            .err()
            .unwrap()
            .code,
        "access_denied"
    );
    assert_eq!(fixture.wire.fetches.load(Ordering::SeqCst), 0);
    assert!(
        fixture
            .acquisition
            .services
            .processes
            .operations()
            .unwrap()
            .is_empty()
    );
}
#[tokio::test]
async fn real_pack_validates_binary_modes_symlinks_and_rejects_truncation_limits_and_bad_checksum()
{
    let fixture = fixture().await;
    let limits = ObjectLimits::default();
    pack::validate_advertisement(&fixture.wire.fixture.advertisement, &limits).unwrap();
    let bytes = pack::extract_pack(&fixture.wire.fixture.response, &limits).unwrap();
    let validated = pack::validate_pack(bytes.clone(), &limits).unwrap();
    assert!(
        validated
            .objects
            .contains(&fixture.wire.fixture.revision.commit)
    );
    let mut tiny = limits.clone();
    tiny.objects = 1;
    assert_eq!(
        pack::validate_pack(bytes.clone(), &tiny)
            .err()
            .unwrap()
            .code,
        "source_limit_exceeded"
    );
    tiny = limits.clone();
    tiny.object_bytes = 8;
    assert_eq!(
        pack::validate_pack(bytes.clone(), &tiny)
            .err()
            .unwrap()
            .code,
        "source_limit_exceeded"
    );
    let mut bad = bytes.clone();
    bad[20] ^= 1;
    assert_eq!(
        pack::validate_pack(bad, &limits).err().unwrap().code,
        "invalid_git_pack"
    );
    for cut in [0, 1, 3, 12, bytes.len() - 1] {
        assert!(pack::validate_pack(bytes[..cut].to_vec(), &limits).is_err());
    }
    for cut in [0, 1, 3, fixture.wire.fixture.response.len() - 1] {
        assert!(pack::extract_pack(&fixture.wire.fixture.response[..cut], &limits).is_err());
    }
}

fn protocol_pack(bytes: &[u8]) -> Vec<u8> {
    let mut out = b"000dpackfile\n".to_vec();
    for chunk in bytes.chunks(65515) {
        out.extend_from_slice(format!("{:04x}", chunk.len() + 5).as_bytes());
        out.push(1);
        out.extend_from_slice(chunk);
    }
    out.extend_from_slice(b"0000");
    out
}

#[test]
fn exact_review_snapshot_accepts_authorized_fork_identity_and_rejects_changed_fields() {
    let expected = crate::providers::PullRequest {
        repository_id: 42,
        id: 900,
        number: 7,
        author_id: 8,
        state: "open".into(),
        draft: false,
        head_sha: "a".repeat(40),
        base_sha: "b".repeat(40),
        head_ref: "fork-feature".into(),
        base_ref: "main".into(),
        head_repository_id: Some(99),
        merged: Some(false),
        merge_commit_sha: None,
        url: "https://github.com/fixture/repo/pull/7".into(),
    };
    verify_review_source(&expected, &expected).unwrap();
    for variant in 0..7 {
        let mut changed = expected.clone();
        match variant {
            0 => changed.id += 1,
            1 => changed.head_repository_id = Some(100),
            2 => changed.head_ref = "other".into(),
            3 => changed.base_ref = "staging".into(),
            4 => changed.author_id += 1,
            5 => changed.merged = Some(true),
            _ => changed.head_sha = "c".repeat(40),
        }
        assert_eq!(
            verify_review_source(&expected, &changed).unwrap_err().code,
            "stale_generation"
        );
    }
}
#[tokio::test]
async fn missing_ancestry_is_a_retained_known_failure_and_cannot_be_replayed() {
    let fixture = fixture().await;
    let source = &fixture.wire.fixture;
    let ids = String::from_utf8(fixture_git(
        &source.binary,
        &source.root,
        &[
            "rev-list",
            "--objects",
            "--no-object-names",
            &source.revision.commit,
        ],
        None,
    ))
    .unwrap();
    let selected = ids
        .lines()
        .filter(|id| *id != source.parent)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let pack = fixture_git(
        &source.binary,
        &source.root,
        &["pack-objects", "--stdout", "--window=0"],
        Some(selected.as_bytes()),
    );
    // The resource validator accepts this internally valid pack; Git must still
    // reject the missing parent before a complete-source capability can exist.
    pack::validate_pack(pack.clone(), &ObjectLimits::default()).unwrap();
    *fixture.wire.response_override.lock().unwrap() = Some(protocol_pack(&pack));
    assert_eq!(
        fixture
            .acquisition
            .acquire(fixture.request.clone())
            .await
            .err()
            .unwrap()
            .code,
        "git_operation_failed"
    );
    let receipt = fixture
        .runtime
        .control
        .operation(fixture.request.effect_id().unwrap())
        .unwrap();
    assert_eq!(receipt.phase, ReceiptPhase::Acknowledged);
    assert_eq!(receipt.observed_result.unwrap()["object_import"], "failed");
    assert!(
        fixture
            .runtime
            .executions
            .active_operations()
            .unwrap()
            .is_empty()
    );
    assert!(
        fixture
            .acquisition
            .acquire(fixture.request.clone())
            .await
            .is_err()
    );
    assert_eq!(fixture.wire.fetches.load(Ordering::SeqCst), 1);
    assert!(
        !fixture
            .acquisition
            .services
            .records
            .path()
            .join(format!(
                "objects-{}/source.json",
                fixture.request.effect_id().unwrap()
            ))
            .exists()
    );
}
#[tokio::test]
async fn changed_final_authority_preserves_import_without_minting_a_source_or_replaying() {
    let mut fixture = fixture().await;
    let original = fixture.acquisition.services.current.clone();
    let complete = fixture.acquisition.services.records.path().join(format!(
        "objects-{}/quarantine/repository.git/refs/kegari/objects/{}",
        fixture.request.effect_id().unwrap(),
        fixture.request.revision.commit
    ));
    fixture.acquisition.services.current = Arc::new(move |_| {
        if complete.exists() {
            Err(Error::new(
                "stale_generation",
                "fixture correction arrived after import",
            ))
        } else {
            Ok(())
        }
    });
    assert_eq!(
        fixture
            .acquisition
            .acquire(fixture.request.clone())
            .await
            .err()
            .unwrap()
            .code,
        "stale_generation"
    );
    let id = fixture.request.effect_id().unwrap();
    let receipt = fixture.runtime.control.operation(id).unwrap();
    assert_eq!(receipt.phase, ReceiptPhase::Acknowledged);
    assert_eq!(
        receipt.observed_result.unwrap()["effect"],
        "exited_evidence_rejected"
    );
    fixture.acquisition.services.current = original;
    assert!(
        fixture
            .acquisition
            .acquire(fixture.request.clone())
            .await
            .is_err()
    );
    assert_eq!(fixture.wire.fetches.load(Ordering::SeqCst), 1);
    assert!(
        fixture
            .acquisition
            .services
            .records
            .path()
            .join(format!(
                "objects-{id}/quarantine/repository.git/objects/pack"
            ))
            .is_dir()
    );
}

#[tokio::test]
async fn actual_squash_import_retains_reviewed_head_and_reproduces_exact_delta() {
    let fixture = fixture().await;
    let f = &fixture.wire.fixture;
    let squash = String::from_utf8(fixture_git(
        &f.binary,
        &f.root,
        &[
            "commit-tree",
            &f.revision.tree,
            "-p",
            &f.parent,
            "-m",
            "fixture: actual squash",
        ],
        None,
    ))
    .unwrap()
    .trim()
    .to_owned();
    assert_ne!(squash, f.revision.commit);
    let expected = BTreeMap::from([
        (squash.clone(), f.revision.tree.clone()),
        (f.revision.commit.clone(), f.revision.tree.clone()),
    ]);
    let wire = fixture_git(
        &f.binary,
        &f.root,
        &["upload-pack", "--stateless-rpc", f.root.to_str().unwrap()],
        Some(&pack::fetch_request(&expected.keys().cloned().collect()).unwrap()),
    );
    let validated = pack::validate_pack(
        pack::extract_pack(&wire, &ObjectLimits::default()).unwrap(),
        &ObjectLimits::default(),
    )
    .unwrap();
    assert!(validated.objects.contains(&squash));
    assert!(validated.objects.contains(&f.revision.commit));
    let quarantine = PrivateDir::create(
        &fixture
            ._temp
            .path()
            .canonicalize()
            .unwrap()
            .join("two-roots"),
    )
    .unwrap();
    let source = quarantine.path().join("repository.git");
    let expected_for_import = expected.clone();
    let git = Git::from_pinned_executable(&f.binary).unwrap();
    git.start_operation(
        crate::git::GitLimits::default(),
        Arc::new(|spawn| spawn()),
        move |owned| {
            owned.import_verified_object_pack(
                &quarantine,
                &validated.bytes,
                &validated.objects,
                &expected_for_import,
            )
        },
    )
    .unwrap()
    .wait()
    .await
    .unwrap();
    let git = Git::new(&f.binary).unwrap();
    let scope = crate::git::WriteScope {
        paths: vec![
            "AGENTS.md".into(),
            "binary.dat".into(),
            "run.sh".into(),
            "link".into(),
        ],
    };
    let generation = bytes_digest(b"fixture-definition");
    let correction = bytes_digest(b"fixture-correction");
    let capture = || crate::git::CaptureDelta {
        repository: fixture.request.repository().clone(),
        parent_id: Uuid::from_u128(100),
        child_id: Uuid::from_u128(101),
        work_generation: &generation,
        correction_generation: &correction,
        scope: scope.clone(),
        base: &f.parent,
        head: &f.revision.commit,
        staging_parent: &f.parent,
        staging_commit: &squash,
    };
    let original = git.capture_delta(&f.root, capture()).unwrap();
    let acquired = git.capture_delta(&source, capture()).unwrap();
    assert_eq!(acquired.manifest, original.manifest);
    assert_eq!(acquired.patch, original.patch);
    assert_eq!(
        git.exact_revision(&source, &squash).unwrap().tree,
        f.revision.tree
    );
    // Omitting the reviewed head is detectably incomplete even though the
    // squash has the same tree and complete production ancestry.
    assert!(
        !git.is_ancestor(&source, &f.revision.commit, &squash)
            .unwrap()
    );
    assert_eq!(
        fs::read_to_string(f.root.join("base.txt")).unwrap(),
        "uncommitted operator work\n"
    );
}

#[tokio::test]
async fn stopped_retained_discovery_requires_exact_receipt_account_and_unchanged_objects() {
    let fixture = fixture().await;
    let acquired = fixture
        .acquisition
        .acquire(fixture.request.clone())
        .await
        .unwrap();
    let records =
        PrivateDir::open(&fixture._temp.path().canonicalize().unwrap().join("objects")).unwrap();
    // Force changes intent and fences all original permits; discovery has no
    // worker permit, does no network, and creates no process or directory.
    let snapshot = fixture.runtime.control.snapshot().unwrap();
    fixture
        .runtime
        .control
        .lifecycle(
            MutationRequest {
                request_id: Uuid::new_v4(),
                runtime_id: snapshot.intent.runtime_id,
                actor: Actor {
                    identity: "fixture-operator".into(),
                    principal: None,
                    roles: BTreeSet::from(["lifecycle".into()]),
                },
                kind: "lifecycle".into(),
                canonical_targets: vec![snapshot.intent.runtime_id.to_string()],
                expected: BTreeMap::new(),
                payload: json!(null),
            },
            crate::control::Lifecycle::ForceStop,
            snapshot.intent.generation,
        )
        .unwrap();
    let before = fixture
        .acquisition
        .services
        .processes
        .observe(fixture.request.permit.operation_id)
        .unwrap()
        .len();
    let list = retained_sources(
        &records,
        &fixture.runtime.control,
        &fixture.acquisition.services.key,
        &fixture.acquisition.services.github,
        std::slice::from_ref(&fixture.request.contract),
    )
    .unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].digest().unwrap(), acquired.digest().unwrap());
    assert_eq!(
        fixture.runtime.control.snapshot().unwrap().intent.desired,
        crate::control::Intent::Stopped
    );
    assert_eq!(fixture.wire.fetches.load(Ordering::SeqCst), 1);
    assert_eq!(
        fixture
            .acquisition
            .services
            .processes
            .observe(fixture.request.permit.operation_id)
            .unwrap()
            .len(),
        before
    );
    let mut configuration = fixture.runtime.configuration.snapshot().unwrap();
    let workspace = configuration.workspaces["example"].id;
    let connection = configuration.workspaces["example"]
        .github_connection
        .clone();
    configuration
        .connections
        .get_mut(&connection)
        .unwrap()
        .account_id = "7".into();
    configuration
        .connections
        .get_mut(&connection)
        .unwrap()
        .endpoint = Some("https://api.github.com/".into());
    let root = list[0]
        .host_read_root(&configuration, workspace, &fixture.request.contract)
        .unwrap();
    root.revalidate_filesystem().unwrap();
    configuration
        .connections
        .get_mut(&connection)
        .unwrap()
        .account_id = "another-account".into();
    assert_eq!(
        list[0]
            .host_read_root(&configuration, workspace, &fixture.request.contract)
            .unwrap_err()
            .code,
        "scope_violation"
    );
    let signed: Signed<SourceRecord> = serde_json::from_slice(
        &list[0]
            .directory
            .read_bytes("source.json", 256 * 1024)
            .unwrap(),
    )
    .unwrap();
    let mut wrong = signed.payload;
    wrong.effect = Uuid::new_v4();
    list[0]
        .directory
        .write_bytes(
            "source.json",
            &serde_json::to_vec(
                &fixture
                    .acquisition
                    .services
                    .key
                    .sign("verified_object_source", &wrong)
                    .unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(
        retained_sources(
            &records,
            &fixture.runtime.control,
            &fixture.acquisition.services.key,
            &fixture.acquisition.services.github,
            std::slice::from_ref(&fixture.request.contract)
        )
        .err()
        .unwrap()
        .code,
        "reconciliation_required"
    );
    list[0]
        .directory
        .write_bytes(
            "source.json",
            &serde_json::to_vec(
                &fixture
                    .acquisition
                    .services
                    .key
                    .sign("verified_object_source", &list[0].record)
                    .unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    fs::write(
        acquired.root().join("config"),
        "[core]\nrepositoryformatversion=0\nbare=true\n# changed retained bytes\n",
    )
    .unwrap();
    assert_eq!(
        retained_sources(
            &records,
            &fixture.runtime.control,
            &fixture.acquisition.services.key,
            &fixture.acquisition.services.github,
            std::slice::from_ref(&fixture.request.contract)
        )
        .err()
        .unwrap()
        .code,
        "ownership_conflict"
    );
}

#[tokio::test]
async fn acquired_clone_preserves_operator_attribution_and_uses_fetched_instructions() {
    let fixture = fixture().await;
    let source = fixture
        .acquisition
        .acquire(fixture.request.clone())
        .await
        .unwrap();
    let root = fixture._temp.path().canonicalize().unwrap();
    let execution = PrivateDir::create(&root.join("acquired-execution")).unwrap();
    let projection = fixture.runtime.observations.snapshot().unwrap();
    let work = projection
        .parent(Uuid::from_u128(2), Uuid::from_u128(100))
        .unwrap()
        .work
        .as_ref()
        .unwrap();
    let mut work = work.clone();
    work.child = Some(Uuid::from_u128(101));
    let identity = crate::execution::DirectoryIdentity {
        schema_version: 1,
        runtime_id: fixture.request.permit.lease.runtime_id,
        directory_id: Uuid::new_v4(),
        repository: source.repository().clone(),
        work,
        repository_path: execution.path().join("repository"),
        operator_checkout: PathBuf::from(&fixture.request.contract.configuration.source),
    };
    fs::write(
        fixture.wire.fixture.root.join("AGENTS.md"),
        "stale operator-only instructions\n",
    )
    .unwrap();
    let git = Git::new(&fixture.wire.fixture.binary).unwrap();
    let prepared = git
        .prepare_acquired_execution_clone(&source, execution.path(), &identity)
        .unwrap();
    assert_eq!(
        fs::read_to_string(prepared.join("AGENTS.md")).unwrap(),
        "Only this fetched revision's instructions apply.\n"
    );
    let record: serde_json::Value = serde_json::from_slice(
        &fs::read(
            execution
                .path()
                .join(format!("prepared-clone-{}.json", identity.directory_id)),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(record["source"], json!(source.root()));
    assert_eq!(
        record["identity"]["operator_checkout"],
        json!(identity.operator_checkout)
    );
    assert_eq!(record["source_provenance"], json!(source.digest().unwrap()));
    fs::write(prepared.join("base.txt"), "owned unfinished work\n").unwrap();
    assert_eq!(
        git.prepare_acquired_execution_clone(&source, execution.path(), &identity)
            .unwrap(),
        prepared
    );
    assert_eq!(
        fs::read_to_string(prepared.join("base.txt")).unwrap(),
        "owned unfinished work\n"
    );
    assert!(
        git.prepare_execution_clone(
            &identity.operator_checkout,
            execution.path(),
            &identity,
            source.revision()
        )
        .is_err()
    );
    assert_eq!(
        fs::read_to_string(fixture.wire.fixture.root.join("AGENTS.md")).unwrap(),
        "stale operator-only instructions\n"
    );
    let review_root = PrivateDir::create(&root.join("ungranted-review")).unwrap();
    let review = crate::git::IndependentCloneIdentity {
        schema_version: 1,
        runtime_id: identity.runtime_id,
        workspace_id: Uuid::from_u128(2),
        directory_id: Uuid::new_v4(),
        repository: identity.repository.clone(),
        pull_request: 4,
        head: source.revision().clone(),
        base: git
            .exact_revision(source.root(), &fixture.wire.fixture.parent)
            .unwrap(),
        review_identity: canonical_digest(1,&json!({"repository":42,"pr":4,"head":source.revision().commit,"base":fixture.wire.fixture.parent})).unwrap(),
        repository_path: review_root.path().join("repository"),
        operator_checkout: identity.operator_checkout,
    };
    assert_eq!(
        git.prepare_acquired_independent_review_clone(&source, review_root.path(), &review)
            .unwrap_err()
            .code,
        "scope_violation"
    );
    assert!(!review.repository_path.exists());
}

#[tokio::test]
async fn admitted_fork_review_acquires_exact_head_and_base_into_independent_clone() {
    let mut fixture = fixture().await;
    let f = &fixture.wire.fixture;
    let git = Git::new(&f.binary).unwrap();
    let base = git.exact_revision(&f.root, &f.parent).unwrap();
    let pr = crate::providers::PullRequest {
        repository_id: 42,
        id: 900,
        number: 4,
        author_id: 91,
        state: "open".into(),
        draft: false,
        head_sha: f.revision.commit.clone(),
        base_sha: base.commit.clone(),
        head_ref: "fork-work".into(),
        base_ref: "main".into(),
        head_repository_id: Some(99),
        merged: Some(false),
        merge_commit_sha: None,
        url: "https://github.com/fixture/repo/pull/4".into(),
    };
    fixture.wire.api_values.lock().unwrap().insert("/repos/fixture/repo/pulls/4".into(),json!({"id":pr.id,"number":pr.number,"title":"Fixture independent review","body":null,"user":{"id":pr.author_id},"state":pr.state,"draft":pr.draft,"head":{"sha":pr.head_sha,"ref":pr.head_ref,"repo":{"id":99}},"base":{"sha":pr.base_sha,"ref":pr.base_ref,"repo":{"id":42}},"merged":false,"merge_commit_sha":null,"html_url":pr.url}));
    fixture.wire.api_values.lock().unwrap().insert(
        format!("/repos/fixture/repo/git/commits/{}", base.commit),
        json!({"sha":base.commit,"tree":{"sha":base.tree},"parents":[]}),
    );
    *fixture.wire.expected_wants.lock().unwrap() = Some(BTreeSet::from([
        base.commit.clone(),
        f.revision.commit.clone(),
    ]));
    let mut projection = fixture.runtime.observations.snapshot().unwrap();
    let workspace = projection.workspaces.get_mut(&Uuid::from_u128(2)).unwrap();
    workspace.independent_review_profile = Some(
        fixture.runtime.configuration.snapshot().unwrap().workspaces["example"]
            .profiles
            .coordination
            .clone(),
    );
    let review_identity = canonical_digest(
        1,
        &json!({"repository":42,"pr":4,"head":pr.head_sha,"base":pr.base_sha}),
    )
    .unwrap();
    workspace.independent_jobs = vec![crate::reconcile::IndependentJob {
        workspace_id: Uuid::from_u128(2),
        repository_id: 42,
        pull_request: pr.clone(),
        pull_request_text: Some(crate::providers::PullRequestText {
            title: "Fixture independent review".into(),
            body: None,
        }),
        review_identity: review_identity.clone(),
        phase: "candidate_requires_admission".into(),
    }];
    let gate = projection
        .independent_gate(
            Uuid::from_u128(2),
            42,
            4,
            chrono::Utc::now(),
            chrono::Duration::seconds(30),
        )
        .unwrap();
    let authority = WorkerAuthority::Independent(gate.clone());
    let permit = fixture
        .runtime
        .control
        .admit(
            &fixture.request.permit.lease,
            Uuid::new_v4(),
            authority.work_key().unwrap(),
            canonical_digest(1, &"fixture independent preparation").unwrap(),
            || Ok(()),
        )
        .unwrap();
    let request = AuthorizedObjectRequest::for_worker(
        &fixture.request.contract,
        f.revision.clone(),
        &permit,
        &authority,
    )
    .unwrap();
    let expected = authority.digest().unwrap();
    fixture.acquisition.services.current = Arc::new(move |request| {
        if request.authority_digest() == expected {
            Ok(())
        } else {
            Err(Error::new("stale_result", "fixture review changed"))
        }
    });
    let source = fixture.acquisition.acquire(request).await.unwrap();
    assert_eq!(source.required_revision(&base.commit), Some(base.clone()));
    let root = PrivateDir::create(
        &fixture
            ._temp
            .path()
            .canonicalize()
            .unwrap()
            .join("independent-acquired"),
    )
    .unwrap();
    let identity = crate::git::IndependentCloneIdentity {
        schema_version: 1,
        runtime_id: permit.lease.runtime_id,
        workspace_id: gate.workspace_id(),
        directory_id: Uuid::new_v4(),
        repository: source.repository().clone(),
        pull_request: 4,
        head: source.revision().clone(),
        base,
        review_identity,
        repository_path: root.path().join("repository"),
        operator_checkout: PathBuf::from(&fixture.request.contract.configuration.source),
    };
    git.prepare_acquired_independent_review_clone(&source, root.path(), &identity)
        .unwrap();
    let proof = git
        .observe_acquired_independent_review_clone(&source, root.path(), &identity)
        .unwrap();
    assert_eq!(proof.identity(), &identity);
    proof.revalidate_filesystem().unwrap();
    assert_eq!(fixture.wire.fetches.load(Ordering::SeqCst), 1);
    assert_eq!(
        fs::read_to_string(identity.repository_path.join("AGENTS.md")).unwrap(),
        "Only this fetched revision's instructions apply.\n"
    );
    assert!(
        git.observe_independent_review_clone(root.path(), &identity)
            .is_err()
    );
}

#[tokio::test]
async fn stopped_cache_miss_performs_no_network_process_or_storage_creation() {
    let fixture = fixture().await;
    let snapshot = fixture.runtime.control.snapshot().unwrap();
    fixture
        .runtime
        .control
        .lifecycle(
            MutationRequest {
                request_id: Uuid::new_v4(),
                runtime_id: snapshot.intent.runtime_id,
                actor: Actor {
                    identity: "fixture-operator".into(),
                    principal: None,
                    roles: BTreeSet::from(["lifecycle".into()]),
                },
                kind: "lifecycle".into(),
                canonical_targets: vec![snapshot.intent.runtime_id.to_string()],
                expected: BTreeMap::new(),
                payload: json!(null),
            },
            crate::control::Lifecycle::ForceStop,
            snapshot.intent.generation,
        )
        .unwrap();
    let root = fixture.acquisition.services.records.path();
    assert_eq!(fs::read_dir(root).unwrap().count(), 0);
    assert_eq!(
        fixture
            .acquisition
            .acquire(fixture.request.clone())
            .await
            .err()
            .unwrap()
            .code,
        "access_denied"
    );
    assert_eq!(fixture.wire.requests.load(Ordering::SeqCst), 0);
    assert_eq!(fs::read_dir(root).unwrap().count(), 0);
    assert!(
        fixture
            .acquisition
            .services
            .processes
            .operations()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn retained_discovery_rejects_removed_changed_receipts_and_changed_pinned_binary() {
    let fixture = fixture().await;
    let acquired = fixture
        .acquisition
        .acquire(fixture.request.clone())
        .await
        .unwrap();
    let records =
        PrivateDir::open(&fixture._temp.path().canonicalize().unwrap().join("objects")).unwrap();
    let discover = || {
        retained_sources(
            &records,
            &fixture.runtime.control,
            &fixture.acquisition.services.key,
            &fixture.acquisition.services.github,
            std::slice::from_ref(&fixture.request.contract),
        )
    };
    let effect = fixture.request.effect_id().unwrap();
    let receipts = fixture
        .runtime
        .services
        .registration
        .root()
        .child("receipts")
        .unwrap();
    let leaf = format!("{effect}.json");
    let bytes = receipts.read_bytes(&leaf, 1024 * 1024).unwrap();
    fs::remove_file(receipts.path().join(&leaf)).unwrap();
    assert_eq!(discover().err().unwrap().code, "record_missing");
    receipts.write_bytes(&leaf, &bytes).unwrap();
    let original = fixture.runtime.control.operation(effect).unwrap();
    let mut changed = original.clone();
    changed.observed_result =
        Some(json!({"source_digest":bytes_digest(b"different acknowledged source")}));
    receipts.write(&leaf, &changed).unwrap();
    assert_eq!(discover().err().unwrap().code, "reconciliation_required");
    receipts.write(&leaf, &original).unwrap();
    assert_eq!(
        discover().unwrap()[0].digest().unwrap(),
        acquired.digest().unwrap()
    );
    // Fixture binary is an owned copy; replacing it cannot affect host Git.
    fs::set_permissions(
        &fixture.wire.fixture.binary,
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(&fixture.wire.fixture.binary)
        .unwrap();
    file.write_all(b"changed owned fixture executable").unwrap();
    drop(file);
    fs::set_permissions(
        &fixture.wire.fixture.binary,
        fs::Permissions::from_mode(0o555),
    )
    .unwrap();
    assert_eq!(discover().err().unwrap().code, "identity_mismatch");
    assert_eq!(fixture.wire.fetches.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn delta_recovery_selects_exact_retained_roots_instead_of_latest_source() {
    let fixture = fixture().await;
    let git = Git::new(&fixture.wire.fixture.binary).unwrap();
    let base = git
        .exact_revision(&fixture.wire.fixture.root, &fixture.wire.fixture.parent)
        .unwrap();
    fixture.wire.api_values.lock().unwrap().insert(
        format!("/repos/fixture/repo/git/commits/{}", base.commit),
        json!({"sha":base.commit,"tree":{"sha":base.tree},"parents":[]}),
    );
    let mut request = fixture.request.clone();
    request
        .required_revisions
        .insert(base.commit.clone(), base.tree.clone());
    *fixture.wire.expected_wants.lock().unwrap() =
        Some(request.required_revisions.keys().cloned().collect());
    let complete = fixture.acquisition.acquire(request).await.unwrap();
    *fixture.wire.expected_wants.lock().unwrap() = None;
    let incomplete = fixture
        .acquisition
        .acquire(fixture.request.clone())
        .await
        .unwrap();

    let generation = bytes_digest(b"fixture work");
    let correction = bytes_digest(b"fixture correction");
    let delta = git
        .capture_delta(
            &fixture.wire.fixture.root,
            crate::git::CaptureDelta {
                repository: fixture.request.repository().clone(),
                parent_id: Uuid::from_u128(100),
                child_id: Uuid::from_u128(101),
                work_generation: &generation,
                correction_generation: &correction,
                scope: crate::git::WriteScope {
                    paths: vec![
                        "AGENTS.md".into(),
                        "binary.dat".into(),
                        "run.sh".into(),
                        "link".into(),
                    ],
                },
                base: &base.commit,
                head: &fixture.wire.fixture.revision.commit,
                staging_parent: &base.commit,
                staging_commit: &fixture.wire.fixture.revision.commit,
            },
        )
        .unwrap();
    assert!(!incomplete.contains_delta(&delta.manifest).unwrap());
    assert!(complete.contains_delta(&delta.manifest).unwrap());
    let expected = complete.digest().unwrap();
    let selected = select_delta_source(vec![incomplete, complete], &git, &delta.manifest)
        .unwrap()
        .unwrap();
    assert_eq!(selected.digest().unwrap(), expected);
    let mut missing = delta.manifest.clone();
    missing.validated_base = Revision {
        commit: "e".repeat(40),
        tree: "f".repeat(40),
    };
    assert!(
        select_delta_source(vec![selected], &git, &missing)
            .unwrap()
            .is_none()
    );
    assert_eq!(fixture.wire.fetches.load(Ordering::SeqCst), 2);
}

/// Actual local Git and actual owned Control/process journal; fixture provider
/// bytes only. This exercises source acquisition/absence mechanics, not a
/// carried acceptance or a complete native parent workflow.
#[tokio::test]
async fn exact_missing_object_probe_uses_owned_scope_and_never_fetches_on_query() {
    let fixture = fixture().await;
    let source = fixture
        .acquisition
        .acquire(fixture.request.clone())
        .await
        .unwrap();
    let control = fixture.runtime.control.clone();
    let held = fixture.request.permit.clone();
    let current = fixture.acquisition.services.current.clone();
    let request = fixture.request.clone();
    let scope = crate::git::GitExecutionScope {
        limits: GitLimits::default(),
        start_gate: Arc::new(move |spawn| {
            control.validate_permit(&held, || {
                current(&request)?;
                spawn()
            })
        }),
        processes: fixture.acquisition.services.processes.clone(),
        permit: fixture.request.permit.clone(),
        executions: fixture.runtime.executions.clone(),
    };
    let base = fixture.acquisition.services.git.clone();
    let git = tokio::task::spawn_blocking(move || base.with_execution_scope(scope))
        .await
        .unwrap()
        .unwrap();
    let fetches = fixture.wire.fetches.load(Ordering::SeqCst);
    let before = fixture
        .acquisition
        .services
        .processes
        .observe(fixture.request.permit.operation_id)
        .unwrap()
        .len();
    let expected = fixture.wire.fixture.revision.clone();
    assert!(
        git.contains_exact_revision(source.root(), &expected)
            .unwrap()
    );
    let absent = Revision {
        commit: "f".repeat(40),
        tree: expected.tree.clone(),
    };
    assert!(!git.contains_exact_revision(source.root(), &absent).unwrap());
    let wrong_tree = Revision {
        commit: expected.commit.clone(),
        tree: "f".repeat(40),
    };
    assert_eq!(
        git.contains_exact_revision(source.root(), &wrong_tree)
            .unwrap_err()
            .code,
        "provenance_mismatch"
    );
    let blob = Revision {
        commit: expected.tree.clone(),
        tree: expected.tree.clone(),
    };
    assert_eq!(
        git.contains_exact_revision(source.root(), &blob)
            .unwrap_err()
            .code,
        "git_operation_failed"
    );
    let invalid = Revision {
        commit: "HEAD".into(),
        tree: expected.tree,
    };
    assert!(
        git.contains_exact_revision(source.root(), &invalid)
            .is_err()
    );
    assert_eq!(fixture.wire.fetches.load(Ordering::SeqCst), fetches);
    let after = fixture
        .acquisition
        .services
        .processes
        .observe(fixture.request.permit.operation_id)
        .unwrap();
    assert!(after.len() > before);
    assert!(after.iter().all(|r| matches!(
        r.state,
        crate::process_records::RecoveryState::OwnedGroupExitRecorded
    )));
    assert!(
        fixture
            .runtime
            .executions
            .active_operations()
            .unwrap()
            .is_empty()
    );
    source.revalidate_filesystem().unwrap();
}

/// The source is produced by the existing actual owned Git import. This tests
/// immutable source history only, not completed carry/native worker authority.
#[tokio::test]
async fn retained_read_history_requires_closed_owner_and_exact_protected_import() {
    let fixture = fixture().await;
    let source = Arc::new(
        fixture
            .acquisition
            .acquire(fixture.request.clone())
            .await
            .unwrap(),
    );
    let control = fixture.runtime.control.clone();
    let key = fixture.acquisition.services.key.clone();
    let error = source
        .retain_read_history(control.clone(), key.clone())
        .err()
        .unwrap();
    assert_eq!(error.code, "source_history_unavailable");
    assert_eq!(
        error.message,
        "retained import requires its actually closed owner"
    );
    // acquire awaited the actual import and journal join; only now retire its
    // real owner. No model completion or native confinement claim is constructed.
    control
        .retire_local(
            fixture.request.permit.operation_id,
            json!({"fixture":"actual object import joined"}),
        )
        .unwrap();
    let proof = source.retain_read_history(control.clone(), key).unwrap();
    let digest = proof.digest().unwrap();
    let operations = canonical_digest(1, &control.operations().unwrap()).unwrap();
    let fetches = fixture.wire.fetches.load(Ordering::SeqCst);
    let requests = fixture.wire.requests.load(Ordering::SeqCst);
    let processes = canonical_digest(
        1,
        &fixture
            .acquisition
            .services
            .processes
            .observe(fixture.request.permit.operation_id)
            .unwrap(),
    )
    .unwrap();
    proof.validate().unwrap();
    // This is the exact already-serialized context used by final carry gates.
    // The validator must not reenter Control snapshot or open a subprocess.
    control.serialize_observation(|_| proof.validate()).unwrap();

    for leaf in ["source.json", "intent.json", "objects.json"] {
        let bytes = source.directory.read_bytes(leaf, 8 * 1024 * 1024).unwrap();
        fs::remove_file(source.directory.path().join(leaf)).unwrap();
        let error = proof.validate().unwrap_err();
        assert_eq!(error.code, "record_missing");
        assert_eq!(error.message, "required control record is missing");
        source.directory.write_bytes(leaf, &bytes).unwrap();
        source.directory.write_bytes(leaf, b"{}").unwrap();
        let error = proof.validate().unwrap_err();
        assert_eq!(error.code, "source_history_unavailable");
        assert_eq!(error.message, "retained import file changed");
        source.directory.write_bytes(leaf, &bytes).unwrap();
        proof.validate().unwrap();
    }
    let receipts = fixture
        .runtime
        .services
        .registration
        .root()
        .child("receipts")
        .unwrap();
    for operation in [
        fixture.request.effect_id().unwrap(),
        fixture.request.permit.operation_id,
    ] {
        let leaf = format!("{operation}.json");
        let bytes = receipts.read_bytes(&leaf, 1024 * 1024).unwrap();
        let original = control.operation(operation).unwrap();
        fs::remove_file(receipts.path().join(&leaf)).unwrap();
        assert_eq!(proof.validate().unwrap_err().code, "record_missing");
        receipts.write_bytes(&leaf, &bytes).unwrap();
        let mut changed = original;
        changed.observed_result = Some(json!({"fixture":"substituted terminal record"}));
        receipts.write(&leaf, &changed).unwrap();
        let error = proof.validate().unwrap_err();
        assert_eq!(error.code, "source_history_unavailable");
        assert_eq!(error.message, "retained import receipt or owner changed");
        receipts.write_bytes(&leaf, &bytes).unwrap();
        proof.validate().unwrap();
    }
    let config_path = source.root().join("config");
    let source_config = fs::read(&config_path).unwrap();
    let mut changed = source_config.clone();
    changed.extend_from_slice(b"\n# changed owned quarantine after observation\n");
    fs::write(&config_path, changed).unwrap();
    let error = proof.validate().unwrap_err();
    assert_eq!(error.code, "ownership_conflict");
    assert_eq!(error.message, "acquired Git source was replaced or changed");
    fs::write(&config_path, source_config).unwrap();
    proof.validate().unwrap();
    assert_eq!(proof.digest().unwrap(), digest);
    assert_eq!(
        canonical_digest(1, &control.operations().unwrap()).unwrap(),
        operations
    );
    assert_eq!(fixture.wire.fetches.load(Ordering::SeqCst), fetches);
    assert_eq!(fixture.wire.requests.load(Ordering::SeqCst), requests);
    assert_eq!(
        canonical_digest(
            1,
            &fixture
                .acquisition
                .services
                .processes
                .observe(fixture.request.permit.operation_id)
                .unwrap()
        )
        .unwrap(),
        processes,
    );
}
