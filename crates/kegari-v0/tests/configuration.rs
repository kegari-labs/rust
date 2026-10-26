//! Real private storage/control serialization with labeled provider/Keychain fixtures.
//! No native Keychain, account login, provider network or agent process is invoked.
use kegari_v0::{
    api::{CommandName, CommandRequest},
    codex::CodexConnectionMetadata,
    config::{Config, Connection},
    configuration::{ConfigManager, ConfigurationInputs},
    control::{Actor, ControlHost, Intent, Lifecycle, MutationRequest, Principal, ReceiptPhase},
    error::{Error, Result},
    platform::{
        BinaryIdentity, HostIdentity, ProcessIdentity, ProcessObservation, ProcessProbe,
        Registration,
    },
    providers::{HttpRequest, HttpResponse, HttpTransport},
    secrets::{Secret, SecretStore},
    transport::TransportAuthentication,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    future::Future,
    path::Path,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use uuid::Uuid;
fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
struct Probe;
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
    fn observe(&self, p: &ProcessIdentity) -> Result<ProcessObservation> {
        Ok(if *p == self.current()? {
            ProcessObservation::Running
        } else {
            ProcessObservation::DifferentIdentity
        })
    }
    fn matching_executable(&self, _: &Path) -> Result<Vec<ProcessIdentity>> {
        Ok(vec![self.current()?])
    }
}
#[derive(Default)]
struct Store {
    values: Mutex<BTreeMap<String, Vec<u8>>>,
    writes: AtomicUsize,
    fail: AtomicBool,
}
impl SecretStore for Store {
    fn read(&self, name: &str) -> Result<Secret> {
        self.values
            .lock()
            .unwrap()
            .get(name)
            .cloned()
            .ok_or_else(|| Error::new("missing_secret", "fixture secret absent"))
            .and_then(Secret::new)
    }
    fn save(&self, name: &str, secret: &Secret) -> Result<()> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        if self.fail.load(Ordering::SeqCst) {
            return Err(Error::new(
                "authentication_required",
                "fixture Keychain locked",
            ));
        }
        self.values
            .lock()
            .unwrap()
            .insert(name.into(), secret.bytes().to_vec());
        Ok(())
    }
}
struct Wire {
    account: AtomicUsize,
    calls: AtomicUsize,
    wait: AtomicBool,
    started: tokio::sync::Notify,
    resume: tokio::sync::Notify,
}
impl Default for Wire {
    fn default() -> Self {
        Self {
            account: AtomicUsize::new(7),
            calls: AtomicUsize::new(0),
            wait: AtomicBool::new(false),
            started: tokio::sync::Notify::new(),
            resume: tokio::sync::Notify::new(),
        }
    }
}
impl HttpTransport for Wire {
    fn send(
        &self,
        request: HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Result<HttpResponse>> + Send + '_>> {
        Box::pin(async move {
            assert_eq!(request.url.as_str(), "https://github.invalid/user");
            assert!(request.body.is_empty());
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.wait.swap(false, Ordering::SeqCst) {
                self.started.notify_one();
                self.resume.notified().await;
            }
            Ok(HttpResponse{status:200,headers:Default::default(),body:serde_json::to_vec(&json!({"id":self.account.load(Ordering::SeqCst),"login":"fixture","type":"User"})).unwrap()})
        })
    }
}
struct Inputs {
    wire: Arc<Wire>,
    environment: Mutex<BTreeMap<String, String>>,
}
impl ConfigurationInputs for Inputs {
    fn environment(&self, name: &str) -> Option<String> {
        self.environment.lock().unwrap().get(name).cloned()
    }
    fn transport(&self, _: &str) -> Option<Arc<dyn HttpTransport>> {
        Some(self.wire.clone())
    }
    fn codex<'a>(
        &'a self,
        _: &'a Connection,
        _: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<CodexConnectionMetadata>> + Send + 'a>> {
        Box::pin(async {
            Err(Error::new(
                "authentication_required",
                "fixture Codex login is absent",
            ))
        })
    }
}
struct Fixture {
    _directory: tempfile::TempDir,
    registration: Arc<Registration>,
    control: Arc<ControlHost>,
    manager: Arc<ConfigManager>,
    store: Arc<Store>,
    inputs: Arc<Inputs>,
    actor: Actor,
}
impl Fixture {
    fn new() -> Self {
        Self::with_configuration(|_| {})
    }
    fn with_configuration(modify: impl FnOnce(&mut Config)) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path().canonicalize().unwrap();
        let uid = unsafe { libc::geteuid() };
        let host = HostIdentity {
            host_id: "fixture-host".into(),
            boot_id: id(1),
            uid,
            user: "fixture".into(),
            home: home.clone(),
        };
        let (registration, control) = Registration::bootstrap_with_probe(
            &host,
            id(2),
            BinaryIdentity::inspect(&std::env::current_exe().unwrap(), false).unwrap(),
            Uuid::new_v4(),
            Arc::new(Probe),
        )
        .unwrap();
        let mut config:Config=serde_json::from_value(json!({"schema_version":1,"runtime":{"id":id(2),"host_id":"fixture-host","execution_root":home.join("runs"),"max_agents":2},"connections":{},"workspaces":{},"control":{"local_users":{uid.to_string():{"id":"fixture-operator","linear_user_id":id(3),"roles":["inspect","configure","lifecycle"]}}}})).unwrap();
        modify(&mut config);
        registration.complete_setup(&config).unwrap();
        let registration = Arc::new(registration);
        let control = Arc::new(control);
        let store = Arc::new(Store::default());
        let inputs = Arc::new(Inputs {
            wire: Arc::new(Wire::default()),
            environment: Mutex::new(BTreeMap::new()),
        });
        let auth = Arc::new(TransportAuthentication::new(config.control.clone(), None));
        let manager = Arc::new(
            ConfigManager::with_inputs(
                config,
                registration.clone(),
                control.clone(),
                store.clone(),
                auth,
                inputs.clone(),
            )
            .unwrap(),
        );
        Self {
            _directory: directory,
            registration,
            control,
            manager,
            store,
            inputs,
            actor: Actor {
                identity: "fixture-operator".into(),
                principal: Some(Principal::Unix { uid }),
                roles: ["inspect".into(), "configure".into(), "lifecycle".into()].into(),
            },
        }
    }
    fn request(&self, command: CommandName, args: Value) -> CommandRequest {
        CommandRequest {
            api_version: 1,
            runtime_id: id(2),
            request_id: Uuid::new_v4(),
            command,
            args,
            expected_config_revision: Some(self.manager.snapshot().unwrap().revision().unwrap()),
            expected_control_generation: Some(self.control.snapshot().unwrap().intent.generation),
        }
    }
    fn add(&self) -> CommandRequest {
        self.request(CommandName::ConnectionAdd,json!({"name":"github","connection":"kind = 'github'\naccount_id = '7'\nendpoint = 'https://github.invalid/'"}))
    }
    fn stored(&self) -> Config {
        Config::parse(
            std::str::from_utf8(
                &self
                    .registration
                    .root()
                    .read_bytes("config.toml", 8 * 1024 * 1024)
                    .unwrap(),
            )
            .unwrap(),
        )
        .unwrap()
    }
}

#[tokio::test]
async fn verified_secret_import_commits_atomically_and_exact_replay_does_not_probe_again() {
    let f = Fixture::new();
    let request = f.add();
    let receipt = f
        .manager
        .execute(
            f.actor.clone(),
            request.clone(),
            Some(Secret::new(b"sentinel-secret".to_vec()).unwrap()),
        )
        .await
        .unwrap();
    assert_eq!(receipt.phase, ReceiptPhase::Acknowledged);
    assert_eq!(f.store.writes.load(Ordering::SeqCst), 1);
    let current = f.manager.snapshot().unwrap();
    assert_eq!(current.revision().unwrap(), f.stored().revision().unwrap());
    let reference = current.connections["github"]
        .credential_ref
        .as_ref()
        .unwrap();
    assert!(reference.starts_with("keychain:import-"));
    assert_eq!(
        f.store
            .read(reference.strip_prefix("keychain:").unwrap())
            .unwrap()
            .bytes(),
        b"sentinel-secret"
    );
    let replay = f
        .manager
        .execute(
            f.actor.clone(),
            request.clone(),
            Some(Secret::new(b"sentinel-secret".to_vec()).unwrap()),
        )
        .await
        .unwrap();
    assert_eq!(replay.operation_id, receipt.operation_id);
    assert_eq!(f.inputs.wire.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        f.manager
            .execute(
                f.actor.clone(),
                request,
                Some(Secret::new(b"different-secret".to_vec()).unwrap())
            )
            .await
            .unwrap_err()
            .code,
        "request_conflict"
    );
    assert_eq!(f.store.writes.load(Ordering::SeqCst), 1);
    let persisted = serde_json::to_string(&receipt).unwrap();
    assert!(!persisted.contains("sentinel-secret"));
    assert!(!persisted.contains(&kegari_v0::digest::bytes_digest(b"sentinel-secret")));
}

#[tokio::test]
async fn wrong_identity_and_stale_configuration_write_no_secret_or_config() {
    let f = Fixture::new();
    let before = f.stored().revision().unwrap();
    f.inputs.wire.account.store(8, Ordering::SeqCst);
    assert_eq!(
        f.manager
            .execute(
                f.actor.clone(),
                f.add(),
                Some(Secret::new(b"secret".to_vec()).unwrap())
            )
            .await
            .unwrap_err()
            .code,
        "identity_mismatch"
    );
    assert_eq!(f.store.writes.load(Ordering::SeqCst), 0);
    assert_eq!(f.stored().revision().unwrap(), before);
    f.inputs.wire.account.store(7, Ordering::SeqCst);
    let mut request = f.add();
    request.expected_config_revision = Some("sha256:v1:wrong".into());
    assert_eq!(
        f.manager
            .execute(
                f.actor.clone(),
                request,
                Some(Secret::new(b"secret".to_vec()).unwrap())
            )
            .await
            .unwrap_err()
            .code,
        "stale_config"
    );
    assert_eq!(f.store.writes.load(Ordering::SeqCst), 0);
    assert_eq!(f.stored().revision().unwrap(), before);
}

#[tokio::test]
async fn keychain_failure_preserves_old_configuration_and_requires_reconciliation() {
    let f = Fixture::new();
    let before = f.stored().revision().unwrap();
    f.store.fail.store(true, Ordering::SeqCst);
    let request = f.add();
    let id = request.request_id;
    assert_eq!(
        f.manager
            .execute(
                f.actor.clone(),
                request,
                Some(Secret::new(b"secret".to_vec()).unwrap())
            )
            .await
            .unwrap_err()
            .code,
        "operation_uncertain"
    );
    assert_eq!(f.stored().revision().unwrap(), before);
    assert_eq!(f.manager.snapshot().unwrap().revision().unwrap(), before);
    assert_eq!(f.control.operation(id).unwrap().phase, ReceiptPhase::Sent);
    assert!(
        !f.control
            .snapshot()
            .unwrap()
            .unresolved_operations
            .is_empty()
    );
}

#[tokio::test]
async fn running_identity_replacement_is_rejected_before_keychain_write() {
    let f = Fixture::new();
    let start = MutationRequest {
        request_id: Uuid::new_v4(),
        runtime_id: id(2),
        actor: f.actor.clone(),
        kind: "start".into(),
        canonical_targets: vec![id(2).to_string()],
        expected: BTreeMap::new(),
        payload: json!({}),
    };
    f.control.lifecycle(start, Lifecycle::Start, 0).unwrap();
    assert_eq!(
        f.control.snapshot().unwrap().intent.desired,
        Intent::Running
    );
    assert_eq!(
        f.manager
            .execute(
                f.actor.clone(),
                f.add(),
                Some(Secret::new(b"secret".to_vec()).unwrap())
            )
            .await
            .unwrap_err()
            .code,
        "configuration_busy"
    );
    assert_eq!(f.store.writes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn principal_role_revocation_during_preflight_is_rechecked_inside_commit_gate() {
    let f = Fixture::new();
    f.inputs.wire.wait.store(true, Ordering::SeqCst);
    let manager = f.manager.clone();
    let actor = f.actor.clone();
    let request = f.add();
    let pending = tokio::spawn(async move {
        manager
            .execute(
                actor,
                request,
                Some(Secret::new(b"secret".to_vec()).unwrap()),
            )
            .await
    });
    f.inputs.wire.started.notified().await;
    let mut replacement = f.manager.snapshot().unwrap();
    replacement
        .control
        .local_users
        .values_mut()
        .next()
        .unwrap()
        .roles = [kegari_v0::config::ControlRole::Inspect].into();
    let request = f.request(
        CommandName::Setup,
        json!({"configuration":toml::to_string(&replacement).unwrap()}),
    );
    f.manager
        .execute(f.actor.clone(), request, None)
        .await
        .unwrap();
    f.inputs.wire.resume.notify_one();
    assert_eq!(pending.await.unwrap().unwrap_err().code, "access_denied");
    assert_eq!(f.store.writes.load(Ordering::SeqCst), 0);
    assert!(f.manager.snapshot().unwrap().connections.is_empty());
}

#[tokio::test]
async fn environment_import_is_frozen_and_replay_detects_changed_secret() {
    let f = Fixture::new();
    f.inputs
        .environment
        .lock()
        .unwrap()
        .insert("GH_TOKEN".into(), "environment-secret".into());
    let mut request = f.add();
    request.args["secret_reference"] = json!("$GH_TOKEN");
    f.manager
        .execute(f.actor.clone(), request.clone(), None)
        .await
        .unwrap();
    assert_eq!(f.store.writes.load(Ordering::SeqCst), 1);
    f.inputs
        .environment
        .lock()
        .unwrap()
        .insert("GH_TOKEN".into(), "changed-secret".into());
    assert_eq!(
        f.manager
            .execute(f.actor.clone(), request, None)
            .await
            .unwrap_err()
            .code,
        "request_conflict"
    );
    assert_eq!(f.store.writes.load(Ordering::SeqCst), 1);
    assert!(
        !toml::to_string(&f.stored())
            .unwrap()
            .contains("environment-secret")
    );
}

#[tokio::test]
async fn metadata_check_is_read_only_and_does_not_claim_operational_qualification() {
    let f = Fixture::new();
    f.manager
        .execute(
            f.actor.clone(),
            f.add(),
            Some(Secret::new(b"secret".to_vec()).unwrap()),
        )
        .await
        .unwrap();
    let before = f.stored().revision().unwrap();
    let writes = f.store.writes.load(Ordering::SeqCst);
    let report = f.manager.check(None).await.unwrap();
    assert_eq!(report["status"], "verified");
    assert_eq!(report["operational_qualification"], "unverified");
    assert_eq!(f.store.writes.load(Ordering::SeqCst), writes);
    assert_eq!(f.stored().revision().unwrap(), before);
}

#[tokio::test]
async fn unbound_or_different_os_principal_cannot_configure_even_with_claimed_role() {
    let f = Fixture::new();
    let mut actor = f.actor.clone();
    actor.principal = None;
    assert_eq!(
        f.manager
            .execute(
                actor,
                f.add(),
                Some(Secret::new(b"secret".to_vec()).unwrap())
            )
            .await
            .unwrap_err()
            .code,
        "access_denied"
    );
    let mut actor = f.actor.clone();
    actor.principal = Some(Principal::Unix { uid: u32::MAX });
    assert_eq!(
        f.manager
            .execute(
                actor,
                f.add(),
                Some(Secret::new(b"secret".to_vec()).unwrap())
            )
            .await
            .unwrap_err()
            .code,
        "access_denied"
    );
    assert_eq!(f.inputs.wire.calls.load(Ordering::SeqCst), 0);
}

fn queue(f: &Fixture, request: &CommandRequest, secret: &[u8]) -> kegari_v0::control::Receipt {
    let mut mutation =
        kegari_v0::app::mutation_request(&f.actor, request, vec![request.runtime_id.to_string()]);
    mutation.kind = format!("command/{:?}", request.command);
    mutation.payload =
        json!({"arguments":request.args,"secret_digest":kegari_v0::digest::bytes_digest(secret)});
    let (receipt, fresh) = f
        .control
        .queue_command(mutation, "configure", || Ok(()))
        .unwrap();
    assert!(fresh);
    receipt
}

#[tokio::test]
async fn background_effect_requires_exact_durable_command_lineage() {
    let f = Fixture::new();
    let mut request = f.add();
    let parent = queue(&f, &request, b"secret");
    request.request_id = kegari_v0::app::command_effect_id(parent.request_id);
    let mut changed = request.clone();
    changed.args["name"] = json!("changed");
    assert_eq!(
        f.manager
            .execute_for_command(
                f.actor.clone(),
                changed,
                Some(Secret::new(b"secret".to_vec()).unwrap()),
                parent.clone()
            )
            .await
            .unwrap_err()
            .code,
        "request_conflict"
    );
    assert_eq!(
        f.manager
            .execute_for_command(
                f.actor.clone(),
                request.clone(),
                Some(Secret::new(b"changed-secret".to_vec()).unwrap()),
                parent.clone()
            )
            .await
            .unwrap_err()
            .code,
        "request_conflict"
    );
    let mut unrelated = request.clone();
    unrelated.request_id = Uuid::new_v4();
    assert_eq!(
        f.manager
            .execute_for_command(
                f.actor.clone(),
                unrelated,
                Some(Secret::new(b"secret".to_vec()).unwrap()),
                parent.clone()
            )
            .await
            .unwrap_err()
            .code,
        "request_conflict"
    );
    assert_eq!(f.inputs.wire.calls.load(Ordering::SeqCst), 0);
    let effect = f
        .manager
        .execute_for_command(
            f.actor.clone(),
            request,
            Some(Secret::new(b"secret".to_vec()).unwrap()),
            parent.clone(),
        )
        .await
        .unwrap();
    assert_eq!(
        effect.expected["parent_command_id"],
        parent.request_id.to_string()
    );
    assert_eq!(
        effect.expected["parent_payload_digest"],
        parent.payload_digest
    );
    assert_eq!(effect.phase, ReceiptPhase::Acknowledged);
    assert_eq!(f.store.writes.load(Ordering::SeqCst), 1);
    let completed = f
        .control
        .complete_command(
            parent.request_id,
            Ok(json!({"effect_operation_id":effect.operation_id,"receipt":effect})),
        )
        .unwrap();
    assert_eq!(completed.phase, ReceiptPhase::Reconciled);
    assert!(f.control.snapshot().unwrap().background_commands.is_empty());
}

#[tokio::test]
async fn completed_command_cannot_later_start_an_effect() {
    let f = Fixture::new();
    let mut request = f.add();
    let parent = queue(&f, &request, b"secret");
    f.control
        .complete_command(
            parent.request_id,
            Err(Error::new(
                "invalid_input",
                "fixture cancellation before verification",
            )),
        )
        .unwrap();
    request.request_id = kegari_v0::app::command_effect_id(parent.request_id);
    assert_eq!(
        f.manager
            .execute_for_command(
                f.actor.clone(),
                request,
                Some(Secret::new(b"secret".to_vec()).unwrap()),
                parent
            )
            .await
            .unwrap_err()
            .code,
        "request_conflict"
    );
    assert_eq!(f.inputs.wire.calls.load(Ordering::SeqCst), 0);
    assert_eq!(f.store.writes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn same_account_reconnect_reuses_secret_and_preserves_revision() {
    let f = Fixture::new();
    f.manager
        .execute(
            f.actor.clone(),
            f.add(),
            Some(Secret::new(b"secret".to_vec()).unwrap()),
        )
        .await
        .unwrap();
    let before = f.manager.snapshot().unwrap().revision().unwrap();
    let request = f.request(CommandName::ConnectionReconnect, json!({"name":"github"}));
    f.manager
        .execute(f.actor.clone(), request, None)
        .await
        .unwrap();
    assert_eq!(f.manager.snapshot().unwrap().revision().unwrap(), before);
    assert_eq!(f.store.writes.load(Ordering::SeqCst), 1);
    assert_eq!(f.inputs.wire.calls.load(Ordering::SeqCst), 2);
}

fn prerequisite_workspace() -> kegari_v0::config::Workspace {
    let sha = "0123456789012345678901234567890123456789";
    let proof = json!({"workflow_ref":"fixture/repo/.github/workflows/ci.yml@refs/heads/trusted","workflow_sha":sha,"job_workflow_ref":format!("fixture/trust/.github/workflows/proof.yml@{sha}"),"job_workflow_sha":sha,"event_name":"workflow_dispatch"});
    serde_json::from_value(json!({
        "id":id(10),"linear_workspace_id":id(11),"linear_connection":"linear","github_connection":"github","codex_connection":"codex","workflow":"/private/tmp/fixture.md",
        "linear":{"teams":"all","projects":"all","users":"all","authorization_label_id":id(12),"states":{}},
        "github":{"repositories":["101"]},"actors":{"authorizers":[id(13)],"correctors":[id(13)],"acceptors":[id(13)]},
        "profiles":{"coordination":{"tool":"codex","model":"fixture","reasoning":"high","speed":"standard"}},
        "routing_labels":{"tool":{id(20).to_string():"codex"},"model":{id(21).to_string():"fixture"},"reasoning":{id(22).to_string():"high"},"speed":{id(23).to_string():"standard"}},
        "max_agents":1,"release":{"mode":"manual","spacing_seconds":0},
        "repositories":{"101":{"source":"/private/tmp/fixture","local_checks":["local"],"integrated_checks":["e2e"],"ci":[{"workflow_id":1,"required_checks":["suite"],"proof":proof}],"production_ci":[{"workflow_id":1,"required_checks":["suite"],"proof":proof}],"no_change_validation":{"workflow_id":1,"workflow_revision":sha,"source_input":"source_sha","operation_input":"operation_id","observed_source_output":"tested_sha","proof":proof},"deployment":"none"}}
    })).unwrap()
}
struct PrerequisiteWire {
    fault: &'static str,
    paths: Mutex<Vec<String>>,
}
impl HttpTransport for PrerequisiteWire {
    fn send(
        &self,
        request: HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Result<HttpResponse>> + Send + '_>> {
        Box::pin(async move {
            assert!(matches!(request.method, kegari_v0::providers::Method::Get));
            assert!(request.body.is_empty());
            let path = request.url.path();
            self.paths.lock().unwrap().push(path.into());
            let sha = "0123456789012345678901234567890123456789";
            let repository = json!({"id":101,"name":"repo","owner":{"id":7,"login":"fixture","type":"User"},"default_branch":"main","private":true,"archived":false,"disabled":false,"allow_squash_merge":true,"permissions":{"pull":true,"push":self.fault!="permission"}});
            let value = match path {
                "/user" => json!({"id":7,"login":"fixture","type":"User"}),
                "/user/repos" => {
                    if self.fault == "missing" {
                        json!([])
                    } else {
                        json!([repository])
                    }
                }
                "/repos/fixture/repo" => repository,
                "/repos/fixture/repo/actions/workflows" => {
                    json!({"total_count":1,"workflows":[{"id":1,"path":".github/workflows/ci.yml","state":if self.fault=="workflow"{"disabled_manually"}else{"active"}}]})
                }
                p if p.starts_with("/repos/fixture/repo/git/commits/") => {
                    json!({"sha":sha,"tree":{"sha":sha},"parents":[]})
                }
                p if p.starts_with("/repos/fixture/repo/git/ref/heads/") => {
                    json!({"ref":format!("refs/heads/{}",p.rsplit('/').next().unwrap()),"object":{"type":"commit","sha":sha}})
                }
                p if p.ends_with("/protection") => {
                    json!({"required_status_checks":{"strict":self.fault!="strict","contexts":["suite","kegari/codex-review"]},"required_pull_request_reviews":{"required_approving_review_count":if self.fault=="reviews"{1}else{0},"require_code_owner_reviews":false}})
                }
                p if p.contains("/rules/branches/") => json!([]),
                _ => panic!("unexpected read path {path}"),
            };
            Ok(HttpResponse {
                status: 200,
                headers: Default::default(),
                body: serde_json::to_vec(&value).unwrap(),
            })
        })
    }
}

#[tokio::test]
async fn repository_prerequisites_read_exact_branches_workflows_and_policy_without_effects() {
    let workspace = prerequisite_workspace();
    let wire = Arc::new(PrerequisiteWire {
        fault: "",
        paths: Mutex::new(vec![]),
    });
    let client = kegari_v0::providers::GithubClient::with_transport(
        kegari_v0::providers::HttpPolicy::new(
            "https://github.invalid/",
            "7".into(),
            std::time::Duration::from_secs(2),
        )
        .unwrap(),
        workspace.github.clone(),
        kegari_v0::providers::Credential::new("fixture".into()).unwrap(),
        Some(wire.clone()),
    )
    .unwrap();
    let result = kegari_v0::configuration::repository_prerequisites(&client, &workspace)
        .await
        .unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0]["branches"].as_array().unwrap().len(), 2);
    assert_eq!(result[0]["trusted_reusable_execution"], "unverified");
    let paths = wire.paths.lock().unwrap();
    for path in [
        "/repos/fixture/repo/branches/main/protection",
        "/repos/fixture/repo/branches/staging/protection",
        "/repos/fixture/repo/rules/branches/main",
        "/repos/fixture/repo/rules/branches/staging",
        "/repos/fixture/repo/git/commits/0123456789012345678901234567890123456789",
    ] {
        assert!(
            paths.iter().any(|p| p == path),
            "missing actual read {path}"
        );
    }
}

#[tokio::test]
async fn repository_prerequisites_block_missing_access_workflows_and_incompatible_policy() {
    for fault in ["missing", "permission", "workflow", "strict", "reviews"] {
        let workspace = prerequisite_workspace();
        let wire = Arc::new(PrerequisiteWire {
            fault,
            paths: Mutex::new(vec![]),
        });
        let client = kegari_v0::providers::GithubClient::with_transport(
            kegari_v0::providers::HttpPolicy::new(
                "https://github.invalid/",
                "7".into(),
                std::time::Duration::from_secs(2),
            )
            .unwrap(),
            workspace.github.clone(),
            kegari_v0::providers::Credential::new("fixture".into()).unwrap(),
            Some(wire),
        )
        .unwrap();
        assert!(
            kegari_v0::configuration::repository_prerequisites(&client, &workspace)
                .await
                .is_err(),
            "{fault} was accepted"
        );
    }
}

#[tokio::test]
async fn invalid_update_fences_known_role_reduction_across_restart_until_explicit_scope_resolution()
{
    let f = Fixture::new();
    let before = f.manager.snapshot().unwrap();
    let mut proposed = before.clone();
    proposed
        .control
        .local_users
        .values_mut()
        .next()
        .unwrap()
        .roles
        .remove(&kegari_v0::config::ControlRole::Lifecycle);
    proposed.runtime.max_agents = 0; // Independent invalid execution setting.
    let request = f.request(
        CommandName::Setup,
        json!({"configuration":toml::to_string(&proposed).unwrap()}),
    );
    assert!(
        f.manager
            .execute(f.actor.clone(), request.clone(), None)
            .await
            .is_err()
    );
    assert_eq!(
        f.manager.snapshot().unwrap().revision().unwrap(),
        before.revision().unwrap()
    );
    assert_eq!(
        f.manager
            .authority_fences()
            .unwrap()
            .check_actor(&f.actor, "lifecycle")
            .unwrap_err()
            .code,
        "access_denied"
    );
    assert!(
        f.manager
            .authority_fences()
            .unwrap()
            .check_actor(&f.actor, "configure")
            .is_ok()
    );
    assert_eq!(f.store.writes.load(Ordering::SeqCst), 0);
    let recovered = ConfigManager::with_inputs(
        before.clone(),
        f.registration.clone(),
        f.control.clone(),
        f.store.clone(),
        Arc::new(TransportAuthentication::new(before.control.clone(), None)),
        f.inputs.clone(),
    )
    .unwrap();
    assert!(
        recovered
            .authority_fences()
            .unwrap()
            .check_actor(&f.actor, "lifecycle")
            .is_err()
    );
    // An unrelated valid connection import cannot restore the revoked role.
    f.manager
        .execute(
            f.actor.clone(),
            f.add(),
            Some(Secret::new(b"fixture".to_vec()).unwrap()),
        )
        .await
        .unwrap();
    assert!(
        f.manager
            .authority_fences()
            .unwrap()
            .check_actor(&f.actor, "lifecycle")
            .is_err()
    );
    let mut resolved = f.manager.snapshot().unwrap();
    resolved
        .control
        .local_users
        .values_mut()
        .next()
        .unwrap()
        .roles
        .remove(&kegari_v0::config::ControlRole::Lifecycle);
    let update = f.request(
        CommandName::Setup,
        json!({"configuration":toml::to_string(&resolved).unwrap()}),
    );
    f.manager
        .execute(f.actor.clone(), update, None)
        .await
        .unwrap();
    assert!(
        f.manager
            .authority_fences()
            .unwrap()
            .denied_roles
            .is_empty()
    );
    assert!(
        !f.manager
            .snapshot()
            .unwrap()
            .control
            .local_users
            .values()
            .next()
            .unwrap()
            .roles
            .contains(&kegari_v0::config::ControlRole::Lifecycle)
    );
}

#[tokio::test]
async fn unknown_runtime_effect_does_not_preserve_reliably_revoked_permission() {
    let f = Fixture::new();
    let unknown = MutationRequest {
        request_id: Uuid::new_v4(),
        runtime_id: id(2),
        actor: f.actor.clone(),
        kind: "fixture_unknown_effect".into(),
        canonical_targets: vec![id(2).to_string()],
        expected: BTreeMap::new(),
        payload: json!({}),
    };
    assert!(
        f.control
            .mutate(
                unknown,
                None,
                Some("configure"),
                || Ok(()),
                |_| Err(Error::new("timeout", "fixture acknowledgement lost"))
            )
            .is_err()
    );
    let mut next = f.manager.snapshot().unwrap();
    next.control
        .local_users
        .values_mut()
        .next()
        .unwrap()
        .roles
        .remove(&kegari_v0::config::ControlRole::Lifecycle);
    next.runtime.max_agents = 0;
    let request = f.request(
        CommandName::Setup,
        json!({"configuration":toml::to_string(&next).unwrap()}),
    );
    assert!(
        f.manager
            .execute(f.actor.clone(), request, None)
            .await
            .is_err()
    );
    assert!(
        f.manager
            .authority_fences()
            .unwrap()
            .check_actor(&f.actor, "lifecycle")
            .is_err()
    );
    assert!(
        f.control
            .operations()
            .unwrap()
            .iter()
            .any(|r| r.kind == "permission_fence" && r.phase == ReceiptPhase::Acknowledged)
    );
    assert!(
        !f.control
            .snapshot()
            .unwrap()
            .unresolved_operations
            .is_empty()
    );
}

#[tokio::test]
async fn running_capacity_reload_applies_to_future_work_without_identity_replacement() {
    let f = Fixture::new();
    f.control
        .lifecycle(
            MutationRequest {
                request_id: Uuid::new_v4(),
                runtime_id: id(2),
                actor: f.actor.clone(),
                kind: "start".into(),
                canonical_targets: vec![id(2).to_string()],
                expected: BTreeMap::new(),
                payload: json!({}),
            },
            Lifecycle::Start,
            0,
        )
        .unwrap();
    let mut next = f.manager.snapshot().unwrap();
    next.runtime.max_agents = 1;
    next.runtime.max_turns = 9;
    let request = f.request(
        CommandName::Setup,
        json!({"configuration":toml::to_string(&next).unwrap()}),
    );
    f.manager
        .execute(f.actor.clone(), request.clone(), None)
        .await
        .unwrap();
    f.manager
        .execute(f.actor.clone(), request, None)
        .await
        .unwrap();
    assert_eq!(f.manager.snapshot().unwrap().runtime.max_turns, 9);
    assert_eq!(
        f.control.snapshot().unwrap().intent.desired,
        Intent::Running
    );
}

#[tokio::test]
async fn missing_persisted_fence_provenance_cannot_restore_old_roles_after_restart() {
    let f = Fixture::new();
    let before = f.manager.snapshot().unwrap();
    let mut next = before.clone();
    next.control
        .local_users
        .values_mut()
        .next()
        .unwrap()
        .roles
        .remove(&kegari_v0::config::ControlRole::Lifecycle);
    next.runtime.max_agents = 0;
    let request = f.request(
        CommandName::Setup,
        json!({"configuration":toml::to_string(&next).unwrap()}),
    );
    assert!(
        f.manager
            .execute(f.actor.clone(), request, None)
            .await
            .is_err()
    );
    // Simulates a prior file surviving an interrupted replacement. No broad file deletion.
    f.registration
        .root()
        .write(
            "permission-fences.json",
            &json!({"schema_version":1,"runtime_id":id(2),"entries":{},"resolutions":{}}),
        )
        .unwrap();
    let result = ConfigManager::with_inputs(
        before.clone(),
        f.registration.clone(),
        f.control.clone(),
        f.store.clone(),
        Arc::new(TransportAuthentication::new(before.control, None)),
        f.inputs.clone(),
    );
    assert_eq!(result.err().unwrap().code, "permission_fenced");
}

fn workspace_fixture_config() -> Config {
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

#[tokio::test]
async fn invalid_workspace_update_cannot_preserve_known_actor_repository_or_resource_permission() {
    for reduction in ["authorizer", "repository", "resource", "connection"] {
        let f = Fixture::with_configuration(|config| {
            let fixture = workspace_fixture_config();
            config.connections = fixture.connections;
            config.workspaces = fixture.workspaces;
            config
                .workspaces
                .get_mut("example")
                .unwrap()
                .test_resources
                .insert(
                    "scratch-fixture".into(),
                    kegari_v0::config::TestResource {
                        path: "/private/tmp/test-resource".into(),
                        writable: true,
                        disposable: true,
                    },
                );
        });
        let mut ws = f.manager.snapshot().unwrap().workspaces["example"].clone();
        match reduction {
            "authorizer" => {
                ws.actors.authorizers = [id(999)].into();
            }
            "repository" => ws.github.repositories = vec!["999".into()],
            "resource" => {
                ws.test_resources
                    .get_mut("scratch-fixture")
                    .unwrap()
                    .writable = false
            }
            "connection" => ws.linear_connection = "removed-account".into(),
            _ => unreachable!(),
        }
        ws.max_agents = 0;
        let request = f.request(
            CommandName::WorkspaceConfigure,
            json!({"workspace":"example","workspace_configuration":toml::to_string(&ws).unwrap()}),
        );
        assert!(
            f.manager
                .execute(f.actor.clone(), request, None)
                .await
                .is_err(),
            "{reduction}"
        );
        assert_eq!(
            f.manager
                .authority_fences()
                .unwrap()
                .check_workspace(ws.id)
                .unwrap_err()
                .code,
            "permission_fenced",
            "{reduction}"
        );
        assert_eq!(f.inputs.wire.calls.load(Ordering::SeqCst), 0);
        assert_eq!(f.store.writes.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn managed_policy_changes_fence_even_when_another_field_is_invalid() {
    let policy: kegari_v0::config::ManagedExecution = serde_json::from_value(json!({
        "lifecycle":"foreground_v1",
        "executables":[{"path":"/private/tmp/declared-tool","digest":kegari_v0::digest::bytes_digest(b"fixture declaration only")}]
    })).unwrap();
    for change in [
        "added",
        "removed",
        "executable",
        "read_root",
        "generated",
        "unchanged",
    ] {
        let f = Fixture::with_configuration(|config| {
            let fixture = workspace_fixture_config();
            config.connections = fixture.connections;
            config.workspaces = fixture.workspaces;
            if change != "added" {
                config
                    .workspaces
                    .get_mut("example")
                    .unwrap()
                    .managed_execution = Some(policy.clone());
            }
        });
        let before = f.manager.snapshot().unwrap().revision().unwrap();
        let mut workspace = f.manager.snapshot().unwrap().workspaces["example"].clone();
        match change {
            "added" => workspace.managed_execution = Some(policy.clone()),
            "removed" => workspace.managed_execution = None,
            "executable" => {
                workspace.managed_execution.as_mut().unwrap().executables[0].digest =
                    kegari_v0::digest::bytes_digest(b"different declared tool")
            }
            "read_root" => workspace
                .managed_execution
                .as_mut()
                .unwrap()
                .read_roots
                .push(kegari_v0::config::ManagedReadRoot {
                    path: "/private/tmp/declared-sdk".into(),
                    directory: true,
                    content: None,
                }),
            "generated" => {
                workspace
                    .managed_execution
                    .as_mut()
                    .unwrap()
                    .generated_executables =
                    Some(kegari_v0::config::ManagedGeneratedExecutables::ScratchBuildV1)
            }
            "unchanged" => {}
            _ => unreachable!(),
        }
        workspace.max_agents = 0;
        let request = f.request(
            CommandName::WorkspaceConfigure,
            json!({"workspace":"example","workspace_configuration":toml::to_string(&workspace).unwrap()}),
        );
        assert!(
            f.manager
                .execute(f.actor.clone(), request, None)
                .await
                .is_err(),
            "{change}"
        );
        let fenced = f
            .manager
            .authority_fences()
            .unwrap()
            .check_workspace(workspace.id);
        if change == "unchanged" {
            fenced.unwrap();
        } else {
            assert_eq!(fenced.unwrap_err().code, "permission_fenced", "{change}");
        }
        assert_eq!(f.manager.snapshot().unwrap().revision().unwrap(), before);
        assert_eq!(f.stored().revision().unwrap(), before);
        assert_eq!(f.inputs.wire.calls.load(Ordering::SeqCst), 0);
        assert_eq!(f.store.writes.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn earlier_success_cannot_resolve_a_later_reduction_and_resolution_provenance_is_checked() {
    let f = Fixture::new();
    let current = f.manager.snapshot().unwrap();
    let good = f.request(
        CommandName::Setup,
        json!({"configuration":toml::to_string(&current).unwrap()}),
    );
    f.manager
        .execute(f.actor.clone(), good, None)
        .await
        .unwrap();
    let mut next = current.clone();
    next.control
        .local_users
        .values_mut()
        .next()
        .unwrap()
        .roles
        .remove(&kegari_v0::config::ControlRole::Lifecycle);
    next.runtime.max_agents = 0;
    let request = f.request(
        CommandName::Setup,
        json!({"configuration":toml::to_string(&next).unwrap()}),
    );
    assert!(
        f.manager
            .execute(f.actor.clone(), request.clone(), None)
            .await
            .is_err()
    );
    assert!(
        f.manager
            .authority_fences()
            .unwrap()
            .check_actor(&f.actor, "lifecycle")
            .is_err()
    );
    let mut stored: Value = f
        .registration
        .root()
        .read("permission-fences.json")
        .unwrap();
    // An altered resolution cannot claim that an earlier successful update covered this later proposal.
    stored["resolutions"]
        .as_object_mut()
        .unwrap()
        .values_mut()
        .next()
        .unwrap()["proposals"] = json!([request.request_id]);
    f.registration
        .root()
        .write("permission-fences.json", &stored)
        .unwrap();
    let result = ConfigManager::with_inputs(
        current.clone(),
        f.registration.clone(),
        f.control.clone(),
        f.store.clone(),
        Arc::new(TransportAuthentication::new(current.control, None)),
        f.inputs.clone(),
    );
    assert_eq!(result.err().unwrap().code, "permission_fenced");
}
