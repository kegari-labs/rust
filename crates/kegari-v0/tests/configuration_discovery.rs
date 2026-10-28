//! Configuration admission through actual typed/provider verifiers and private
//! transactions. Provider wire, Codex metadata and process identity are injected;
//! no native probe, model, Keychain write, service or network action is performed.
use kegari_config::{Config, Connection};
use kegari_digest::canonical_digest;
use kegari_platform::{
    BinaryIdentity, HostIdentity, ProcessIdentity, ProcessObservation, ProcessProbe,
};
use kegari_providers::HttpTransport;
use kegari_providers::{HttpRequest, HttpResponse, Method};
use kegari_secrets::{Secret, SecretStore};
use kegari_types::api::{CommandName, CommandRequest};
use kegari_types::error::{Error, Result};
use kegari_v0::config::{config_revision, load_workflows};
use kegari_v0::{
    codex::{CodexConnectionMetadata, PROTOCOL_PROFILE, SUPPORTED_CODEX_VERSION},
    configuration::{ConfigManager, ConfigurationInputs, prepare_initial},
    control::{Actor, Principal, ReceiptPhase},
    platform::Registration,
    transport::TransportAuthentication,
};
use serde_json::{Value, json};
use std::{
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use uuid::Uuid;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

#[derive(Clone, Copy)]
enum Observation {
    Readable,
    ReadRevoked,
    WrongAccount,
}
struct Wire {
    observation: Observation,
    paths: Mutex<Vec<String>>,
}
impl HttpTransport for Wire {
    fn send(
        &self,
        request: HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Result<HttpResponse>> + Send + '_>> {
        Box::pin(async move {
            self.paths.lock().unwrap().push(request.url.to_string());
            let value = match request.url.host_str().unwrap() {
                "linear.invalid" => {
                    assert_eq!(request.method, Method::Post);
                    let body: Value = serde_json::from_slice(&request.body).unwrap();
                    assert_eq!(
                        body["query"],
                        "query KegariIdentity { viewer { id name } organization { id } }"
                    );
                    assert_eq!(body["variables"], json!({}));
                    json!({"data":{"viewer":{"id":id(5),"name":"Fixture"},"organization":{"id":id(3)}}})
                }
                "github.invalid" => {
                    assert_eq!(request.method, Method::Get);
                    assert!(request.body.is_empty());
                    match request.url.path() {
                        "/user" => {
                            json!({"id":if matches!(self.observation, Observation::WrongAccount) {8} else {7},"login":"fixture","type":"User"})
                        }
                        "/user/repos" => {
                            json!([{"id":42,"name":"repo","owner":{"id":7,"login":"fixture","type":"User"},"default_branch":"main","private":true,"archived":false,"disabled":false,"allow_squash_merge":false,"permissions":{"pull":!matches!(self.observation, Observation::ReadRevoked),"push":false}}])
                        }
                        path => panic!("unexpected downstream provider request: {path}"),
                    }
                }
                host => panic!("unexpected provider: {host}"),
            };
            Ok(HttpResponse {
                status: 200,
                headers: Default::default(),
                body: serde_json::to_vec(&value).unwrap(),
            })
        })
    }
}
#[derive(Default)]
struct Store {
    saves: AtomicUsize,
}
impl SecretStore for Store {
    fn read(&self, name: &str) -> Result<Secret> {
        assert!(matches!(name, "linear-fixture" | "github-fixture"));
        Secret::new(b"injected-fixture-credential".to_vec())
    }
    fn save(&self, _: &str, _: &Secret) -> Result<()> {
        self.saves.fetch_add(1, Ordering::SeqCst);
        Err(Error::new(
            "fixture_unexpected_write",
            "discovery must not save a secret",
        ))
    }
}
struct Inputs {
    wire: Arc<Wire>,
    root: PathBuf,
    codex_calls: AtomicUsize,
}
impl ConfigurationInputs for Inputs {
    fn environment(&self, _: &str) -> Option<String> {
        None
    }
    fn transport(&self, name: &str) -> Option<Arc<dyn HttpTransport>> {
        assert!(matches!(name, "linear" | "github"));
        Some(self.wire.clone())
    }
    fn codex<'a>(
        &'a self,
        connection: &'a Connection,
        probe_root: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<CodexConnectionMetadata>> + Send + 'a>> {
        Box::pin(async move {
            assert!(probe_root.starts_with(&self.root));
            assert!(
                !probe_root.exists(),
                "injected metadata must not create a native probe"
            );
            self.codex_calls.fetch_add(1, Ordering::SeqCst);
            Ok(CodexConnectionMetadata {
                supported_version: SUPPORTED_CODEX_VERSION.into(),
                protocol_profile: PROTOCOL_PROFILE.into(),
                account_id: "codex-fixture".into(),
                auth_home: self.root.join("auth"),
                permission_digest: canonical_digest(1, &"injected permission metadata")?,
                argument_digest: Some(canonical_digest(1, &connection.argv)?),
                model_catalog: vec![
                    json!({"model":"fixture-model","supportedReasoningEfforts":[{"reasoningEffort":"high"}]}),
                ],
                rate_limits: json!({}),
                observed_at: chrono::Utc::now().to_rfc3339(),
            })
        })
    }
}
struct Fixture {
    _temp: tempfile::TempDir,
    config: Config,
    host: HostIdentity,
    inputs: Arc<Inputs>,
    store: Arc<Store>,
}
impl Fixture {
    fn new(observation: Observation) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        std::fs::create_dir(root.join("source")).unwrap();
        std::fs::create_dir(root.join("auth")).unwrap();
        let profile =
            json!({"tool":"codex","model":"fixture-model","reasoning":"high","speed":"standard"});
        // The four delivery declarations and the workflow are genuinely absent
        // from the input, before parsing, validation or authority construction.
        let value = json!({"schema_version":1,"runtime":{"id":id(1),"host_id":"discovery-fixture","execution_root":root.join("runs"),"max_agents":2},
            "connections":{
                "linear":{"kind":"linear","account_id":id(5),"endpoint":"https://linear.invalid/graphql","credential_ref":"keychain:linear-fixture"},
                "github":{"kind":"github","account_id":"7","endpoint":"https://github.invalid/","credential_ref":"keychain:github-fixture"},
                "codex":{"kind":"codex","account_id":"codex-fixture","auth_home":root.join("auth"),"max_agents":1}
            },
            "workspaces":{"example":{"id":id(2),"linear_workspace_id":id(3),"linear_connection":"linear","github_connection":"github","codex_connection":"codex",
                "linear":{"teams":"all","projects":"all","users":"all","authorization_label_id":id(4),"states":{id(10).to_string():{"prepared":id(11),"running":id(12),"blocked":id(13),"child_done":id(14),"parent_accepted":id(15),"cancelled":id(16)}}},
                "github":{"organizations":[],"repositories":["42"],"pr_authors":[]},
                "actors":{"authorizers":[id(5)],"correctors":[id(5)],"acceptors":[id(5)]},
                "profiles":{"coordination":profile},"routing_labels":{"tool":{id(20).to_string():"codex"},"model":{id(21).to_string():"fixture-model"},"reasoning":{id(22).to_string():"high"},"speed":{id(23).to_string():"standard"}},
                "max_agents":1,"release":{"mode":"manual","spacing_seconds":0},"repositories":{"42":{"source":root.join("source")}}
            }},
            "control":{"local_users":{"501":{"id":"fixture-operator","linear_user_id":id(5),"roles":["inspect","configure","lifecycle"]}}}
        });
        let config: Config = serde_json::from_value(value).unwrap();
        let config = Config::parse(&toml::to_string(&config).unwrap()).unwrap();
        let host = HostIdentity {
            host_id: "discovery-fixture".into(),
            boot_id: id(90),
            uid: 501,
            user: "fixture".into(),
            home: root.clone(),
        };
        let inputs = Arc::new(Inputs {
            wire: Arc::new(Wire {
                observation,
                paths: Mutex::new(vec![]),
            }),
            root,
            codex_calls: AtomicUsize::new(0),
        });
        Self {
            _temp: temp,
            config,
            host,
            inputs,
            store: Arc::new(Store::default()),
        }
    }
    fn assert_observation_only(&self) {
        assert_eq!(self.store.saves.load(Ordering::SeqCst), 0);
        assert!(self.inputs.codex_calls.load(Ordering::SeqCst) > 0);
        assert!(self.inputs.wire.paths.lock().unwrap().iter().all(|url| {
            [
                "https://linear.invalid/graphql",
                "https://github.invalid/user",
            ]
            .contains(&url.as_str())
                || url.starts_with("https://github.invalid/user/repos?")
        }));
    }
}
fn assert_discovery_report(report: &Value) {
    assert_eq!(report["configuration_admission"], "verified");
    assert_eq!(report["status"], "blocked");
    assert_eq!(report["operational_qualification"], "unverified");
    let repository_check = report["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|check| check["check"] == "repository_prerequisites")
        .unwrap();
    assert_eq!(repository_check["configuration_admission"], "verified");
    let repository = &repository_check["repositories"][0];
    assert_eq!(repository["read_access"], true);
    assert_eq!(repository["delivery_prerequisites"]["status"], "blocked");
    assert_eq!(
        repository["delivery_prerequisites"]["error"]["code"],
        "delivery_policy_unresolved"
    );
    assert_eq!(
        repository["delivery_prerequisites"]["error"]["details"]["unresolved_fields"],
        json!(["ci", "production_ci", "no_change_validation", "deployment"])
    );
    assert_eq!(
        repository["delivery_prerequisites"]["error"]["details"]["delivery_metadata_observed"],
        false
    );
}

#[tokio::test]
async fn initial_preflight_admits_discovery_but_reports_unresolved_delivery_and_no_qualification() {
    let f = Fixture::new(Observation::Readable);
    let original = config_revision(&f.config).unwrap();
    let prepared = prepare_initial(
        f.config.clone(),
        &f.host,
        f.store.as_ref(),
        f.inputs.as_ref(),
        &f.host.home.join("probes"),
    )
    .await
    .unwrap();
    assert_eq!(config_revision(&prepared.config).unwrap(), original);
    assert_discovery_report(&prepared.report);
    let workflows = load_workflows(&prepared.config).unwrap();
    assert!(workflows[&id(2)].path.is_none());
    assert!(workflows[&id(2)].embedded.is_some());
    for name in ["ci", "production_ci", "no_change_validation", "deployment"] {
        assert!(serde_json::to_value(&prepared.config).unwrap()["workspaces"]["example"]["repositories"]["42"].get(name).is_none());
    }
    assert!(!f.host.home.join("probes").exists());
    f.assert_observation_only();
}

#[tokio::test]
async fn initial_preflight_still_refuses_revoked_read_access_and_wrong_account_identity() {
    for observation in [Observation::ReadRevoked, Observation::WrongAccount] {
        let f = Fixture::new(observation);
        let error = prepare_initial(
            f.config.clone(),
            &f.host,
            f.store.as_ref(),
            f.inputs.as_ref(),
            &f.host.home.join("probes"),
        )
        .await
        .err()
        .unwrap();
        assert_eq!(error.code, "configuration_unverified");
        assert_eq!(error.details["configuration_admission"], "blocked");
        assert_eq!(error.details["status"], "blocked");
        assert_eq!(error.details["operational_qualification"], "unverified");
        let expected = if matches!(observation, Observation::ReadRevoked) {
            "repository_inaccessible"
        } else {
            "identity_mismatch"
        };
        assert!(
            error.details["checks"]
                .as_array()
                .unwrap()
                .iter()
                .any(|check| check["error"]["code"] == expected)
        );
        assert!(!f.host.home.join("probes").exists());
        f.assert_observation_only();
    }
}

struct Probe(ProcessIdentity);
impl ProcessProbe for Probe {
    fn current(&self) -> Result<ProcessIdentity> {
        Ok(self.0.clone())
    }
    fn observe(&self, expected: &ProcessIdentity) -> Result<ProcessObservation> {
        Ok(if expected == &self.0 {
            ProcessObservation::Running
        } else {
            ProcessObservation::DifferentIdentity
        })
    }
    fn matching_executable(&self, path: &Path) -> Result<Vec<ProcessIdentity>> {
        Ok(if path == self.0.executable {
            vec![self.0.clone()]
        } else {
            vec![]
        })
    }
}

#[tokio::test]
async fn actual_setup_and_workspace_add_commit_discovery_configuration_then_report_delivery_blockers()
 {
    for command in [CommandName::Setup, CommandName::WorkspaceAdd] {
        let f = Fixture::new(Observation::Readable);
        let executable = std::env::current_exe().unwrap().canonicalize().unwrap();
        let probe = Arc::new(Probe(ProcessIdentity {
            pid: 7001,
            parent_pid: 7000,
            process_group: 7001,
            uid: f.host.uid,
            boot_id: f.host.boot_id,
            started_seconds: 1,
            started_microseconds: 0,
            executable: executable.clone(),
        }));
        let (registration, control) = Registration::bootstrap_with_probe(
            &f.host,
            id(1),
            BinaryIdentity::inspect(&executable, false).unwrap(),
            Uuid::new_v4(),
            probe,
        )
        .unwrap();
        let mut initial = f.config.clone();
        initial.workspaces.clear();
        if command == CommandName::Setup {
            initial.connections.clear();
        }
        registration.complete_setup(&initial).unwrap();
        let registration = Arc::new(registration);
        let control = Arc::new(control);
        let authentication = Arc::new(TransportAuthentication::new(initial.control.clone(), None));
        let manager = ConfigManager::with_inputs(
            initial.clone(),
            registration.clone(),
            control.clone(),
            f.store.clone(),
            authentication,
            f.inputs.clone(),
        )
        .unwrap();
        let args = if command == CommandName::Setup {
            json!({"configuration":toml::to_string(&f.config).unwrap()})
        } else {
            json!({"name":"example","workspace_configuration":toml::to_string(&f.config.workspaces["example"]).unwrap()})
        };
        let request = CommandRequest {
            api_version: 1,
            runtime_id: id(1),
            request_id: Uuid::new_v4(),
            command,
            args,
            expected_config_revision: Some(config_revision(&initial).unwrap()),
            expected_control_generation: Some(control.snapshot().unwrap().intent.generation),
        };
        let actor = Actor {
            identity: "fixture-operator".into(),
            principal: Some(Principal::Unix { uid: f.host.uid }),
            roles: ["inspect".into(), "configure".into(), "lifecycle".into()].into(),
        };
        let receipt = manager
            .execute(actor.clone(), request.clone(), None)
            .await
            .unwrap();
        assert_eq!(receipt.phase, ReceiptPhase::Acknowledged);
        assert_eq!(
            config_revision(&manager.snapshot().unwrap()).unwrap(),
            config_revision(&f.config).unwrap()
        );
        let stored = Config::parse(
            std::str::from_utf8(
                &registration
                    .root()
                    .read_bytes("config.toml", 1_048_576)
                    .unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            config_revision(&stored).unwrap(),
            config_revision(&f.config).unwrap()
        );
        assert_discovery_report(&manager.check(None).await.unwrap());
        let calls = f.inputs.wire.paths.lock().unwrap().len();
        let probes = f.inputs.codex_calls.load(Ordering::SeqCst);
        assert_eq!(
            manager
                .execute(actor, request, None)
                .await
                .unwrap()
                .operation_id,
            receipt.operation_id
        );
        assert_eq!(f.inputs.wire.paths.lock().unwrap().len(), calls);
        assert_eq!(f.inputs.codex_calls.load(Ordering::SeqCst), probes);
        assert!(control.snapshot().unwrap().active_operations.is_empty());
        assert!(
            control
                .snapshot()
                .unwrap()
                .pending_local_processes
                .is_empty()
        );
        f.assert_observation_only();
    }
}
