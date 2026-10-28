//! Synthetic provider facts only; shared with production-path correction tests.
use chrono::Utc;
use kegari_config::Config;
use kegari_providers::*;
use kegari_types::error::Result;
use reqwest::header::HeaderMap;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
};
use uuid::Uuid;
pub(crate) fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
pub(crate) fn config() -> Config {
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
pub(crate) struct Fixture {
    issues: BTreeMap<Uuid, Value>,
    pub(crate) comments: BTreeMap<Uuid, Vec<Value>>,
    pub(crate) github_values: BTreeMap<String, Value>,
    pub(crate) github_transport: Option<Arc<dyn HttpTransport>>,
    pub(crate) before_issue_read: Option<Arc<dyn Fn() -> Result<()> + Send + Sync>>,
    pub(crate) before_identity_read: Option<Arc<dyn Fn() -> Result<()> + Send + Sync>>,
    relations: Vec<Value>,
    history: BTreeMap<Uuid, Vec<Value>>,
    selected: bool,
    fail_comments: bool,
    changing: bool,
    discovery_count: u32,
    pub(crate) requests: Vec<(String, Value)>,
    prs: Vec<Value>,
    repository_available: bool,
    pub(crate) comment_response_lost: bool,
    pub(crate) duplicate_publication: bool,
    pub(crate) edit_route_on_read: bool,
}
impl Fixture {
    pub(crate) fn set_read_repositories(&mut self, repositories: &[&str]) {
        let issue = self.issues.get_mut(&id(101)).unwrap();
        issue["description"] = json!(issue["description"].as_str().unwrap().replace(
            "\"read_repositories\":[]",
            &format!(
                "\"read_repositories\":{}",
                serde_json::to_string(repositories).unwrap()
            ),
        ));
    }

    pub(crate) fn set_parent_title(&mut self, title: &str) {
        self.issues.get_mut(&id(100)).unwrap()["title"] = json!(title);
    }

    pub(crate) fn set_selected(&mut self, selected: bool) {
        self.selected = selected;
    }

    pub(crate) fn set_production_targets(&mut self, targets: &BTreeSet<String>) {
        let issue = self.issues.get_mut(&id(100)).unwrap();
        issue["description"] = json!(issue["description"].as_str().unwrap().replace(
            "\"production_targets\":[]",
            &format!(
                "\"production_targets\":{}",
                serde_json::to_string(targets).unwrap()
            )
        ));
    }
    pub(crate) fn new() -> Arc<Mutex<Self>> {
        Arc::new(Mutex::new(Self {
            issues: BTreeMap::from([
                (id(100), issue(100, None)),
                (id(101), issue(101, Some(100))),
            ]),
            comments: BTreeMap::new(),
            github_values: BTreeMap::new(),
            github_transport: None,
            before_issue_read: None,
            before_identity_read: None,
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
            edit_route_on_read: false,
        }))
    }
}
pub(crate) struct Wire(pub(crate) Arc<Mutex<Fixture>>);
impl HttpTransport for Wire {
    fn send(
        &self,
        request: HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Result<HttpResponse>> + Send + '_>> {
        Box::pin(async move {
            if matches!(
                request.url.host_str(),
                Some("github.invalid" | "api.github.com" | "github.com" | "fixture.blob.invalid")
            ) {
                let delegated = self.0.lock().unwrap().github_transport.clone();
                if let Some(transport) = delegated {
                    return transport.send(request).await;
                }
            }
            let mut f = self.0.lock().unwrap();
            let body: Value = if request.body.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&request.body).unwrap()
            };
            f.requests.push((request.url.to_string(), body.clone()));
            let value = if matches!(
                request.url.host_str(),
                Some("github.invalid" | "api.github.com" | "github.com")
            ) {
                if let Some(value) = f.github_values.get(request.url.path()) {
                    value.clone()
                } else {
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
                }
            } else {
                let query = body["query"].as_str().unwrap_or_else(|| {
                    panic!("unexpected fixture non-GraphQL request {}", request.url)
                });
                let input_id = body["variables"]["id"]
                    .as_str()
                    .and_then(|s| Uuid::parse_str(s).ok());
                let data = if query.contains("KegariIdentity") {
                    if let Some(before) = f.before_identity_read.take() {
                        before()?;
                    }
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
                    if f.comment_response_lost {
                        return Err(kegari_types::error::Error::new(
                            "timeout",
                            "fixture loses the acknowledgement after storing the comment",
                        ));
                    }
                    json!({"commentCreate":{"success":true,"comment":{"id":comment,"issue":{"id":issue},"user":{"id":id(5)}}}})
                } else if query.contains("KegariIssue(") {
                    if let Some(before) = &f.before_issue_read {
                        before()?;
                    }
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
                    if f.edit_route_on_read
                        && let Some(comments) = f.comments.get_mut(&id(101))
                        && let Some(comment) = comments.first_mut()
                    {
                        comment["body"] = json!("route changed during reread");
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
pub(crate) struct CacheConfiguration(pub(crate) Config);
impl crate::app::ConfigurationControl for CacheConfiguration {
    fn authority_fences(&self) -> Result<crate::configuration::AuthorityFences> {
        Ok(Default::default())
    }
    fn snapshot(&self) -> Result<Config> {
        Ok(self.0.clone())
    }
    fn execute(
        &self,
        _: crate::control::Actor,
        _: kegari_types::api::CommandRequest,
        _: Option<kegari_secrets::Secret>,
        _: crate::control::Receipt,
    ) -> crate::app::HostFuture<'_, crate::control::Receipt> {
        Box::pin(async { panic!("observation must not mutate configuration") })
    }
    fn check(&self, _: Option<Uuid>) -> crate::app::HostFuture<'_, Value> {
        Box::pin(async { panic!("observation must not launch configuration checks") })
    }
}
pub(crate) struct CacheSecrets;
impl kegari_secrets::SecretStore for CacheSecrets {
    fn read(&self, _: &str) -> Result<kegari_secrets::Secret> {
        kegari_secrets::Secret::new(b"fixture".to_vec())
    }
    fn save(&self, _: &str, _: &kegari_secrets::Secret) -> Result<()> {
        panic!("read-only observation must not save credentials")
    }
}
pub(crate) struct CacheInputs(pub(crate) Arc<Mutex<Fixture>>);
impl crate::configuration::ConfigurationInputs for CacheInputs {
    fn environment(&self, _: &str) -> Option<String> {
        None
    }
    fn transport(&self, _: &str) -> Option<Arc<dyn HttpTransport>> {
        Some(Arc::new(Wire(self.0.clone())))
    }
    fn codex<'a>(
        &'a self,
        _: &'a kegari_config::Connection,
        _: &'a std::path::Path,
    ) -> Pin<Box<dyn Future<Output = Result<crate::codex::CodexConnectionMetadata>> + Send + 'a>>
    {
        Box::pin(async { panic!("provider observation must not start Codex") })
    }
}
