//! Delivery integration fixtures use production reconciliation/provider parsers.
//! HTTP observations are deterministic fixtures, never live qualification.
#[path = "support/pinned_git.rs"]
mod pinned_git;
#[path = "support/process_probe.rs"]
mod process_probe;
use base64::Engine as _;
use chrono::{Duration, Utc};
use kegari_v0::{
    config::Config,
    control::{Actor, ControlHost, Lease, Lifecycle, MutationRequest},
    delivery::*,
    digest::canonical_digest,
    error::{Error, Result},
    evidence::{AttestationKey, Publication},
    evidence::{ReviewAttestation, ReviewKind, RevisionBinding, Verdict, no_change_digest},
    git::{Git, RepositoryId},
    providers::*,
    reconcile::*,
    storage::PrivateDir,
    workflow::Workflow,
};
use reqwest::header::HeaderMap;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration as StdDuration,
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
    lose_write: bool,
    corrupt_publication: bool,
    run_attempt: u64,
    check_conclusion: String,
    dispatch_count: usize,
    archive: Vec<u8>,
    artifact_name: String,
    extra_check: bool,
    environment_replaced: bool,
    production: String,
    production_tree: String,
    main_reads: usize,
    move_main_on_read: Option<usize>,
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
            lose_write: false,
            corrupt_publication: false,
            run_attempt: 1,
            check_conclusion: "success".into(),
            dispatch_count: 0,
            archive: vec![],
            artifact_name: String::new(),
            extra_check: false,
            environment_replaced: false,
            production: "a".repeat(40),
            production_tree: "b".repeat(40),
            main_reads: 0,
            move_main_on_read: None,
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
            if request.url.host_str() == Some("fixture.blob.invalid") {
                assert!(!request.headers.contains_key("authorization"));
                return Ok(HttpResponse {
                    status: 200,
                    headers: HeaderMap::new(),
                    body: f.archive.clone(),
                });
            }
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
                    "/repos/fixture/repo/environments" => {
                        json!({"total_count":1,"environments":[{"id":88,"name":"production","url":"https://github.invalid/repos/fixture/repo/environments/production","protection_rules":[],"deployment_branch_policy":null}]})
                    }
                    "/repos/fixture/repo/environments/production" => {
                        json!({"id":if f.environment_replaced{89}else{88},"name":"production","url":"https://github.invalid/repos/fixture/repo/environments/production","protection_rules":[],"deployment_branch_policy":null})
                    }
                    "/repos/fixture/repo/actions/runs/50/artifacts" => {
                        json!({"total_count":1,"artifacts":[{"id":80,"name":f.artifact_name}]})
                    }
                    "/repos/fixture/repo/actions/artifacts/80" => {
                        json!({"id":80,"workflow_run":{"id":50,"repository_id":42,"head_sha":"1".repeat(40)},"expired":false,"size_in_bytes":f.archive.len(),"digest":kegari_v0::digest::bytes_digest(&f.archive)})
                    }
                    "/repos/fixture/repo/actions/artifacts/80/zip" => {
                        let mut headers = HeaderMap::new();
                        headers.insert(
                            "location",
                            "https://fixture.blob.invalid/archive?grant=fixture"
                                .parse()
                                .unwrap(),
                        );
                        return Ok(HttpResponse {
                            status: 302,
                            headers,
                            body: vec![],
                        });
                    }
                    "/repos/fixture/repo/git/ref/heads/main" => {
                        f.main_reads += 1;
                        if f.move_main_on_read == Some(f.main_reads) {
                            f.production = "c".repeat(40);
                            f.production_tree = "d".repeat(40);
                        }
                        json!({"ref":"refs/heads/main","object":{"type":"commit","sha":f.production}})
                    }
                    path if path.starts_with("/repos/fixture/repo/git/commits/") => {
                        json!({"sha":f.production,"tree":{"sha":f.production_tree},"parents":[]})
                    }
                    "/repos/fixture/repo/git/ref/heads/trusted" => {
                        json!({"ref":"refs/heads/trusted","object":{"type":"commit","sha":"1".repeat(40)}})
                    }
                    "/repos/fixture/repo/actions/workflows" => {
                        json!({"total_count":1,"workflows":[{"id":3,"path":".github/workflows/caller.yml","state":"active"}]})
                    }
                    "/repos/fixture/repo/actions/workflows/3/dispatches" => {
                        f.dispatch_count += 1;
                        if f.lose_write {
                            return Err(Error::new(
                                "provider_transport",
                                "fixture lost dispatch acknowledgement",
                            ));
                        }
                        json!({"workflow_run_id":50})
                    }
                    "/repos/fixture/repo/actions/runs/50"
                    | "/repos/fixture/repo/actions/runs/50/attempts/1" => {
                        json!({"id":50,"repository":{"id":42},"run_attempt":f.run_attempt,"workflow_id":3,"actor":{"id":7},"triggering_actor":{"id":7},"event":"workflow_dispatch","head_sha":"1".repeat(40),"status":"completed","conclusion":f.check_conclusion,"path":".github/workflows/caller.yml","html_url":"https://github.invalid/fixture/repo/actions/runs/50"})
                    }
                    "/repos/fixture/repo/actions/runs/50/attempts/1/jobs" => {
                        json!({"total_count":1,"jobs":[{"id":70,"run_id":50,"run_attempt":1,"name":"suite","head_sha":"1".repeat(40),"status":"completed","conclusion":f.check_conclusion,"check_run_url":"https://github.invalid/repos/fixture/repo/check-runs/60","steps":[]}]})
                    }
                    path if path.ends_with("/check-runs") => {
                        json!({"total_count":1,"check_runs":[{"id":60,"name":"suite","head_sha":"1".repeat(40),"status":"completed","conclusion":f.check_conclusion,"started_at":"2026-01-04T00:00:00Z","completed_at":"2026-01-04T00:01:00Z"}]})
                    }
                    path => panic!("unexpected GitHub path {path}"),
                }
            } else {
                let query = body["query"].as_str().unwrap();
                let input_id = body["variables"]["id"]
                    .as_str()
                    .and_then(|s| Uuid::parse_str(s).ok());
                let data = if query.contains("KegariComment") && !query.contains("KegariComments") {
                    let issue =
                        Uuid::parse_str(body["variables"]["input"]["issueId"].as_str().unwrap())
                            .unwrap();
                    let comment_id = Uuid::new_v4();
                    let text = body["variables"]["input"]["body"]
                        .as_str()
                        .unwrap()
                        .to_owned();
                    let text = if f.corrupt_publication {
                        format!("tampered {text}")
                    } else {
                        text
                    };
                    f.comments.entry(issue).or_default().push(json!({"id":comment_id,"issue":{"id":issue},"body":text,"createdAt":"2026-01-04T00:00:00Z","updatedAt":"2026-01-04T00:00:00Z","editedAt":null,"user":{"id":id(5),"isMe":true},"botActor":null,"externalUser":null,"url":format!("https://linear.invalid/comment/{comment_id}")}));
                    if f.lose_write {
                        return Err(Error::new(
                            "provider_transport",
                            "injected lost acknowledgement after actual fixture write",
                        ));
                    }
                    json!({"commentCreate":{"success":true,"comment":{"id":comment_id,"issue":{"id":issue},"user":{"id":id(5)}}}})
                } else if query.contains("KegariState") {
                    let issue = Uuid::parse_str(body["variables"]["id"].as_str().unwrap()).unwrap();
                    let state = body["variables"]["input"]["stateId"].clone();
                    f.issues.get_mut(&issue).unwrap()["state"]["id"] = state.clone();
                    json!({"issueUpdate":{"success":true,"issue":{"id":issue,"state":{"id":state}}}})
                } else if query.contains("KegariIdentity") {
                    json!({"viewer":{"id":id(5),"name":"fixture"},"organization":{"id":id(3)}})
                } else if query.contains("KegariIssue") {
                    json!({"issue": f.issues[&input_id.unwrap()]})
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
            let mut value = value;
            if f.extra_check {
                for field in ["jobs", "check_runs"] {
                    if let Some(entries) = value.get_mut(field).and_then(Value::as_array_mut) {
                        let mut second = entries[0].clone();
                        second["id"] = json!(if field == "jobs" { 71 } else { 61 });
                        second["name"] = json!("suite-two");
                        if field == "jobs" {
                            second["check_run_url"] =
                                json!("https://github.invalid/repos/fixture/repo/check-runs/61");
                        }
                        entries.push(second);
                        value["total_count"] = json!(2);
                    }
                }
            }
            Ok(HttpResponse {
                status: 200,
                headers: HeaderMap::new(),
                body: serde_json::to_vec(&value).unwrap(),
            })
        })
    }
}
fn workflow() -> Workflow {
    Workflow::parse("---\nchecks:\n  local:\n    local:\n      argv: ['/usr/bin/true']\n      result_artifact: /private/tmp/result.json\n  integrated:\n    e2e:\n      argv: ['/usr/bin/true']\n      result_artifact: /private/tmp/e2e.json\n---\nUse prepared context", "/private/tmp/WORKFLOW.md").unwrap()
}
async fn refresh(
    config: &Config,
    fixture: &Arc<Mutex<Fixture>>,
    refs: &ReconcileRefs,
) -> RuntimeProjection {
    let mut clients = BTreeMap::new();
    let mut workflows = BTreeMap::new();
    for workspace in config.workspaces.values() {
        let wire: Arc<dyn HttpTransport> = Arc::new(Wire(fixture.clone()));
        let linear = LinearClient::with_transport(
            HttpPolicy::new(
                "https://linear.invalid/graphql",
                id(5).to_string(),
                StdDuration::from_secs(2),
            )
            .unwrap(),
            id(3),
            workspace.linear.clone(),
            Credential::new("fixture".into()).unwrap(),
            Some(wire.clone()),
        )
        .unwrap();
        let github = GithubClient::with_transport(
            HttpPolicy::new(
                "https://github.invalid/",
                "7".into(),
                StdDuration::from_secs(2),
            )
            .unwrap(),
            workspace.github.clone(),
            Credential::new("fixture".into()).unwrap(),
            Some(wire),
        )
        .unwrap();
        clients.insert(workspace.id, WorkspaceClients { linear, github });
        workflows.insert(workspace.id, workflow());
    }
    Reconciler::default()
        .refresh(config, &clients, &workflows, refs)
        .await
        .unwrap()
}
fn publish(projection: &RuntimeProjection, refs: &mut ReconcileRefs) {
    projection.retain_references(refs);
    let parent = projection.parent(id(2), id(100)).unwrap();
    let key = AttestationKey::new("fixture".into(), vec![7; 32]).unwrap();
    let signed = key
        .sign(
            "parent_authorization",
            parent.authorization_candidate.as_ref().unwrap(),
        )
        .unwrap();
    let publication = Publication {
        provider: "linear".into(),
        record_id: id(950).to_string(),
        writer_account_id: id(5).to_string(),
        payload_digest: canonical_digest(1, &signed).unwrap(),
        observed_at: Utc::now(),
        url: "https://linear.invalid/comment/950".into(),
    };
    refs.get_mut(&id(2))
        .unwrap()
        .get_mut(&id(100))
        .unwrap()
        .authorization = Some(
        VerifiedAuthorization::verify(&key, &signed, &publication, &id(5).to_string()).unwrap(),
    );
}

struct DeliveryFixture {
    _temp: tempfile::TempDir,
    coordinator: DeliveryCoordinator,
    host: Arc<ControlHost>,
    admission: DeliveryAdmission,
    key: Arc<AttestationKey>,
    wire: Arc<Mutex<Fixture>>,
    current: Arc<std::sync::atomic::AtomicBool>,
    parent: ParentProjection,
    observer: kegari_v0::deployment::ProductionObserver,
    repository_contract: RepositoryContract,
}
impl DeliveryFixture {
    async fn new() -> Self {
        Self::with_checks(false).await
    }
    async fn with_checks(extra_check: bool) -> Self {
        Self::build(extra_check, false).await
    }
    async fn build(extra_check: bool, candidate: bool) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().canonicalize().unwrap();
        let mut cfg = config();
        let actual_revision = if candidate {
            let source = directory.join("source");
            std::fs::create_dir(&source).unwrap();
            fixture_git(&source, &["init", "-q", "-b", "main"]);
            std::fs::write(source.join("source.txt"), "production\n").unwrap();
            fixture_git(&source, &["add", "source.txt"]);
            fixture_git(
                &source,
                &[
                    "-c",
                    "user.name=Fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                    "commit",
                    "-qm",
                    "production fixture",
                ],
            );
            cfg.workspaces
                .get_mut("example")
                .unwrap()
                .repositories
                .get_mut("42")
                .unwrap()
                .source = source.to_str().unwrap().into();
            Some((
                fixture_git(&source, &["rev-parse", "HEAD"]),
                fixture_git(&source, &["rev-parse", "HEAD^{tree}"]),
            ))
        } else {
            None
        };
        if extra_check {
            cfg.workspaces
                .get_mut("example")
                .unwrap()
                .repositories
                .get_mut("42")
                .unwrap()
                .ci[0]
                .required_checks
                .push("suite-two".into());
        }
        let wire = Fixture::new();
        wire.lock().unwrap().extra_check = extra_check;
        if let Some((commit, tree)) = &actual_revision {
            let mut fixture = wire.lock().unwrap();
            fixture.production = commit.clone();
            fixture.production_tree = tree.clone();
        }
        let mut refs = ReconcileRefs::default();
        let first = refresh(&cfg, &wire, &refs).await;
        publish(&first, &mut refs);
        let projection = refresh(&cfg, &wire, &refs).await;
        let gate = projection
            .parent(id(2), id(100))
            .unwrap()
            .fresh_gate(
                if candidate { None } else { Some(id(101)) },
                GatePurpose::NewOperation,
                &DependencyOutcomes::default(),
                Utc::now(),
                Duration::seconds(60),
            )
            .unwrap();
        let host = Arc::new(
            ControlHost::setup(
                PrivateDir::create(&directory.join("control")).unwrap(),
                id(1),
                Uuid::new_v4(),
            )
            .unwrap(),
        );
        host.lifecycle(
            MutationRequest {
                request_id: Uuid::new_v4(),
                runtime_id: id(1),
                actor: Actor {
                    principal: None,
                    identity: "fixture-operator".into(),
                    roles: BTreeSet::from(["lifecycle".into()]),
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
        host.attach_reconciled(lease.clone()).unwrap();
        let workspace = &cfg.workspaces["example"];
        let contract = RepositoryContract {
            configuration_name: "42".into(),
            repository: RepositoryId {
                hostname: "github.com".into(),
                id: "42".into(),
            },
            provider: RepositoryIdentity {
                id: 42,
                owner: "fixture".into(),
                name: "repo".into(),
                owner_id: 7,
                organization_id: None,
            },
            configuration: workspace.repositories["42"].clone(),
        };
        let revision = RevisionBinding {
            repository: contract.repository.clone(),
            head: actual_revision
                .as_ref()
                .map(|r| r.0.clone())
                .unwrap_or_else(|| "a".repeat(40)),
            base: actual_revision
                .as_ref()
                .map(|r| r.0.clone())
                .unwrap_or_else(|| "a".repeat(40)),
            tested_commit: actual_revision
                .as_ref()
                .map(|r| r.0.clone())
                .unwrap_or_else(|| "a".repeat(40)),
            tested_tree: actual_revision
                .as_ref()
                .map(|r| r.1.clone())
                .unwrap_or_else(|| "b".repeat(40)),
        };
        let scope = DeliveryScope {
            schema_version: 1,
            runtime_id: id(1),
            phase: if candidate {
                DeliveryPhase::CandidateAssembly
            } else {
                DeliveryPhase::NoChange
            },
            work: gate.work().clone(),
            repository: contract.repository.clone(),
            revision,
            fresh_gate_digest: gate.digest().unwrap(),
            contract_digest: contract.digest().unwrap(),
            candidate_vector_digest: None,
        };
        let permit = host
            .admit(
                &lease,
                Uuid::new_v4(),
                gate.work_key().unwrap(),
                scope.digest().unwrap(),
                || Ok(()),
            )
            .unwrap();
        let admission = DeliveryAdmission::bind(permit, gate.clone(), scope).unwrap();
        let current = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let callback_flag = current.clone();
        let callback_parent = projection.parent(id(2), id(100)).unwrap().clone();
        let callback: GateCheck = Arc::new(move |actual| {
            if !callback_flag.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(Error::new("stale_result", "fixture correction arrived"));
            }
            let current_gate = callback_parent.fresh_gate(
                actual.work().child,
                GatePurpose::NewOperation,
                &DependencyOutcomes::default(),
                Utc::now(),
                Duration::seconds(60),
            )?;
            actual.compare_current(&current_gate, Utc::now(), Duration::seconds(60))
        });
        let transport: Arc<dyn HttpTransport> = Arc::new(Wire(wire.clone()));
        let broker = ProviderBroker {
            linear: Some(
                LinearClient::with_transport(
                    HttpPolicy::new(
                        "https://linear.invalid/graphql",
                        id(5).to_string(),
                        StdDuration::from_secs(2),
                    )
                    .unwrap(),
                    id(3),
                    workspace.linear.clone(),
                    Credential::new("fixture".into()).unwrap(),
                    Some(transport.clone()),
                )
                .unwrap(),
            ),
            github: Some(
                GithubClient::with_transport(
                    HttpPolicy::new(
                        "https://github.invalid/",
                        "7".into(),
                        StdDuration::from_secs(2),
                    )
                    .unwrap(),
                    workspace.github.clone(),
                    Credential::new("fixture".into()).unwrap(),
                    Some(transport),
                )
                .unwrap(),
            ),
        };
        let key =
            Arc::new(AttestationKey::new("fixture-attestation".into(), vec![17; 32]).unwrap());
        let observer = kegari_v0::deployment::ProductionObserver::new(
            broker.github.as_ref().unwrap().clone(),
            host.clone(),
            key.clone(),
            PrivateDir::create(&directory.join("deployment-proofs")).unwrap(),
            callback.clone(),
        )
        .unwrap();
        let repository_contract = contract.clone();
        let coordinator = DeliveryCoordinator::new(
            DeliveryServices {
                host: host.clone(),
                broker,
                records: Arc::new(PrivateDir::create(&directory.join("records")).unwrap()),
                key: key.clone(),
                git: Arc::new(Git::new(pinned_git::prepare(&directory)).unwrap()),
                retained_git_sources: BTreeMap::new(),
                current_gate: callback,
                executions: Arc::new(kegari_v0::worker::ExecutionRegistry::default()),
                processes: Arc::new(
                    kegari_v0::process_records::ProcessJournal::with_probe(
                        PrivateDir::create(&directory.join("processes")).unwrap(),
                        cfg.runtime.id,
                        Arc::new(process_probe::FixtureBootProbe::new()),
                    )
                    .unwrap(),
                ),
            },
            vec![contract],
        )
        .unwrap();
        Self {
            _temp: temp,
            coordinator,
            host,
            admission,
            key,
            wire,
            current,
            parent: projection.parent(id(2), id(100)).unwrap().clone(),
            observer,
            repository_contract,
        }
    }
    fn review(&self) -> kegari_v0::evidence::Signed<ReviewAttestation> {
        let scope = self.admission.scope();
        let delta = no_change_digest(
            &scope.work,
            &scope.revision,
            &BTreeSet::from(["exact outcome".into()]),
            "behavior already exists",
        )
        .unwrap();
        self.key
            .sign(
                "codex_review",
                &ReviewAttestation {
                    schema_version: 1,
                    runtime_id: id(1),
                    owner_generation: 1,
                    operation_id: self.admission.operation_id(),
                    work: scope.work.clone(),
                    revision: scope.revision.clone(),
                    pull_request: None,
                    delta_digest: delta,
                    verdict: Verdict::Pass,
                    unresolved_findings: vec![],
                    supporting_links: vec![
                        "https://github.invalid/fixture/repo/actions/runs/1".into(),
                    ],
                    review: ReviewKind::Agent {
                        session_id: "deterministic-fixture-review-session".into(),
                        instruction_manifest: canonical_digest(1, &json!("fixture instructions"))
                            .unwrap(),
                    },
                },
            )
            .unwrap()
    }
    fn writes(&self) -> usize {
        self.wire
            .lock()
            .unwrap()
            .comments
            .get(&id(101))
            .map_or(0, Vec::len)
    }
}

#[tokio::test]
async fn nochange_review_requires_observed_signed_publication_and_reuses_one_durable_write() {
    let f = DeliveryFixture::new().await;
    let signed = f.review();
    let first = f
        .coordinator
        .publish_review(&f.admission, &signed)
        .await
        .unwrap();
    assert_eq!(first.verified().digest(), signed.digest().unwrap());
    assert_eq!(first.publication().provider, "linear");
    assert_eq!(f.writes(), 1);
    let repeated = f
        .coordinator
        .publish_review(&f.admission, &signed)
        .await
        .unwrap();
    assert_eq!(
        repeated.publication().record_id,
        first.publication().record_id
    );
    assert_eq!(f.writes(), 1);
}

#[tokio::test]
async fn unknown_publication_is_not_blindly_repeated_and_does_not_mint_review_evidence() {
    let f = DeliveryFixture::new().await;
    f.wire.lock().unwrap().lose_write = true;
    assert_eq!(
        f.coordinator
            .publish_review(&f.admission, &f.review())
            .await
            .unwrap_err()
            .code,
        "operation_uncertain"
    );
    assert_eq!(f.writes(), 1);
    f.wire.lock().unwrap().lose_write = false;
    assert!(
        f.coordinator
            .publish_review(&f.admission, &f.review())
            .await
            .is_err()
    );
    assert_eq!(f.writes(), 1);
    assert!(!f.host.snapshot().unwrap().unresolved_operations.is_empty());
}

#[tokio::test]
async fn tampered_publication_and_changed_authority_cannot_promote_a_review() {
    let f = DeliveryFixture::new().await;
    f.wire.lock().unwrap().corrupt_publication = true;
    assert_eq!(
        f.coordinator
            .publish_review(&f.admission, &f.review())
            .await
            .unwrap_err()
            .code,
        "evidence_unpublished"
    );
    assert_eq!(f.writes(), 1);
    let other = DeliveryFixture::new().await;
    other
        .current
        .store(false, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(
        other
            .coordinator
            .publish_review(&other.admission, &other.review())
            .await
            .unwrap_err()
            .code,
        "stale_result"
    );
    assert_eq!(other.writes(), 0);
}

#[tokio::test]
async fn wrong_signed_work_and_failed_review_never_grant_a_successful_gate() {
    let f = DeliveryFixture::new().await;
    let mut wrong = f.review();
    wrong.payload.work.parent = Uuid::new_v4();
    wrong = f.key.sign("codex_review", &wrong.payload).unwrap();
    assert_eq!(
        f.coordinator
            .publish_review(&f.admission, &wrong)
            .await
            .unwrap_err()
            .code,
        "stale_result"
    );
    assert_eq!(f.writes(), 0);
    let mut failed = f.review();
    failed.payload.verdict = Verdict::Findings;
    failed
        .payload
        .unresolved_findings
        .push("fixture finding".into());
    failed = f.key.sign("codex_review", &failed.payload).unwrap();
    assert_eq!(
        f.coordinator
            .publish_review(&f.admission, &failed)
            .await
            .unwrap_err()
            .code,
        "review_gate"
    );
    assert_eq!(f.writes(), 1);
}

// Public synthetic key used only for deterministic OIDC fixtures.
const FIXTURE_KEY: &str = "MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQDFTEX6ChIN+tc0MHn6ey3eu77bUMu7G62F77UjloREcMWQSyO2wSvUf6lXqZ3j7QLOgOjIEUrSpalLDe9uTDnHsJ4qe+QfFiGDdFIVtsTpFGRAtJu0SONWAWZeSVnQe79ssv9UXGRHCQwgvLbUHCtDp6fFZTEnNnuAWknStUhNHJjQ5pog5V1aFTzvJWDVy8/tYTrLY0WJtB8j3wxaATRnZKh6LQvaiqPKvjuMpqjOuPBWL7Sfw6EtsIUsbdKN3JCwwaVBw8SUtxA+pqCUohOPO7rJUvAAk+WHI6AOIrYvgOzX8WBFpc/FwucT/7CnvDOS3QRBTLRRn1ifeU41K9pzAgMBAAECggEATn7MEHDVQg6cdD0dKpS7aTRU+LiqdaTLYtA9xwzpTrl3QLNEYD92irLH0wK+uKHhTftnocGPfUM/mYbPzZgyHMqeAtULq3qIlt4+kjkHGpxsTnoMwKuHFv6zlaulOhchz8NfZlbj5oco9hVewAD9DYu0agqahbldZXGGkXhibZrR+JX1qLn6zMYxdRjAgRwBvC4YmRKi5CVKanetswwheOblGEXJMfl+9PbCJpG4DiDuPe4q8S7iczCVMnrBrzgLtor+Zzgue5uc48nR9QuuE1t5u5ioP3E04YCAsLj31lOnH2ycrVufGalY0tFujDlpkYYiuQi3s7vAn2MEqerA0QKBgQDsoGOn9Pm6yL2Q5GQQEJlbYayGCWgUuT59JdFzMUkA7G8bPDXU+3Y5+1JtNp+1itV9hAYwao4iGtBXBEVpEwUNZkyxO9wJvXsZOHAZL2RFCZlcQSVpacFXcm+/JNPuDGIlvJT1WycEKbq5/kURYY6OfN9zLP+1AEuRag8p33D+XQKBgQDVc5IQSX8W7P7ar3umP74zCWt+bpK+swrYK/FH6oNVskp/uIIHh8T5kF8SFppQwXQlSbRlcabNWAFhtjBD1kqSuCOSExNTW+CXf7xfi3AVp5IFZEdUW7PcMAXg3Cyg6RmAP4OmdDPbt+lIg+0TCMGHXebnY7dINCIdOYClD+OPDwKBgB0MD0+13dYVAFhCARqLvxedpN0LWJB6sPYR52U5npmTbIwuVvlfKQMzG8o2U2KouU5gH5Zf7jS3j+2uo4qjgBJGBZIYJAm6DdtptzEQRUHITOArSxlmlwpSDO+s6KsnMAEr5ik8WEzFhpuAYCAjFqndIFz2ICmq8JvYJiZgvHEtAoGAElmHGwNWe6AQILipRpeAQF708L+MufwmWIMekPImLD3Rjb1M4FzZoCK7KZDr3zliRLpbWcU1c723ELFa4NoROx+9Xxdot+fUHPIMPaJ9Fj97nB8MysG/apRuTPBgncysNow/QxYGR0M+OM+ul85O5NY7vXSjEv/4Vs7Uslv1Uh0CgYEAiEa8RJbaWjOi3vxlhsjVhlUBNR6LK7s8oQYjutn0oeJLZn4kSrYEtkhXjkjkntvY+5BvhAq5ZVCXiGhS5W+gw19cdVSl6yN33QNbBluq3Lj/ZLJYbOwtLr2PgAp+Hu8GktAOhRuMI4YoOkoS5xK9ZjKvS4BnJCnm0wF+7c1+08k=";
const FIXTURE_MODULUS: &str = "xUxF-goSDfrXNDB5-nst3ru-21DLuxuthe-1I5aERHDFkEsjtsEr1H-pV6md4-0CzoDoyBFK0qWpSw3vbkw5x7CeKnvkHxYhg3RSFbbE6RRkQLSbtEjjVgFmXklZ0Hu_bLL_VFxkRwkMILy21BwrQ6enxWUxJzZ7gFpJ0rVITRyY0OaaIOVdWhU87yVg1cvP7WE6y2NFibQfI98MWgE0Z2Soei0L2oqjyr47jKaozrjwVi-0n8OhLbCFLG3SjdyQsMGlQcPElLcQPqaglKITjzu6yVLwAJPlhyOgDiK2L4Ds1_FgRaXPxcLnE_-wp7wzkt0EQUy0UZ9Yn3lONSvacw";
fn token(claims: Value) -> String {
    let encode = |v: Value| {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(serde_json::to_vec(&v).unwrap())
    };
    let signed = format!(
        "{}.{}",
        encode(json!({"alg":"RS256","kid":"fixture-key","typ":"JWT"})),
        encode(claims)
    );
    let der = base64::engine::general_purpose::STANDARD
        .decode(FIXTURE_KEY)
        .unwrap();
    let key = ring::signature::RsaKeyPair::from_pkcs8(&der).unwrap();
    let mut signature = vec![0; key.public().modulus_len()];
    key.sign(
        &ring::signature::RSA_PKCS1_SHA256,
        &ring::rand::SystemRandom::new(),
        signed.as_bytes(),
        &mut signature,
    )
    .unwrap();
    format!(
        "{signed}.{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(signature)
    )
}
fn verifier() -> HostedProofVerifier {
    HostedProofVerifier::from_trusted_jwks(&json!({"keys":[{"kid":"fixture-key","alg":"RS256","kty":"RSA","use":"sig","n":FIXTURE_MODULUS,"e":"AQAB"}]})).unwrap()
}

fn hosted_token(f: &DeliveryFixture, pending: &PendingExactValidation) -> HostedToken {
    hosted_token_for(f, pending, 60)
}
fn hosted_token_for(
    f: &DeliveryFixture,
    pending: &PendingExactValidation,
    check_id: u64,
) -> HostedToken {
    let e = HostedProofExpectation {
        context_digest: None,
        runtime_id: id(1),
        operation_id: f.admission.operation_id(),
        challenge: pending.challenge().challenge,
        repository_id: 42,
        repository_owner_id: 7,
        run_id: 50,
        run_attempt: 1,
        check_run_id: check_id,
        actor_id: 7,
        trigger_sha: "1".repeat(40),
        tested_source_sha: f.admission.scope().revision.tested_commit.clone(),
        workflow_ref: "fixture/repo/.github/workflows/caller.yml@refs/heads/trusted".into(),
        workflow_sha: "1".repeat(40),
        job_workflow_ref: format!(
            "fixture/trusted/.github/workflows/proof.yml@{}",
            "1".repeat(40)
        ),
        job_workflow_sha: "1".repeat(40),
        event_name: "workflow_dispatch".into(),
    };
    let now = Utc::now().timestamp();
    let claims = json!({"iss":GITHUB_OIDC_ISSUER,"aud":e.audience().unwrap(),"sub":"repo:fixture/repo:ref:refs/heads/trusted","jti":Uuid::new_v4(),"iat":now,"nbf":now-10,"exp":now+300,
        "repository_id":"42","repository_owner_id":"7","run_id":"50","run_attempt":"1","check_run_id":check_id.to_string(),"actor_id":"7","sha":e.trigger_sha,"workflow_ref":e.workflow_ref,"workflow_sha":e.workflow_sha,"job_workflow_ref":e.job_workflow_ref,"job_workflow_sha":e.job_workflow_sha,"runner_environment":"github-hosted","event_name":e.event_name});
    HostedToken {
        expectation: e,
        token: token(claims),
    }
}
#[tokio::test]
async fn exact_dispatch_is_one_owned_request_and_proves_source_distinct_from_trigger() {
    let f = DeliveryFixture::new().await;
    let pending = f
        .coordinator
        .dispatch_exact_validation(&f.admission)
        .await
        .unwrap();
    let repeated = f
        .coordinator
        .dispatch_exact_validation(&f.admission)
        .await
        .unwrap();
    assert_eq!(pending.receipt().request_id, repeated.receipt().request_id);
    assert_eq!(
        pending.challenge().challenge,
        repeated.challenge().challenge
    );
    assert_eq!(f.wire.lock().unwrap().dispatch_count, 1);
    install_hosted_archive(&f, &pending, hosted_token(&f, &pending), None);
    f.coordinator
        .acquire_exact_validation(&f.admission, &pending, &verifier())
        .await
        .unwrap();
    let wire = f.wire.lock().unwrap();
    let (_, body) = wire
        .requests
        .iter()
        .find(|(url, _)| url.ends_with("/dispatches"))
        .unwrap();
    assert_eq!(body["ref"], "trusted");
    assert_eq!(body["inputs"]["source_sha"], "a".repeat(40));
    assert_eq!(
        body["inputs"]["operation_id"],
        pending.receipt().request_id.to_string()
    );
    assert_eq!(
        body["inputs"]["kegari_challenge"],
        pending.challenge().challenge.to_string()
    );
    assert!(
        wire.requests
            .iter()
            .any(|(url, _)| url.contains(&format!("/commits/{}/check-runs", "1".repeat(40))))
    );
}
#[tokio::test]
async fn exact_dispatch_unknown_and_failed_or_other_challenge_do_not_pass() {
    let f = DeliveryFixture::new().await;
    f.wire.lock().unwrap().lose_write = true;
    assert_eq!(
        f.coordinator
            .dispatch_exact_validation(&f.admission)
            .await
            .unwrap_err()
            .code,
        "operation_uncertain"
    );
    f.wire.lock().unwrap().lose_write = false;
    let unknown = f
        .coordinator
        .dispatch_exact_validation(&f.admission)
        .await
        .unwrap();
    assert!(!unknown.receipt().remote_ids.contains_key("workflow_run_id"));
    assert_eq!(f.wire.lock().unwrap().dispatch_count, 1);
    let f = DeliveryFixture::new().await;
    let pending = f
        .coordinator
        .dispatch_exact_validation(&f.admission)
        .await
        .unwrap();
    let mut jwt = hosted_token(&f, &pending);
    jwt.token = token(json!({"invalid":"wrong signed source"}));
    install_hosted_archive(&f, &pending, jwt, None);
    assert_eq!(
        f.coordinator
            .acquire_exact_validation(&f.admission, &pending, &verifier())
            .await
            .unwrap_err()
            .code,
        "invalid_hosted_proof"
    );
    install_hosted_archive(&f, &pending, hosted_token(&f, &pending), None);
    f.wire.lock().unwrap().check_conclusion = "skipped".into();
    assert!(
        f.coordinator
            .acquire_exact_validation(&f.admission, &pending, &verifier())
            .await
            .is_err()
    );
    f.wire.lock().unwrap().check_conclusion = "success".into();
    f.wire.lock().unwrap().run_attempt = 2;
    assert!(
        f.coordinator
            .acquire_exact_validation(&f.admission, &pending, &verifier())
            .await
            .is_err()
    );
}

async fn retained_child_fixture(f: &DeliveryFixture) -> Uuid {
    use kegari_v0::evidence::*;
    let review = f.review();
    let published = f
        .coordinator
        .publish_review(&f.admission, &review)
        .await
        .unwrap();
    let pending = f
        .coordinator
        .dispatch_exact_validation(&f.admission)
        .await
        .unwrap();
    let jwt = hosted_token(f, &pending);
    let proof_dir = PrivateDir::create(
        &f._temp
            .path()
            .canonicalize()
            .unwrap()
            .join("retained-fixture-proof"),
    )
    .unwrap();
    let proof = verifier()
        .verify_and_record(&jwt.token, &jwt.expectation, Utc::now(), &proof_dir)
        .unwrap();
    let scope = f.admission.scope();
    let now = Utc::now();
    let artifact = ArtifactReference {
        reference: "deterministic-fixture:executed-check-record".into(),
        digest: canonical_digest(1, &json!("fixture artifact")).unwrap(),
    };
    let mut runs = Vec::new();
    for kind in [CheckKind::Local, CheckKind::HostedCi] {
        let local = kind == CheckKind::Local;
        runs.push(CheckRun {
            id: if local {
                "local".into()
            } else {
                "suite".into()
            },
            kind,
            operation_id: f.admission.operation_id(),
            work_digest: scope.work.digest().unwrap(),
            revision: scope.revision.clone(),
            workflow_id: if local { None } else { Some(3) },
            run_id: if local { None } else { Some(50) },
            attempt: 1,
            runner: if local {
                Runner::LocalSandbox
            } else {
                Runner::GitHubHosted
            },
            command: if local {
                vec!["/usr/bin/true".into()]
            } else {
                vec![]
            },
            environment_identity: if local {
                "explicit deterministic local fixture".into()
            } else {
                canonical_digest(1, &proof).unwrap()
            },
            started_at: Some("2026-01-04T00:00:00Z".parse().unwrap()),
            completed_at: Some("2026-01-04T00:01:00Z".parse().unwrap()),
            conclusion: Conclusion::Success,
            artifacts: vec![artifact.clone()],
        });
    }
    let record = ChildResultRecord {
        schema_version: 1,
        runtime_id: id(1),
        publication_operation: Uuid::new_v4(),
        contract_digest: scope.contract_digest.clone(),
        workflow_digest: workflow().digest,
        evidence: ChildEvidence {
            schema_version: 1,
            work: scope.work.clone(),
            revision: scope.revision.clone(),
            fulfilled_criteria: BTreeSet::from(["exact outcome".into()]),
            outcome: ChildOutcome::NoChange {
                reason: "behavior already exists".into(),
                change_gates_inapplicable: BTreeSet::from([
                    "branch".into(),
                    "pull_request".into(),
                    "staging_merge".into(),
                ]),
                exact_sha_validation: "deterministic fixture retained dispatch".into(),
            },
            checks: CheckSnapshot {
                pagination_complete: true,
                observed_at: now,
                runs,
            },
            review_digest: published.verified().digest().into(),
            results_for_dependents: vec![artifact],
        },
        review,
        review_publication: published.publication().clone(),
        review_status: None,
        hosted_proofs: vec![proof],
        protection: None,
    };
    let signed = f.key.sign("child_result", &record).unwrap();
    let comment_id = Uuid::new_v4();
    f.wire.lock().unwrap().comments.entry(id(101)).or_default().push(json!({"id":comment_id,"issue":{"id":id(101)},"body":encode_signed_publication("child_result",&signed).unwrap(),"createdAt":"2026-01-04T00:00:00Z","updatedAt":"2026-01-04T00:00:00Z","editedAt":null,"user":{"id":id(5),"isMe":true},"botActor":null,"externalUser":null,"url":format!("https://linear.invalid/comment/{comment_id}")}));
    comment_id
}
#[tokio::test]
async fn child_recovery_requires_authentic_complete_record_and_closure_replays_once() {
    let f = DeliveryFixture::new().await;
    let comment = retained_child_fixture(&f).await;
    let criteria = BTreeSet::from(["exact outcome".into()]);
    let child = f
        .coordinator
        .recover_child_result(&f.admission, &workflow(), &criteria, comment)
        .await
        .unwrap();
    assert_eq!(
        f.wire.lock().unwrap().issues[&id(101)]["state"]["id"],
        json!(id(11))
    );
    let first = f
        .coordinator
        .close_child(&f.admission, &child, &config())
        .await
        .unwrap();
    let replay = f
        .coordinator
        .close_child(&f.admission, &child, &config())
        .await
        .unwrap();
    assert_eq!(first.request_id, replay.request_id);
    let wire = f.wire.lock().unwrap();
    assert_eq!(wire.issues[&id(101)]["state"]["id"], json!(id(14)));
    assert_eq!(
        wire.requests
            .iter()
            .filter(|(_, b)| b["query"]
                .as_str()
                .is_some_and(|q| q.contains("KegariState")))
            .count(),
        1
    );
}
#[tokio::test]
async fn stopped_child_hydration_reads_current_gates_without_admitting_or_writing() {
    let f = DeliveryFixture::new().await;
    let comment = retained_child_fixture(&f).await;
    let observation = f
        .coordinator
        .observation(
            f.admission.fresh_gate().clone(),
            f.admission.scope().revision.clone(),
        )
        .unwrap();
    let snapshot = f.host.snapshot().unwrap();
    f.host
        .lifecycle(
            MutationRequest {
                request_id: Uuid::new_v4(),
                runtime_id: id(1),
                actor: Actor {
                    principal: None,
                    identity: "fixture-operator".into(),
                    roles: BTreeSet::from(["lifecycle".into()]),
                },
                kind: "lifecycle".into(),
                canonical_targets: vec![id(1).to_string()],
                expected: BTreeMap::new(),
                payload: Value::Null,
            },
            Lifecycle::ForceStop,
            snapshot.intent.generation,
        )
        .unwrap();
    let criteria = BTreeSet::from(["exact outcome".into()]);
    assert!(
        f.coordinator
            .recover_child_result(&f.admission, &workflow(), &criteria, comment)
            .await
            .is_err()
    );
    let writes = f
        .wire
        .lock()
        .unwrap()
        .requests
        .iter()
        .filter(|(_, b)| b["query"].as_str().is_some_and(|q| q.contains("mutation")))
        .count();
    let before = f.host.snapshot().unwrap();
    let child = f
        .coordinator
        .hydrate_child_result(&observation, &workflow(), &criteria, comment)
        .await
        .unwrap();
    assert_eq!(child.verified().evidence().work, f.admission.scope().work);
    let parent_gate = f
        .parent
        .fresh_gate(
            None,
            GatePurpose::NewOperation,
            &DependencyOutcomes::default(),
            Utc::now(),
            Duration::seconds(60),
        )
        .unwrap();
    let contract = f
        .coordinator
        .parent_contract(parent_gate, &f.parent, &workflow())
        .unwrap();
    let gates = BTreeMap::from([(id(101), f.admission.fresh_gate().clone())]);
    let refreshed = f
        .coordinator
        .refresh_delivery(
            ParentHydrationInputs {
                contract: &contract,
                observer: &f.observer,
                child_gates: &gates,
                workflow: &workflow(),
                production_integrations: &[],
            },
            &config(),
        )
        .await
        .unwrap();
    // E2E source/local/CI/review proof alone is not an integrated candidate
    // responsibility. Recovery must not expose it as a completed dependency.
    assert!(
        !refreshed
            .parent_delivery()
            .verified_children()
            .contains_key(&id(101))
    );
    assert!(refreshed.parent_delivery().verified_parent().is_none());
    assert_eq!(
        f.host.snapshot().unwrap().intent.generation,
        before.intent.generation
    );
    assert_eq!(
        f.wire
            .lock()
            .unwrap()
            .requests
            .iter()
            .filter(|(_, b)| b["query"].as_str().is_some_and(|q| q.contains("mutation")))
            .count(),
        writes
    );
    f.wire.lock().unwrap().run_attempt = 2;
    assert_eq!(
        f.coordinator
            .hydrate_child_result(&observation, &workflow(), &criteria, comment)
            .await
            .unwrap_err()
            .code,
        "validation_gate"
    );
    f.current.store(false, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(
        f.coordinator
            .hydrate_child_result(&observation, &workflow(), &criteria, comment)
            .await
            .unwrap_err()
            .code,
        "stale_result"
    );
}
#[tokio::test]
async fn tracker_done_and_edited_signed_result_cannot_substitute_for_completion() {
    let f = DeliveryFixture::new().await;
    let comment = retained_child_fixture(&f).await;
    {
        let mut wire = f.wire.lock().unwrap();
        wire.issues.get_mut(&id(101)).unwrap()["state"]["id"] = json!(id(14));
        let c = wire
            .comments
            .get_mut(&id(101))
            .unwrap()
            .iter_mut()
            .find(|c| c["id"] == json!(comment))
            .unwrap();
        c["body"] = json!(
            c["body"]
                .as_str()
                .unwrap()
                .replace("exact outcome", "unreviewed outcome")
        );
    }
    assert_eq!(
        f.coordinator
            .recover_child_result(
                &f.admission,
                &workflow(),
                &BTreeSet::from(["exact outcome".into()]),
                comment
            )
            .await
            .unwrap_err()
            .code,
        "invalid_attestation"
    );
}

fn zip_fixture(name: &str, data: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    writer
        .start_file(
            name,
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored),
        )
        .unwrap();
    writer.write_all(data).unwrap();
    writer.finish().unwrap().into_inner()
}
fn install_hosted_archive(
    f: &DeliveryFixture,
    pending: &PendingExactValidation,
    jwt: HostedToken,
    source: Option<&str>,
) {
    let scope = f.admission.scope();
    let mut proofs = vec![json!({"check_run_id":60,"token":jwt.token})];
    if f.wire.lock().unwrap().extra_check {
        proofs.push(json!({"check_run_id":61,"token":hosted_token_for(f,pending,61).token}));
    }
    let value = json!({"schema_version":1,"runtime_id":id(1),"phase_operation":f.admission.operation_id(),"dispatch_request":pending.receipt().request_id,"challenge":pending.challenge().challenge,
        "repository_id":42,"run_id":50,"run_attempt":1,"tested_tree":scope.revision.tested_tree,"outputs":{"source_sha":source.unwrap_or(&scope.revision.tested_commit)},"proofs":proofs});
    let mut wire = f.wire.lock().unwrap();
    wire.archive = zip_fixture(
        "kegari-validation.json",
        &serde_json::to_vec(&value).unwrap(),
    );
    wire.artifact_name = format!("kegari-validation-{}-1", f.admission.operation_id());
}
#[test]
fn hosted_zip_rejects_traversal_symlinks_oversize_and_corrupt_crc_without_extracting() {
    let valid = json!({"schema_version":1,"runtime_id":id(1),"phase_operation":id(2),"dispatch_request":id(3),"challenge":id(4),"repository_id":42,"run_id":50,"run_attempt":1,"tested_tree":"b".repeat(40),"outputs":{"source_sha":"a".repeat(40)},"proofs":[]});
    let bytes = serde_json::to_vec(&valid).unwrap();
    assert!(read_hosted_source_archive(&zip_fixture("kegari-validation.json", &bytes)).is_ok());
    for name in [
        "../kegari-validation.json",
        "/kegari-validation.json",
        "dir/../kegari-validation.json",
        "dir\\kegari-validation.json",
    ] {
        assert!(read_hosted_source_archive(&zip_fixture(name, &bytes)).is_err());
    }
    assert!(
        read_hosted_source_archive(&zip_fixture("kegari-validation.json", &vec![0; 1_048_577]))
            .is_err()
    );
    let mut bad = zip_fixture("kegari-validation.json", &bytes);
    let pos = bad.windows(bytes.len()).position(|s| s == bytes).unwrap();
    bad[pos] ^= 1;
    assert_eq!(
        read_hosted_source_archive(&bad).unwrap_err().code,
        "hosted_artifact"
    );
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    writer
        .add_symlink(
            "kegari-validation.json",
            "/tmp/escape",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
    assert!(read_hosted_source_archive(&writer.finish().unwrap().into_inner()).is_err());
}
#[tokio::test]
async fn real_downloaded_output_must_name_tested_source_and_survive_archive_checks() {
    let f = DeliveryFixture::new().await;
    let pending = f
        .coordinator
        .dispatch_exact_validation(&f.admission)
        .await
        .unwrap();
    install_hosted_archive(
        &f,
        &pending,
        hosted_token(&f, &pending),
        Some(&"c".repeat(40)),
    );
    assert_eq!(
        f.coordinator
            .acquire_exact_validation(&f.admission, &pending, &verifier())
            .await
            .unwrap_err()
            .code,
        "hosted_artifact"
    );
    install_hosted_archive(&f, &pending, hosted_token(&f, &pending), None);
    f.wire.lock().unwrap().archive = zip_fixture("../kegari-validation.json", b"{}");
    assert_eq!(
        f.coordinator
            .acquire_exact_validation(&f.admission, &pending, &verifier())
            .await
            .unwrap_err()
            .code,
        "hosted_artifact"
    );
}

#[tokio::test]
async fn child_recovery_reobserves_review_and_later_hosted_attempt() {
    let f = DeliveryFixture::new().await;
    let comment = retained_child_fixture(&f).await;
    f.wire.lock().unwrap().run_attempt = 2;
    let criteria = BTreeSet::from(["exact outcome".into()]);
    assert_eq!(
        f.coordinator
            .recover_child_result(&f.admission, &workflow(), &criteria, comment)
            .await
            .unwrap_err()
            .code,
        "validation_gate"
    );
    f.wire.lock().unwrap().run_attempt = 1;
    f.wire
        .lock()
        .unwrap()
        .comments
        .get_mut(&id(101))
        .unwrap()
        .retain(|c| c["id"] == json!(comment));
    assert_eq!(
        f.coordinator
            .recover_child_result(&f.admission, &workflow(), &criteria, comment)
            .await
            .unwrap_err()
            .code,
        "evidence_unpublished"
    );
}

#[test]
fn shared_protection_parser_blocks_missing_contexts_and_malformed_approvals() {
    let identity = RepositoryId {
        hostname: "github.com".into(),
        id: "42".into(),
    };
    let repository = Repository {
        identity: RepositoryIdentity {
            id: 42,
            owner: "fixture".into(),
            name: "repo".into(),
            owner_id: 7,
            organization_id: None,
        },
        default_branch: "main".into(),
        private: true,
        archived: false,
        disabled: false,
        squash_allowed: Some(true),
        permissions: json!({"push":true}),
    };
    let policy = json!({"required_status_checks":{"strict":true,"contexts":["suite","kegari/codex-review"]},"required_pull_request_reviews":null});
    let required = BTreeSet::from(["suite".into(), "kegari/codex-review".into()]);
    assert!(
        validate_repository_prerequisites(
            &identity,
            &repository,
            "staging",
            &"a".repeat(40),
            &policy,
            &[],
            &required
        )
        .is_ok()
    );
    for review in [
        json!({"required_approving_review_count":1}),
        json!({"required_approving_review_count":"0"}),
        json!({"require_code_owner_reviews":"false"}),
        json!({"require_code_owner_reviews":true}),
    ] {
        let mut invalid = policy.clone();
        invalid["required_pull_request_reviews"] = review;
        assert!(
            validate_repository_prerequisites(
                &identity,
                &repository,
                "staging",
                &"a".repeat(40),
                &invalid,
                &[],
                &required
            )
            .is_err()
        );
    }
    let mut missing = policy;
    missing["required_status_checks"]["contexts"] = json!(["suite"]);
    assert!(
        validate_repository_prerequisites(
            &identity,
            &repository,
            "staging",
            &"a".repeat(40),
            &missing,
            &[],
            &required
        )
        .is_err()
    );
}

#[tokio::test]
async fn multiple_required_jobs_keep_their_own_signed_check_identity_when_reobserved() {
    let f = DeliveryFixture::with_checks(true).await;
    let pending = f
        .coordinator
        .dispatch_exact_validation(&f.admission)
        .await
        .unwrap();
    install_hosted_archive(&f, &pending, hosted_token(&f, &pending), None);
    let passed = f
        .coordinator
        .acquire_exact_validation(&f.admission, &pending, &verifier())
        .await
        .unwrap();
    f.coordinator
        .observe_owned_validation_current(&f.admission, &passed)
        .await
        .unwrap();
}

#[tokio::test]
async fn lost_publication_ack_is_reconciled_from_one_real_record_and_never_resent() {
    let f = DeliveryFixture::new().await;
    let signed = f.review();
    f.wire.lock().unwrap().lose_write = true;
    assert!(
        f.coordinator
            .publish_review(&f.admission, &signed)
            .await
            .is_err()
    );
    f.wire.lock().unwrap().lose_write = false;
    let actor = Actor {
        principal: None,
        identity: "fixture-recovery-operator".into(),
        roles: BTreeSet::from(["recover".into()]),
    };
    let receipt = f
        .coordinator
        .reconcile_receipt(
            &actor,
            f.admission.operation_id(),
            "publish-no-change-review",
        )
        .await
        .unwrap();
    assert_eq!(receipt.phase, kegari_v0::control::ReceiptPhase::Reconciled);
    assert!(f.host.snapshot().unwrap().unresolved_operations.is_empty());
    f.coordinator
        .publish_review(&f.admission, &signed)
        .await
        .unwrap();
    assert_eq!(f.writes(), 1);
}
#[tokio::test]
async fn ambiguous_or_missing_publication_cannot_release_its_unknown_hold() {
    let f = DeliveryFixture::new().await;
    f.wire.lock().unwrap().lose_write = true;
    assert!(
        f.coordinator
            .publish_review(&f.admission, &f.review())
            .await
            .is_err()
    );
    {
        let mut wire = f.wire.lock().unwrap();
        let mut duplicate = wire.comments[&id(101)][0].clone();
        duplicate["id"] = json!(Uuid::new_v4());
        wire.comments.get_mut(&id(101)).unwrap().push(duplicate);
    }
    let actor = Actor {
        principal: None,
        identity: "fixture-recovery-operator".into(),
        roles: BTreeSet::from(["recover".into()]),
    };
    assert_eq!(
        f.coordinator
            .reconcile_receipt(
                &actor,
                f.admission.operation_id(),
                "publish-no-change-review"
            )
            .await
            .unwrap_err()
            .code,
        "ownership_conflict"
    );
    f.wire.lock().unwrap().comments.clear();
    assert_eq!(
        f.coordinator
            .reconcile_receipt(
                &actor,
                f.admission.operation_id(),
                "publish-no-change-review"
            )
            .await
            .unwrap_err()
            .code,
        "operation_uncertain"
    );
    assert!(!f.host.snapshot().unwrap().unresolved_operations.is_empty());
    assert_eq!(f.writes(), 0);
}

#[tokio::test]
async fn deployment_environment_is_resolved_by_canonical_id_and_same_name_recreation_is_rejected() {
    let fixture = Fixture::new();
    let cfg = config();
    let client = GithubClient::with_transport(
        HttpPolicy::new(
            "https://github.invalid/",
            "7".into(),
            StdDuration::from_secs(2),
        )
        .unwrap(),
        cfg.workspaces["example"].github.clone(),
        Credential::new("fixture".into()).unwrap(),
        Some(Arc::new(Wire(fixture.clone()))),
    )
    .unwrap();
    let repository = RepositoryIdentity {
        id: 42,
        owner: "fixture".into(),
        name: "repo".into(),
        owner_id: 7,
        organization_id: None,
    };
    assert_eq!(
        client.environment_by_id(&repository, 88).await.unwrap().id,
        88
    );
    assert_eq!(
        client
            .environment_by_id(&repository, 90)
            .await
            .unwrap_err()
            .code,
        "incompletely_prepared"
    );
    fixture.lock().unwrap().environment_replaced = true;
    assert_eq!(
        client
            .environment_by_id(&repository, 88)
            .await
            .unwrap_err()
            .code,
        "identity_mismatch"
    );
}

fn fixture_git(path: &std::path::Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .current_dir(path)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().into()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn parent_slice_uses_actual_production_and_preserves_dirty_operator_checkout() {
    let f = DeliveryFixture::build(false, true).await;
    let source = f._temp.path().join("source");
    std::fs::write(source.join("source.txt"), "unrelated staged\n").unwrap();
    fixture_git(&source, &["add", "source.txt"]);
    std::fs::write(source.join("source.txt"), "unrelated unstaged\n").unwrap();
    let index = std::fs::read(source.join(".git/index")).unwrap();
    let root =
        PrivateDir::create(&f._temp.path().canonicalize().unwrap().join("candidates")).unwrap();
    let required = BTreeSet::from([id(101)]);
    let dependencies = BTreeMap::new();
    let satisfied = BTreeSet::new();
    let input = || SliceAssembly {
        execution_root: root.path(),
        staged: &[],
        required_children: &required,
        child_dependencies: &dependencies,
        satisfied_children: &satisfied,
        production_integrations: &[],
        identity: kegari_v0::git::CommitIdentity {
            name: "Fixture".into(),
            email: "fixture@example.invalid".into(),
        },
    };
    let slice = f
        .coordinator
        .assemble_isolated_slice(&f.admission, input())
        .await
        .unwrap();
    assert_eq!(
        slice.manifest().result.as_ref(),
        Some(&slice.manifest().production)
    );
    assert_eq!(
        std::fs::read_to_string(slice.path().join("source.txt")).unwrap(),
        "production\n"
    );
    assert_eq!(
        std::fs::read_to_string(source.join("source.txt")).unwrap(),
        "unrelated unstaged\n"
    );
    assert_eq!(std::fs::read(source.join(".git/index")).unwrap(), index);
    let replay = f
        .coordinator
        .assemble_isolated_slice(&f.admission, input())
        .await
        .unwrap();
    assert_eq!(
        slice.provenance_digest().unwrap(),
        replay.provenance_digest().unwrap()
    );
    assert_eq!(slice.operation_id(), f.admission.operation_id());
    let read_root = f
        .coordinator
        .retained_candidate_host_root(&f.admission, slice.operation_id())
        .unwrap();
    read_root.revalidate_filesystem().unwrap();
    assert_eq!(read_root.root(), slice.path());
    assert_eq!(read_root.repository(), &f.admission.scope().repository);
    assert_eq!(
        read_root.config_revision(),
        f.admission.fresh_gate().config_revision()
    );
    let contract = f
        .coordinator
        .parent_contract(f.admission.fresh_gate().clone(), &f.parent, &workflow())
        .unwrap();
    let baseline = f.observer.observe(&f.repository_contract).await.unwrap();
    assert_eq!(
        f.coordinator
            .prepare_parent_vector(&contract, &f.observer, &[], &[&baseline])
            .await
            .unwrap_err()
            .code,
        "candidate_provenance"
    );
    let vector = f
        .coordinator
        .prepare_parent_vector(&contract, &f.observer, &[&slice], &[&baseline])
        .await
        .unwrap();
    assert_eq!(vector.candidates(), &[slice.manifest().clone()]);
    // Candidate construction and the exact E2E assignment precede child
    // completion. No child result or integrated check has been fabricated.
    let e2e_gate = f
        .parent
        .fresh_gate(
            Some(id(101)),
            GatePurpose::NewOperation,
            &DependencyOutcomes::default(),
            Utc::now(),
            chrono::Duration::seconds(30),
        )
        .unwrap();
    let assignment = f
        .coordinator
        .integrated_worker_assignment(
            &contract,
            &e2e_gate,
            &vector,
            &f.observer,
            &[&slice],
            json!({"purpose":"pending E2E responsibility fixture"}),
        )
        .await
        .unwrap();
    assert_eq!(assignment.candidate_vector_digest(), Some(vector.digest()));
    assert_eq!(
        assignment.source().unwrap().revision(),
        slice.manifest().result.as_ref().unwrap()
    );
    assert!(
        f.coordinator
            .verify_integrated_checks(&contract, &e2e_gate, &vector, &f.observer, &[])
            .await
            .is_err()
    );
    let authority = kegari_v0::worker::WorkerAuthority::Authorized(e2e_gate.clone());
    let mut request = kegari_v0::runtime_workers::LaunchRequest {
        lease: Lease {
            runtime_id: id(1),
            owner_generation: 1,
            owner_id: Uuid::new_v4(),
        },
        operation_id: Uuid::new_v4(),
        phase: kegari_v0::scheduler::PhaseKey {
            work: authority.work_key().unwrap(),
            generation: canonical_digest(1, &e2e_gate.work().generation).unwrap(),
            phase: assignment.phase().into(),
            retry_grant: None,
        },
        authority,
        source: None,
        delivery: Some(assignment.clone()),
        unfinished: None,
        source_advanced: None,
        retried_unfinished: None,
        review_retry: None,
        max_seconds: 60,
        max_turns: 2,
    };
    request.validate_binding().unwrap();
    request.phase.phase = "implementation".into();
    assert!(request.validate_binding().is_err());
    request.phase.phase = assignment.phase().into();
    request.source = Some(kegari_v0::runtime_workers::SourceSelection {
        repository: "42".into(),
        revision: kegari_v0::git::Revision {
            commit: "e".repeat(40),
            tree: "f".repeat(40),
        },
    });
    assert!(request.validate_binding().is_err());
    let hydrated = f
        .coordinator
        .hydrate_parent_vector(&contract, &f.observer, vector.digest())
        .await
        .unwrap();
    assert_eq!(hydrated.digest(), vector.digest());
    assert_eq!(hydrated.prepared_at(), vector.prepared_at());
    assert_eq!(std::fs::read(source.join(".git/index")).unwrap(), index);
    assert_eq!(f.writes(), 0);
    {
        let mut wire = f.wire.lock().unwrap();
        wire.production = "c".repeat(40);
    }
    assert_eq!(
        f.coordinator
            .reobserve_parent_vector(&contract, &f.observer, &vector)
            .await
            .unwrap_err()
            .code,
        "stale_result"
    );
    f.wire.lock().unwrap().production = slice.manifest().production.commit.clone();
    let marker = slice.path().join(".git/kegari-candidate.json");
    let mut data: Value = serde_json::from_slice(&std::fs::read(&marker).unwrap()).unwrap();
    data["selected_contributions"] =
        json!({canonical_digest(1,&"forged").unwrap():[canonical_digest(1,&"credit").unwrap()]});
    std::fs::write(&marker, serde_json::to_vec(&data).unwrap()).unwrap();
    assert_eq!(
        read_root.revalidate_filesystem().unwrap_err().code,
        "execution_ownership"
    );
    assert_eq!(
        f.coordinator
            .recover_isolated_slice(&f.admission, slice.operation_id())
            .await
            .unwrap_err()
            .code,
        "provenance_mismatch"
    );
}

#[tokio::test]
async fn parent_contract_binds_complete_authorized_scope_and_workflow() {
    let f = DeliveryFixture::build(false, true).await;
    let contract = f
        .coordinator
        .parent_contract(f.admission.fresh_gate().clone(), &f.parent, &workflow())
        .unwrap();
    assert_eq!(contract.required_children(), &BTreeSet::from([id(101)]));
    assert_eq!(
        contract.repositories(),
        &BTreeSet::from([f.admission.scope().repository.clone()])
    );
    assert_eq!(contract.integrated_commands()["e2e"], vec!["/usr/bin/true"]);
    assert_eq!(contract.e2e_child(), id(101));
    let mut changed = f.parent.clone();
    changed.metadata.as_mut().unwrap().visual = true;
    assert_eq!(
        f.coordinator
            .parent_contract(f.admission.fresh_gate().clone(), &changed, &workflow())
            .unwrap_err()
            .code,
        "scope_violation"
    );
    let mut changed = f.parent.clone();
    changed
        .children
        .get_mut(&id(101))
        .unwrap()
        .scope
        .as_mut()
        .unwrap()
        .write_paths = vec![".".into()];
    assert_eq!(
        f.coordinator
            .parent_contract(f.admission.fresh_gate().clone(), &changed, &workflow())
            .unwrap_err()
            .code,
        "scope_violation"
    );
    let mut changed = f.parent.clone();
    changed.required_children.clear();
    assert_eq!(
        f.coordinator
            .parent_contract(f.admission.fresh_gate().clone(), &changed, &workflow())
            .unwrap_err()
            .code,
        "stale_result"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn production_change_during_candidate_build_retains_candidate_without_promoting_it() {
    let f = DeliveryFixture::build(false, true).await;
    f.wire.lock().unwrap().move_main_on_read = Some(2);
    let root =
        PrivateDir::create(&f._temp.path().canonicalize().unwrap().join("candidates")).unwrap();
    let required = BTreeSet::from([id(101)]);
    let dependencies = BTreeMap::new();
    let satisfied = BTreeSet::new();
    let input = || SliceAssembly {
        execution_root: root.path(),
        staged: &[],
        required_children: &required,
        child_dependencies: &dependencies,
        satisfied_children: &satisfied,
        production_integrations: &[],
        identity: kegari_v0::git::CommitIdentity {
            name: "Fixture".into(),
            email: "fixture@example.invalid".into(),
        },
    };
    assert_eq!(
        f.coordinator
            .assemble_isolated_slice(&f.admission, input())
            .await
            .unwrap_err()
            .code,
        "stale_result"
    );
    let candidate = root
        .path()
        .join(format!("candidate-{}", f.admission.operation_id()));
    assert_eq!(
        std::fs::read_to_string(candidate.join("source.txt")).unwrap(),
        "production\n"
    );
    let receipt = std::fs::read(candidate.join(".git/kegari-candidate.json")).unwrap();
    assert_eq!(
        f.coordinator
            .assemble_isolated_slice(&f.admission, input())
            .await
            .unwrap_err()
            .code,
        "stale_result"
    );
    assert_eq!(
        std::fs::read(candidate.join(".git/kegari-candidate.json")).unwrap(),
        receipt
    );
}

#[tokio::test]
async fn delivery_refresh_does_not_promote_tracker_done_without_current_authenticated_gates() {
    let f = DeliveryFixture::build(false, true).await;
    let contract = f
        .coordinator
        .parent_contract(f.admission.fresh_gate().clone(), &f.parent, &workflow())
        .unwrap();
    {
        let mut wire = f.wire.lock().unwrap();
        wire.issues.get_mut(&id(101)).unwrap()["state"]["id"] = json!(id(14));
        wire.issues.get_mut(&id(100)).unwrap()["state"]["id"] = json!(id(15));
    }
    let mut configuration = config();
    configuration
        .workspaces
        .get_mut("example")
        .unwrap()
        .repositories
        .get_mut("42")
        .unwrap()
        .source = f.repository_contract.configuration.source.clone();
    let before = f.host.snapshot().unwrap();
    let snapshot = f
        .coordinator
        .refresh_delivery(
            ParentHydrationInputs {
                contract: &contract,
                observer: &f.observer,
                child_gates: &BTreeMap::new(),
                workflow: &workflow(),
                production_integrations: &[],
            },
            &configuration,
        )
        .await
        .unwrap();
    assert!(snapshot.parent_delivery().verified_children().is_empty());
    assert!(snapshot.parent_delivery().verified_parent().is_none());
    assert!(snapshot.parent_delivery().verified_acceptance().is_none());
    assert!(!snapshot.blockers().is_empty());
    assert_eq!(snapshot.work(), contract.work());
    assert_eq!(
        f.host.snapshot().unwrap().intent.generation,
        before.intent.generation
    );
    assert_eq!(f.writes(), 0);
}
