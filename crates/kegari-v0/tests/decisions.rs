//! Decision commands through production provider parsers. All provider facts
//! and process observations below are labeled deterministic fixtures.
#[path = "support/pinned_git.rs"]
mod pinned_git;
use chrono::Utc;
use kegari_config::Config;
use kegari_digest::canonical_digest;
use kegari_providers::*;
use kegari_secrets::AttestationKey;
use kegari_types::error::Result;
use kegari_v0::reconcile::*;
use reqwest::header::HeaderMap;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
};
use uuid::Uuid;
fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
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
fn issue(n: u128, parent: Option<u128>) -> Value {
    let metadata = if parent.is_none() {
        json!({"schema_version":1,"kind":"parent","required_children":[id(101)],"e2e_child":id(101),"visual":false,"production_targets":[]})
    } else {
        json!({"schema_version":1,"kind":"child","responsibility":"testing","write_repository":"42","write_paths":["tests/"],"read_repositories":[],"test_resources":[]})
    };
    json!({"id":id(n),"identifier":format!("TEST-{n}"),"title":format!("Fixture {n}"),"description":format!("Outcome: validate exact behavior. Acceptance criteria: all specified checks pass.\n```kegari\n{metadata}\n```"),"priority":2,"createdAt":"2026-01-01T00:00:00Z","updatedAt":"2026-01-02T00:00:00Z","url":format!("https://linear.invalid/issue/{n}"),"team":{"id":id(10)},"project":null,"creator":{"id":id(5)},"assignee":null,"parent":parent.map(|n| json!({"id":id(n)})),"state":{"id":id(11),"name":"Prepared","type":"unstarted"},"labelIds":if parent.is_none(){vec![id(4)]}else{vec![id(20),id(21),id(22),id(23)]}})
}
fn connection(nodes: Vec<Value>) -> Value {
    json!({"nodes":nodes,"pageInfo":{"hasNextPage":false,"endCursor":null}})
}
struct Fixture {
    issues: BTreeMap<Uuid, Value>,
    comments: BTreeMap<Uuid, Vec<Value>>,
    relations: Vec<Value>,
    history: BTreeMap<Uuid, Vec<Value>>,
    selected: bool,
    fail_comments: bool,
    changing: bool,
    discovery_count: u32,
    requests: Vec<(String, Value)>,
    prs: Vec<Value>,
    repository_available: bool,
    comment_response_lost: bool,
    duplicate_publication: bool,
    correction_after_acceptance: bool,
    tamper_acceptance_after_first_read: Option<String>,
    acceptance_read: bool,
    tamper_decision: Option<(String, String)>,
    decision_read: bool,
}
impl Fixture {
    fn new() -> Arc<Mutex<Self>> {
        Arc::new(Mutex::new(Self {
            issues: BTreeMap::from([
                (id(100), issue(100, None)),
                (id(101), issue(101, Some(100))),
            ]),
            comments: BTreeMap::new(),
            relations: Vec::new(),
            history: BTreeMap::from([(
                id(100),
                vec![
                    json!({"id":id(900),"actor":{"id":id(5)},"actorId":id(5),"actors":[{"id":id(5)}],"botActor":null,"addedLabelIds":[id(4)],"removedLabelIds":[],"createdAt":"2026-01-03T00:00:00Z","updatedAt":"2026-01-03T00:00:00Z"}),
                ],
            )]),
            selected: true,
            fail_comments: false,
            changing: false,
            discovery_count: 0,
            requests: Vec::new(),
            prs: Vec::new(),
            repository_available: true,
            comment_response_lost: false,
            duplicate_publication: false,
            correction_after_acceptance: false,
            tamper_acceptance_after_first_read: None,
            acceptance_read: false,
            tamper_decision: None,
            decision_read: false,
        }))
    }
}
struct Wire(Arc<Mutex<Fixture>>);
impl HttpTransport for Wire {
    fn send(
        &self,
        request: HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Result<HttpResponse>> + Send + '_>> {
        Box::pin(async move {
            let mut f = self.0.lock().unwrap();
            let body: Value = if request.body.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&request.body).unwrap()
            };
            f.requests.push((request.url.to_string(), body.clone()));
            let value = if request.url.host_str() == Some("github.invalid") {
                match request.url.path() {
                    "/user" => json!({"id":7,"login":"fixture","type":"User"}),
                    "/repos/fixture/repo" => {
                        json!({"id":42,"owner":{"id":7,"login":"fixture","type":"User"},"name":"repo","default_branch":"main","private":true,"archived":false,"disabled":false,"allow_squash_merge":true,"permissions":{"pull":true,"push":true}})
                    }
                    "/user/repos" => {
                        if f.repository_available {
                            json!([{"id":42,"owner":{"id":7,"login":"fixture","type":"User"},"name":"repo","default_branch":"main","private":true,"archived":false,"disabled":false,"allow_squash_merge":true,"permissions":{"pull":true,"push":true}}])
                        } else {
                            json!([])
                        }
                    }
                    "/repos/fixture/repo/pulls" => json!(f.prs),
                    path => panic!("unexpected GitHub path {path}"),
                }
            } else {
                let query = body["query"].as_str().unwrap();
                let input_id = body["variables"]["id"]
                    .as_str()
                    .and_then(|s| Uuid::parse_str(s).ok());
                let data = if query.contains("KegariIdentity") {
                    json!({"viewer":{"id":id(5),"name":"fixture"},"organization":{"id":id(3)}})
                } else if query.contains("mutation KegariComment") {
                    let issue: Uuid =
                        serde_json::from_value(body["variables"]["input"]["issueId"].clone())
                            .unwrap();
                    let published_body = body["variables"]["input"]["body"].clone();
                    let comment = id(1500 + f.requests.len() as u128);
                    let value = json!({"id":comment,"body":published_body,"createdAt":Utc::now(),"updatedAt":Utc::now(),"editedAt":null,"url":format!("https://linear.invalid/comment/{comment}"),"user":{"id":id(5)},"onBehalfOf":null,"botActor":null,"issue":{"id":issue}});
                    f.comments.entry(issue).or_default().push(value.clone());
                    if f.duplicate_publication {
                        let mut duplicate = value;
                        duplicate["id"] = json!(id(9500));
                        f.comments.entry(issue).or_default().push(duplicate);
                    }
                    if f.correction_after_acceptance
                        && published_body.as_str().is_some_and(|body| {
                            body.starts_with("<!-- kegari:v0:human_acceptance -->")
                        })
                    {
                        f.comments.entry(issue).or_default().push(json!({"id":id(977),"body":"Correct the exact expected output before approval","createdAt":Utc::now(),"updatedAt":Utc::now(),"editedAt":null,"url":"https://linear.invalid/comment/977","user":{"id":id(5)},"onBehalfOf":null,"botActor":null,"issue":{"id":issue}}));
                    }
                    if f.comment_response_lost {
                        return Err(kegari_types::error::Error::new(
                            "timeout",
                            "fixture loses the acknowledgement after storing the comment",
                        ));
                    }
                    json!({"commentCreate":{"success":true,"comment":{"id":comment,"issue":{"id":issue},"user":{"id":id(5)}}}})
                } else if query.contains("mutation KegariState") {
                    let target =
                        Uuid::parse_str(body["variables"]["id"].as_str().unwrap()).unwrap();
                    let state = body["variables"]["input"]["stateId"].clone();
                    f.issues.get_mut(&target).unwrap()["state"]["id"] = state.clone();
                    json!({"issueUpdate":{"success":true,"issue":{"id":target,"state":{"id":state}}}})
                } else if query.contains("KegariIssue(") {
                    json!({"issue":f.issues[&input_id.unwrap()]})
                } else if query.contains("KegariParents") {
                    f.discovery_count += 1;
                    if f.changing {
                        f.issues.get_mut(&id(100)).unwrap()["title"] =
                            json!(format!("Changed during collection {}", f.discovery_count));
                    }
                    json!({"issues":connection(if f.selected {f.issues.values().filter(|i|i["parent"].is_null()).cloned().collect()}else{vec![]})})
                } else if query.contains("KegariRefresh") {
                    let ids: Vec<Uuid> =
                        serde_json::from_value(body["variables"]["filter"]["id"]["in"].clone())
                            .unwrap();
                    json!({"issues":connection(ids.iter().filter_map(|i|f.issues.get(i).cloned()).collect())})
                } else if query.contains("KegariChildren") {
                    json!({"issue":{"children":connection(f.issues.values().filter(|i|i["parent"]["id"]==json!(input_id)).cloned().collect())}})
                } else if query.contains("KegariLabels") {
                    let labels: Vec<Value> = f.issues[&input_id.unwrap()]["labelIds"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|i| json!({"id":i,"name":format!("Label {i}")}))
                        .collect();
                    json!({"issue":{"labels":connection(labels)}})
                } else if query.contains("KegariComments") {
                    if let Some((purpose, mode)) = f.tamper_decision.clone() {
                        let marker = format!("<!-- kegari:v0:{purpose} -->");
                        let target = input_id.unwrap();
                        if f.comments.get(&target).is_some_and(|comments| {
                            comments
                                .iter()
                                .any(|c| c["body"].as_str().is_some_and(|b| b.contains(&marker)))
                        }) {
                            if f.decision_read {
                                let comments = f.comments.get_mut(&target).unwrap();
                                if mode == "delete" {
                                    comments.retain(|c| {
                                        !c["body"].as_str().is_some_and(|b| b.contains(&marker))
                                    });
                                } else {
                                    comments
                                        .iter_mut()
                                        .find(|c| {
                                            c["body"].as_str().is_some_and(|b| b.contains(&marker))
                                        })
                                        .unwrap()["body"] = json!("Changed signed decision bytes");
                                }
                                f.tamper_decision = None;
                            } else {
                                f.decision_read = true;
                            }
                        }
                    }
                    if input_id == Some(id(100))
                        && f.comments.get(&id(100)).is_some_and(|comments| {
                            comments.iter().any(|c| {
                                c["body"].as_str().is_some_and(|b| {
                                    b.starts_with("<!-- kegari:v0:human_acceptance -->")
                                })
                            })
                        })
                    {
                        if f.acceptance_read {
                            if let Some(mode) = f.tamper_acceptance_after_first_read.take() {
                                let comments = f.comments.get_mut(&id(100)).unwrap();
                                if mode == "delete" {
                                    comments.retain(|c| {
                                        !c["body"].as_str().is_some_and(|b| {
                                            b.starts_with("<!-- kegari:v0:human_acceptance -->")
                                        })
                                    });
                                } else {
                                    let comment = comments
                                        .iter_mut()
                                        .find(|c| {
                                            c["body"].as_str().is_some_and(|b| {
                                                b.starts_with("<!-- kegari:v0:human_acceptance -->")
                                            })
                                        })
                                        .unwrap();
                                    comment["body"] =
                                        json!("Changed publication bytes after acknowledgement");
                                    comment["editedAt"] = json!(Utc::now());
                                }
                            }
                        } else {
                            f.acceptance_read = true;
                        }
                    }
                    if f.fail_comments {
                        return Ok(HttpResponse {status:200,headers:HeaderMap::new(),body:serde_json::to_vec(&json!({"data":{"issue":{"comments":connection(vec![])}},"errors":[{"message":"partial"}]})).unwrap()});
                    }
                    json!({"issue":{"comments":connection(f.comments.get(&input_id.unwrap()).cloned().unwrap_or_default())}})
                } else if query.contains("KegariHistory") {
                    json!({"issue":{"history":connection(f.history.get(&input_id.unwrap()).cloned().unwrap_or_default())}})
                } else if query.contains("KegariDependencies") {
                    let field = if query.contains("inverseRelations") {
                        "inverseRelations"
                    } else {
                        "relations"
                    };
                    let direction = if field == "relations" {
                        "issue"
                    } else {
                        "relatedIssue"
                    };
                    json!({"issue":{field:connection(f.relations.iter().filter(|r|r[direction]["id"]==json!(input_id)).cloned().collect())}})
                } else {
                    panic!("unexpected Linear query {query}")
                };
                json!({"data":data})
            };
            Ok(HttpResponse {
                status: 200,
                headers: HeaderMap::new(),
                body: serde_json::to_vec(&value).unwrap(),
            })
        })
    }
}
use kegari_platform::{ProcessIdentity, ProcessObservation, ProcessProbe};
use kegari_secrets::{Secret, SecretStore};
use kegari_storage::PrivateDir;
use kegari_types::api::{CommandName, CommandRequest};
use kegari_types::error::Error;
use kegari_v0::{
    app::{ConfigurationControl, HostFuture, mutation_request},
    control::{
        Actor, ControlHost, MutationRequest, Principal, Receipt, ReceiptPhase, ReceiptStore,
        RunReference,
    },
    decisions::*,
    evidence::{VerifiedAcceptance, WorkIdentity, encode_signed_publication},
    housekeeping::Housekeeping,
    observations::ObservationCache,
};
use std::{
    path::{Path, PathBuf},
    sync::RwLock,
};
struct Configuration(RwLock<Config>);
impl ConfigurationControl for Configuration {
    fn authority_fences(&self) -> Result<kegari_v0::configuration::AuthorityFences> {
        Ok(Default::default())
    }
    fn snapshot(&self) -> Result<Config> {
        Ok(self.0.read().unwrap().clone())
    }
    fn execute(
        &self,
        _: Actor,
        _: CommandRequest,
        _: Option<Secret>,
        _: Receipt,
    ) -> HostFuture<'_, Receipt> {
        Box::pin(async { panic!("decision fixture must not configure") })
    }
    fn check(&self, _: Option<Uuid>) -> HostFuture<'_, Value> {
        Box::pin(async { panic!("decision fixture must not authenticate Codex") })
    }
}
struct Secrets;
impl SecretStore for Secrets {
    fn read(&self, _: &str) -> Result<Secret> {
        Secret::new(b"synthetic-provider-credential".to_vec())
    }
    fn save(&self, _: &str, _: &Secret) -> Result<()> {
        panic!("decision fixture must not store secrets")
    }
}
struct Inputs(Arc<Mutex<Fixture>>);
impl kegari_v0::configuration::ConfigurationInputs for Inputs {
    fn environment(&self, _: &str) -> Option<String> {
        None
    }
    fn transport(&self, _: &str) -> Option<Arc<dyn HttpTransport>> {
        Some(Arc::new(Wire(self.0.clone())))
    }
    fn codex<'a>(
        &'a self,
        _: &'a kegari_config::Connection,
        _: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<kegari_v0::codex::CodexConnectionMetadata>> + Send + 'a>>
    {
        Box::pin(async { panic!("human decisions must not start Codex") })
    }
}
struct NoProcesses;
impl ProcessProbe for NoProcesses {
    fn current(&self) -> Result<ProcessIdentity> {
        Err(Error::new("fixture", "no native process probe"))
    }
    fn observe(&self, _: &ProcessIdentity) -> Result<ProcessObservation> {
        Ok(ProcessObservation::Gone)
    }
    fn matching_executable(&self, _: &Path) -> Result<Vec<ProcessIdentity>> {
        Ok(vec![])
    }
}
type PublishedRetryHook = Box<dyn FnOnce(&VerifiedRetryGrant) -> Result<()> + Send>;
#[derive(Default)]
struct Evidence {
    delivery: Mutex<ParentDelivery>,
    corrections: Mutex<Vec<VerifiedExplicitCorrection>>,
    retries: Mutex<Vec<VerifiedRetryGrant>>,
    retry: Mutex<Option<RetryTarget>>,
    recovery_authorizations: Mutex<Vec<Receipt>>,
    after_retry_publication: Mutex<Option<PublishedRetryHook>>,
}
impl DecisionEvidence for Evidence {
    fn current(&self, _: &kegari_v0::authority::IssueKey) -> Result<ParentDelivery> {
        Ok(self.delivery.lock().unwrap().clone())
    }
    fn outcomes(&self) -> Result<DependencyOutcomes> {
        Ok(DependencyOutcomes::default())
    }
    fn retry_target(&self, _: &WorkIdentity) -> Result<RetryTarget> {
        self.retry
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| Error::new("retry_unavailable", "fixture has no resolved retry phase"))
    }
    fn published_retry_target(&self, expected: &VerifiedRetryGrant) -> Result<RetryTarget> {
        let hook = self.after_retry_publication.lock().unwrap().take();
        if let Some(hook) = hook {
            hook(expected)?;
        }
        let target = self.retry.lock().unwrap().clone().ok_or_else(|| {
            Error::new("retry_unavailable", "fixture has no resolved retry phase")
        })?;
        if target.phase != *expected.phase()
            || target.admissions != expected.record().prior_admissions.keys().copied().collect()
        {
            return Err(Error::new("stale_result", "fixture retry history changed"));
        }
        Ok(target)
    }
    fn record_acceptance(
        &self,
        _: &kegari_v0::authority::IssueKey,
        a: VerifiedAcceptance,
    ) -> Result<()> {
        self.delivery.lock().unwrap().acceptance(a)
    }
    fn record_correction(&self, c: VerifiedExplicitCorrection) -> Result<()> {
        self.corrections.lock().unwrap().push(c);
        Ok(())
    }
    fn retry_committed(&self, r: VerifiedRetryGrant) -> Result<()> {
        self.retries.lock().unwrap().push(r);
        Ok(())
    }
    fn release(&self, _: AuthorizedDecision) -> HostFuture<'_, Receipt> {
        Box::pin(async { Err(Error::new("fixture", "no synthetic release success")) })
    }
    fn recover_release(&self, decision: AuthorizedDecision) -> HostFuture<'_, Receipt> {
        Box::pin(async move {
            self.recovery_authorizations
                .lock()
                .unwrap()
                .push(decision.authorization_receipt()?.clone());
            Err(Error::new("fixture", "no synthetic recovery success"))
        })
    }
    fn reconcile(&self, _: AuthorizedOperationRecovery) -> HostFuture<'_, Receipt> {
        Box::pin(async { Err(Error::new("fixture", "no synthetic reconciliation success")) })
    }
}
struct DecisionFixture {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    wire: Arc<Mutex<Fixture>>,
    configuration: Arc<Configuration>,
    control: Arc<ControlHost>,
    cache: Arc<ObservationCache>,
    key: Arc<AttestationKey>,
    evidence: Arc<Evidence>,
    manager: DecisionManager,
}
impl DecisionFixture {
    async fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let workflow = root.join("WORKFLOW.md");
        std::fs::write(&workflow,"---\nchecks:\n  local:\n    local:\n      argv: ['/usr/bin/true']\n      result_artifact: '{{ paths.scratch }}/local.json'\n  integrated:\n    e2e:\n      argv: ['/usr/bin/true']\n      result_artifact: '{{ paths.scratch }}/e2e.json'\n---\nUse prepared context\n").unwrap();
        let mut cfg = config();
        cfg.workspaces.get_mut("example").unwrap().workflow =
            Some(workflow.to_str().unwrap().into());
        let configuration = Arc::new(Configuration(RwLock::new(cfg)));
        let wire = Fixture::new();
        let inputs = Arc::new(Inputs(wire.clone()));
        let key = Arc::new(
            AttestationKey::new("synthetic-decision-fixture".into(), vec![29; 32]).unwrap(),
        );
        let control = Arc::new(
            ControlHost::setup(
                PrivateDir::create(&root.join("control")).unwrap(),
                id(1),
                id(55),
            )
            .unwrap(),
        );
        let cache = Arc::new(
            ObservationCache::with_inputs(
                configuration.clone(),
                Arc::new(Secrets),
                key.clone(),
                PrivateDir::create(&root.join("references")).unwrap(),
                inputs.clone(),
                control.clone(),
            )
            .await
            .unwrap(),
        );
        let first = cache.refresh().await.unwrap();
        let authorization = key
            .sign(
                "parent_authorization",
                first
                    .parent(id(2), id(100))
                    .unwrap()
                    .authorization_candidate
                    .as_ref()
                    .unwrap(),
            )
            .unwrap();
        let body = encode_signed_publication("parent_authorization", &authorization).unwrap();
        wire.lock().unwrap().comments.insert(id(100),vec![json!({"id":id(950),"body":body,"createdAt":"2026-01-04T00:00:00Z","updatedAt":"2026-01-04T00:00:00Z","editedAt":null,"url":"https://linear.invalid/comment/950","user":{"id":id(5)},"onBehalfOf":null,"botActor":null,"issue":{"id":id(100)}})]);
        cache.refresh().await.unwrap();
        let housekeeping = Arc::new(
            Housekeeping::new(
                PrivateDir::open(&root.join("control")).unwrap(),
                control.clone(),
                Arc::new(NoProcesses),
                pinned_git::prepare(&root),
            )
            .unwrap(),
        );
        let evidence = Arc::new(Evidence::default());
        let manager = DecisionManager::new(DecisionDependencies {
            configuration: configuration.clone(),
            control: control.clone(),
            observations: cache.clone(),
            secrets: Arc::new(Secrets),
            key: key.clone(),
            evidence: evidence.clone(),
            housekeeping,
            inputs,
        });
        Self {
            _tmp: temp,
            root,
            wire,
            configuration,
            control,
            cache,
            key,
            evidence,
            manager,
        }
    }
    fn actor(&self) -> Actor {
        Actor {
            identity: "operator".into(),
            principal: Some(Principal::Unix { uid: 501 }),
            roles: ["inspect", "correct", "accept", "release", "recover"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
        }
    }
    fn command(&self, command: CommandName, args: Value) -> (CommandRequest, Receipt) {
        let mut request = CommandRequest {
            api_version: 1,
            request_id: Uuid::new_v4(),
            runtime_id: id(1),
            command: command.clone(),
            args,
            expected_config_revision: Some(
                kegari_v0::config::config_revision(&self.configuration.snapshot().unwrap())
                    .unwrap(),
            ),
            expected_control_generation: Some(self.control.snapshot().unwrap().intent.generation),
        };
        let mut mutation = mutation_request(
            &self.actor(),
            &request,
            self.manager.command_targets(&request).unwrap(),
        );
        mutation.kind = format!("command/{:?}", command);
        mutation.payload = json!({"arguments":request.args,"secret_digest":null});
        let (receipt, fresh) = self
            .control
            .queue_command(
                mutation,
                kegari_v0::api::command_role(&command, &request.args),
                || Ok(()),
            )
            .unwrap();
        assert!(fresh);
        request.request_id = kegari_v0::app::command_effect_id(receipt.request_id);
        (request, receipt)
    }
    fn writes(&self) -> usize {
        self.wire
            .lock()
            .unwrap()
            .requests
            .iter()
            .filter(|(_, v)| v["query"].as_str().is_some_and(|q| q.contains("mutation ")))
            .count()
    }
}

#[tokio::test]
async fn stopped_correction_is_authenticated_published_once_and_preserves_command_lineage() {
    let f = DecisionFixture::new().await;
    let (request,parent)=f.command(CommandName::Correct,json!({"parent":"TEST-100","message":"Please preserve the exact binary output and add the missing assertion."}));
    let receipt = f
        .manager
        .execute(f.actor(), request.clone(), parent.clone())
        .await
        .unwrap();
    assert_eq!(
        receipt.expected["parent_command_id"],
        parent.request_id.to_string()
    );
    assert_eq!(
        receipt.expected["parent_payload_digest"],
        parent.payload_digest
    );
    assert_eq!(
        f.control.snapshot().unwrap().intent.desired,
        kegari_v0::control::Intent::Stopped
    );
    assert_eq!(f.writes(), 1);
    {
        let corrections = f.evidence.corrections.lock().unwrap();
        assert_eq!(corrections.len(), 1);
        assert_eq!(corrections[0].record().linear_actor, id(5));
        assert_eq!(corrections[0].record().control_actor, "operator");
        assert!(!corrections[0].publication().record_id.is_empty());
    }
    let replay = f.manager.execute(f.actor(), request, parent).await.unwrap();
    assert_eq!(receipt.payload_digest, replay.payload_digest);
    assert_eq!(f.writes(), 1);
    let provider = f
        .control
        .operations()
        .unwrap()
        .into_iter()
        .find(|r| r.kind == "linear_comment")
        .unwrap();
    assert!(provider.expected.contains_key("provider_body_digest"));
    assert_eq!(provider.expected["provider_writer"], id(5).to_string());
    assert_eq!(provider.expected["explicit_correction"], "true");
}
#[tokio::test]
async fn wrong_lineage_and_revoked_mapped_linear_permission_never_publish() {
    let f = DecisionFixture::new().await;
    let (mut request, parent) = f.command(
        CommandName::Correct,
        json!({"parent":"TEST-100","message":"Exact requested correction"}),
    );
    request.args["message"] = json!("substituted intent");
    assert_eq!(
        f.manager
            .execute(f.actor(), request.clone(), parent.clone())
            .await
            .unwrap_err()
            .code,
        "request_conflict"
    );
    assert_eq!(f.writes(), 0);
    request.args["message"] = json!("Exact requested correction");
    f.configuration
        .0
        .write()
        .unwrap()
        .workspaces
        .get_mut("example")
        .unwrap()
        .actors
        .correctors = BTreeSet::from([id(99)]);
    let error = f
        .manager
        .execute(f.actor(), request, parent)
        .await
        .unwrap_err();
    assert_eq!(error.code, "stale_config");
    assert_eq!(f.writes(), 0);
}
#[tokio::test]
async fn lost_correction_response_retains_unknown_receipt_and_never_resends() {
    let f = DecisionFixture::new().await;
    let (request, parent) = f.command(
        CommandName::Correct,
        json!({"parent":"TEST-100","message":"One correction even if acknowledgement is lost"}),
    );
    f.wire.lock().unwrap().comment_response_lost = true;
    assert_eq!(
        f.manager
            .execute(f.actor(), request, parent)
            .await
            .unwrap_err()
            .code,
        "operation_uncertain"
    );
    assert_eq!(f.writes(), 1);
    assert!(
        f.control
            .operations()
            .unwrap()
            .iter()
            .any(|r| r.kind == "linear_comment" && r.phase == ReceiptPhase::Sent)
    );
    assert!(f.evidence.corrections.lock().unwrap().is_empty());
}
#[tokio::test]
async fn absent_parent_evidence_cannot_close_or_accept_even_when_tracker_looks_done() {
    let f = DecisionFixture::new().await;
    f.wire.lock().unwrap().issues.get_mut(&id(100)).unwrap()["state"]["id"] = json!(id(15));
    let (request, parent) = f.command(
        CommandName::Approve,
        json!({"parent":"TEST-100","result":"invented-result-token"}),
    );
    assert_eq!(
        f.manager
            .execute(f.actor(), request, parent)
            .await
            .unwrap_err()
            .code,
        "evidence_missing"
    );
    assert_eq!(f.writes(), 0);
}
#[tokio::test]
async fn correction_signature_binds_plaintext_principal_target_and_actual_writer() {
    let f = DecisionFixture::new().await;
    let (request, parent) = f.command(
        CommandName::Correct,
        json!({"parent":"TEST-100","message":"Bound human intent"}),
    );
    f.manager.execute(f.actor(), request, parent).await.unwrap();
    let raw = f.wire.lock().unwrap().comments[&id(100)]
        .last()
        .unwrap()
        .clone();
    let mut comment = Comment {
        id: serde_json::from_value(raw["id"].clone()).unwrap(),
        issue_id: id(100),
        body: raw["body"].as_str().unwrap().into(),
        body_digest: kegari_digest::bytes_digest(raw["body"].as_str().unwrap().as_bytes()),
        user_id: Some(id(5)),
        delegated: false,
        bot: false,
        created_at: Some(Utc::now()),
        updated_at: Some(Utc::now()),
        edited_at: None,
        url: raw["url"].as_str().unwrap().into(),
    };
    let key = f
        .cache
        .snapshot()
        .unwrap()
        .parent(id(2), id(100))
        .unwrap()
        .key
        .clone();
    let verify = |c: &Comment| {
        VerifiedExplicitCorrection::verify_comment(
            c,
            &key,
            id(1),
            id(2),
            &BTreeSet::from([id(5)]),
            &id(5).to_string(),
            &f.key,
        )
    };
    assert!(verify(&comment).is_ok());
    comment.body = comment
        .body
        .replace("Bound human intent", "Changed human intent");
    comment.body_digest = kegari_digest::bytes_digest(comment.body.as_bytes());
    assert!(verify(&comment).is_err());
    comment.body = raw["body"].as_str().unwrap().into();
    comment.body_digest = kegari_digest::bytes_digest(comment.body.as_bytes());
    comment.user_id = Some(id(99));
    assert!(verify(&comment).is_err());
    comment.user_id = Some(id(5));
    comment.delegated = true;
    assert!(verify(&comment).is_err());
}
#[tokio::test]
async fn retry_grants_exact_child_phase_budget_only_after_terminal_history() {
    let f = DecisionFixture::new().await;
    let (work, admission) = prepare_retry_fixture(&f);
    let (request,parent)=f.command(CommandName::Retry,json!({"work":"TEST-101","reason":"The fixture transport blocker is resolved; grant one bounded phase budget"}));
    let receipt = f.manager.execute(f.actor(), request, parent).await.unwrap();
    assert_eq!(f.writes(), 1);
    assert_eq!(receipt.phase, ReceiptPhase::Acknowledged);
    let grants = f.evidence.retries.lock().unwrap();
    assert_eq!(grants.len(), 1);
    let grant = &grants[0];
    assert_eq!(grant.work(), &work);
    assert_eq!(grant.new_budget().max_attempts, 3);
    assert_eq!(
        grant.phase_key().retry_grant,
        Some(grant.record().operation_id)
    );
    assert_eq!(
        grant
            .record()
            .prior_admissions
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        vec![admission]
    );
}
#[path = "support/parent_evidence.rs"]
mod parent_evidence;
#[tokio::test]
async fn stopped_approval_publishes_exact_acceptance_before_configured_final_closure() {
    let f = DecisionFixture::new().await;
    let projection = f.cache.snapshot().unwrap();
    let p = projection.parent(id(2), id(100)).unwrap();
    let (_artifacts, parent) = parent_evidence::parent(
        p.work.as_ref().unwrap(),
        p.children[&id(101)].work.as_ref().unwrap(),
        &f.key,
    );
    let token = parent.token().to_string();
    f.evidence.delivery.lock().unwrap().parent(parent);
    let (request, command) = f.command(
        CommandName::Approve,
        json!({"parent":"TEST-100","result":token}),
    );
    let receipt = f
        .manager
        .execute(f.actor(), request, command)
        .await
        .unwrap();
    assert_eq!(receipt.phase, ReceiptPhase::Acknowledged);
    assert_eq!(f.writes(), 2);
    assert_eq!(
        f.wire.lock().unwrap().issues[&id(100)]["state"]["id"],
        json!(id(15))
    );
    assert!(
        f.evidence
            .delivery
            .lock()
            .unwrap()
            .verified_acceptance()
            .is_some()
    );
    assert_eq!(
        f.control.snapshot().unwrap().intent.desired,
        kegari_v0::control::Intent::Stopped
    );
    let operations = f.control.operations().unwrap();
    let closure = operations
        .iter()
        .find(|r| r.kind == "linear_state")
        .unwrap();
    let acceptance = operations
        .iter()
        .find(|r| r.kind == "linear_comment")
        .unwrap();
    assert!(closure.updated_at >= acceptance.updated_at);
}
#[tokio::test]
async fn stale_token_and_current_unclassified_input_prevent_acceptance_write() {
    let f = DecisionFixture::new().await;
    let projection = f.cache.snapshot().unwrap();
    let p = projection.parent(id(2), id(100)).unwrap();
    let (_artifacts, parent) = parent_evidence::parent(
        p.work.as_ref().unwrap(),
        p.children[&id(101)].work.as_ref().unwrap(),
        &f.key,
    );
    let token = parent.token().to_owned();
    f.evidence.delivery.lock().unwrap().parent(parent);
    let (request, command) = f.command(
        CommandName::Approve,
        json!({"parent":"TEST-100","result":"stale"}),
    );
    assert_eq!(
        f.manager
            .execute(f.actor(), request, command)
            .await
            .unwrap_err()
            .code,
        "stale_result"
    );
    assert_eq!(f.writes(), 0);
    f.wire.lock().unwrap().comments.get_mut(&id(100)).unwrap().push(json!({"id":id(951),"body":"Please change the expected output before releasing","createdAt":Utc::now(),"updatedAt":Utc::now(),"editedAt":null,"url":"https://linear.invalid/comment/951","user":{"id":id(5)},"onBehalfOf":null,"botActor":null,"issue":{"id":id(100)}}));
    let (request, command) = f.command(
        CommandName::Approve,
        json!({"parent":"TEST-100","result":token}),
    );
    assert_eq!(
        f.manager
            .execute(f.actor(), request, command)
            .await
            .unwrap_err()
            .code,
        "decision_blocked"
    );
    assert_eq!(f.writes(), 0);
}
#[tokio::test]
async fn correction_observed_after_acceptance_publication_prevents_final_closure() {
    let f = DecisionFixture::new().await;
    let projection = f.cache.snapshot().unwrap();
    let p = projection.parent(id(2), id(100)).unwrap();
    let (_artifacts, parent) = parent_evidence::parent(
        p.work.as_ref().unwrap(),
        p.children[&id(101)].work.as_ref().unwrap(),
        &f.key,
    );
    let token = parent.token().to_string();
    f.evidence.delivery.lock().unwrap().parent(parent);
    f.wire.lock().unwrap().correction_after_acceptance = true;
    let (request, command) = f.command(
        CommandName::Approve,
        json!({"parent":"TEST-100","result":token}),
    );
    assert_eq!(
        f.manager
            .execute(f.actor(), request, command)
            .await
            .unwrap_err()
            .code,
        "operation_uncertain"
    );
    assert_eq!(f.writes(), 1);
    assert_eq!(
        f.wire.lock().unwrap().issues[&id(100)]["state"]["id"],
        json!(id(11))
    );
    assert!(
        f.evidence
            .delivery
            .lock()
            .unwrap()
            .verified_acceptance()
            .is_none()
    );
}
#[tokio::test]
async fn duplicate_provider_publication_cannot_be_adopted_as_one_human_decision() {
    let f = DecisionFixture::new().await;
    f.wire.lock().unwrap().duplicate_publication = true;
    let (request, parent) = f.command(
        CommandName::Correct,
        json!({"parent":"TEST-100","message":"One exact correction"}),
    );
    assert_eq!(
        f.manager
            .execute(f.actor(), request, parent)
            .await
            .unwrap_err()
            .code,
        "operation_uncertain"
    );
    assert_eq!(f.writes(), 1);
    assert!(f.evidence.corrections.lock().unwrap().is_empty());
}

#[tokio::test]
async fn already_accepted_state_still_requires_current_unchanged_acceptance_publication() {
    for tamper in [Some("delete"), Some("edit"), None] {
        let f = DecisionFixture::new().await;
        let projection = f.cache.snapshot().unwrap();
        let p = projection.parent(id(2), id(100)).unwrap();
        let (_artifacts, parent) = parent_evidence::parent(
            p.work.as_ref().unwrap(),
            p.children[&id(101)].work.as_ref().unwrap(),
            &f.key,
        );
        let token = parent.token().to_owned();
        f.evidence.delivery.lock().unwrap().parent(parent);
        {
            let mut wire = f.wire.lock().unwrap();
            wire.issues.get_mut(&id(100)).unwrap()["state"]["id"] = json!(id(15));
            wire.tamper_acceptance_after_first_read = tamper.map(str::to_owned);
        }
        let (request, command) = f.command(
            CommandName::Approve,
            json!({"parent":"TEST-100","result":token}),
        );
        let result = f.manager.execute(f.actor(), request, command).await;
        if tamper.is_some() {
            assert_eq!(result.unwrap_err().code, "operation_uncertain");
            assert!(
                f.evidence
                    .delivery
                    .lock()
                    .unwrap()
                    .verified_acceptance()
                    .is_none()
            );
        } else {
            assert_eq!(result.unwrap().phase, ReceiptPhase::Acknowledged);
            assert!(
                f.evidence
                    .delivery
                    .lock()
                    .unwrap()
                    .verified_acceptance()
                    .is_some()
            );
        }
        assert_eq!(
            f.writes(),
            1,
            "already-accepted tracker state requires a publication but no second state write"
        );
    }
}

#[tokio::test]
async fn acceptance_cache_rejects_another_evidence_token_in_the_same_work_generation() {
    use kegari_v0::evidence::{
        AcceptanceActor, AcceptanceRequest, Publication, prepare_acceptance, verify_acceptance,
    };
    let f = DecisionFixture::new().await;
    let projection = f.cache.snapshot().unwrap();
    let p = projection.parent(id(2), id(100)).unwrap();
    let (_first_files, first) = parent_evidence::parent(
        p.work.as_ref().unwrap(),
        p.children[&id(101)].work.as_ref().unwrap(),
        &f.key,
    );
    let (_second_files, second) = parent_evidence::parent(
        p.work.as_ref().unwrap(),
        p.children[&id(101)].work.as_ref().unwrap(),
        &f.key,
    );
    assert_eq!(first.manifest().work, second.manifest().work);
    assert_ne!(first.token(), second.token());
    let permitted = [id(5).to_string()].into();
    let signed = prepare_acceptance(
        &first,
        AcceptanceRequest {
            supplied_token: first.token(),
            current_work: &first.manifest().work,
            pending_correction: false,
            actor: AcceptanceActor {
                control_actor: "fixture-operator",
                linear_actor: &id(5).to_string(),
                provider_writer: &id(5).to_string(),
                has_accept_role: true,
                permitted_acceptors: &permitted,
            },
            runtime_id: id(1),
            operation_id: Uuid::new_v4(),
            now: Utc::now(),
        },
        &f.key,
    )
    .unwrap();
    let publication = Publication {
        provider: "linear".into(),
        record_id: id(990).to_string(),
        writer_account_id: id(5).to_string(),
        payload_digest: kegari_digest::canonical_digest(1, &signed).unwrap(),
        observed_at: Utc::now(),
        url: "https://linear.invalid/comment/990".into(),
    };
    let acceptance = verify_acceptance(
        &signed,
        &publication,
        &first,
        &first.manifest().work,
        false,
        &permitted,
        &f.key,
    )
    .unwrap();
    let mut cache = kegari_v0::reconcile::ParentDelivery::default();
    cache.parent(second);
    assert_eq!(
        cache.acceptance(acceptance).unwrap_err().code,
        "stale_result"
    );
    assert!(cache.verified_acceptance().is_none());
}

fn prepare_retry_fixture(f: &DecisionFixture) -> (WorkIdentity, Uuid) {
    let projection = f.cache.snapshot().unwrap();
    let parent = projection.parent(id(2), id(100)).unwrap();
    let work = parent.children[&id(101)].work.clone().unwrap();
    let key =
        kegari_v0::authority::IssueKey::new(&work.linear_endpoint, work.linear_workspace, id(101))
            .unwrap()
            .identity()
            .unwrap();
    let phase = kegari_v0::scheduler::PhaseKey {
        work: key.clone(),
        generation: canonical_digest(1, &work.generation).unwrap(),
        phase: "implementation".into(),
        retry_grant: None,
    };
    let store = ReceiptStore::new(PrivateDir::open(&f.root.join("control/receipts")).unwrap());
    let admission = id(888);
    let reference = RunReference {
        normalized_state: Some("todo".into()),
        workspace: id(2),
        account: "codex-fixture".into(),
        phase: phase.clone(),
        profile_digest: canonical_digest(1, &"profile").unwrap(),
        instruction_digest: canonical_digest(1, &"instruction").unwrap(),
        max_attempts: 3,
        max_turns: 20,
        max_review_rounds: 3,
        attempt: 1,
    };
    let (mut run, _) = store
        .prepare(
            &MutationRequest {
                request_id: admission,
                runtime_id: id(1),
                actor: f.actor(),
                kind: "admission".into(),
                canonical_targets: vec![key],
                expected: BTreeMap::from([(
                    "run".into(),
                    serde_json::to_string(&reference).unwrap(),
                )]),
                payload: json!({"synthetic":"durable admission fixture"}),
            },
            1,
        )
        .unwrap();
    run.phase = ReceiptPhase::Reconciled;
    run.observed_result = Some(json!({"outcome":"failed_retryable","reconciled":true}));
    store.save(&run).unwrap();
    *f.evidence.retry.lock().unwrap() = Some(RetryTarget {
        phase,
        admissions: BTreeSet::from([admission]),
    });

    (work, admission)
}

#[tokio::test]
async fn edited_or_deleted_retry_and_correction_publications_never_install_new_authority() {
    for purpose in ["operator_retry", "explicit_correction"] {
        for tamper in ["delete", "edit"] {
            let f = DecisionFixture::new().await;
            let (request, command) = if purpose == "operator_retry" {
                prepare_retry_fixture(&f);
                f.command(
                    CommandName::Retry,
                    json!({"work":"TEST-101","reason":"Exact bounded synthetic retry"}),
                )
            } else {
                f.command(CommandName::Correct,json!({"parent":"TEST-100","message":"Actual explicit correction must remain present"}))
            };
            f.wire.lock().unwrap().tamper_decision = Some((purpose.into(), tamper.into()));
            assert_eq!(
                f.manager
                    .execute(f.actor(), request, command)
                    .await
                    .unwrap_err()
                    .code,
                "operation_uncertain",
                "{purpose} {tamper}"
            );
            assert!(f.evidence.retries.lock().unwrap().is_empty());
            assert!(f.evidence.corrections.lock().unwrap().is_empty());
            assert_eq!(f.writes(), 1);
            if purpose == "explicit_correction" {
                let projection = f.cache.admission_snapshot().unwrap();
                assert!(
                    projection
                        .parent(id(2), id(100))
                        .unwrap()
                        .blockers
                        .iter()
                        .any(|b| b.code == "corrective_publication_pending")
                );
            }
        }
    }
}

#[tokio::test]
async fn stopped_recovery_reaches_backend_with_pending_correction_and_retains_unknown_effect() {
    use kegari_v0::control::{Lease, Lifecycle};
    let f = DecisionFixture::new().await;
    let work = f
        .cache
        .admission_snapshot()
        .unwrap()
        .parent(id(2), id(100))
        .unwrap()
        .key
        .identity()
        .unwrap();
    let mut lifecycle_actor = f.actor();
    lifecycle_actor.roles.insert("lifecycle".into());
    let lifecycle = |generation| MutationRequest {
        request_id: Uuid::new_v4(),
        runtime_id: id(1),
        actor: lifecycle_actor.clone(),
        kind: "lifecycle".into(),
        canonical_targets: vec![id(1).to_string()],
        expected: BTreeMap::new(),
        payload: json!({"generation":generation}),
    };
    f.control
        .lifecycle(lifecycle(0), Lifecycle::Start, 0)
        .unwrap();
    let lease = Lease {
        runtime_id: id(1),
        owner_generation: 1,
        owner_id: Uuid::new_v4(),
    };
    f.control.attach_reconciled(lease.clone()).unwrap();
    let permit = f
        .control
        .admit(
            &lease,
            Uuid::new_v4(),
            work.clone(),
            canonical_digest(1, &"fixture immutable plan").unwrap(),
            || Ok(()),
        )
        .unwrap();
    let external = MutationRequest {
        request_id: Uuid::new_v4(),
        runtime_id: id(1),
        actor: f.actor(),
        kind: "fixture_external_effect".into(),
        canonical_targets: vec![work],
        expected: BTreeMap::new(),
        payload: Value::Null,
    };
    let external_id = external.request_id;
    let (_pending, ()) = f
        .control
        .begin_effect(external, &permit, || Ok(()), |_| Ok(()))
        .unwrap();
    f.control
        .retire_local(
            permit.operation_id,
            json!({"fixture":"local work ended; external outcome unknown"}),
        )
        .unwrap();
    f.control
        .lifecycle(lifecycle(1), Lifecycle::Stop, 1)
        .unwrap();
    f.wire.lock().unwrap().comments.get_mut(&id(100)).unwrap().push(json!({"id":id(999),"body":"Please correct the existing child to preserve the previous inclusive behavior.","createdAt":"2026-01-05T00:00:00Z","updatedAt":"2026-01-05T00:00:00Z","editedAt":null,"url":"https://linear.invalid/comment/999","user":{"id":id(5)},"onBehalfOf":null,"botActor":null,"issue":{"id":id(100)}}));
    let projection = f.cache.refresh().await.unwrap();
    assert!(
        !projection
            .parent(id(2), id(100))
            .unwrap()
            .corrections
            .pending_ids()
            .is_empty()
    );
    let (request,parent)=f.command(CommandName::ReleaseRecover,json!({"parent":"TEST-100","operation":permit.operation_id,"action":"abandon","reason":"Retain uncertain work and retire only known unstarted steps"}));
    let error = f
        .manager
        .execute(f.actor(), request, parent.clone())
        .await
        .unwrap_err();
    assert_eq!(
        error.code, "fixture",
        "actual decision authorization should reach the explicitly non-executing test backend"
    );
    let authorizations = f.evidence.recovery_authorizations.lock().unwrap();
    assert_eq!(authorizations.len(), 1);
    let authorization = &authorizations[0];
    assert_eq!(authorization.phase, ReceiptPhase::Acknowledged);
    assert_eq!(
        authorization.expected["parent_command_id"],
        parent.operation_id.to_string()
    );
    assert_eq!(
        f.control
            .operation(authorization.operation_id)
            .unwrap()
            .payload_digest,
        authorization.payload_digest
    );
    assert!(
        f.control
            .snapshot()
            .unwrap()
            .unresolved_operations
            .contains_key(&external_id)
    );
    assert_eq!(
        f.control.operation(external_id).unwrap().phase,
        ReceiptPhase::Sent
    );
    assert!(f.control.snapshot().unwrap().active_operations.is_empty());
    assert_eq!(f.writes(), 0);
}

#[tokio::test]
async fn retry_finalization_allows_only_its_own_started_budget_and_preserves_holds() {
    use kegari_v0::control::{
        Lease, Lifecycle, PreparationAdmission, PreparationReference, RunAdmission,
    };
    for preparation in [false, true] {
        for unrelated_budget in [false, true] {
            let f = DecisionFixture::new().await;
            prepare_retry_fixture(&f);
            let mut actor = f.actor();
            actor.roles.insert("lifecycle".into());
            f.control
                .lifecycle(
                    MutationRequest {
                        request_id: Uuid::new_v4(),
                        runtime_id: id(1),
                        actor: actor.clone(),
                        kind: "lifecycle".into(),
                        canonical_targets: vec![id(1).to_string()],
                        expected: BTreeMap::new(),
                        payload: json!({"start":true}),
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
            f.control.attach_reconciled(lease.clone()).unwrap();
            let control = f.control.clone();
            let owned = Arc::new(Mutex::new(None));
            let observed = owned.clone();
            *f.evidence.after_retry_publication.lock().unwrap() = Some(Box::new(move |grant| {
                // The provider has acknowledged the grant. Simulate the independent
                // runtime consuming it before the command's final readback gate.
                let mut phase = grant.phase_key();
                if unrelated_budget {
                    phase.retry_grant = Some(Uuid::new_v4());
                }
                let operation = Uuid::new_v4();
                let work = phase.work.clone();
                let permit = if preparation {
                    let reference = PreparationReference {
                        worker_operation: operation,
                        workspace: id(2),
                        account: "codex-fixture".into(),
                        phase,
                    };
                    control.admit_preparation(
                        &lease,
                        PreparationAdmission {
                            request_id: reference.operation_id()?,
                            work_key: work.clone(),
                            scope_digest: canonical_digest(1, &"actual fixture successor")?,
                            reference,
                        },
                        || Ok(()),
                    )?
                } else {
                    control.admit_run(
                        &lease,
                        RunAdmission {
                            request_id: operation,
                            work_key: work.clone(),
                            scope_digest: canonical_digest(1, &"actual fixture successor")?,
                            reference: RunReference {
                                normalized_state: Some("todo".into()),
                                workspace: id(2),
                                account: "codex-fixture".into(),
                                phase,
                                profile_digest: canonical_digest(1, &"profile")?,
                                instruction_digest: canonical_digest(1, &"instructions")?,
                                max_attempts: grant.new_budget().max_attempts,
                                max_turns: grant.new_budget().max_turns,
                                max_review_rounds: grant.new_budget().max_review_rounds,
                                attempt: 1,
                            },
                        },
                        || Ok(()),
                    )?
                };
                let operation = permit.operation_id;
                let effect = Uuid::new_v4();
                let (_pending, ()) = control.begin_effect(
                    MutationRequest {
                        request_id: effect,
                        runtime_id: id(1),
                        actor,
                        kind: "fixture_pending_successor_effect".into(),
                        canonical_targets: vec![work],
                        expected: BTreeMap::new(),
                        payload: Value::Null,
                    },
                    &permit,
                    || Ok(()),
                    |_| Ok(()),
                )?;
                *observed.lock().unwrap() = Some((operation, effect));
                Ok(())
            }));
            let (request, parent) = f.command(
                CommandName::Retry,
                json!({"work":"TEST-101","reason":"Resolved blocker; use one exact bounded retry"}),
            );
            let result = f.manager.execute(f.actor(), request, parent).await;
            if unrelated_budget {
                assert_eq!(result.unwrap_err().code, "operation_uncertain");
                assert!(f.evidence.retries.lock().unwrap().is_empty());
            } else {
                assert_eq!(result.unwrap().phase, ReceiptPhase::Acknowledged);
                assert_eq!(f.evidence.retries.lock().unwrap().len(), 1);
            }
            assert_eq!(f.writes(), 1);
            let (operation, effect) = owned.lock().unwrap().unwrap();
            let snapshot = f.control.snapshot().unwrap();
            assert!(
                snapshot
                    .active_operations
                    .iter()
                    .any(|p| p.operation_id == operation)
            );
            assert!(snapshot.unresolved_operations.contains_key(&effect));
            assert_eq!(
                f.control.operation(effect).unwrap().phase,
                ReceiptPhase::Sent
            );
        }
    }
}
