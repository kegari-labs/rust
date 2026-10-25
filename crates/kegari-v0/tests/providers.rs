//! Production provider algorithms with an injected wire boundary. No request in
//! these tests contacts Linear, GitHub, or a live agent.
use base64::Engine;
use kegari_v0::{
    config::{GithubSelection, LinearSelection, Selector},
    control::{Actor, ControlHost, MutationRequest, ReceiptPhase},
    digest::bytes_digest,
    error::{Error, Result},
    providers::*,
    storage::PrivateDir,
};
use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};
use uuid::Uuid;

#[derive(Default)]
struct Wire {
    responses: Mutex<VecDeque<Result<HttpResponse>>>,
    requests: Mutex<Vec<HttpRequest>>,
}
impl Wire {
    fn fixture(responses: Vec<Result<HttpResponse>>) -> Arc<Self> {
        Arc::new(Self {
            responses: Mutex::new(responses.into()),
            requests: Mutex::new(Vec::new()),
        })
    }
    fn len(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
    fn body(&self, index: usize) -> Value {
        serde_json::from_slice(&self.requests.lock().unwrap()[index].body).unwrap()
    }
}
impl HttpTransport for Wire {
    fn send(
        &self,
        request: HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Result<HttpResponse>> + Send + '_>> {
        Box::pin(async move {
            self.requests.lock().unwrap().push(request);
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected request")
        })
    }
}
fn response(v: Value) -> Result<HttpResponse> {
    Ok(HttpResponse {
        status: 200,
        headers: HeaderMap::new(),
        body: serde_json::to_vec(&v).unwrap(),
    })
}
fn linked(v: Value, next: &str) -> Result<HttpResponse> {
    let mut r = response(v).unwrap();
    r.headers.insert(
        "link",
        HeaderValue::from_str(&format!("<{next}>; rel=\"next\"")).unwrap(),
    );
    Ok(r)
}
fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
fn linear_identity() -> Result<HttpResponse> {
    response(json!({"data":{"viewer":{"id":id(1),"name":"Fixture"},"organization":{"id":id(2)}}}))
}
fn linear(wire: Arc<Wire>) -> LinearClient {
    LinearClient::with_transport(
        HttpPolicy::new(
            "https://linear.invalid/graphql",
            id(1).to_string(),
            Duration::from_secs(2),
        )
        .unwrap(),
        id(2),
        LinearSelection {
            teams: Selector::All("all".into()),
            projects: Selector::All("all".into()),
            users: Selector::All("all".into()),
            authorization_label_id: id(3),
            states: BTreeMap::new(),
        },
        Credential::new("test-linear-secret".into()).unwrap(),
        Some(wire),
    )
    .unwrap()
}
fn connection(nodes: Value, more: bool, cursor: Value) -> Value {
    json!({"nodes":nodes,"pageInfo":{"hasNextPage":more,"endCursor":cursor}})
}
fn issue(n: u128) -> Value {
    json!({"id":id(n),"identifier":format!("TEST-{n}"),"title":"Fixture task","description":"Scoped fixture","priority":2.0,"createdAt":"2026-01-01T00:00:00Z","updatedAt":"2026-01-02T00:00:00Z","url":"https://linear.invalid/issue/test","team":{"id":id(10)},"project":null,"creator":{"id":id(1)},"assignee":null,"parent":null,"state":{"id":id(11),"name":"Todo","type":"unstarted"},"labelIds":[]})
}
fn labels() -> Result<HttpResponse> {
    response(json!({"data":{"issue":{"labels":connection(json!([]),false,Value::Null)}}}))
}
fn gh_identity() -> Result<HttpResponse> {
    response(json!({"id":7,"login":"fixture"}))
}
fn repo() -> RepositoryIdentity {
    RepositoryIdentity {
        id: 42,
        owner: "fixture".into(),
        name: "repo".into(),
        owner_id: 7,
        organization_id: None,
    }
}
fn gh_repo() -> Result<HttpResponse> {
    response(
        json!({"id":42,"owner":{"id":7,"login":"fixture","type":"User"},"name":"repo","default_branch":"main","private":true,"archived":false,"disabled":false,"allow_squash_merge":true,"permissions":{"pull":true,"push":true}}),
    )
}
fn github(wire: Arc<Wire>) -> GithubClient {
    GithubClient::with_transport(
        HttpPolicy::new(
            "https://github.invalid/api/v3",
            "7".into(),
            Duration::from_secs(2),
        )
        .unwrap(),
        GithubSelection {
            repositories: vec!["42".into()],
            organizations: vec![],
            pr_authors: vec!["7".into()],
        },
        Credential::new("test-github-secret".into()).unwrap(),
        Some(wire),
    )
    .unwrap()
}
const HEAD: &str = "1111111111111111111111111111111111111111";
const BASE: &str = "2222222222222222222222222222222222222222";
fn pr(n: u64) -> Value {
    json!({"id":n+100,"number":n,"user":{"id":7},"base":{"sha":BASE,"ref":"staging","repo":{"id":42}},"head":{"sha":HEAD,"ref":"TEST-1-title","repo":{"id":42}},"state":"open","draft":false,"merged":false,"merge_commit_sha":null,"html_url":format!("https://github.invalid/fixture/repo/pull/{n}")})
}

#[tokio::test]
async fn linear_complete_refresh_has_exact_requested_partition_and_empty_is_no_request() {
    let wire = Wire::fixture(vec![
        linear_identity(),
        response(json!({"data":{"issues":connection(json!([issue(20)]),false,Value::Null)}})),
        labels(),
    ]);
    let client = linear(wire.clone());
    let empty = client.refresh(&BTreeSet::new()).await.unwrap();
    assert!(empty.issues.is_empty());
    assert_eq!(wire.len(), 0);
    let result = client
        .refresh(&BTreeSet::from([id(20), id(21)]))
        .await
        .unwrap();
    assert_eq!(result.issues.keys().copied().collect::<Vec<_>>(), [id(20)]);
    assert_eq!(result.missing, BTreeSet::from([id(21)]));
    assert_eq!(
        wire.body(1)["variables"]["filter"]["id"]["in"],
        json!([id(20), id(21)])
    );
    assert_eq!(
        wire.requests.lock().unwrap()[0].headers["authorization"],
        "test-linear-secret"
    );
}
#[tokio::test]
async fn linear_last_page_partial_graphql_errors_discard_prior_pages() {
    let wire = Wire::fixture(vec![
        linear_identity(),
        response(json!({"data":{"issues":connection(json!([issue(20)]),true,json!("cursor-a"))}})),
        response(
            json!({"data":{"issues":connection(json!([issue(21)]),false,Value::Null)},"errors":[{"message":"private issue content must not leak","extensions":{"code":"INTERNAL"}}]}),
        ),
    ]);
    let error = linear(wire.clone()).discover_parents().await.unwrap_err();
    assert_eq!(error.code, "provider_response");
    assert!(
        !serde_json::to_string(&error)
            .unwrap()
            .contains("private issue")
    );
    assert_eq!(wire.len(), 3);
    assert_eq!(wire.body(2)["variables"]["after"], "cursor-a");
}
#[tokio::test]
async fn linear_cursor_loops_and_blank_labels_cannot_hide_incomplete_sets() {
    let wire = Wire::fixture(vec![
        linear_identity(),
        response(json!({"data":{"issues":connection(json!([issue(20)]),true,json!("same"))}})),
        response(json!({"data":{"issues":connection(json!([issue(21)]),true,json!("same"))}})),
    ]);
    assert_eq!(
        linear(wire).discover_parents().await.unwrap_err().code,
        "provider_pagination"
    );
    let mut value = issue(20);
    value["labelIds"] = json!([id(31), id(32)]);
    let wire = Wire::fixture(vec![
        linear_identity(),
        response(json!({"data":{"issues":connection(json!([value]),false,Value::Null)}})),
        response(
            json!({"data":{"issue":{"labels":connection(json!([{"id":id(31),"name":"  "}]),false,Value::Null)}}}),
        ),
    ]);
    assert_eq!(
        linear(wire).discover_parents().await.unwrap_err().code,
        "provider_response"
    );
}
#[tokio::test]
async fn linear_http_400_rate_limit_retains_millisecond_reset_without_raw_body() {
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-ratelimit-requests-remaining",
        HeaderValue::from_static("0"),
    );
    headers.insert(
        "x-ratelimit-requests-reset",
        HeaderValue::from_static("1800000000123"),
    );
    let wire = Wire::fixture(vec![Ok(HttpResponse {
        status: 400,
        headers,
        body: serde_json::to_vec(
            &json!({"errors":[{"message":"secret-value","extensions":{"code":"RATELIMITED"}}]}),
        )
        .unwrap(),
    })]);
    let e = linear(wire).identity().await.unwrap_err();
    assert_eq!(e.code, "rate_limited");
    assert_eq!(
        e.details["rate_limit"]["reset_at"],
        "2027-01-15T08:00:00.123Z"
    );
    assert!(!serde_json::to_string(&e).unwrap().contains("secret-value"));
}
#[tokio::test]
async fn github_complete_pages_keep_filters_and_accept_same_repository_id_alias() {
    let wire = Wire::fixture(vec![
        gh_identity(),
        gh_repo(),
        linked(
            json!([pr(1)]),
            "https://github.invalid/api/v3/repositories/42/pulls?state=all&per_page=100&page=2",
        ),
        response(json!([pr(2)])),
    ]);
    let prs = github(wire.clone()).pull_requests(&repo()).await.unwrap();
    assert_eq!(prs.iter().map(|p| p.number).collect::<Vec<_>>(), [1, 2]);
    let requests = wire.requests.lock().unwrap();
    assert_eq!(requests[0].url.path(), "/api/v3/user");
    assert_eq!(
        requests[2].headers["x-github-api-version"],
        GITHUB_API_VERSION
    );
    assert_eq!(
        requests[2].headers["authorization"],
        "Bearer test-github-secret"
    );
}
#[tokio::test]
async fn github_continuation_cannot_change_origin_target_filters_or_sequence() {
    for next in [
        "https://evil.invalid/api/v3/repos/fixture/repo/pulls?state=all&per_page=100&page=2",
        "https://github.invalid/api/v3/repos/fixture/other/pulls?state=all&per_page=100&page=2",
        "https://github.invalid/api/v3/repos/fixture/repo/pulls?state=open&per_page=100&page=2",
        "https://github.invalid/api/v3/repos/fixture/repo/pulls?state=all&per_page=100&page=1",
        "https://github.invalid/api/v3/repos/fixture/repo/pulls?state=all&per_page=100&page=2&page=2",
    ] {
        let wire = Wire::fixture(vec![gh_identity(), gh_repo(), linked(json!([pr(1)]), next)]);
        assert_eq!(
            github(wire.clone())
                .pull_requests(&repo())
                .await
                .unwrap_err()
                .code,
            "provider_pagination"
        );
        assert_eq!(wire.len(), 3);
    }
}
#[tokio::test]
async fn github_last_page_failures_and_missing_total_count_items_are_atomic() {
    let wire = Wire::fixture(vec![
        gh_identity(),
        gh_repo(),
        linked(
            json!([pr(1)]),
            "https://github.invalid/api/v3/repos/fixture/repo/pulls?state=all&per_page=100&page=2",
        ),
        Ok(HttpResponse {
            status: 503,
            headers: HeaderMap::new(),
            body: b"private response".to_vec(),
        }),
    ]);
    assert_eq!(
        github(wire).pull_requests(&repo()).await.unwrap_err().code,
        "provider_status"
    );
    let wire = Wire::fixture(vec![
        gh_identity(),
        gh_repo(),
        response(json!({"total_count":2,"check_runs":[{"id":5}]})),
    ]);
    assert_eq!(
        github(wire).checks(&repo(), HEAD).await.unwrap_err().code,
        "provider_pagination"
    );
}
#[tokio::test]
async fn configured_account_and_repository_transfer_are_checked_before_reads() {
    let wire = Wire::fixture(vec![response(json!({"id":8,"login":"other"}))]);
    assert_eq!(
        github(wire.clone())
            .pull_requests(&repo())
            .await
            .unwrap_err()
            .code,
        "identity_mismatch"
    );
    assert_eq!(wire.len(), 1);
    let mut transferred: Value = serde_json::from_slice(&gh_repo().unwrap().body).unwrap();
    transferred["owner"]["id"] = json!(8);
    let wire = Wire::fixture(vec![gh_identity(), response(transferred)]);
    assert_eq!(
        github(wire.clone())
            .pull_requests(&repo())
            .await
            .unwrap_err()
            .code,
        "identity_mismatch"
    );
    assert_eq!(wire.len(), 2);
}
fn control_fixture() -> (tempfile::TempDir, Arc<ControlHost>, Uuid) {
    let temp = tempfile::tempdir().unwrap();
    let runtime = Uuid::new_v4();
    let private = PrivateDir::create(&temp.path().canonicalize().unwrap().join("state")).unwrap();
    let host = Arc::new(ControlHost::setup(private, runtime, Uuid::new_v4()).unwrap());
    (temp, host, runtime)
}
fn request(runtime: Uuid, request_id: Uuid) -> MutationRequest {
    MutationRequest {
        runtime_id: runtime,
        request_id,
        actor: Actor {
            identity: "fixture-operator".into(),
            roles: BTreeSet::from(["provider".into()]),
        },
        kind: "ignored".into(),
        canonical_targets: vec!["ignored".into()],
        expected: BTreeMap::new(),
        payload: json!({"ignored":true}),
    }
}
fn status() -> ProviderMutation {
    ProviderMutation::GithubStatus {
        repository: repo(),
        head: HEAD.into(),
        state: StatusState::Success,
        description: "Reviewed exact delta".into(),
        target_url: "https://github.invalid/fixture/repo/pull/1".into(),
    }
}
fn commit_response() -> Result<HttpResponse> {
    response(json!({"sha":HEAD,"tree":{"sha":BASE},"parents":[{"sha":BASE}]}))
}
#[tokio::test]
async fn broker_posts_exact_status_once_and_replay_binds_actual_command() {
    let wire = Wire::fixture(vec![
        gh_identity(),
        gh_repo(),
        commit_response(),
        response(json!({"id":90})),
    ]);
    let broker = ProviderBroker {
        linear: None,
        github: Some(github(wire.clone())),
    };
    let (_temp, host, runtime) = control_fixture();
    let operation = Uuid::new_v4();
    let receipt = broker
        .mutate(
            host.clone(),
            request(runtime, operation),
            None,
            Some("provider".into()),
            status(),
            || Ok(()),
        )
        .await
        .unwrap();
    assert_eq!(receipt.phase, ReceiptPhase::Acknowledged);
    assert_eq!(receipt.kind, "github_status");
    assert!(receipt.canonical_targets[0].contains(":42"));
    assert_eq!(wire.len(), 4);
    assert_eq!(
        wire.body(3),
        json!({"state":"success","context":"kegari/codex-review","description":"Reviewed exact delta","target_url":"https://github.invalid/fixture/repo/pull/1"})
    );
    assert_eq!(
        wire.requests.lock().unwrap()[3].url.path(),
        format!("/api/v3/repos/fixture/repo/statuses/{HEAD}")
    );
    broker
        .mutate(
            host.clone(),
            request(runtime, operation),
            None,
            Some("provider".into()),
            status(),
            || Err(Error::new("must_not_run", "replay")),
        )
        .await
        .unwrap();
    assert_eq!(wire.len(), 4);
    let mut changed = status();
    if let ProviderMutation::GithubStatus { description, .. } = &mut changed {
        *description = "different payload".into();
    }
    assert_eq!(
        broker
            .mutate(
                host,
                request(runtime, operation),
                None,
                Some("provider".into()),
                changed,
                || Ok(())
            )
            .await
            .unwrap_err()
            .code,
        "request_conflict"
    );
    assert_eq!(wire.len(), 4);
}
#[tokio::test]
async fn rejected_fresh_gate_never_reads_or_writes_and_unknown_write_holds_new_requests() {
    let wire = Wire::fixture(vec![]);
    let broker = ProviderBroker {
        linear: None,
        github: Some(github(wire.clone())),
    };
    let (_temp, host, runtime) = control_fixture();
    assert_eq!(
        broker
            .mutate(
                host,
                request(runtime, Uuid::new_v4()),
                None,
                Some("provider".into()),
                status(),
                || Err(Error::new("stale_generation", "correction"))
            )
            .await
            .unwrap_err()
            .code,
        "stale_generation"
    );
    assert_eq!(wire.len(), 0);
    let wire = Wire::fixture(vec![
        gh_identity(),
        gh_repo(),
        commit_response(),
        Err(Error::new("provider_transport", "response lost")),
    ]);
    let broker = ProviderBroker {
        linear: None,
        github: Some(github(wire.clone())),
    };
    let (_temp, host, runtime) = control_fixture();
    let operation = Uuid::new_v4();
    assert_eq!(
        broker
            .mutate(
                host.clone(),
                request(runtime, operation),
                None,
                Some("provider".into()),
                status(),
                || Ok(())
            )
            .await
            .unwrap_err()
            .code,
        "operation_uncertain"
    );
    assert_eq!(host.operation(operation).unwrap().phase, ReceiptPhase::Sent);
    assert_eq!(
        broker
            .mutate(
                host,
                request(runtime, Uuid::new_v4()),
                None,
                Some("provider".into()),
                status(),
                || Ok(())
            )
            .await
            .unwrap_err()
            .code,
        "reconciliation_required"
    );
    assert_eq!(wire.len(), 4);
}
#[test]
fn credentials_and_debug_values_never_disclose_secrets() {
    assert!(!format!("{:?}", Credential::new("secret".into()).unwrap()).contains("\"secret\""));
    assert!(!format!("{:?}", status()).contains("Reviewed"));
    for endpoint in [
        "http://github.invalid",
        "https://user:pass@github.invalid",
        "https://github.invalid?token=value",
        "https://github.invalid/#fragment",
    ] {
        assert!(HttpPolicy::new(endpoint, "7".into(), Duration::from_secs(1)).is_err());
    }
}

// Generated disposable offline test key; never used as a production trust root.
const FIXTURE_KEY: &str = "MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQDFTEX6ChIN+tc0MHn6ey3eu77bUMu7G62F77UjloREcMWQSyO2wSvUf6lXqZ3j7QLOgOjIEUrSpalLDe9uTDnHsJ4qe+QfFiGDdFIVtsTpFGRAtJu0SONWAWZeSVnQe79ssv9UXGRHCQwgvLbUHCtDp6fFZTEnNnuAWknStUhNHJjQ5pog5V1aFTzvJWDVy8/tYTrLY0WJtB8j3wxaATRnZKh6LQvaiqPKvjuMpqjOuPBWL7Sfw6EtsIUsbdKN3JCwwaVBw8SUtxA+pqCUohOPO7rJUvAAk+WHI6AOIrYvgOzX8WBFpc/FwucT/7CnvDOS3QRBTLRRn1ifeU41K9pzAgMBAAECggEATn7MEHDVQg6cdD0dKpS7aTRU+LiqdaTLYtA9xwzpTrl3QLNEYD92irLH0wK+uKHhTftnocGPfUM/mYbPzZgyHMqeAtULq3qIlt4+kjkHGpxsTnoMwKuHFv6zlaulOhchz8NfZlbj5oco9hVewAD9DYu0agqahbldZXGGkXhibZrR+JX1qLn6zMYxdRjAgRwBvC4YmRKi5CVKanetswwheOblGEXJMfl+9PbCJpG4DiDuPe4q8S7iczCVMnrBrzgLtor+Zzgue5uc48nR9QuuE1t5u5ioP3E04YCAsLj31lOnH2ycrVufGalY0tFujDlpkYYiuQi3s7vAn2MEqerA0QKBgQDsoGOn9Pm6yL2Q5GQQEJlbYayGCWgUuT59JdFzMUkA7G8bPDXU+3Y5+1JtNp+1itV9hAYwao4iGtBXBEVpEwUNZkyxO9wJvXsZOHAZL2RFCZlcQSVpacFXcm+/JNPuDGIlvJT1WycEKbq5/kURYY6OfN9zLP+1AEuRag8p33D+XQKBgQDVc5IQSX8W7P7ar3umP74zCWt+bpK+swrYK/FH6oNVskp/uIIHh8T5kF8SFppQwXQlSbRlcabNWAFhtjBD1kqSuCOSExNTW+CXf7xfi3AVp5IFZEdUW7PcMAXg3Cyg6RmAP4OmdDPbt+lIg+0TCMGHXebnY7dINCIdOYClD+OPDwKBgB0MD0+13dYVAFhCARqLvxedpN0LWJB6sPYR52U5npmTbIwuVvlfKQMzG8o2U2KouU5gH5Zf7jS3j+2uo4qjgBJGBZIYJAm6DdtptzEQRUHITOArSxlmlwpSDO+s6KsnMAEr5ik8WEzFhpuAYCAjFqndIFz2ICmq8JvYJiZgvHEtAoGAElmHGwNWe6AQILipRpeAQF708L+MufwmWIMekPImLD3Rjb1M4FzZoCK7KZDr3zliRLpbWcU1c723ELFa4NoROx+9Xxdot+fUHPIMPaJ9Fj97nB8MysG/apRuTPBgncysNow/QxYGR0M+OM+ul85O5NY7vXSjEv/4Vs7Uslv1Uh0CgYEAiEa8RJbaWjOi3vxlhsjVhlUBNR6LK7s8oQYjutn0oeJLZn4kSrYEtkhXjkjkntvY+5BvhAq5ZVCXiGhS5W+gw19cdVSl6yN33QNbBluq3Lj/ZLJYbOwtLr2PgAp+Hu8GktAOhRuMI4YoOkoS5xK9ZjKvS4BnJCnm0wF+7c1+08k=";
const FIXTURE_MODULUS: &str = "xUxF-goSDfrXNDB5-nst3ru-21DLuxuthe-1I5aERHDFkEsjtsEr1H-pV6md4-0CzoDoyBFK0qWpSw3vbkw5x7CeKnvkHxYhg3RSFbbE6RRkQLSbtEjjVgFmXklZ0Hu_bLL_VFxkRwkMILy21BwrQ6enxWUxJzZ7gFpJ0rVITRyY0OaaIOVdWhU87yVg1cvP7WE6y2NFibQfI98MWgE0Z2Soei0L2oqjyr47jKaozrjwVi-0n8OhLbCFLG3SjdyQsMGlQcPElLcQPqaglKITjzu6yVLwAJPlhyOgDiK2L4Ds1_FgRaXPxcLnE_-wp7wzkt0EQUy0UZ9Yn3lONSvacw";
fn expectation() -> HostedProofExpectation {
    HostedProofExpectation {
        runtime_id: id(100),
        operation_id: id(101),
        challenge: id(102),
        repository_id: 42,
        repository_owner_id: 7,
        run_id: 50,
        run_attempt: 1,
        check_run_id: 60,
        actor_id: 7,
        trigger_sha: HEAD.into(),
        tested_source_sha: BASE.into(),
        workflow_ref: "fixture/repo/.github/workflows/validate.yml@refs/heads/validation"
            .to_string(),
        workflow_sha: HEAD.into(),
        job_workflow_ref: format!("fixture/trusted/.github/workflows/proof.yml@{HEAD}"),
        job_workflow_sha: HEAD.into(),
        event_name: "workflow_dispatch".into(),
    }
}
fn proof_claims(e: &HostedProofExpectation) -> Value {
    json!({"iss":GITHUB_OIDC_ISSUER,"aud":e.audience().unwrap(),"sub":"repo:fixture/repo:ref:refs/heads/validation","jti":"fixture-token-1","iat":1800000000,"nbf":1799999970,"exp":1800000300,"repository_id":e.repository_id.to_string(),"repository_owner_id":e.repository_owner_id.to_string(),"run_id":e.run_id.to_string(),"run_attempt":e.run_attempt.to_string(),"check_run_id":e.check_run_id.to_string(),"actor_id":e.actor_id.to_string(),"sha":e.trigger_sha,"workflow_ref":e.workflow_ref,"workflow_sha":e.workflow_sha,"job_workflow_ref":e.job_workflow_ref,"job_workflow_sha":e.job_workflow_sha,"runner_environment":"github-hosted","event_name":e.event_name})
}
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
#[test]
fn signed_hosted_proof_is_exact_job_bound_and_durably_replay_owned() {
    let temp = tempfile::tempdir().unwrap();
    let dir = PrivateDir::create(&temp.path().canonicalize().unwrap().join("proofs")).unwrap();
    let e = expectation();
    let jwt = token(proof_claims(&e));
    let now = chrono::DateTime::from_timestamp(1800000001, 0).unwrap();
    let verifier = verifier();
    let verified = verifier.verify_and_record(&jwt, &e, now, &dir).unwrap();
    assert_eq!(verified.expectation, e);
    assert_eq!(verified.token_digest, bytes_digest(jwt.as_bytes()));
    assert_eq!(
        verifier
            .verify_and_record(&jwt, &e, now + chrono::Duration::seconds(1), &dir)
            .unwrap(),
        verified
    );
    let records = dir.records().unwrap();
    let record = std::fs::read_to_string(dir.path().join(&records[0])).unwrap();
    assert!(!record.contains(&jwt));
    assert!(!record.contains("fixture-token-1"));
    let mut other = e.clone();
    other.operation_id = id(999);
    let different = token(proof_claims(&other));
    assert_eq!(
        verifier
            .verify_and_record(&different, &other, now, &dir)
            .unwrap_err()
            .code,
        "proof_replay"
    );
}
#[test]
fn hosted_proof_rejects_self_hosted_sibling_jobs_wrong_revision_audience_and_time() {
    let temp = tempfile::tempdir().unwrap();
    let dir = PrivateDir::create(&temp.path().canonicalize().unwrap().join("proofs")).unwrap();
    let e = expectation();
    let now = chrono::DateTime::from_timestamp(1800000001, 0).unwrap();
    let verifier = verifier();
    for (field, value) in [
        ("runner_environment", json!("self-hosted")),
        ("check_run_id", json!("61")),
        ("run_attempt", json!("2")),
        ("repository_id", json!("43")),
        ("actor_id", json!("8")),
        ("sha", json!(BASE)),
        ("workflow_sha", json!(BASE)),
        ("job_workflow_sha", json!(BASE)),
        ("aud", json!("https://github.com/fixture")),
        ("iss", json!("https://evil.invalid")),
        ("nbf", json!(1800000900)),
        ("exp", json!(1799999900)),
        ("iat", json!(1799999000)),
    ] {
        let mut claims = proof_claims(&e);
        claims[field] = value;
        assert!(
            verifier
                .verify_and_record(&token(claims), &e, now, &dir)
                .is_err(),
            "accepted changed {field}"
        );
    }
    assert!(dir.records().unwrap().is_empty());
    let valid = token(proof_claims(&e));
    let mut parts = valid.split('.').map(str::to_owned).collect::<Vec<_>>();
    parts[1] = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&proof_claims(&e))
            .unwrap()
            .into_iter()
            .chain(*b" ")
            .collect::<Vec<_>>(),
    );
    assert!(
        verifier
            .verify_and_record(&parts.join("."), &e, now, &dir)
            .is_err()
    );
}
#[tokio::test]
async fn oidc_fetch_uses_only_fixed_https_issuer_without_provider_credentials() {
    let wire = Wire::fixture(vec![response(
        json!({"keys":[{"kid":"fixture-key","alg":"RS256","kty":"RSA","use":"sig","n":FIXTURE_MODULUS,"e":"AQAB"}]}),
    )]);
    HostedProofVerifier::fetch_with_transport(Duration::from_secs(1), wire.as_ref())
        .await
        .unwrap();
    let requests = wire.requests.lock().unwrap();
    assert_eq!(requests[0].url.as_str(), GITHUB_OIDC_JWKS);
    assert!(!requests[0].headers.contains_key("authorization"));
    assert_eq!(requests[0].method, Method::Get);
}
fn run(attempt: u64) -> Value {
    json!({"id":50,"repository":{"id":42},"run_attempt":attempt,"workflow_id":9,"actor":{"id":7},"triggering_actor":{"id":7},"event":"workflow_dispatch","head_sha":HEAD,"status":"in_progress","conclusion":null,"path":".github/workflows/validate.yml","html_url":"https://github.invalid/fixture/repo/actions/runs/50"})
}
#[tokio::test]
async fn dispatch_is_revision_checked_and_cancel_requires_exact_owned_attempt() {
    let wire = Wire::fixture(vec![
        gh_identity(),
        gh_repo(),
        response(json!({"ref":"refs/heads/validation","object":{"type":"commit","sha":HEAD}})),
        gh_identity(),
        gh_repo(),
        response(
            json!({"total_count":1,"workflows":[{"id":9,"path":".github/workflows/validate.yml","state":"active"}]}),
        ),
        response(json!({"workflow_run_id":50})),
        gh_identity(),
        gh_repo(),
        response(run(2)),
    ]);
    let broker = ProviderBroker {
        linear: None,
        github: Some(github(wire.clone())),
    };
    let (_temp, host, runtime) = control_fixture();
    let dispatch = Uuid::new_v4();
    let command = ProviderMutation::GithubDispatch {
        repository: repo(),
        workflow_id: 9,
        workflow_path: ".github/workflows/validate.yml".into(),
        reference: "heads/validation".into(),
        workflow_sha: HEAD.into(),
        source_sha: BASE.into(),
        source_input: "source_sha".into(),
        operation_input: "operation_id".into(),
        inputs: BTreeMap::from([("suite".into(), "full".into())]),
    };
    let receipt = broker
        .mutate(
            host.clone(),
            request(runtime, dispatch),
            None,
            Some("provider".into()),
            command,
            || Ok(()),
        )
        .await
        .unwrap();
    assert_eq!(receipt.remote_ids["workflow_run_id"], "50");
    assert_eq!(
        wire.body(6),
        json!({"ref":"validation","return_run_details":true,"inputs":{"suite":"full","source_sha":BASE,"operation_id":dispatch}})
    );
    let cancel = ProviderMutation::GithubCancel {
        repository: repo(),
        run_id: 50,
        attempt: 1,
        workflow_id: 9,
        head: HEAD.into(),
        dispatch_operation: dispatch,
    };
    assert_eq!(
        broker
            .mutate(
                host,
                request(runtime, Uuid::new_v4()),
                None,
                Some("provider".into()),
                cancel,
                || Ok(())
            )
            .await
            .unwrap_err()
            .code,
        "scope_conflict"
    );
    assert_eq!(wire.len(), 10);
}
#[tokio::test]
async fn runner_labels_never_establish_hosted_execution() {
    let wire = Wire::fixture(vec![
        gh_identity(),
        gh_repo(),
        response(run(1)),
        response(
            json!({"total_count":1,"jobs":[{"id":70,"run_id":50,"run_attempt":1,"name":"test","head_sha":HEAD,"status":"completed","conclusion":"success","labels":["ubuntu-latest","github-hosted"],"runner_name":"GitHub Actions 1","check_run_url":"https://github.invalid/api/v3/repos/fixture/repo/check-runs/60","steps":[{"name":"Test","status":"completed","conclusion":"success","number":1}]}]}),
        ),
    ]);
    let jobs = github(wire).jobs(&repo(), 50, 1).await.unwrap();
    assert_eq!(jobs[0].environment, RunnerEnvironment::Unknown);
    assert_eq!(jobs[0].check_run_id, 60);
}
#[tokio::test]
async fn linear_signed_upload_has_separate_receipts_and_never_forwards_account_secret() {
    let asset = "https://uploads.linear.invalid/asset/recording.mp4";
    let bytes = b"fixture-mp4-file".to_vec();
    let grant = json!({"data":{"fileUpload":{"success":true,"uploadFile":{"uploadUrl":"https://blob.invalid/upload?signature=private-grant","assetUrl":asset,"headers":[{"key":"x-upload-proof","value":"signed-header"}],"contentType":"video/mp4","filename":"recording.mp4","size":bytes.len()}}}});
    let issue_read = || response(json!({"data":{"issue":issue(20)}}));
    let wire = Wire::fixture(vec![
        linear_identity(),
        issue_read(),
        labels(),
        response(grant),
        linear_identity(),
        issue_read(),
        labels(),
        Ok(HttpResponse {
            status: 200,
            headers: HeaderMap::new(),
            body: Vec::new(),
        }),
    ]);
    let broker = ProviderBroker {
        linear: Some(linear(wire.clone())),
        github: None,
    };
    let (temp, host, runtime) = control_fixture();
    let operation = Uuid::new_v4();
    let receipt = broker
        .mutate(
            host.clone(),
            request(runtime, operation),
            None,
            Some("provider".into()),
            ProviderMutation::LinearUploadRequest {
                issue: id(20),
                filename: "recording.mp4".into(),
                content_type: "video/mp4".into(),
                size: bytes.len() as u32,
            },
            || Ok(()),
        )
        .await
        .unwrap();
    assert_eq!(receipt.observed_result.unwrap()["uploaded"], false);
    let uploaded = broker
        .mutate(
            host,
            request(runtime, Uuid::new_v4()),
            None,
            Some("provider".into()),
            ProviderMutation::LinearUploadBytes {
                issue: id(20),
                grant_operation: operation,
                asset_url: asset.into(),
                content_digest: bytes_digest(&bytes),
                bytes: bytes.clone(),
            },
            || Ok(()),
        )
        .await
        .unwrap();
    assert_eq!(uploaded.observed_result.unwrap()["associated"], false);
    let requests = wire.requests.lock().unwrap();
    assert_eq!(requests[7].method, Method::Put);
    assert_eq!(requests[7].body, bytes);
    assert!(!requests[7].headers.contains_key("authorization"));
    assert_eq!(requests[7].headers["x-upload-proof"], "signed-header");
    assert_eq!(requests[7].headers["content-type"], "video/mp4");
    drop(requests);
    for entry in std::fs::read_dir(temp.path().join("state/receipts")).unwrap() {
        let entry = entry.unwrap();
        if entry.path().extension().is_some_and(|s| s == "json") {
            let text = std::fs::read_to_string(entry.path()).unwrap();
            assert!(!text.contains("private-grant"));
            assert!(!text.contains("signed-header"));
            assert!(!text.contains("test-linear-secret"));
        }
    }
}
#[tokio::test]
async fn artifact_download_uses_only_provider_grant_and_checks_archive_digest() {
    let bytes = b"fixture-archive-with-proof".to_vec();
    let mut headers = HeaderMap::new();
    headers.insert(
        "location",
        HeaderValue::from_static("https://artifact.blob.invalid/archive?signature=secret"),
    );
    let wire = Wire::fixture(vec![
        gh_identity(),
        gh_repo(),
        response(
            json!({"id":80,"workflow_run":{"id":50,"repository_id":42,"head_sha":HEAD},"expired":false,"size_in_bytes":bytes.len(),"digest":bytes_digest(&bytes)}),
        ),
        Ok(HttpResponse {
            status: 302,
            headers,
            body: Vec::new(),
        }),
        Ok(HttpResponse {
            status: 200,
            headers: HeaderMap::new(),
            body: bytes.clone(),
        }),
    ]);
    let artifact = github(wire.clone())
        .artifact_archive(&repo(), 80, 50, HEAD)
        .await
        .unwrap();
    assert_eq!(artifact.bytes, bytes);
    let requests = wire.requests.lock().unwrap();
    assert!(requests[3].headers.contains_key("authorization"));
    assert!(!requests[4].headers.contains_key("authorization"));
    assert_eq!(requests[4].url.host_str(), Some("artifact.blob.invalid"));
    assert!(!format!("{artifact:?}").contains("fixture-archive"));
}
