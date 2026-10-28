//! Production provider algorithms with an injected wire boundary. No request in
//! these tests contacts Linear, GitHub, or a live agent.
use base64::Engine;
use kegari_config::{GithubSelection, LinearSelection, Selector};
use kegari_digest::bytes_digest;
use kegari_providers::*;
use kegari_types::error::{Error, Result};
use kegari_v0::{
    control::{Actor, ControlHost, MutationRequest, ReceiptPhase},
    providers::{
        GITHUB_OIDC_ISSUER, GITHUB_OIDC_JWKS, HostedProofExpectation, HostedProofVerifier,
        LinearAssetReference, ProviderBroker, ProviderMutation, download_uploaded_asset,
    },
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
    response(json!({"id":7,"login":"fixture","type":"User"}))
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
fn github(wire: Arc<dyn HttpTransport>) -> GithubClient {
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
            principal: None,
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
        target_url: "https://github.invalid/fixture/repo/pull/1#issuecomment-90".into(),
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
        json!({"state":"success","context":"kegari/codex-review","description":"Reviewed exact delta","target_url":"https://github.invalid/fixture/repo/pull/1#issuecomment-90"})
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
async fn commit_publication_binds_explicit_metadata_and_exact_validated_sha() {
    for returned in [HEAD, BASE] {
        let wire = Wire::fixture(vec![
            gh_identity(),
            gh_repo(),
            response(json!({"sha":BASE,"truncated":false,"tree":[]})),
            gh_identity(),
            gh_repo(),
            commit_response(),
            response(json!({"sha":returned})),
        ]);
        let broker = ProviderBroker {
            linear: None,
            github: Some(github(wire.clone())),
        };
        let (_temp, host, runtime) = control_fixture();
        let operation = Uuid::new_v4();
        let identity = CommitIdentity {
            name: "Fixture author".into(),
            email: "fixture@example.invalid".into(),
            date: "2026-01-01T00:00:00Z".into(),
        };
        let outcome = broker
            .mutate(
                host.clone(),
                request(runtime, operation),
                None,
                Some("provider".into()),
                ProviderMutation::GithubCommit {
                    repository: repo(),
                    message: "Validated commit".into(),
                    tree: BASE.into(),
                    parent: HEAD.into(),
                    author: identity.clone(),
                    committer: identity,
                    expected_sha: HEAD.into(),
                },
                || Ok(()),
            )
            .await;
        assert_eq!(wire.body(6)["author"]["date"], "2026-01-01T00:00:00Z");
        assert_eq!(wire.body(6)["committer"]["name"], "Fixture author");
        if returned == HEAD {
            assert!(outcome.is_ok());
        } else {
            assert_eq!(outcome.unwrap_err().code, "operation_uncertain");
            assert_eq!(host.operation(operation).unwrap().phase, ReceiptPhase::Sent);
        }
    }
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
fn provider_mutation_debug_values_never_disclose_payloads() {
    assert!(!format!("{:?}", status()).contains("Reviewed"));
}

// Generated disposable offline test key; never used as a production trust root.
const FIXTURE_KEY: &str = "MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQDFTEX6ChIN+tc0MHn6ey3eu77bUMu7G62F77UjloREcMWQSyO2wSvUf6lXqZ3j7QLOgOjIEUrSpalLDe9uTDnHsJ4qe+QfFiGDdFIVtsTpFGRAtJu0SONWAWZeSVnQe79ssv9UXGRHCQwgvLbUHCtDp6fFZTEnNnuAWknStUhNHJjQ5pog5V1aFTzvJWDVy8/tYTrLY0WJtB8j3wxaATRnZKh6LQvaiqPKvjuMpqjOuPBWL7Sfw6EtsIUsbdKN3JCwwaVBw8SUtxA+pqCUohOPO7rJUvAAk+WHI6AOIrYvgOzX8WBFpc/FwucT/7CnvDOS3QRBTLRRn1ifeU41K9pzAgMBAAECggEATn7MEHDVQg6cdD0dKpS7aTRU+LiqdaTLYtA9xwzpTrl3QLNEYD92irLH0wK+uKHhTftnocGPfUM/mYbPzZgyHMqeAtULq3qIlt4+kjkHGpxsTnoMwKuHFv6zlaulOhchz8NfZlbj5oco9hVewAD9DYu0agqahbldZXGGkXhibZrR+JX1qLn6zMYxdRjAgRwBvC4YmRKi5CVKanetswwheOblGEXJMfl+9PbCJpG4DiDuPe4q8S7iczCVMnrBrzgLtor+Zzgue5uc48nR9QuuE1t5u5ioP3E04YCAsLj31lOnH2ycrVufGalY0tFujDlpkYYiuQi3s7vAn2MEqerA0QKBgQDsoGOn9Pm6yL2Q5GQQEJlbYayGCWgUuT59JdFzMUkA7G8bPDXU+3Y5+1JtNp+1itV9hAYwao4iGtBXBEVpEwUNZkyxO9wJvXsZOHAZL2RFCZlcQSVpacFXcm+/JNPuDGIlvJT1WycEKbq5/kURYY6OfN9zLP+1AEuRag8p33D+XQKBgQDVc5IQSX8W7P7ar3umP74zCWt+bpK+swrYK/FH6oNVskp/uIIHh8T5kF8SFppQwXQlSbRlcabNWAFhtjBD1kqSuCOSExNTW+CXf7xfi3AVp5IFZEdUW7PcMAXg3Cyg6RmAP4OmdDPbt+lIg+0TCMGHXebnY7dINCIdOYClD+OPDwKBgB0MD0+13dYVAFhCARqLvxedpN0LWJB6sPYR52U5npmTbIwuVvlfKQMzG8o2U2KouU5gH5Zf7jS3j+2uo4qjgBJGBZIYJAm6DdtptzEQRUHITOArSxlmlwpSDO+s6KsnMAEr5ik8WEzFhpuAYCAjFqndIFz2ICmq8JvYJiZgvHEtAoGAElmHGwNWe6AQILipRpeAQF708L+MufwmWIMekPImLD3Rjb1M4FzZoCK7KZDr3zliRLpbWcU1c723ELFa4NoROx+9Xxdot+fUHPIMPaJ9Fj97nB8MysG/apRuTPBgncysNow/QxYGR0M+OM+ul85O5NY7vXSjEv/4Vs7Uslv1Uh0CgYEAiEa8RJbaWjOi3vxlhsjVhlUBNR6LK7s8oQYjutn0oeJLZn4kSrYEtkhXjkjkntvY+5BvhAq5ZVCXiGhS5W+gw19cdVSl6yN33QNbBluq3Lj/ZLJYbOwtLr2PgAp+Hu8GktAOhRuMI4YoOkoS5xK9ZjKvS4BnJCnm0wF+7c1+08k=";
const FIXTURE_MODULUS: &str = "xUxF-goSDfrXNDB5-nst3ru-21DLuxuthe-1I5aERHDFkEsjtsEr1H-pV6md4-0CzoDoyBFK0qWpSw3vbkw5x7CeKnvkHxYhg3RSFbbE6RRkQLSbtEjjVgFmXklZ0Hu_bLL_VFxkRwkMILy21BwrQ6enxWUxJzZ7gFpJ0rVITRyY0OaaIOVdWhU87yVg1cvP7WE6y2NFibQfI98MWgE0Z2Soei0L2oqjyr47jKaozrjwVi-0n8OhLbCFLG3SjdyQsMGlQcPElLcQPqaglKITjzu6yVLwAJPlhyOgDiK2L4Ds1_FgRaXPxcLnE_-wp7wzkt0EQUy0UZ9Yn3lONSvacw";
fn expectation() -> HostedProofExpectation {
    HostedProofExpectation {
        context_digest: None,
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
        expected_production: None,
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

struct StalledWire {
    inner: Arc<Wire>,
    call: usize,
    entered: tokio::sync::Semaphore,
    released: tokio::sync::Semaphore,
}
impl HttpTransport for StalledWire {
    fn send(
        &self,
        request: HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Result<HttpResponse>> + Send + '_>> {
        Box::pin(async move {
            let response = self.inner.send(request).await;
            if self.inner.len() == self.call {
                self.entered.add_permits(1);
                self.released.acquire().await.unwrap().forget();
            }
            response
        })
    }
}
async fn responsive_snapshot(host: Arc<ControlHost>) -> kegari_v0::control::ControlSnapshot {
    tokio::time::timeout(
        Duration::from_secs(2),
        tokio::task::spawn_blocking(move || host.snapshot()),
    )
    .await
    .expect("network wait held control lock")
    .unwrap()
    .unwrap()
}
#[tokio::test]
async fn provider_preflight_keeps_control_responsive_and_rechecks_revocation_before_send() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let wire = Arc::new(StalledWire {
        inner: Wire::fixture(vec![gh_identity(), gh_repo(), commit_response()]),
        call: 1,
        entered: tokio::sync::Semaphore::new(0),
        released: tokio::sync::Semaphore::new(0),
    });
    let broker = ProviderBroker {
        linear: None,
        github: Some(github(wire.clone())),
    };
    let (_temp, host, runtime) = control_fixture();
    let operation = Uuid::new_v4();
    let allowed = Arc::new(AtomicBool::new(true));
    let owned_host = host.clone();
    let owned_allowed = allowed.clone();
    let task = tokio::spawn(async move {
        broker
            .mutate(
                owned_host,
                request(runtime, operation),
                None,
                Some("provider".into()),
                status(),
                move || {
                    if owned_allowed.load(Ordering::SeqCst) {
                        Ok(())
                    } else {
                        Err(Error::new("access_denied", "fixture authority revoked"))
                    }
                },
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), wire.entered.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    let snapshot = responsive_snapshot(host.clone()).await;
    assert_eq!(snapshot.pending_external_effects, vec![operation]);
    assert_eq!(
        host.operation(operation).unwrap().phase,
        ReceiptPhase::Prepared
    );
    host.serialize_observation(|_| {
        allowed.store(false, Ordering::SeqCst);
        Ok(())
    })
    .unwrap();
    wire.released.add_permits(1);
    assert_eq!(task.await.unwrap().unwrap_err().code, "access_denied");
    assert_eq!(wire.inner.len(), 3);
    assert!(
        wire.inner
            .requests
            .lock()
            .unwrap()
            .iter()
            .all(|request| request.method == Method::Get)
    );
    let receipt = host.operation(operation).unwrap();
    assert_eq!(receipt.phase, ReceiptPhase::Reconciled);
    assert_eq!(receipt.observed_result.unwrap()["effect"], "not_sent");
    assert!(host.snapshot().unwrap().pending_external_effects.is_empty());
    assert!(
        !host
            .snapshot()
            .unwrap()
            .unresolved_operations
            .contains_key(&operation)
    );
}
#[tokio::test]
async fn provider_response_survives_caller_abort_replay_and_refuses_premature_recovery() {
    use kegari_v0::control::Reconciliation;
    let wire = Arc::new(StalledWire {
        inner: Wire::fixture(vec![
            gh_identity(),
            gh_repo(),
            commit_response(),
            response(json!({"id":90})),
        ]),
        call: 4,
        entered: tokio::sync::Semaphore::new(0),
        released: tokio::sync::Semaphore::new(0),
    });
    let broker = ProviderBroker {
        linear: None,
        github: Some(github(wire.clone())),
    };
    let (_temp, host, runtime) = control_fixture();
    let operation = Uuid::new_v4();
    let owned_host = host.clone();
    let owned_broker = broker.clone();
    let caller = tokio::spawn(async move {
        owned_broker
            .mutate(
                owned_host,
                request(runtime, operation),
                None,
                Some("provider".into()),
                status(),
                || Ok(()),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), wire.entered.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    let snapshot = responsive_snapshot(host.clone()).await;
    assert_eq!(snapshot.pending_external_effects, vec![operation]);
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    let replay = broker
        .mutate(
            host.clone(),
            request(runtime, operation),
            None,
            Some("provider".into()),
            status(),
            || panic!("replay must not execute gate"),
        )
        .await
        .unwrap();
    assert_eq!(replay.phase, ReceiptPhase::Sent);
    assert_eq!(wire.inner.len(), 4);
    assert_eq!(
        host.snapshot().unwrap().pending_external_effects,
        vec![operation]
    );
    let mut recover = request(runtime, Uuid::new_v4()).actor;
    recover.roles.insert("recover".into());
    assert_eq!(
        host.reconcile_operation(&recover, operation, |_| Ok(Reconciliation::Incomplete))
            .unwrap_err()
            .code,
        "operation_active"
    );
    let mut stop = request(runtime, Uuid::new_v4());
    stop.actor.roles.insert("lifecycle".into());
    assert_eq!(
        host.stop_service(stop, || panic!("active network task forbids shutdown"))
            .unwrap_err()
            .code,
        "stop_in_progress"
    );
    wire.released.add_permits(1);
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if host.snapshot().unwrap().pending_external_effects.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let receipt = host.operation(operation).unwrap();
    assert_eq!(receipt.phase, ReceiptPhase::Acknowledged);
    assert_eq!(receipt.remote_ids["record_id"], "90");
    assert_eq!(wire.inner.len(), 4);
    assert!(
        !host
            .snapshot()
            .unwrap()
            .unresolved_operations
            .contains_key(&operation)
    );
}

#[test]
fn hosted_audience_binds_optional_target_context_without_accepting_source_only_tokens() {
    let temp = tempfile::tempdir().unwrap();
    let directory =
        PrivateDir::create(&temp.path().canonicalize().unwrap().join("proofs")).unwrap();
    let now = chrono::DateTime::from_timestamp(1800000001, 0).unwrap();
    let plain = expectation();
    let mut target = plain.clone();
    target.context_digest = Some(bytes_digest(
        b"exact target/environment/deployment/source/artifact",
    ));
    assert_ne!(plain.audience().unwrap(), target.audience().unwrap());
    let verifier = verifier();
    assert_eq!(
        verifier
            .verify_and_record(&token(proof_claims(&plain)), &target, now, &directory)
            .unwrap_err()
            .code,
        "invalid_hosted_proof"
    );
    let jwt = token(proof_claims(&target));
    verifier
        .verify_and_record(&jwt, &target, now, &directory)
        .unwrap();
    target.context_digest = Some(bytes_digest(b"another deployed artifact"));
    assert_eq!(
        verifier
            .verify_and_record(&jwt, &target, now, &directory)
            .unwrap_err()
            .code,
        "invalid_hosted_proof"
    );
}

async fn uploaded_video_fixture(
    asset: &str,
    inline: bool,
) -> (
    tempfile::TempDir,
    Arc<ControlHost>,
    LinearClient,
    Arc<Wire>,
    LinearAssetReference,
    Vec<u8>,
    Value,
) {
    let bytes = b"synthetic-video-bytes-not-playback-evidence".to_vec();
    let body = if inline {
        format!("![Kegari validation recording]({asset})\n\nfixture signed publication placeholder")
    } else {
        format!("Incidental URL in text: {asset}")
    };
    let comment = json!({"id":id(51),"body":body,"createdAt":"2026-01-01T00:00:00Z","updatedAt":"2026-01-01T00:00:00Z","editedAt":null,"url":"https://linear.app/comment/fixture","user":{"id":id(1)},"onBehalfOf":null,"botActor":null,"issue":{"id":id(20)}});
    let issue_read = || response(json!({"data":{"issue":issue(20)}}));
    let wire = Wire::fixture(vec![
        linear_identity(),
        issue_read(),
        labels(),
        response(
            json!({"data":{"fileUpload":{"success":true,"uploadFile":{"uploadUrl":"https://blob.invalid/upload?signature=private","assetUrl":asset,"headers":[],"contentType":"video/mp4","filename":"recording.mp4","size":bytes.len()}}}}),
        ),
        linear_identity(),
        issue_read(),
        labels(),
        Ok(HttpResponse {
            status: 200,
            headers: HeaderMap::new(),
            body: vec![],
        }),
        linear_identity(),
        issue_read(),
        labels(),
        response(
            json!({"data":{"commentCreate":{"success":true,"comment":{"id":id(51),"issue":{"id":id(20)},"user":{"id":id(1)}}}}}),
        ),
    ]);
    let client = LinearClient::with_transport(
        HttpPolicy::new(
            "https://api.linear.app/graphql",
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
        Some(wire.clone()),
    )
    .unwrap();
    let broker = ProviderBroker {
        linear: Some(client.clone()),
        github: None,
    };
    let (temp, host, runtime) = control_fixture();
    let reference = LinearAssetReference {
        issue: id(20),
        grant_operation: Uuid::new_v4(),
        upload_operation: Uuid::new_v4(),
        association_operation: Uuid::new_v4(),
    };
    let grant = broker
        .mutate(
            host.clone(),
            request(runtime, reference.grant_operation),
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
    assert_eq!(
        grant.expected["provider_upload_size"],
        bytes.len().to_string()
    );
    assert_eq!(grant.expected["provider_upload_filename"], "recording.mp4");
    assert_eq!(grant.expected["provider_upload_content_type"], "video/mp4");
    let uploaded = broker
        .mutate(
            host.clone(),
            request(runtime, reference.upload_operation),
            None,
            Some("provider".into()),
            ProviderMutation::LinearUploadBytes {
                issue: id(20),
                grant_operation: reference.grant_operation,
                asset_url: asset.into(),
                content_digest: bytes_digest(&bytes),
                bytes: bytes.clone(),
            },
            || Ok(()),
        )
        .await
        .unwrap();
    assert_eq!(
        uploaded.expected["provider_upload_grant"],
        reference.grant_operation.to_string()
    );
    assert_eq!(
        uploaded.expected["provider_content_digest"],
        bytes_digest(&bytes)
    );
    assert_eq!(uploaded.expected["provider_asset_url"], asset);
    assert_eq!(uploaded.expected["provider_writer"], id(1).to_string());
    broker
        .mutate(
            host.clone(),
            request(runtime, reference.association_operation),
            None,
            Some("provider".into()),
            ProviderMutation::LinearComment {
                issue: id(20),
                body,
            },
            || Ok(()),
        )
        .await
        .unwrap();
    (temp, host, client, wire, reference, bytes, comment)
}
fn recording_reads(
    wire: &Wire,
    bytes: Vec<u8>,
    before: Vec<Value>,
    after: Vec<Value>,
    status: u16,
    mime: &str,
) {
    let page = |comments| {
        response(
            json!({"data":{"issue":{"comments":{"nodes":comments,"pageInfo":{"hasNextPage":false,"endCursor":null}}}}}),
        )
    };
    let mut headers = HeaderMap::new();
    headers.insert("content-type", HeaderValue::from_str(mime).unwrap());
    headers.insert(
        "content-length",
        HeaderValue::from_str(&bytes.len().to_string()).unwrap(),
    );
    wire.responses.lock().unwrap().extend([
        linear_identity(),
        response(json!({"data":{"issue":issue(20)}})),
        labels(),
        linear_identity(),
        page(before),
        Ok(HttpResponse {
            status,
            headers,
            body: bytes,
        }),
        linear_identity(),
        page(after),
    ]);
}
#[tokio::test]
async fn private_video_readback_binds_owned_receipts_and_exact_native_association() {
    let asset = "https://uploads.linear.app/fixture/recording.mp4";
    let (_temp, host, client, wire, reference, bytes, comment) =
        uploaded_video_fixture(asset, true).await;
    recording_reads(
        &wire,
        bytes.clone(),
        vec![comment.clone()],
        vec![comment],
        200,
        "video/mp4",
    );
    let proof = download_uploaded_asset(&client, &host, &reference)
        .await
        .unwrap();
    assert_eq!(proof.bytes(), bytes);
    assert_eq!(proof.digest(), bytes_digest(&bytes));
    assert_eq!(proof.comment().id, id(51));
    assert_eq!(proof.issue(), id(20));
    assert_eq!(proof.grant_operation(), reference.grant_operation);
    let requests = wire.requests.lock().unwrap();
    let download = requests.iter().find(|r| r.url.as_str() == asset).unwrap();
    assert_eq!(download.method, Method::Get);
    assert_eq!(download.headers["authorization"], "test-linear-secret");
    assert!(!format!("{proof:?}").contains("synthetic-video-bytes"));
}
#[tokio::test]
async fn asset_readback_rejects_untrusted_hosts_and_non_owned_operation_references_before_get() {
    for asset in [
        "https://uploads.linear.app.evil.invalid/file.mp4",
        "https://uploads.linear.app:8443/file.mp4",
        "https://blob.invalid/file.mp4",
    ] {
        let (_temp, host, client, wire, reference, _, _) =
            uploaded_video_fixture(asset, true).await;
        let before = wire.len();
        assert!(
            download_uploaded_asset(&client, &host, &reference)
                .await
                .is_err(),
            "{asset}"
        );
        assert_eq!(wire.len(), before);
    }
    let (_temp, host, client, wire, mut reference, _, _) =
        uploaded_video_fixture("https://uploads.linear.app/fixture/file.mp4", true).await;
    reference.upload_operation = reference.grant_operation;
    let before = wire.len();
    assert!(
        download_uploaded_asset(&client, &host, &reference)
            .await
            .is_err()
    );
    assert_eq!(wire.len(), before);
}
#[tokio::test]
async fn asset_readback_rejects_missing_edited_duplicated_association_and_wrong_bytes() {
    for variant in [
        "missing",
        "edited",
        "duplicate",
        "incidental",
        "later_edit",
        "bytes",
        "mime",
        "redirect",
        "writer",
    ] {
        let (_temp, host, client, wire, reference, mut bytes, comment) = uploaded_video_fixture(
            "https://uploads.linear.app/fixture/file.mp4",
            variant != "incidental",
        )
        .await;
        let mut before = vec![comment.clone()];
        let mut after = vec![comment.clone()];
        let mut status = 200;
        let mut mime = "video/mp4";
        match variant {
            "missing" => before.clear(),
            "edited" => before[0]["body"] = json!("edited"),
            "duplicate" => {
                let mut copy = comment.clone();
                copy["id"] = json!(id(52));
                before.push(copy);
            }
            "later_edit" => after[0]["body"] = json!("changed during readback"),
            "bytes" => bytes[0] ^= 1,
            "mime" => mime = "text/html",
            "redirect" => status = 302,
            "writer" => before[0]["user"]["id"] = json!(id(99)),
            "incidental" => {}
            _ => unreachable!(),
        }
        recording_reads(&wire, bytes, before, after, status, mime);
        assert!(
            download_uploaded_asset(&client, &host, &reference)
                .await
                .is_err(),
            "{variant}"
        );
        assert!(
            wire.requests
                .lock()
                .unwrap()
                .iter()
                .all(|r| r.url.host_str() == Some("api.linear.app")
                    || r.url.host_str() == Some("uploads.linear.app")
                    || r.url.host_str() == Some("blob.invalid"))
        );
    }
}

#[test]
fn provider_pull_request_text_bound_stays_aligned_with_instruction_context() {
    assert_eq!(MAX_PULL_REQUEST_TEXT_BYTES, 1_048_576);
    assert_eq!(
        MAX_PULL_REQUEST_TEXT_BYTES,
        kegari_v0::instructions::MAX_CONTEXT_BYTES
    );
}
