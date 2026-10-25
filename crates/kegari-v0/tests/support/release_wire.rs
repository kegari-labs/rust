#![allow(dead_code)] // Shared deterministic wire harness; no production exemption.
use super::*;
use crate as fixture_api;
#[path = "deployment_wire.rs"]
mod target_wire;
// Delivery integration fixtures use production reconciliation/provider parsers.
// HTTP observations are deterministic fixtures, never live qualification.
mod pinned_git {
    // An owned test-tool copy, not host installation or native qualification.
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        path::{Path, PathBuf},
    };
    pub fn prepare(root: &Path) -> PathBuf {
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
        let identity = crate::platform::BinaryIdentity::inspect(&target, false).unwrap();
        assert_eq!(identity.digest, crate::digest::bytes_digest(&expected));
        target
    }
}
mod process_probe {
    // Deterministic boot identity for owned process fixtures. This deliberately
    // does not qualify native host boot identity or detached-process containment.
    use crate::{
        error::{Error, Result},
        platform::{ProcessIdentity, ProcessObservation, ProcessProbe},
    };
    use std::path::{Path, PathBuf};
    use uuid::Uuid;
    pub struct FixtureBootProbe {
        boot: Uuid,
    }
    impl FixtureBootProbe {
        pub fn new() -> Self {
            Self {
                boot: Uuid::new_v4(),
            }
        }
    }
    impl FixtureBootProbe {
        fn process(&self, pid: u32) -> Result<Option<ProcessIdentity>> {
            let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
            let size = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
            let length = unsafe {
                libc::proc_pidinfo(
                    pid as i32,
                    libc::PROC_PIDTBSDINFO,
                    0,
                    info.as_mut_ptr().cast(),
                    size,
                )
            };
            if length == 0 {
                return Ok(None);
            }
            if length != size {
                return Err(Error::invalid("incomplete fixture process observation"));
            }
            let info = unsafe { info.assume_init() };
            let mut path = [0u8; 4096];
            let length = unsafe {
                libc::proc_pidpath(pid as i32, path.as_mut_ptr().cast(), path.len() as u32)
            };
            if length <= 0 {
                return Ok(None);
            }
            let executable = PathBuf::from(
                std::str::from_utf8(&path[..length as usize])
                    .map_err(|_| Error::invalid("fixture process path"))?
                    .trim_end_matches('\0'),
            );
            Ok(Some(ProcessIdentity {
                pid,
                parent_pid: info.pbi_ppid,
                process_group: info.pbi_pgid,
                uid: info.pbi_uid,
                boot_id: self.boot,
                started_seconds: info.pbi_start_tvsec,
                started_microseconds: info.pbi_start_tvusec,
                executable,
            }))
        }
    }
    impl ProcessProbe for FixtureBootProbe {
        fn current(&self) -> Result<ProcessIdentity> {
            self.process(std::process::id())?
                .ok_or_else(|| Error::invalid("fixture host absent"))
        }
        fn observe(&self, expected: &ProcessIdentity) -> Result<ProcessObservation> {
            Ok(match self.process(expected.pid)? {
                Some(current) if &current == expected => ProcessObservation::Running,
                Some(_) => ProcessObservation::DifferentIdentity,
                None => ProcessObservation::Gone,
            })
        }
        fn matching_executable(&self, path: &Path) -> Result<Vec<ProcessIdentity>> {
            let mut pids = [0i32; 4096];
            let bytes = std::mem::size_of_val(&pids) as i32;
            // PROC_PPID_ONLY=6, verified from the installed Apple SDK proc_info.h.
            let length = unsafe {
                libc::proc_listpids(6, std::process::id(), pids.as_mut_ptr().cast(), bytes)
            };
            if length < 0 || length >= bytes || !(length as usize).is_multiple_of(4) {
                return Err(Error::invalid("fixture child inventory incomplete"));
            }
            let mut found = Vec::new();
            for pid in &pids[..length as usize / 4] {
                if *pid > 0
                    && let Some(p) = self.process(*pid as u32)?
                    && p.executable == path
                {
                    found.push(p);
                }
            }
            Ok(found)
        }
    }
}
use crate::{
    config::Config,
    control::{Actor, ControlHost, Lease, Lifecycle, MutationRequest},
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
use chrono::{Duration, Utc};
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
    merge_parent: Option<String>,
    main_reads: usize,
    move_main_on_read: Option<usize>,
    object_blobs: BTreeMap<String, String>,
    object_commit: Value,
    object_tree: String,
    branch: String,
    pr_comments: BTreeMap<u64, Vec<Value>>,
    statuses: BTreeMap<String, Vec<Value>>,
    created_prs: usize,
    workflow_id: u64,
    edit_source_after_blob: bool,
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
            merge_parent: None,
            main_reads: 0,
            move_main_on_read: None,
            object_blobs: BTreeMap::new(),
            object_commit: Value::Null,
            object_tree: String::new(),
            branch: String::new(),
            pr_comments: BTreeMap::new(),
            statuses: BTreeMap::new(),
            created_prs: 0,
            workflow_id: 3,
            edit_source_after_blob: false,
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
                    "/repos/fixture/repo/pulls" if request.method == Method::Post => {
                        f.created_prs += 1;
                        let pr = json!({"id":170,"number":17,"user":{"id":7},"state":"open","draft":false,
                            "head":{"sha":f.object_commit["sha"],"ref":body["head"],"repo":{"id":42}},
                            "base":{"sha":f.production,"ref":body["base"],"repo":{"id":42}},"merged":false,"merge_commit_sha":null,
                            "html_url":"https://github.invalid/fixture/repo/pull/17"});
                        f.prs = vec![pr.clone()];
                        pr
                    }
                    "/repos/fixture/repo/pulls" => json!(f.prs),
                    "/repos/fixture/repo/git/blobs" => {
                        if f.edit_source_after_blob {
                            f.pr_comments.get_mut(&8).unwrap()[0]["body"] =
                                json!("source review changed during object publication");
                        }
                        json!({"sha":f.object_blobs[body["content"].as_str().unwrap()]})
                    }
                    "/repos/fixture/repo/git/trees" => json!({"sha":f.object_tree}),
                    path if path.starts_with("/repos/fixture/repo/git/trees/") => {
                        json!({"sha":path.rsplit('/').next().unwrap(),"truncated":false,"tree":[]})
                    }
                    "/repos/fixture/repo/git/commits" => {
                        for field in ["tree", "message", "author", "committer"] {
                            assert_eq!(
                                body[field], f.object_commit[field],
                                "exact commit field {field}"
                            );
                        }
                        assert_eq!(body["parents"], json!([f.production]));
                        json!({"sha":f.object_commit["sha"]})
                    }
                    "/repos/fixture/repo/git/refs" => {
                        f.branch = body["ref"]
                            .as_str()
                            .unwrap()
                            .trim_start_matches("refs/")
                            .into();
                        json!({"ref":body["ref"],"object":{"sha":body["sha"]}})
                    }
                    path if path
                        .starts_with("/repos/fixture/repo/git/ref/heads/kegari-release/") =>
                    {
                        json!({"ref":format!("refs/{}",f.branch),"object":{"type":"commit","sha":f.object_commit["sha"]}})
                    }
                    path if path.starts_with("/repos/fixture/repo/issues/")
                        && path.ends_with("/comments") =>
                    {
                        let pr = path.split('/').nth(5).unwrap().parse::<u64>().unwrap();
                        if request.method == Method::Post {
                            let id = 100 + f.pr_comments.get(&pr).map_or(0, Vec::len);
                            let item = json!({"id":id,"body":body["body"],"user":{"id":7},"html_url":format!("https://github.invalid/fixture/repo/pull/{pr}#comment{id}")});
                            f.pr_comments.entry(pr).or_default().push(item.clone());
                            item
                        } else {
                            json!(f.pr_comments.get(&pr).cloned().unwrap_or_default())
                        }
                    }
                    path if path.starts_with("/repos/fixture/repo/statuses/") => {
                        let head = path.rsplit('/').next().unwrap();
                        let item = json!({"id":200,"context":body["context"],"state":body["state"],"description":body["description"],"target_url":body["target_url"],"creator":{"id":7},"created_at":"2026-01-04T00:00:00Z"});
                        f.statuses
                            .entry(head.into())
                            .or_default()
                            .push(item.clone());
                        item
                    }
                    path if path.starts_with("/repos/fixture/repo/commits/")
                        && path.ends_with("/statuses") =>
                    {
                        let head = path.split('/').nth(5).unwrap();
                        json!(f.statuses.get(head).cloned().unwrap_or_default())
                    }
                    "/repos/fixture/repo/pulls/17" => {
                        f.prs.iter().find(|pr| pr["number"] == 17).unwrap().clone()
                    }
                    "/repos/fixture/repo/pulls/8" => {
                        f.prs.iter().find(|pr| pr["number"] == 8).unwrap().clone()
                    }
                    "/repos/fixture/repo/pulls/17/merge" => {
                        assert_eq!(body["merge_method"], "squash");
                        assert_eq!(body["sha"], f.prs[0]["head"]["sha"]);
                        f.merge_parent = Some(f.production.clone());
                        f.prs[0]["merged"] = json!(true);
                        f.prs[0]["state"] = json!("closed");
                        f.prs[0]["merge_commit_sha"] = json!("c".repeat(40));
                        if f.lose_write {
                            return Err(Error::new(
                                "provider_transport",
                                "fixture accepted merge then lost response",
                            ));
                        }
                        json!({"merged":true,"sha":"c".repeat(40),"message":"merged fixture"})
                    }
                    "/repos/fixture/repo/branches/main/protection" => {
                        json!({"required_status_checks":{"strict":true,"contexts":["suite","kegari/codex-review"]},"required_pull_request_reviews":null})
                    }
                    "/repos/fixture/repo/rules/branches/main" => json!([]),
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
                        json!({"id":80,"workflow_run":{"id":50,"repository_id":42,"head_sha":"1".repeat(40)},"expired":false,"size_in_bytes":f.archive.len(),"digest":crate::digest::bytes_digest(&f.archive)})
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
                        if f.object_commit["sha"]
                            .as_str()
                            .is_some_and(|sha| path.ends_with(sha))
                        {
                            json!({"sha":f.object_commit["sha"],"tree":{"sha":f.object_tree},"parents":[{"sha":f.production}]})
                        } else if path.ends_with(&"c".repeat(40)) {
                            json!({"sha":"c".repeat(40),"tree":{"sha":if f.object_tree.is_empty(){&f.production_tree}else{&f.object_tree}},"parents":[{"sha":f.merge_parent.as_ref().unwrap_or(&f.production)}]})
                        } else {
                            json!({"sha":f.production,"tree":{"sha":f.production_tree},"parents":[]})
                        }
                    }
                    "/repos/fixture/repo/git/ref/heads/trusted" => {
                        json!({"ref":"refs/heads/trusted","object":{"type":"commit","sha":"1".repeat(40)}})
                    }
                    "/repos/fixture/repo/actions/workflows" => {
                        json!({"total_count":2,"workflows":[{"id":2,"path":".github/workflows/caller.yml","state":"active"},{"id":3,"path":".github/workflows/caller.yml","state":"active"}]})
                    }
                    "/repos/fixture/repo/actions/workflows/2/dispatches"
                    | "/repos/fixture/repo/actions/workflows/3/dispatches" => {
                        f.dispatch_count += 1;
                        f.workflow_id = request
                            .url
                            .path()
                            .split('/')
                            .nth(6)
                            .unwrap()
                            .parse()
                            .unwrap();
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
                        json!({"id":50,"repository":{"id":42},"run_attempt":f.run_attempt,"workflow_id":f.workflow_id,"actor":{"id":7},"triggering_actor":{"id":7},"event":"workflow_dispatch","head_sha":"1".repeat(40),"status":"completed","conclusion":f.check_conclusion,"path":".github/workflows/caller.yml","html_url":"https://github.invalid/fixture/repo/actions/runs/50"})
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
    coordinator: Arc<DeliveryCoordinator>,
    host: Arc<ControlHost>,
    admission: DeliveryAdmission,
    key: Arc<AttestationKey>,
    wire: Arc<Mutex<Fixture>>,
    current: Arc<std::sync::atomic::AtomicBool>,
    parent: ParentProjection,
    observer: crate::deployment::ProductionObserver,
    repository_contract: RepositoryContract,
}

/// Test-only provider protocol used by a real RuntimeFixture. Upstream review
/// and check facts remain explicitly synthetic; no native sandbox proof exists.
#[derive(Clone)]
pub(crate) struct RuntimeSeedWire(Arc<Mutex<Fixture>>);
impl RuntimeSeedWire {
    pub(crate) fn new(production: &crate::git::Revision) -> Self {
        let inner = Fixture::new();
        {
            let mut wire = inner.lock().unwrap();
            wire.production = production.commit.clone();
            wire.production_tree = production.tree.clone();
        }
        Self(inner)
    }
    pub(crate) fn transport(&self) -> Arc<dyn HttpTransport> {
        Arc::new(Wire(self.0.clone()))
    }
    pub(crate) fn release_effect_count(&self) -> usize {
        let wire = self.0.lock().unwrap();
        wire.created_prs + wire.dispatch_count + usize::from(wire.merge_parent.is_some())
    }
    pub(crate) fn set_production(&self, production: &crate::git::Revision) {
        let mut wire = self.0.lock().unwrap();
        wire.production = production.commit.clone();
        wire.production_tree = production.tree.clone();
    }
}
pub(crate) struct RuntimeCandidateSeed {
    _root: tempfile::TempDir,
    pub(crate) parent: crate::evidence::VerifiedParent,
    pub(crate) slice: IsolatedSlice,
    pub(crate) child: PublishedChild,
}
impl RuntimeCandidateSeed {
    pub(crate) fn original_parent_body(&self, coordinator: &DeliveryCoordinator) -> String {
        coordinator
            .fixture_original_parent_body(&self.parent, &self.slice, id(1))
            .unwrap()
    }
    /// Repeat actual patch capture/assembly for the newly authorized correction.
    /// The upstream reviewed/staged child is synthetic provider input, while
    /// every candidate object and its control/process lineage is real.
    pub(crate) async fn corrected_slice(
        &self,
        coordinator: &DeliveryCoordinator,
        admission: &DeliveryAdmission,
        wire: &RuntimeSeedWire,
    ) -> IsolatedSlice {
        let crate::evidence::ChildOutcome::Changed { delta, .. } =
            &self.child.verified().evidence().outcome
        else {
            panic!("seed must retain the changed child delta")
        };
        let source = std::path::Path::new(
            &coordinator
                .contract(admission)
                .unwrap()
                .configuration
                .source,
        );
        let captured = coordinator
            .git
            .capture_delta(
                source,
                crate::git::CaptureDelta {
                    repository: delta.repository.clone(),
                    parent_id: admission.scope().work.parent,
                    child_id: delta.child_id,
                    work_generation: &admission.scope().work.generation.definition,
                    correction_generation: &admission.scope().work.generation.correction,
                    scope: delta.scope.clone(),
                    base: &delta.validated_base.commit,
                    head: &delta.validated_head.commit,
                    staging_parent: &delta.staging_parent.commit,
                    staging_commit: &delta.staging_commit.commit,
                },
            )
            .unwrap();
        let root = self
            ._root
            .path()
            .canonicalize()
            .unwrap()
            .join("corrected-candidate-runs");
        PrivateDir::create(&root).unwrap();
        assemble_wire_slice(coordinator, admission, &wire.0, &captured, &root).await
    }
}
pub(crate) async fn runtime_candidate_seed(
    coordinator: Arc<DeliveryCoordinator>,
    admission: DeliveryAdmission,
    parent: ParentProjection,
    wire: &RuntimeSeedWire,
) -> RuntimeCandidateSeed {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().canonicalize().unwrap();
    let contract = coordinator.contract(&admission).unwrap().clone();
    let observer = crate::deployment::ProductionObserver::new(
        coordinator.github().unwrap().clone(),
        coordinator.host.clone(),
        coordinator.key.clone(),
        PrivateDir::create(&directory.join("deployment-proofs")).unwrap(),
        coordinator.current_gate.clone(),
    )
    .unwrap();
    let f = DeliveryFixture {
        _temp: root,
        host: coordinator.host.clone(),
        key: coordinator.key.clone(),
        coordinator,
        admission,
        wire: wire.0.clone(),
        current: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        parent,
        observer,
        repository_contract: contract,
    };
    let (f, parent, slice, child) = changed_candidate_from(f, true).await;
    RuntimeCandidateSeed {
        _root: f._temp,
        parent,
        slice,
        child,
    }
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
        let callback: GateCheck = Arc::new(move |actual| {
            if !callback_flag.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(Error::new("stale_result", "fixture correction arrived"));
            }
            actual.compare_current(&gate, Utc::now(), Duration::seconds(60))
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
        let observer = crate::deployment::ProductionObserver::new(
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
                executions: Arc::new(crate::worker::ExecutionRegistry::default()),
                processes: Arc::new(
                    crate::process_records::ProcessJournal::with_probe(
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
            coordinator: Arc::new(coordinator),
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
    fn review(&self) -> crate::evidence::Signed<ReviewAttestation> {
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

async fn release_fixture() -> (DeliveryFixture, ReleasePlan, ReleaseEffectRecord) {
    let mut f = DeliveryFixture::build(false, true).await;
    f.host
        .finish(
            f.admission.operation_id(),
            json!({"fixture":"prior metadata complete"}),
        )
        .unwrap();
    let mut scope = f.admission.scope.clone();
    scope.phase = DeliveryPhase::Release;
    scope.candidate_vector_digest = Some(canonical_digest(1, &json!("fixture vector")).unwrap());
    scope.revision.head = "d".repeat(40);
    scope.revision.tested_commit = scope.revision.head.clone();
    scope.revision.tested_tree = "e".repeat(40);
    let permit = f
        .host
        .admit(
            &f.admission.permit.lease,
            Uuid::new_v4(),
            f.admission.gate.work_key().unwrap(),
            scope.digest().unwrap(),
            || Ok(()),
        )
        .unwrap();
    f.admission = DeliveryAdmission::bind(permit, f.admission.gate.clone(), scope.clone()).unwrap();
    let base = crate::git::Revision {
        commit: scope.revision.base.clone(),
        tree: "b".repeat(40),
    };
    let kind = StepKind::Merge {
        repository: scope.repository.clone(),
        pull_request: 17,
        production_base: base,
        candidate: crate::git::Revision {
            commit: scope.revision.head.clone(),
            tree: scope.revision.tested_tree.clone(),
        },
        selected_delta_digests: vec![
            canonical_digest(1, &json!("selected actual fixture patch")).unwrap(),
        ],
    };
    let plan = ReleasePlan {
        schema_version: 1,
        runtime_id: id(1),
        operation_id: Uuid::new_v4(),
        parent_id: scope.work.parent,
        linear_workspace: scope.work.linear_workspace,
        generation: scope.work.generation.clone(),
        evidence_token: canonical_digest(1, &json!("parent")).unwrap(),
        acceptance_digest: canonical_digest(1, &json!("acceptance")).unwrap(),
        carried_acceptance: None,
        resources: BTreeSet::from([ResourceKey::Repository {
            repository: scope.repository.clone(),
        }]),
        dependencies: vec![],
        spacing_seconds: 0,
        deployment_free: BTreeSet::from([scope.repository.clone()]),
        steps: vec![crate::release::ReleaseStep {
            id: "merge".into(),
            depends_on: BTreeSet::new(),
            kind: kind.clone(),
        }],
    };
    plan.validate().unwrap();
    let record = ReleaseEffectRecord {
        schema_version: 1,
        plan_digest: plan.digest().unwrap(),
        payload: crate::release::StepPayload {
            operation_id: plan.operation_id,
            step_id: "merge".into(),
            attempt: 1,
            prerequisite_attempts: BTreeMap::new(),
            plan_digest: plan.digest().unwrap(),
            kind,
            exact_source_commit: None,
        },
        scope: scope.clone(),
        admission_operation: f.admission.operation_id(),
        admission_scope_digest: f.admission.permit.scope_digest.clone(),
        request_id: Some(Uuid::new_v4()),
        command: Some(ProviderMutation::GithubMerge {
            repository: f.repository_contract.provider.clone(),
            pr: 17,
            expected_head: scope.revision.head.clone(),
            expected_base: scope.revision.base.clone(),
        }),
        deployment_step: None,
        challenge: None,
    };
    f.coordinator
        .records
        .write(
            &release_effect_leaf(plan.operation_id, "merge", 1),
            &f.key.sign("release_effect", &record).unwrap(),
        )
        .unwrap();
    {
        let mut wire = f.wire.lock().unwrap();
        wire.production_tree = scope.revision.tested_tree.clone();
        wire.prs = vec![
            json!({"id":170,"number":17,"user":{"id":7},"state":"open","draft":false,
            "head":{"sha":scope.revision.head,"ref":"kegari-release/fixture","repo":{"id":42}},
            "base":{"sha":scope.revision.base,"ref":"main","repo":{"id":42}},"merged":false,"merge_commit_sha":null,
            "html_url":"https://github.invalid/fixture/repo/pull/17"}),
        ];
    }
    (f, plan, record)
}
async fn send_fixture_merge(f: &DeliveryFixture, record: &ReleaseEffectRecord) -> Result<Receipt> {
    send_fixture_merge_gate(f, record, true).await
}
async fn send_fixture_merge_gate(
    f: &DeliveryFixture,
    record: &ReleaseEffectRecord,
    admitted: bool,
) -> Result<Receipt> {
    f.coordinator
        .broker
        .mutate(
            f.host.clone(),
            MutationRequest {
                request_id: record.request_id.unwrap(),
                runtime_id: id(1),
                actor: Actor {
                    principal: None,
                    identity: format!("owner:{}", f.admission.permit.lease.owner_id),
                    roles: BTreeSet::new(),
                },
                kind: String::new(),
                canonical_targets: vec![],
                expected: BTreeMap::from([
                    ("release_plan".into(), record.plan_digest.clone()),
                    ("release_step".into(), record.payload.step_id.clone()),
                    ("release_attempt".into(), record.payload.attempt.to_string()),
                    ("delivery_scope".into(), record.scope.digest().unwrap()),
                ]),
                payload: Value::Null,
            },
            Some(f.admission.permit.clone()),
            None,
            record.command.clone().unwrap(),
            move || {
                if admitted {
                    Ok(())
                } else {
                    Err(Error::new("stale_result", "fixture final gate rejected"))
                }
            },
        )
        .await
}
#[tokio::test]
async fn known_broker_rejection_recovers_without_remote_probe_or_replay_after_receipt_reconciliation()
 {
    let (f, plan, record) = release_fixture().await;
    assert!(send_fixture_merge_gate(&f, &record, false).await.is_err());
    let receipt = f.host.operation(record.request_id.unwrap()).unwrap();
    assert_eq!(receipt.phase, crate::control::ReceiptPhase::Reconciled);
    assert_eq!(
        receipt.observed_result.as_ref().unwrap()["effect"],
        "not_sent"
    );
    let requests_before = f.wire.lock().unwrap().requests.len();
    let first = f
        .coordinator
        .recover_owned_release_step(&plan, "merge", 1, &f.observer)
        .await
        .unwrap();
    assert!(matches!(first.outcome(), ReleaseStepOutcome::Failed(_)));
    assert_eq!(f.wire.lock().unwrap().requests.len(), requests_before);
    f.host
        .reconcile_owned_checked(
            &receipt,
            crate::control::Reconciliation::UniqueVerified {
                remote_ids: BTreeMap::new(),
                result: json!({"release":"known failure observed"}),
                payload_digest: receipt.payload_digest.clone(),
                actor: receipt.actor.clone(),
            },
            || Ok(()),
        )
        .unwrap();
    let again = f
        .coordinator
        .recover_owned_release_step(&plan, "merge", 1, &f.observer)
        .await
        .unwrap();
    assert_eq!(again.outcome(), first.outcome());
    assert_eq!(again.observed_at(), first.observed_at());
    assert_eq!(again.receipt().unwrap().operation_id, receipt.operation_id);
    assert_eq!(f.wire.lock().unwrap().requests.len(), requests_before);
}

#[tokio::test]
async fn production_merge_lost_ack_is_observed_without_resending_and_checks_actual_parent_tree() {
    let (f, plan, record) = release_fixture().await;
    f.wire.lock().unwrap().lose_write = true;
    assert!(send_fixture_merge(&f, &record).await.is_err());
    let before = f
        .wire
        .lock()
        .unwrap()
        .requests
        .iter()
        .filter(|(url, _)| url.ends_with("/merge"))
        .count();
    let observed = f
        .coordinator
        .recover_release_step(&f.admission, &plan, "merge", 1, &f.observer)
        .await
        .unwrap();
    assert_eq!(observed.outcome(), &ReleaseStepOutcome::Succeeded);
    let Some(ObservedEffect::Merge {
        commit,
        parent_commit,
        ..
    }) = observed.effect()
    else {
        panic!("actual merge proof absent")
    };
    assert_eq!(commit.commit, "c".repeat(40));
    assert_eq!(commit.tree, "e".repeat(40));
    assert_eq!(parent_commit, &f.admission.scope.revision.base);
    assert_eq!(
        observed.receipt().unwrap().request_id,
        record.request_id.unwrap()
    );
    assert_eq!(
        observed.receipt().unwrap().phase,
        crate::control::ReceiptPhase::Sent
    );
    f.coordinator
        .recover_release_step(&f.admission, &plan, "merge", 1, &f.observer)
        .await
        .unwrap();
    assert_eq!(
        before,
        f.wire
            .lock()
            .unwrap()
            .requests
            .iter()
            .filter(|(url, _)| url.ends_with("/merge"))
            .count()
    );
    f.wire.lock().unwrap().production_tree = "f".repeat(40);
    assert_eq!(
        f.coordinator
            .recover_release_step(&f.admission, &plan, "merge", 1, &f.observer)
            .await
            .err()
            .unwrap()
            .code,
        "release_provenance"
    );
    assert_eq!(
        f.host.operation(record.request_id.unwrap()).unwrap().phase,
        crate::control::ReceiptPhase::Sent
    );
}
#[tokio::test]
async fn production_merge_observation_rejects_wrong_plan_pr_and_signed_intent_tampering() {
    let (f, plan, record) = release_fixture().await;
    send_fixture_merge(&f, &record).await.unwrap();
    let mut wrong = plan.clone();
    wrong.evidence_token = canonical_digest(1, &json!("another candidate")).unwrap();
    assert!(
        f.coordinator
            .recover_release_step(&f.admission, &wrong, "merge", 1, &f.observer)
            .await
            .is_err()
    );
    f.wire.lock().unwrap().prs[0]["head"]["sha"] = json!("f".repeat(40));
    assert!(
        f.coordinator
            .recover_release_step(&f.admission, &plan, "merge", 1, &f.observer)
            .await
            .is_err()
    );
    let mut signed = f.key.sign("release_effect", &record).unwrap();
    signed.payload.request_id = Some(Uuid::new_v4());
    f.coordinator
        .records
        .write(&release_effect_leaf(plan.operation_id, "merge", 1), &signed)
        .unwrap();
    assert!(
        f.coordinator
            .recover_release_step(&f.admission, &plan, "merge", 1, &f.observer)
            .await
            .is_err()
    );
}
#[tokio::test]
async fn production_merge_pending_and_closed_are_not_successful_effects() {
    let (f, plan, record) = release_fixture().await;
    send_fixture_merge(&f, &record).await.unwrap();
    {
        let mut wire = f.wire.lock().unwrap();
        wire.prs[0]["merged"] = json!(false);
        wire.prs[0]["state"] = json!("open");
    }
    let pending = f
        .coordinator
        .recover_release_step(&f.admission, &plan, "merge", 1, &f.observer)
        .await
        .unwrap();
    assert_eq!(pending.outcome(), &ReleaseStepOutcome::Pending);
    assert!(pending.effect().is_none());
    f.wire.lock().unwrap().prs[0]["state"] = json!("closed");
    let failed = f
        .coordinator
        .recover_release_step(&f.admission, &plan, "merge", 1, &f.observer)
        .await
        .unwrap();
    assert!(matches!(failed.outcome(), ReleaseStepOutcome::Failed(_)));
    assert!(failed.effect().is_none());
}
#[tokio::test]
async fn production_recovery_is_read_only_when_stopped_and_fails_revoked_current_authority() {
    let (f, plan, record) = release_fixture().await;
    send_fixture_merge(&f, &record).await.unwrap();
    let generation = f.host.snapshot().unwrap().intent.generation;
    f.host
        .lifecycle(
            MutationRequest {
                request_id: Uuid::new_v4(),
                runtime_id: id(1),
                actor: Actor {
                    principal: None,
                    identity: "fixture".into(),
                    roles: BTreeSet::from(["lifecycle".into()]),
                },
                kind: "stop".into(),
                canonical_targets: vec![id(1).to_string()],
                expected: BTreeMap::new(),
                payload: Value::Null,
            },
            Lifecycle::Stop,
            generation,
        )
        .unwrap();
    let mut observation = f.admission.observation.clone();
    observation.admitted = None;
    observation.scope.phase = DeliveryPhase::Recovery;
    assert_eq!(
        f.coordinator
            .recover_release_step(&observation, &plan, "merge", 1, &f.observer)
            .await
            .unwrap()
            .outcome(),
        &ReleaseStepOutcome::Succeeded
    );
    f.current.store(false, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(
        f.coordinator
            .recover_release_step(&observation, &plan, "merge", 1, &f.observer)
            .await
            .err()
            .unwrap()
            .code,
        "stale_result"
    );
    let writes_before = f
        .wire
        .lock()
        .unwrap()
        .requests
        .iter()
        .filter(|(url, _)| url.ends_with("/merge"))
        .count();
    let readback = f
        .coordinator
        .recover_owned_release_step(&plan, "merge", 1, &f.observer)
        .await
        .unwrap();
    assert_eq!(readback.outcome(), &ReleaseStepOutcome::Succeeded);
    assert_eq!(
        readback.receipt().unwrap().operation_id,
        record.request_id.unwrap()
    );
    assert_eq!(
        f.wire
            .lock()
            .unwrap()
            .requests
            .iter()
            .filter(|(url, _)| url.ends_with("/merge"))
            .count(),
        writes_before
    );
    // This narrow observation has no authority to promote readiness or create
    // a replacement dispatch when the original work has been corrected.
    assert!(
        f.coordinator
            .recover_owned_release_step(&plan, "merge", 2, &f.observer)
            .await
            .is_err()
    );
}

use base64::Engine as _;
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
async fn child_ci_fixture() -> DeliveryFixture {
    let mut f = DeliveryFixture::new().await;
    f.host
        .finish(
            f.admission.operation_id(),
            json!({"fixture":"prior metadata complete"}),
        )
        .unwrap();
    f.repository_contract.configuration.ci[0].workflow_id = 3;
    Arc::get_mut(&mut f.coordinator).unwrap().contracts.insert(
        f.repository_contract.repository.clone(),
        f.repository_contract.clone(),
    );
    let mut scope = f.admission.scope.clone();
    scope.phase = DeliveryPhase::HostedValidation;
    scope.contract_digest = f.repository_contract.digest().unwrap();
    let permit = f
        .host
        .admit(
            &f.admission.permit.lease,
            Uuid::new_v4(),
            f.admission.gate.work_key().unwrap(),
            scope.digest().unwrap(),
            || Ok(()),
        )
        .unwrap();
    f.admission = DeliveryAdmission::bind(permit, f.admission.gate.clone(), scope).unwrap();
    f
}
fn child_ci_artifact(f: &DeliveryFixture, source: &str) {
    let action = "child-ci-3";
    let record: EffectRecord = f
        .coordinator
        .records
        .read(&format!(
            "delivery-effect-{}.json",
            &crate::digest::bytes_digest(
                format!("{}:{action}", f.admission.operation_id()).as_bytes()
            )[7..]
        ))
        .unwrap();
    let pending = PendingExactValidation {
        work: f.admission.scope.work.clone(),
        revision: f.admission.scope.revision.clone(),
        receipt: f.host.operation(record.request_id).unwrap(),
        challenge: f.coordinator.hosted_challenge(&f.admission).unwrap(),
    };
    let token = hosted_token(f, &pending);
    let value = json!({"schema_version":1,"runtime_id":id(1),"phase_operation":f.admission.operation_id(),"dispatch_request":pending.receipt.request_id,"challenge":pending.challenge.challenge,
        "repository_id":42,"run_id":50,"run_attempt":1,"tested_tree":f.admission.scope.revision.tested_tree,"outputs":{"source_sha":source},"proofs":[{"check_run_id":60,"token":token.token}]});
    let mut wire = f.wire.lock().unwrap();
    wire.archive = zip_fixture(
        "kegari-validation.json",
        &serde_json::to_vec(&value).unwrap(),
    );
    wire.artifact_name = format!("kegari-child-{}-3-1", f.admission.operation_id());
}
#[tokio::test]
async fn child_ci_reads_actual_private_artifact_and_reuses_owned_dispatch_then_rejects_human_rerun()
{
    let f = child_ci_fixture().await;
    assert!(
        f.coordinator
            .acquire_child_validation(&f.admission, &verifier())
            .await
            .is_err()
    );
    assert_eq!(f.wire.lock().unwrap().dispatch_count, 1);
    child_ci_artifact(&f, &f.admission.scope.revision.tested_commit);
    let passed = f
        .coordinator
        .acquire_child_validation(&f.admission, &verifier())
        .await
        .unwrap();
    assert_eq!(passed.checks().len(), 1);
    assert_eq!(
        passed.checks()[0].kind,
        crate::evidence::CheckKind::HostedCi
    );
    assert_eq!(
        passed.checks()[0].runner,
        crate::evidence::Runner::GitHubHosted
    );
    assert_eq!(f.wire.lock().unwrap().dispatch_count, 1);
    f.wire.lock().unwrap().archive.clear();
    f.coordinator
        .acquire_child_validation(&f.admission, &verifier())
        .await
        .unwrap();
    f.wire.lock().unwrap().run_attempt = 2;
    assert!(
        f.coordinator
            .acquire_child_validation(&f.admission, &verifier())
            .await
            .is_err()
    );
    assert_eq!(f.wire.lock().unwrap().dispatch_count, 1);
}
#[tokio::test]
async fn child_ci_wrong_source_and_failed_required_job_never_publish_validation() {
    let f = child_ci_fixture().await;
    assert!(
        f.coordinator
            .acquire_child_validation(&f.admission, &verifier())
            .await
            .is_err()
    );
    child_ci_artifact(&f, &"f".repeat(40));
    assert!(
        f.coordinator
            .acquire_child_validation(&f.admission, &verifier())
            .await
            .is_err()
    );
    child_ci_artifact(&f, &f.admission.scope.revision.tested_commit);
    f.wire.lock().unwrap().check_conclusion = "skipped".into();
    assert!(
        f.coordinator
            .acquire_child_validation(&f.admission, &verifier())
            .await
            .is_err()
    );
    assert_eq!(
        f.coordinator
            .records
            .read::<Signed<ChildValidationRecord>>(&format!(
                "child-hosted-{}.json",
                f.admission.operation_id()
            ))
            .err()
            .unwrap()
            .code,
        "record_missing"
    );
    assert_eq!(f.wire.lock().unwrap().dispatch_count, 1);
}
fn fixture_publication<T: serde::Serialize>(
    value: &T,
    provider: &str,
    record: &str,
    writer: &str,
    url: &str,
) -> Publication {
    Publication {
        provider: provider.into(),
        record_id: record.into(),
        writer_account_id: writer.into(),
        payload_digest: canonical_digest(1, value).unwrap(),
        observed_at: Utc::now(),
        url: url.into(),
    }
}
async fn changed_candidate_fixture() -> (
    DeliveryFixture,
    crate::evidence::VerifiedParent,
    IsolatedSlice,
    PublishedChild,
) {
    changed_candidate_from(DeliveryFixture::build(false, true).await, false).await
}
async fn assemble_seed_slice(
    f: &DeliveryFixture,
    delta: &crate::git::DeltaArtifact,
    root: &std::path::Path,
) -> IsolatedSlice {
    assemble_wire_slice(&f.coordinator, &f.admission, &f.wire, delta, root).await
}
async fn assemble_wire_slice(
    coordinator: &DeliveryCoordinator,
    admission: &DeliveryAdmission,
    wire: &Arc<Mutex<Fixture>>,
    delta: &crate::git::DeltaArtifact,
    root: &std::path::Path,
) -> IsolatedSlice {
    // The reviewed staging source is explicit synthetic upstream input. The
    // candidate itself must be created by the real bounded/journaled adapter.
    let digest = delta.manifest.digest().unwrap();
    let chunk = format!("delta-{}-0.bin", &digest[7..]);
    coordinator
        .records
        .write_bytes(&chunk, &delta.patch)
        .unwrap();
    coordinator.records.write(
        &format!("delta-{}.json", &digest[7..]),
        &coordinator.key.sign("staged_delta", &json!({"manifest":delta.manifest,"patch_chunks":[chunk],"merge_request":id(818)})).unwrap(),
    ).unwrap();
    let pr = json!({"id":80,"number":8,"user":{"id":7},"state":"closed","draft":false,
        "head":{"sha":delta.manifest.validated_head.commit,"ref":"fixture-child","repo":{"id":42}},
        "base":{"sha":delta.manifest.staging_parent.commit,"ref":"staging","repo":{"id":42}},
        "merged":true,"merge_commit_sha":delta.manifest.staging_commit.commit,
        "html_url":"https://github.invalid/fixture/repo/pull/8"});
    {
        let mut current = wire.lock().unwrap();
        current.prs.retain(|p| p["number"] != 8);
        current.prs.push(pr);
    }
    let staged = StagedDelta {
        delta: delta.clone(),
        pull_request: 8,
        merge_receipt: id(818),
        protection: crate::evidence::RepositoryProtection {
            repository: delta.manifest.repository.clone(),
            branch: "staging".into(),
            observed_base: delta.manifest.staging_parent.commit.clone(),
            required_contexts: BTreeSet::from([crate::evidence::REVIEW_CONTEXT.into()]),
            strict_current_base: true,
            squash_allowed: true,
            extra_approvals_satisfied: true,
            code_owner_reviews_satisfied: true,
            admin_bypass: false,
            observed_at: Utc::now(),
        },
    };
    coordinator
        .assemble_isolated_slice(
            admission,
            SliceAssembly {
                execution_root: root,
                staged: &[&staged],
                required_children: admission.gate.required_children(),
                child_dependencies: &BTreeMap::new(),
                satisfied_children: &BTreeSet::new(),
                production_integrations: &[],
                identity: crate::git::CommitIdentity {
                    name: "Fixture".into(),
                    email: "fixture@example.invalid".into(),
                },
            },
        )
        .await
        .unwrap()
}
async fn changed_candidate_from(
    mut f: DeliveryFixture,
    actual_assembly: bool,
) -> (
    DeliveryFixture,
    crate::evidence::VerifiedParent,
    IsolatedSlice,
    PublishedChild,
) {
    use crate::evidence::*;
    let source = std::path::PathBuf::from(&f.repository_contract.configuration.source);
    let base = f.admission.scope.revision.base.clone();
    fixture_git(&source, &["checkout", "-qb", "child"]);
    std::fs::create_dir(source.join("tests")).unwrap();
    std::fs::write(source.join("tests/outcome.txt"), "exact scoped behavior\n").unwrap();
    fixture_git(&source, &["add", "tests/outcome.txt"]);
    fixture_git(
        &source,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-qm",
            "child fixture",
        ],
    );
    let head = fixture_git(&source, &["rev-parse", "HEAD"]);
    fixture_git(&source, &["checkout", "main"]);
    fixture_git(&source, &["merge", "--squash", "child"]);
    fixture_git(
        &source,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-qm",
            "staging squash fixture",
        ],
    );
    let squash = fixture_git(&source, &["rev-parse", "HEAD"]);
    let mut child_work = f.admission.scope.work.clone();
    child_work.child = Some(id(101));
    let delta = f
        .coordinator
        .git
        .capture_delta(
            &source,
            crate::git::CaptureDelta {
                repository: f.repository_contract.repository.clone(),
                parent_id: id(100),
                child_id: id(101),
                work_generation: &child_work.generation.definition,
                correction_generation: &child_work.generation.correction,
                scope: crate::git::WriteScope {
                    paths: vec!["tests/".into()],
                },
                base: &base,
                head: &head,
                staging_parent: &base,
                staging_commit: &squash,
            },
        )
        .unwrap();
    let root = f
        ._temp
        .path()
        .canonicalize()
        .unwrap()
        .join("candidate-runs");
    PrivateDir::create(&root).unwrap();
    let actual_slice = if actual_assembly {
        Some(assemble_seed_slice(&f, &delta, &root).await)
    } else {
        None
    };
    let candidate = if let Some(slice) = &actual_slice {
        f.coordinator
            .git
            .inspect_candidate(&root, slice.operation_id())
            .unwrap()
    } else {
        f.coordinator
            .git
            .assemble_candidate(
                &source,
                &root,
                crate::git::CandidateRequest {
                    operation_id: Uuid::new_v4(),
                    runtime_id: id(1),
                    repository: f.repository_contract.repository.clone(),
                    parent_id: id(100),
                    work_generation: &child_work.generation.definition,
                    correction_generation: &child_work.generation.correction,
                    production_commit: &base,
                    deltas: std::slice::from_ref(&delta),
                    child_dependencies: &BTreeMap::new(),
                    satisfied_children: &BTreeSet::new(),
                    production_integrations: &[],
                    identity: crate::git::CommitIdentity {
                        name: "Fixture".into(),
                        email: "fixture@example.invalid".into(),
                    },
                },
            )
            .unwrap()
    };
    let child_revision = RevisionBinding {
        repository: f.repository_contract.repository.clone(),
        head: head.clone(),
        base: base.clone(),
        tested_commit: head.clone(),
        tested_tree: delta.manifest.validated_head.tree.clone(),
    };
    let signed_review = f
        .key
        .sign(
            "codex_review",
            &ReviewAttestation {
                schema_version: 1,
                runtime_id: id(1),
                owner_generation: 1,
                operation_id: Uuid::new_v4(),
                work: child_work.clone(),
                revision: child_revision.clone(),
                pull_request: Some(8),
                delta_digest: delta.manifest.review_digest().unwrap(),
                verdict: Verdict::Pass,
                unresolved_findings: vec![],
                supporting_links: vec!["https://github.invalid/fixture/repo/pull/8".into()],
                review: ReviewKind::Agent {
                    session_id: "actual-fixture-role-protocol".into(),
                    instruction_manifest: canonical_digest(1, &json!("fixture instructions"))
                        .unwrap(),
                },
            },
        )
        .unwrap();
    let review_publication = fixture_publication(
        &signed_review,
        "github",
        "10",
        "7",
        "https://github.invalid/fixture/repo/pull/8#comment10",
    );
    let status = ReviewStatus {
        repository: child_revision.repository.clone(),
        head: head.clone(),
        context: REVIEW_CONTEXT.into(),
        state: "success".into(),
        writer_account_id: "7".into(),
        attestation_record_id: "10".into(),
    };
    let review = verify_review(
        &signed_review,
        &review_publication,
        Some(&status),
        &f.key,
        ReviewRequirements {
            work: &child_work,
            revision: &child_revision,
            delta_digest: &signed_review.payload.delta_digest,
            writer: "7",
        },
    )
    .unwrap();
    let now = Utc::now();
    let requirements = vec![
        CheckRequirement {
            id: "local".into(),
            kind: CheckKind::Local,
            workflow_id: None,
            command: vec!["fixture-local".into()],
        },
        CheckRequirement {
            id: "suite".into(),
            kind: CheckKind::HostedCi,
            workflow_id: Some(1),
            command: vec![],
        },
    ];
    let checks = CheckSnapshot {
        pagination_complete: true,
        observed_at: now,
        runs: requirements
            .iter()
            .map(|r| CheckRun {
                id: r.id.clone(),
                kind: r.kind.clone(),
                operation_id: Uuid::new_v4(),
                work_digest: child_work.digest().unwrap(),
                revision: child_revision.clone(),
                workflow_id: r.workflow_id,
                run_id: r.workflow_id.map(|_| 50),
                attempt: 1,
                runner: if r.kind == CheckKind::Local {
                    Runner::LocalSandbox
                } else {
                    Runner::GitHubHosted
                },
                command: r.command.clone(),
                environment_identity: "explicit synthetic prior child observation".into(),
                started_at: Some(now - Duration::seconds(2)),
                completed_at: Some(now - Duration::seconds(1)),
                conclusion: Conclusion::Success,
                artifacts: vec![artifact_bytes(
                    "fixture://child-output",
                    b"prior passed observations",
                )],
            })
            .collect(),
    };
    let criteria = BTreeSet::from(["outcome".into()]);
    let evidence = ChildEvidence {
        schema_version: 1,
        work: child_work.clone(),
        revision: child_revision.clone(),
        fulfilled_criteria: criteria.clone(),
        outcome: ChildOutcome::Changed {
            pull_request: 8,
            delta: Box::new(delta.manifest.clone()),
        },
        checks,
        review_digest: review.digest().into(),
        results_for_dependents: vec![artifact_bytes(
            "fixture://result",
            b"complete scoped outcome",
        )],
    };
    let protection = RepositoryProtection {
        repository: child_revision.repository.clone(),
        branch: "staging".into(),
        observed_base: base.clone(),
        required_contexts: BTreeSet::from([REVIEW_CONTEXT.into(), "suite".into()]),
        strict_current_base: true,
        squash_allowed: true,
        extra_approvals_satisfied: true,
        code_owner_reviews_satisfied: true,
        admin_bypass: false,
        observed_at: now,
    };
    let published = fixture_publication(
        &evidence,
        "linear",
        &id(809).to_string(),
        &id(5).to_string(),
        "https://linear.invalid/comment/809",
    );
    let verified = verify_child(
        &evidence,
        ChildRequirements {
            work: &child_work,
            criteria: &criteria,
            revision: &child_revision,
            checks: &requirements,
            required_contexts: &BTreeSet::from([REVIEW_CONTEXT.into()]),
        },
        &review,
        Some(&protection),
        &published,
        &id(5).to_string(),
    )
    .unwrap();
    let signed = f
        .key
        .sign(
            "child_result",
            &ChildResultRecord {
                schema_version: 1,
                runtime_id: id(1),
                publication_operation: Uuid::new_v4(),
                contract_digest: f.repository_contract.digest().unwrap(),
                workflow_digest: f.admission.gate.workflow_digest().into(),
                evidence: evidence.clone(),
                review: signed_review.clone(),
                review_publication: review_publication.clone(),
                review_status: Some(status.clone()),
                hosted_proofs: vec![],
                protection: Some(protection),
            },
        )
        .unwrap();
    let child_publication = fixture_publication(
        &signed,
        "linear",
        &id(809).to_string(),
        &id(5).to_string(),
        "https://linear.invalid/comment/809",
    );
    let child = PublishedChild {
        verified: verified.clone(),
        signed: signed.clone(),
        publication: child_publication,
    };
    let mut manifest = ParentEvidence {
        schema_version: 1,
        work: f.admission.scope.work.clone(),
        required_children: BTreeSet::from([id(101)]),
        e2e_child: id(101),
        child_results: BTreeMap::from([(id(101), verified.digest().into())]),
        required_contributions: BTreeMap::from([(
            id(101),
            delta.manifest.original_contributions().unwrap(),
        )]),
        executive_summary: "Actual isolated fixture candidate".into(),
        touched_components: vec!["tests/outcome.txt".into()],
        fulfilled_criteria: criteria.clone(),
        caveats: vec!["Synthetic prior provider checks; actual local Git objects".into()],
        go: true,
        baselines: vec![ProductionBaseline {
            repository: child_revision.repository.clone(),
            production: candidate.manifest.production.clone(),
            deployed_revision: None,
            target_revisions: BTreeMap::new(),
            deployed_artifact: None,
            target_artifacts: BTreeMap::new(),
            deployment_free: true,
            successor_recovery_operation: None,
        }],
        candidates: vec![candidate.manifest.clone()],
        dependency_snapshot_digest: canonical_digest(1, &json!("none")).unwrap(),
        production_targets: BTreeSet::new(),
        integrated_results: vec![],
        visual: false,
        video: None,
        publication_operation: Uuid::new_v4(),
        created_at: now,
        supersedes: None,
    };
    manifest.integrated_results.push(IntegratedRun {
        id: "integrated".into(),
        operation_id: Uuid::new_v4(),
        e2e_child: id(101),
        candidate_vector_digest: manifest.candidate_vector_digest().unwrap(),
        command: vec!["fixture-e2e".into()],
        test_resources: BTreeSet::new(),
        started_at: now - Duration::seconds(2),
        completed_at: now - Duration::seconds(1),
        conclusion: Conclusion::Success,
        after_staging_integration: true,
        artifacts: vec![artifact_bytes(
            "fixture://e2e",
            b"complete integrated observation",
        )],
    });
    let parent = verify_parent(
        &manifest,
        ParentRequirements {
            work: &manifest.work,
            required_children: &manifest.required_children,
            e2e_child: id(101),
            criteria: &criteria,
            integrated_commands: &BTreeMap::from([(
                "integrated".into(),
                vec!["fixture-e2e".into()],
            )]),
            test_resources: &BTreeSet::new(),
            production_targets: &BTreeSet::new(),
            visual: false,
            required_contributions: &manifest.required_contributions,
            production_integrations: &[],
        },
        &[verified],
        &fixture_publication(
            &manifest,
            "linear",
            "parent",
            "5",
            "https://linear.invalid/parent",
        ),
        "5",
    )
    .unwrap();
    let slice = if let Some(slice) = actual_slice {
        slice
    } else {
        IsolatedSlice {
            record: f
                .key
                .sign(
                    "isolated_slice",
                    &SliceRecord {
                        intent: SliceIntent {
                            schema_version: 1,
                            operation_id: candidate.manifest.operation_id,
                            request_id: Uuid::new_v4(),
                            scope: f.admission.scope.clone(),
                            execution_root: root,
                            production: candidate.manifest.production.clone(),
                            selected: vec![delta.manifest.digest().unwrap()],
                            dependencies: BTreeMap::new(),
                            satisfied_children: BTreeSet::new(),
                            integrations: vec![],
                            identity: crate::git::CommitIdentity {
                                name: "Fixture".into(),
                                email: "fixture@example.invalid".into(),
                            },
                        },
                        candidate: candidate.manifest.clone(),
                        receipt_id: Uuid::new_v4(),
                    },
                )
                .unwrap(),
            candidate,
        }
    };
    if !actual_assembly {
        f.host
            .finish(
                f.admission.operation_id(),
                json!({"fixture":"candidate complete"}),
            )
            .unwrap();
        let mut scope = f.admission.scope.clone();
        scope.phase = DeliveryPhase::Release;
        scope.revision = RevisionBinding {
            repository: child_revision.repository,
            head: slice.manifest().result.as_ref().unwrap().commit.clone(),
            base: base.clone(),
            tested_commit: slice.manifest().result.as_ref().unwrap().commit.clone(),
            tested_tree: slice.manifest().result.as_ref().unwrap().tree.clone(),
        };
        scope.candidate_vector_digest = Some(manifest.candidate_vector_digest().unwrap());
        let permit = f
            .host
            .admit(
                &f.admission.permit.lease,
                Uuid::new_v4(),
                f.admission.gate.work_key().unwrap(),
                scope.digest().unwrap(),
                || Ok(()),
            )
            .unwrap();
        f.admission = DeliveryAdmission::bind(permit, f.admission.gate.clone(), scope).unwrap();
    }
    let objects = f
        .coordinator
        .git
        .publication_objects(
            slice.path(),
            &slice.manifest().production,
            slice.manifest().result.as_ref().unwrap(),
        )
        .unwrap();
    {
        let mut wire = f.wire.lock().unwrap();
        wire.object_blobs = objects
            .blobs()
            .iter()
            .map(|(id, bytes)| {
                (
                    base64::engine::general_purpose::STANDARD.encode(bytes),
                    id.clone(),
                )
            })
            .collect();
        wire.object_tree = objects.revision().tree.clone();
        wire.object_commit = json!({"sha":objects.revision().commit,"tree":objects.revision().tree,"message":objects.message(),"author":objects.author(),"committer":objects.committer()});
        wire.comments.insert(id(101),vec![json!({"id":id(809),"issue":{"id":id(101)},"body":record_body("child_result",&signed).unwrap(),"createdAt":"2026-01-04T00:00:00Z","updatedAt":"2026-01-04T00:00:00Z","editedAt":null,"user":{"id":id(5),"isMe":true},"botActor":null,"externalUser":null,"url":"https://linear.invalid/comment/809"})]);
        wire.pr_comments.insert(8,vec![json!({"id":10,"body":record_body("codex_review",&signed_review).unwrap(),"user":{"id":7},"html_url":review_publication.url})]);
        wire.statuses.insert(head,vec![json!({"id":11,"context":REVIEW_CONTEXT,"state":"success","target_url":review_publication.url,"creator":{"id":7},"created_at":"2026-01-04T00:00:00Z"})]);
    }
    (f, parent, slice, child)
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_isolated_objects_publish_one_owned_production_pr_with_derived_child_review() {
    let (f, parent, slice, child) = changed_candidate_fixture().await;
    let prepared = f
        .coordinator
        .prepare_production_candidate(&f.admission, &parent, &slice, std::slice::from_ref(&child))
        .await
        .unwrap();
    assert_eq!(
        prepared.revision().head,
        slice.manifest().result.as_ref().unwrap().commit
    );
    assert_eq!(prepared.pull_request(), 17);
    let restored = f
        .coordinator
        .hydrate_production_candidate(&f.admission, &parent, prepared.operation_id())
        .await
        .unwrap();
    assert_eq!(restored.branch(), prepared.branch());
    f.coordinator
        .prepare_production_candidate(&f.admission, &parent, &slice, &[child])
        .await
        .unwrap();
    {
        let wire = f.wire.lock().unwrap();
        assert_eq!(wire.created_prs, 1);
        assert_eq!(wire.dispatch_count, 1);
        let body = wire.pr_comments[&17][0]["body"].as_str().unwrap();
        let signed: Signed<ReviewAttestation> =
            crate::evidence::decode_signed_publication(body, "codex_review", &f.key).unwrap();
        assert!(matches!(signed.payload.review, ReviewKind::Derived { .. }));
        assert_eq!(
            wire.statuses[&prepared.revision().head][0]["description"],
            "Carries verified unchanged child reviews"
        );
    }

    let record = &prepared.signed.payload;
    let pending = PendingExactValidation {
        work: f.admission.scope.work.clone(),
        revision: f.admission.scope.revision.clone(),
        receipt: f.host.operation(record.dispatches[&2]).unwrap(),
        challenge: record.challenge.clone(),
    };
    let proof = hosted_token(&f, &pending);
    let value = json!({"schema_version":1,"runtime_id":id(1),"phase_operation":f.admission.operation_id(),"dispatch_request":pending.receipt.request_id,"challenge":pending.challenge.challenge,
        "repository_id":42,"run_id":50,"run_attempt":1,"tested_tree":f.admission.scope.revision.tested_tree,"outputs":{"source_sha":f.admission.scope.revision.tested_commit},"proofs":[{"check_run_id":60,"token":proof.token}]});
    {
        let mut wire = f.wire.lock().unwrap();
        wire.archive = zip_fixture(
            "kegari-validation.json",
            &serde_json::to_vec(&value).unwrap(),
        );
        wire.artifact_name = format!("kegari-production-{}-2-1", f.admission.operation_id());
    }
    let production = f
        .coordinator
        .acquire_production_validation(&f.admission, &prepared, &parent, &verifier())
        .await
        .unwrap();
    assert_eq!(production.revision(), prepared.revision());
    let recovered = f
        .coordinator
        .hydrate_production_validation(&f.admission, &prepared, &parent)
        .await
        .unwrap();
    assert_eq!(production.digest().unwrap(), recovered.digest().unwrap());
    let acceptors = BTreeSet::from(["human".into()]);
    let accepted = crate::evidence::prepare_acceptance(
        &parent,
        crate::evidence::AcceptanceRequest {
            supplied_token: parent.token(),
            current_work: &parent.manifest().work,
            pending_correction: false,
            actor: crate::evidence::AcceptanceActor {
                control_actor: "fixture-operator",
                linear_actor: "human",
                provider_writer: "5",
                has_accept_role: true,
                permitted_acceptors: &acceptors,
            },
            runtime_id: id(1),
            operation_id: Uuid::new_v4(),
            now: Utc::now(),
        },
        &f.key,
    )
    .unwrap();
    let acceptance = crate::evidence::verify_acceptance(
        &accepted,
        &fixture_publication(
            &accepted,
            "linear",
            "acceptance",
            "5",
            "https://linear.invalid/acceptance",
        ),
        &parent,
        &parent.manifest().work,
        false,
        &acceptors,
        &f.key,
    )
    .unwrap();
    let mut workspace = config().workspaces.remove("example").unwrap();
    workspace
        .repositories
        .insert("42".into(), f.repository_contract.configuration.clone());
    let repositories = [ReleaseRepositoryEvidence {
        contract: &f.repository_contract,
        evidence: ReleaseStepEvidence::Changed {
            prepared: &prepared,
            production: &production,
        },
    }];
    let plan = build_release_plan(ReleasePlanInput {
        runtime_id: id(1),
        operation_id: Uuid::new_v4(),
        workspace: &workspace,
        parent: &parent,
        acceptance: &acceptance,
        carried: None,
        repositories: &repositories,
        dependencies: vec![],
    })
    .unwrap();
    assert_eq!(plan.steps.len(), 1);
    assert_eq!(
        plan.deployment_free,
        BTreeSet::from([f.repository_contract.repository.clone()])
    );
    assert!(
        matches!(&plan.steps[0].kind,StepKind::Merge {candidate,..} if candidate==slice.manifest().result.as_ref().unwrap())
    );
    assert!(
        build_release_plan(ReleasePlanInput {
            runtime_id: id(1),
            operation_id: Uuid::new_v4(),
            workspace: &workspace,
            parent: &parent,
            acceptance: &acceptance,
            carried: None,
            repositories: &[],
            dependencies: vec![]
        })
        .is_err()
    );
    let original_review = f.wire.lock().unwrap().pr_comments[&8][0]["body"].clone();
    f.wire.lock().unwrap().pr_comments.get_mut(&8).unwrap()[0]["body"] =
        json!("edited source review");
    assert!(
        f.coordinator
            .hydrate_production_validation(&f.admission, &prepared, &parent)
            .await
            .is_err()
    );
    f.wire.lock().unwrap().pr_comments.get_mut(&8).unwrap()[0]["body"] = original_review;
    f.host
        .retire_local(
            f.admission.operation_id(),
            json!({"preparation":"complete"}),
        )
        .unwrap();
    let facts = || crate::release::AdmissionFacts {
        running: true,
        included: true,
        authorized: true,
        no_pending_correction: true,
        preparation_complete: true,
        provider_reads_complete: true,
        repository_policy_usable: true,
        credentials_usable: true,
        manual_mode: true,
        explicit_manual_request: Some(id(992)),
        actor_may_release: true,
        owner_generation: 1,
        control_generation: 1,
        required_step_ids: plan.steps.iter().map(|s| s.id.clone()).collect(),
    };
    let proof = || crate::release::DeliveryEvidence {
        parent: &parent,
        acceptance: &acceptance,
        carried: None,
        production: std::slice::from_ref(&production),
    };
    let mut queue = crate::release::ReleaseQueue::default();
    let eligibility =
        crate::release::verify_release_eligibility(&plan, proof(), facts(), &queue, Utc::now())
            .unwrap();
    let mut delivery = crate::reconcile::ParentDelivery::default();
    delivery.parent(parent.clone());
    delivery.acceptance(acceptance.clone()).unwrap();
    delivery.release(eligibility.clone()).unwrap();
    let mut replacement = accepted.payload.clone();
    replacement.operation_id = Uuid::new_v4();
    let replacement = f.key.sign("human_acceptance", &replacement).unwrap();
    let replacement = crate::evidence::verify_acceptance(
        &replacement,
        &fixture_publication(
            &replacement,
            "linear",
            "replacement-acceptance",
            "5",
            "https://linear.invalid/replacement-acceptance",
        ),
        &parent,
        &parent.manifest().work,
        false,
        &acceptors,
        &f.key,
    )
    .unwrap();
    assert_ne!(replacement.digest(), acceptance.digest());
    delivery.acceptance(replacement).unwrap();
    assert_eq!(
        delivery.release(eligibility.clone()).unwrap_err().code,
        "stale_result"
    );
    let raced_operation = Uuid::new_v4();
    let raced = f.host.admit_observed(
        &f.admission.permit.lease,
        raced_operation,
        f.admission.gate.work_key().unwrap(),
        plan.digest().unwrap(),
        |_| {
            delivery.require_release_selection(
                parent.token(),
                acceptance.digest(),
                acceptance.acceptance().accepted_at,
            )
        },
    );
    assert_eq!(raced.unwrap_err().code, "stale_result");
    assert!(f.host.operation(raced_operation).is_err());
    delivery.acceptance(acceptance.clone()).unwrap();
    delivery
        .require_release_selection(
            parent.token(),
            acceptance.digest(),
            acceptance.acceptance().accepted_at,
        )
        .unwrap();
    assert!(
        delivery
            .require_release_selection(
                parent.token(),
                acceptance.digest(),
                acceptance.acceptance().accepted_at + chrono::Duration::seconds(1)
            )
            .is_err()
    );
    let mut attempt =
        crate::release::ReleaseAttempt::admit(plan.clone(), proof(), facts(), Utc::now()).unwrap();
    queue.reserve(&plan, Utc::now()).unwrap();
    let permit = f
        .host
        .admit(
            &f.admission.permit.lease,
            plan.operation_id,
            f.admission.gate.work_key().unwrap(),
            plan.digest().unwrap(),
            || Ok(()),
        )
        .unwrap();
    let revisions = BTreeMap::from([(
        f.repository_contract.repository.clone(),
        slice.manifest().production.clone(),
    )]);
    let fresh = crate::release::FreshReleaseFacts {
        generation: &plan.generation,
        evidence_token: &plan.evidence_token,
        owner_generation: 1,
        authority_current: true,
        pending_correction: false,
        provider_reads_complete: true,
        resource_lock_owned: queue.owns_all(&plan),
        credentials_usable: true,
        repository_policy_current: true,
        force_stopped: false,
        current_revisions: &revisions,
    };
    let mut retry = attempt.clone();
    let first_payload = retry
        .prepare_step(&plan.steps[0].id, &fresh, |_| Ok(()))
        .unwrap();
    assert_eq!(first_payload.attempt, 1);
    retry.mark_sent(&first_payload, &fresh, |_| Ok(())).unwrap();
    retry
        .observe_failure(
            &plan.steps[0].id,
            "known provider rejection".into(),
            Some("fixture-effect".into()),
            Utc::now(),
            |_| Ok(()),
        )
        .unwrap();
    // A corrected plan can inherit an exact known hold before it receives
    // execution authority. These are domain observations, not a new Permit.
    let mut predecessor = retry.clone();
    predecessor.plan.operation_id = Uuid::new_v4();
    predecessor.plan.generation.correction =
        crate::digest::canonical_digest(1, &"prior corrective generation").unwrap();
    let baseline = crate::release::ReconciledBaseline {
        provider_reads_complete: true,
        coherent: true,
        observed_at: Utc::now(),
        observations: parent
            .manifest()
            .baselines
            .iter()
            .map(|b| crate::release::BaselineObservation {
                resource: crate::release::ResourceKey::Repository {
                    repository: b.repository.clone(),
                },
                production_commit: b.production.commit.clone(),
                deployed_commit: None,
                artifact_digest: None,
                provider_record: "synthetic current main fixture observation".into(),
            })
            .collect(),
    };
    predecessor
        .supersede_plan(
            crate::release::RecoveryDecision {
                actor: "fixture-corrector",
                reason: "fresh accepted corrected plan",
                now: Utc::now(),
            },
            &plan,
            &acceptance,
            &baseline,
            |_| Ok(()),
        )
        .unwrap();
    assert_eq!(predecessor.assigned_successor, Some(plan.operation_id));
    assert!(predecessor.hold_reason.is_some());
    assert!(!predecessor.completed());
    let mut successor_queue = crate::release::ReleaseQueue::default();
    successor_queue.restore_hold(&predecessor).unwrap();
    successor_queue.eligible(&plan, Utc::now()).unwrap();
    let mut unassigned = plan.clone();
    unassigned.operation_id = Uuid::new_v4();
    assert!(successor_queue.eligible(&unassigned, Utc::now()).is_err());
    assert!(
        predecessor
            .supersede_plan(
                crate::release::RecoveryDecision {
                    actor: "fixture-corrector",
                    reason: "cannot reassign",
                    now: Utc::now(),
                },
                &unassigned,
                &acceptance,
                &baseline,
                |_| Ok(())
            )
            .is_err()
    );
    let first_failure = retry.clone();
    retry
        .observe_failure(
            &plan.steps[0].id,
            "known provider rejection".into(),
            Some("fixture-effect".into()),
            Utc::now(),
            |_| panic!("idempotent observation cannot rewrite history"),
        )
        .unwrap();
    assert_eq!(retry, first_failure);
    let mut resumed_queue = crate::release::ReleaseQueue::default();
    resumed_queue.reconcile_attempt(&retry).unwrap();
    assert!(resumed_queue.eligible(&plan, Utc::now()).is_err());
    crate::release::verify_release_resume_eligibility(
        &retry,
        proof(),
        facts(),
        &resumed_queue,
        Utc::now(),
    )
    .unwrap();
    let mut competing = retry.clone();
    competing.plan.operation_id = Uuid::new_v4();
    competing.plan.parent_id = Uuid::new_v4();
    let mut overlapping = resumed_queue.clone();
    overlapping.restore_hold(&competing).unwrap();
    assert!(
        crate::release::verify_release_resume_eligibility(
            &retry,
            proof(),
            facts(),
            &overlapping,
            Utc::now()
        )
        .is_err()
    );
    retry
        .resume(
            "fixture-operator",
            "provider prerequisite repaired",
            &parent,
            true,
            Utc::now(),
            |_| Ok(()),
        )
        .unwrap();
    resumed_queue.reconcile_attempt(&retry).unwrap();
    assert!(resumed_queue.owns_all(&plan));
    resumed_queue.eligible(&plan, Utc::now()).unwrap();
    let second_payload = retry
        .prepare_step(&plan.steps[0].id, &fresh, |_| Ok(()))
        .unwrap();
    assert_eq!(second_payload.attempt, 2);
    assert_ne!(
        canonical_digest(1, &first_payload).unwrap(),
        canonical_digest(1, &second_payload).unwrap()
    );
    assert!(retry.mark_sent(&first_payload, &fresh, |_| Ok(())).is_err());
    let second_dispatch = retry
        .mark_sent(&second_payload, &fresh, |_| Ok(()))
        .unwrap();
    let mut recovered: crate::release::ReleaseAttempt =
        serde_json::from_value(serde_json::to_value(&retry).unwrap()).unwrap();
    recovered.validate_recovered().unwrap();
    assert_eq!(recovered.step_attempt(&plan.steps[0].id).unwrap(), 2);
    recovered.recovery_records.clear();
    assert_eq!(
        recovered.validate_recovered().unwrap_err().code,
        "corrupt_record"
    );
    let payload = attempt
        .prepare_step(&plan.steps[0].id, &fresh, |_| Ok(()))
        .unwrap();
    let dispatch = attempt.mark_sent(&payload, &fresh, |_| Ok(())).unwrap();
    let mut wrong_scope = f.admission.scope.clone();
    wrong_scope.repository.id = "999".into();
    assert!(
        f.coordinator
            .bind_release_step(
                &permit,
                &f.admission.gate,
                wrong_scope,
                &plan,
                &dispatch,
                &eligibility
            )
            .is_err()
    );
    let admission = f
        .coordinator
        .bind_release_step(
            &permit,
            &f.admission.gate,
            f.admission.scope.clone(),
            &plan,
            &dispatch,
            &eligibility,
        )
        .unwrap();
    let generation = f.host.snapshot().unwrap().intent.generation;
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
                kind: "pause".into(),
                canonical_targets: vec![id(1).to_string()],
                expected: BTreeMap::new(),
                payload: Value::Null,
            },
            Lifecycle::Pause,
            generation,
        )
        .unwrap();
    let expected_plan = plan.digest().unwrap();
    let expected_step = plan.steps[0].id.clone();
    let observed = f
        .coordinator
        .dispatch_release_step(ReleaseStepRequest {
            admission: &admission,
            dispatch: &dispatch,
            plan: &plan,
            parent: &parent,
            acceptance: &acceptance,
            carried: None,
            evidence: ReleaseStepEvidence::Changed {
                prepared: &prepared,
                production: &production,
            },
            observer: &f.observer,
            verifier: &verifier(),
            final_gate: Arc::new(move |p, s| {
                if p.digest()? != expected_plan || s != expected_step {
                    return Err(Error::invalid("fixture queue binding changed"));
                }
                Ok(())
            }),
        })
        .await
        .unwrap();
    assert_eq!(observed.outcome(), &ReleaseStepOutcome::Succeeded);
    attempt
        .observe_success(
            &plan.steps[0].id,
            observed.effect().unwrap().clone(),
            observed.observed_at(),
            |_| Ok(()),
        )
        .unwrap();
    assert!(attempt.completed());
    assert!(queue.record_success(&attempt).is_ok());
    // Current main has genuinely advanced to this plan's observed squash. The
    // original signed policy/CI is recoverable only through the owned path.
    {
        let mut wire = f.wire.lock().unwrap();
        wire.production = "c".repeat(40);
        wire.production_tree = slice.manifest().result.as_ref().unwrap().tree.clone();
    }
    let recovery = f
        .coordinator
        .observation(f.admission.gate.clone(), production.revision().clone())
        .unwrap();
    assert!(
        f.coordinator
            .hydrate_production_validation(&recovery, &prepared, &parent)
            .await
            .is_err()
    );
    let (_, historical) = f
        .coordinator
        .hydrate_owned_merge_production(MergedProductionRecovery {
            observation: &recovery,
            parent: &parent,
            preparation: prepared.operation_id(),
            plan: &plan,
            merge_step: &plan.steps[0].id,
            attempt: 1,
            observer: &f.observer,
        })
        .await
        .unwrap();
    assert_eq!(historical.digest().unwrap(), production.digest().unwrap());
    f.wire.lock().unwrap().production = "d".repeat(40);
    assert!(
        f.coordinator
            .hydrate_owned_merge_production(MergedProductionRecovery {
                observation: &recovery,
                parent: &parent,
                preparation: prepared.operation_id(),
                plan: &plan,
                merge_step: &plan.steps[0].id,
                attempt: 1,
                observer: &f.observer,
            })
            .await
            .is_err()
    );
    f.wire.lock().unwrap().production = "c".repeat(40);
    let retry_admission = f
        .coordinator
        .bind_release_step(
            &permit,
            &f.admission.gate,
            f.admission.scope.clone(),
            &plan,
            &second_dispatch,
            &eligibility,
        )
        .unwrap();
    f.current.store(false, std::sync::atomic::Ordering::SeqCst);
    let not_sent = f
        .coordinator
        .dispatch_release_step(ReleaseStepRequest {
            admission: &retry_admission,
            dispatch: &second_dispatch,
            plan: &plan,
            parent: &parent,
            acceptance: &acceptance,
            carried: None,
            evidence: ReleaseStepEvidence::Changed {
                prepared: &prepared,
                production: &production,
            },
            observer: &f.observer,
            verifier: &verifier(),
            final_gate: Arc::new(|_, _| Ok(())),
        })
        .await
        .unwrap();
    assert!(
        matches!(not_sent.outcome(), ReleaseStepOutcome::Failed(reason) if reason.contains("preflight did not dispatch"))
    );
    assert_eq!(not_sent.attempt(), 2);
    assert!(not_sent.receipt().is_none());
    let recovered_no_send = f
        .coordinator
        .recover_owned_release_step(&plan, &plan.steps[0].id, 2, &f.observer)
        .await
        .unwrap();
    assert_eq!(recovered_no_send.outcome(), not_sent.outcome());
    assert!(
        f.coordinator
            .records
            .read::<Signed<ReleaseEffectRecord>>(&release_effect_leaf(
                plan.operation_id,
                &plan.steps[0].id,
                2
            ))
            .is_err()
    );
    assert_eq!(
        f.wire
            .lock()
            .unwrap()
            .requests
            .iter()
            .filter(|(url, _)| url.ends_with("/pulls/17/merge"))
            .count(),
        1
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn production_preparation_rechecks_source_review_and_main_after_object_publication() {
    for change_review in [true, false] {
        let (f, parent, slice, child) = changed_candidate_fixture().await;
        {
            let mut wire = f.wire.lock().unwrap();
            if change_review {
                wire.edit_source_after_blob = true;
            } else {
                wire.move_main_on_read = Some(wire.main_reads + 2);
            }
        }
        assert!(
            f.coordinator
                .prepare_production_candidate(&f.admission, &parent, &slice, &[child])
                .await
                .is_err()
        );
        let wire = f.wire.lock().unwrap();
        assert_eq!(wire.created_prs, 0);
        assert_eq!(wire.dispatch_count, 0);
        assert!(!wire.pr_comments.contains_key(&17));
        assert!(!wire.statuses.contains_key(&f.admission.scope.revision.head));
    }
}

// The runtime storage regression consumes the actual wire adapter's opaque
// observation; it does not construct or deserialize a proof.
pub(crate) async fn observed_merge_for_runtime() -> (
    ReleasePlan,
    crate::evidence::WorkIdentity,
    ObservedReleaseStep,
) {
    let (f, plan, record) = release_fixture().await;
    send_fixture_merge(&f, &record).await.unwrap();
    let observed = f
        .coordinator
        .recover_release_step(&f.admission, &plan, "merge", 1, &f.observer)
        .await
        .unwrap();
    (plan, f.admission.scope.work.clone(), observed)
}

#[tokio::test]
async fn release_effect_recovery_requires_exact_attempt_and_preserves_older_intent() {
    let (f, plan, record) = release_fixture().await;
    send_fixture_merge(&f, &record).await.unwrap();
    let first = f
        .coordinator
        .recover_release_step(&f.admission, &plan, "merge", 1, &f.observer)
        .await
        .unwrap();
    assert_eq!(first.attempt(), 1);
    assert!(
        f.coordinator
            .recover_release_step(&f.admission, &plan, "merge", 2, &f.observer)
            .await
            .is_err()
    );
    let first_bytes = f
        .coordinator
        .records
        .read_bytes(
            &release_effect_leaf(plan.operation_id, "merge", 1),
            1024 * 1024,
        )
        .unwrap();
    let mut altered = record.clone();
    altered.payload.attempt = 2;
    // Copying an old receipt to a newer ordinal is not a valid dispatch lineage.
    f.coordinator
        .records
        .write(
            &release_effect_leaf(plan.operation_id, "merge", 2),
            &f.key.sign("release_effect", &altered).unwrap(),
        )
        .unwrap();
    assert!(
        f.coordinator
            .recover_release_step(&f.admission, &plan, "merge", 2, &f.observer)
            .await
            .is_err()
    );
    let again = f
        .coordinator
        .recover_release_step(&f.admission, &plan, "merge", 1, &f.observer)
        .await
        .unwrap();
    assert_eq!(
        again.receipt().unwrap().operation_id,
        first.receipt().unwrap().operation_id
    );
    assert_eq!(
        first_bytes,
        f.coordinator
            .records
            .read_bytes(
                &release_effect_leaf(plan.operation_id, "merge", 1),
                1024 * 1024
            )
            .unwrap()
    );
    assert_eq!(
        f.wire
            .lock()
            .unwrap()
            .requests
            .iter()
            .filter(|(url, _)| url.ends_with("/merge"))
            .count(),
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn original_release_reader_requires_actual_native_publications_and_immutable_receipts() {
    let (f, parent, slice, _) = changed_candidate_fixture().await;
    let writer = id(5).to_string();
    let acceptors = BTreeSet::from([writer.clone()]);
    let owner = format!("owner:{}", f.admission.permit.lease.owner_id);
    let accepted = crate::evidence::prepare_acceptance(
        &parent,
        crate::evidence::AcceptanceRequest {
            supplied_token: parent.token(),
            current_work: &parent.manifest().work,
            pending_correction: false,
            actor: crate::evidence::AcceptanceActor {
                control_actor: &owner,
                linear_actor: &writer,
                provider_writer: &writer,
                has_accept_role: true,
                permitted_acceptors: &acceptors,
            },
            runtime_id: id(1),
            operation_id: Uuid::new_v4(),
            now: Utc::now(),
        },
        &f.key,
    )
    .unwrap();
    let repository = f.repository_contract.repository.clone();
    let mut plan = ReleasePlan {
        schema_version: 1,
        runtime_id: id(1),
        operation_id: Uuid::new_v4(),
        parent_id: parent.manifest().work.parent,
        linear_workspace: parent.manifest().work.linear_workspace,
        generation: parent.manifest().work.generation.clone(),
        evidence_token: parent.token().into(),
        acceptance_digest: accepted.digest().unwrap(),
        carried_acceptance: None,
        resources: BTreeSet::from([ResourceKey::Repository {
            repository: repository.clone(),
        }]),
        dependencies: vec![],
        spacing_seconds: 0,
        deployment_free: BTreeSet::from([repository.clone()]),
        steps: vec![crate::release::ReleaseStep {
            id: "merge".into(),
            depends_on: BTreeSet::new(),
            kind: StepKind::Merge {
                repository,
                pull_request: 17,
                production_base: slice.manifest().production.clone(),
                candidate: slice.manifest().result.clone().unwrap(),
                selected_delta_digests: slice.manifest().ordered_delta_digests.clone(),
            },
        }],
    };
    f.host
        .finish(
            f.admission.operation_id(),
            json!({"fixture":"preparation finished"}),
        )
        .unwrap();
    let permit = f
        .host
        .admit(
            &f.admission.permit.lease,
            parent.manifest().publication_operation,
            f.admission.gate.work_key().unwrap(),
            canonical_digest(1, &"fixture original parent publication").unwrap(),
            || Ok(()),
        )
        .unwrap();
    let parent_body = f
        .coordinator
        .fixture_original_parent_body(&parent, &slice, id(1))
        .unwrap();
    let mut ids = Vec::new();
    for (request_id, body) in [
        (Uuid::new_v4(), parent_body),
        (
            accepted.payload.operation_id,
            record_body("human_acceptance", &accepted).unwrap(),
        ),
    ] {
        let receipt = f
            .coordinator
            .broker
            .mutate(
                f.host.clone(),
                MutationRequest {
                    request_id,
                    runtime_id: id(1),
                    actor: Actor {
                        principal: None,
                        identity: format!("owner:{}", permit.lease.owner_id),
                        roles: BTreeSet::new(),
                    },
                    kind: String::new(),
                    canonical_targets: vec![],
                    expected: BTreeMap::new(),
                    payload: Value::Null,
                },
                Some(permit.clone()),
                None,
                ProviderMutation::LinearComment {
                    issue: parent.manifest().work.parent,
                    body,
                },
                || Ok(()),
            )
            .await
            .unwrap();
        ids.push(Uuid::parse_str(&receipt.remote_ids["comment_id"]).unwrap());
    }
    f.host
        .finish(
            permit.operation_id,
            json!({"fixture":"original publication complete"}),
        )
        .unwrap();
    f.host
        .admit(
            &permit.lease,
            plan.operation_id,
            permit.work_key.clone(),
            plan.digest().unwrap(),
            || Ok(()),
        )
        .unwrap();
    let proof = f
        .coordinator
        .release_origin(&plan, ids[0], ids[1])
        .await
        .unwrap();
    assert_eq!(proof.manifest(), parent.manifest());
    assert_eq!(proof.plan().digest().unwrap(), plan.digest().unwrap());
    assert_eq!(
        proof.slice_operation(&slice.manifest().repository).unwrap(),
        slice.operation_id()
    );
    let saved = f.wire.lock().unwrap().comments[&id(100)].clone();
    f.wire
        .lock()
        .unwrap()
        .comments
        .get_mut(&id(100))
        .unwrap()
        .retain(|comment| comment["id"] != json!(ids[1]));
    assert!(
        f.coordinator
            .release_origin(&plan, ids[0], ids[1])
            .await
            .is_err()
    );
    f.wire
        .lock()
        .unwrap()
        .comments
        .insert(id(100), saved.clone());
    f.wire
        .lock()
        .unwrap()
        .comments
        .get_mut(&id(100))
        .unwrap()
        .iter_mut()
        .find(|comment| comment["id"] == json!(ids[0]))
        .unwrap()["body"] = json!("edited signed parent record");
    assert!(
        f.coordinator
            .release_origin(&plan, ids[0], ids[1])
            .await
            .is_err()
    );
    f.wire.lock().unwrap().comments.insert(id(100), saved);
    f.host
        .finish(plan.operation_id, json!({"fixture":"original retired"}))
        .unwrap();
    plan.operation_id = Uuid::new_v4();
    plan.carried_acceptance = Some(crate::evidence::CarriedAcceptance {
        schema_version: 1,
        original_reviewed_token: plan.evidence_token.clone(),
        original_acceptance_digest: plan.acceptance_digest.clone(),
        current_evidence_token: plan.evidence_token.clone(),
        unchanged_patch_proof: canonical_digest(1, &"fixture carry").unwrap(),
        production_validation_digest: canonical_digest(1, &"fixture validation").unwrap(),
        created_at: Utc::now(),
    });
    f.host
        .admit(
            &permit.lease,
            plan.operation_id,
            permit.work_key.clone(),
            plan.digest().unwrap(),
            || Ok(()),
        )
        .unwrap();
    assert_eq!(
        f.coordinator
            .release_origin(&plan, ids[0], ids[1])
            .await
            .unwrap_err()
            .code,
        "candidate_provenance",
        "carried-acceptance history remains an explicit unsupported prerequisite, never silently direct acceptance"
    );
}

#[path = "lineage_wire.rs"]
mod lineage_wire;
pub(crate) use lineage_wire::{RuntimeLineageWire, TargetReleaseFixture};

/// Accepted upstream plan/scope are explicit synthetic inputs. This regression
/// uses actual host admissions, provider broker receipts, signed artifacts/OIDC,
/// private storage and native-shaped HTTP readback. It grants no integrated test
/// capability and never executes Git, a repository command or a remote request.
#[tokio::test]
async fn terminal_target_history_binds_plan_permit_and_reconciled_original_effect() {
    let f = target_wire::Fixture::new();
    let contract = target_wire::contract(true);
    let (initial, initial_permit, _, _) = f.prepare(&contract).await;
    f.host
        .retire_local(
            initial.operation_id(),
            json!({"fixture":"initial owned metadata finished"}),
        )
        .unwrap();
    {
        let mut state = f.state.lock().unwrap();
        state.run_id = 51;
        state.deployment_id = 92;
    }
    let scope = initial.scope().clone();
    let target = ResourceKey::Target {
        provider: "github".into(),
        id: "service-production".into(),
    };
    let deploy = StepKind::DeployExisting {
        repository: contract.repository.clone(),
        target: target.clone(),
        workflow_id: 3,
        workflow_revision: "1".repeat(40),
        dispatch_ref: "heads/trusted".into(),
        source_input: "source_sha".into(),
        source: target_wire::source(),
        expected_artifact: None,
    };
    let verify = StepKind::VerifyDeployed {
        repository: contract.repository.clone(),
        target,
        source: target_wire::source(),
        deployment_step: "deploy".into(),
        expected_artifact: None,
    };
    let plan = ReleasePlan {
        schema_version: 1,
        runtime_id: id(1),
        operation_id: Uuid::new_v4(),
        parent_id: scope.work.parent,
        linear_workspace: scope.work.linear_workspace,
        generation: scope.work.generation.clone(),
        evidence_token: canonical_digest(1, &"synthetic accepted parent").unwrap(),
        acceptance_digest: canonical_digest(1, &"synthetic native acceptance").unwrap(),
        carried_acceptance: None,
        resources: BTreeSet::from([
            ResourceKey::Repository {
                repository: contract.repository.clone(),
            },
            ResourceKey::Target {
                provider: "github".into(),
                id: "service-production".into(),
            },
        ]),
        dependencies: vec![],
        spacing_seconds: 0,
        deployment_free: BTreeSet::new(),
        steps: vec![
            crate::release::ReleaseStep {
                id: "deploy".into(),
                depends_on: BTreeSet::new(),
                kind: deploy.clone(),
            },
            crate::release::ReleaseStep {
                id: "verify".into(),
                depends_on: BTreeSet::from(["deploy".into()]),
                kind: verify.clone(),
            },
        ],
    };
    plan.validate().unwrap();
    let permit = f
        .host
        .admit(
            &initial_permit.lease,
            plan.operation_id,
            initial_permit.work_key.clone(),
            plan.digest().unwrap(),
            || Ok(()),
        )
        .unwrap();
    // Synthetic upstream accepted scope; actual plan permit is retained exactly.
    let admission = DeliveryAdmission {
        permit: permit.clone(),
        observation: DeliveryObservation {
            gate: initial.fresh_gate().clone(),
            scope: scope.clone(),
            admitted: Some(permit.clone()),
        },
    };
    assert_ne!(scope.digest().unwrap(), permit.scope_digest);
    let challenge = f
        .observer
        .prepare_target(
            &contract,
            &admission,
            &permit,
            "deploy",
            target_wire::source(),
        )
        .await
        .unwrap();
    let command = f.observer.dispatch_command(&contract, &challenge).unwrap();
    let mut request = target_wire::request(challenge.dispatch_request());
    request.expected.extend(BTreeMap::from([
        ("release_plan".into(), plan.digest().unwrap()),
        ("release_step".into(), "deploy".into()),
        ("release_attempt".into(), "1".into()),
    ]));
    let receipt = ProviderBroker {
        linear: None,
        github: Some(f.github.clone()),
    }
    .mutate(
        f.host.clone(),
        request,
        Some(permit.clone()),
        None,
        command.clone(),
        || Ok(()),
    )
    .await
    .unwrap();
    f.artifact(&contract, &challenge, |_| {});
    let target_proof = f
        .observer
        .observe_target(
            &contract,
            &admission,
            &permit,
            &challenge,
            &receipt,
            &target_wire::verifier(),
        )
        .await
        .unwrap();
    let key = Arc::new(AttestationKey::new("fixture".into(), vec![7; 32]).unwrap());
    let base = f.records.parent().unwrap();
    let records = Arc::new(PrivateDir::create(&base.join("delivery-history")).unwrap());
    let coordinator = DeliveryCoordinator::new(
        DeliveryServices {
            host: f.host.clone(),
            broker: ProviderBroker {
                linear: None,
                github: Some(f.github.clone()),
            },
            records: records.clone(),
            key: key.clone(),
            git: Arc::new(Git::new(pinned_git::prepare(base)).unwrap()),
            retained_git_sources: BTreeMap::new(),
            current_gate: Arc::new(|_| {
                Err(Error::new(
                    "fixture_no_execution",
                    "historical reads cannot invoke execution authority",
                ))
            }),
            executions: Arc::new(crate::worker::ExecutionRegistry::default()),
            processes: Arc::new(
                crate::process_records::ProcessJournal::with_probe(
                    PrivateDir::create(&base.join("historical-processes")).unwrap(),
                    id(1),
                    Arc::new(process_probe::FixtureBootProbe::new()),
                )
                .unwrap(),
            ),
        },
        vec![contract.clone()],
    )
    .unwrap();
    let mut record = ReleaseEffectRecord {
        schema_version: 1,
        plan_digest: plan.digest().unwrap(),
        payload: crate::release::StepPayload {
            operation_id: plan.operation_id,
            step_id: "deploy".into(),
            attempt: 1,
            prerequisite_attempts: BTreeMap::new(),
            plan_digest: plan.digest().unwrap(),
            kind: deploy,
            exact_source_commit: Some(target_wire::source().commit),
        },
        scope,
        admission_operation: permit.operation_id,
        admission_scope_digest: permit.scope_digest.clone(),
        request_id: Some(challenge.dispatch_request()),
        command: Some(command),
        deployment_step: Some("deploy".into()),
        challenge: Some(challenge.challenge()),
    };
    records
        .write(
            &release_effect_leaf(plan.operation_id, "deploy", 1),
            &key.sign("release_effect", &record).unwrap(),
        )
        .unwrap();
    let expected = coordinator
        .release_target_effect(&plan, &record, &target_proof, false)
        .unwrap();
    assert!(
        coordinator
            .recover_terminal_release_step(&plan, "deploy", 1, &expected, &f.observer)
            .await
            .is_err()
    );
    let reconciled=f.host.reconcile_owned_checked(&receipt,crate::control::Reconciliation::UniqueVerified {remote_ids:receipt.remote_ids.clone(),result:json!({"release_plan":plan.digest().unwrap(),"step":"deploy","outcome":"succeeded","effect":expected}),payload_digest:receipt.payload_digest.clone(),actor:receipt.actor.clone()},||Ok(())).unwrap();
    assert_eq!(reconciled.phase, crate::control::ReceiptPhase::Reconciled);
    // A crash may leave the proof file but no indexes after reconciliation.
    // Repair uses that immutable original proof, not an expired/new JWT.
    for leaf in PrivateDir::open(&f.records).unwrap().records().unwrap() {
        if leaf.starts_with("target-index-")
            || leaf.starts_with("target-digest-")
            || leaf.starts_with("target-dispatch-")
        {
            std::fs::remove_file(f.records.join(leaf)).unwrap();
        }
    }
    {
        let mut state = f.state.lock().unwrap();
        state.archive.clear();
        state.requests.clear();
    }
    let replayed = f
        .observer
        .observe_target(
            &contract,
            &admission,
            &permit,
            &challenge,
            &reconciled,
            &target_wire::verifier(),
        )
        .await
        .unwrap();
    assert_eq!(replayed.digest().unwrap(), target_proof.digest().unwrap());
    assert!(
        f.state
            .lock()
            .unwrap()
            .requests
            .iter()
            .all(|(url, body)| body.is_null() && !url.contains("/artifacts"))
    );
    // The ordinary current proof remains usable after ACK fields are replaced
    // by the runtime's exact reconciled effect, before target replacement.
    assert_eq!(
        f.observer
            .recover_target(&contract, "deploy")
            .await
            .unwrap()
            .digest()
            .unwrap(),
        target_proof.digest().unwrap()
    );
    assert!(
        coordinator
            .recover_terminal_release_step(&plan, "deploy", 1, &expected, &f.observer)
            .await
            .is_err()
    );
    let replacement = f.replace_target(&contract, &admission, &permit).await;
    assert_eq!(replacement.deployment_id(), 93);
    assert_eq!(
        f.observer
            .recover_target(&contract, "deploy")
            .await
            .unwrap()
            .deployment_id(),
        93
    );
    f.state.lock().unwrap().requests.clear();
    let observed = coordinator
        .recover_terminal_release_step(&plan, "deploy", 1, &expected, &f.observer)
        .await
        .unwrap();
    assert_eq!(observed.effect(), Some(&expected));
    let verification = coordinator
        .release_target_effect(&plan, &record, &target_proof, true)
        .unwrap();
    record.payload.step_id = "verify".into();
    record.payload.kind = verify;
    record.payload.prerequisite_attempts = BTreeMap::from([("deploy".into(), 1)]);
    record.request_id = None;
    record.command = None;
    record.challenge = None;
    records
        .write(
            &release_effect_leaf(plan.operation_id, "verify", 1),
            &key.sign("release_effect", &record).unwrap(),
        )
        .unwrap();
    let verified = coordinator
        .recover_terminal_release_step(&plan, "verify", 1, &verification, &f.observer)
        .await
        .unwrap();
    assert_eq!(verified.effect(), Some(&verification));
    assert!(
        coordinator
            .recover_terminal_release_step(&plan, "deploy", 2, &expected, &f.observer)
            .await
            .is_err()
    );
    let mut wrong = verification.clone();
    if let ObservedEffect::Verification {
        evidence_digest, ..
    } = &mut wrong
    {
        *evidence_digest = bytes_digest(b"another proof");
    }
    assert!(
        coordinator
            .recover_terminal_release_step(&plan, "verify", 1, &wrong, &f.observer)
            .await
            .is_err()
    );
    assert!(
        f.state
            .lock()
            .unwrap()
            .requests
            .iter()
            .all(|(_, body)| body.is_null())
    );
    // A changed original dispatch observation cannot be rescued by a still
    // matching protected proof or by the newer successful target.
    f.host.reconcile_owned_checked(&reconciled,crate::control::Reconciliation::UniqueVerified {remote_ids:reconciled.remote_ids.clone(),result:json!({"release_plan":plan.digest().unwrap(),"step":"deploy","outcome":"succeeded","effect":verification}),payload_digest:reconciled.payload_digest.clone(),actor:reconciled.actor.clone()},||Ok(())).unwrap();
    assert!(
        coordinator
            .recover_terminal_release_step(&plan, "deploy", 1, &expected, &f.observer)
            .await
            .is_err()
    );
}
