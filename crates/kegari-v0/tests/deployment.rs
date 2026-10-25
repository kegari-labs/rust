//! Production adapter tests use only synthetic HTTP and owned private directories.
use base64::Engine as _;
use chrono::{Duration, Utc};
use kegari_v0::{
    authority::{Corrections, IssueKey},
    config::{GithubSelection, Profile, ReleaseMode, Speed},
    control::{Actor, ControlHost, Lease, Lifecycle, MutationRequest, Permit},
    delivery::{DeliveryAdmission, DeliveryPhase, DeliveryScope, RepositoryContract},
    deployment::*,
    digest::bytes_digest,
    error::{Error, Result},
    evidence::{AttestationKey, Generation, RevisionBinding, WorkIdentity},
    git::{RepositoryId, Revision},
    providers::*,
    reconcile::{DependencyOutcomes, GatePurpose, ParentProjection},
    storage::PrivateDir,
};
use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    io::Write,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration as StdDuration,
};
use uuid::Uuid;
fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
fn source() -> Revision {
    Revision {
        commit: "a".repeat(40),
        tree: "b".repeat(40),
    }
}
fn contract(deploy: bool) -> RepositoryContract {
    let pin = json!({"workflow_ref":"fixture/repo/.github/workflows/deploy.yml@refs/heads/trusted","workflow_sha":"1".repeat(40),"job_workflow_ref":format!("fixture/trusted/.github/workflows/proof.yml@{}","1".repeat(40)),"job_workflow_sha":"1".repeat(40),"event_name":"workflow_dispatch"});
    let route = json!({"workflow_id":3,"workflow_revision":"1".repeat(40),"source_input":"source_sha","operation_input":"operation_id","observed_source_output":"source_sha","proof":pin});
    let deployment = if deploy {
        json!({"steps":[{"id":"deploy","target_id":"service-production","environment_id":"88","workflow":route}]})
    } else {
        json!("none")
    };
    RepositoryContract{configuration_name:"42".into(),repository:RepositoryId{hostname:"github.invalid".into(),id:"42".into()},provider:RepositoryIdentity{id:42,owner:"fixture".into(),name:"repo".into(),owner_id:7,organization_id:None},configuration:serde_json::from_value(json!({"hostname":"github.invalid","source":"/private/tmp/source","local_checks":["local"],"integrated_checks":["e2e"],"ci":[{"workflow_id":1,"required_checks":["suite"],"proof":pin}],"production_ci":[{"workflow_id":2,"required_checks":["suite"],"proof":pin}],"no_change_validation":route,"deployment":deployment})).unwrap()}
}
struct State {
    archive: Vec<u8>,
    artifact_name: String,
    requests: Vec<(String, Value)>,
    main: String,
    main_reads: u32,
    move_main: bool,
    status: String,
    deployment_id: u64,
    attempt: u64,
    extra_job: bool,
    environment: u64,
    extra_target: bool,
    fail_last_page: bool,
    older_failure: Option<&'static str>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            archive: vec![],
            artifact_name: String::new(),
            requests: vec![],
            main: "a".repeat(40),
            main_reads: 0,
            move_main: false,
            status: "success".into(),
            deployment_id: 91,
            attempt: 1,
            extra_job: false,
            environment: 88,
            extra_target: false,
            fail_last_page: false,
            older_failure: None,
        }
    }
}
struct Wire(Arc<Mutex<State>>);
impl HttpTransport for Wire {
    fn send(
        &self,
        request: HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Result<HttpResponse>> + Send + '_>> {
        Box::pin(async move {
            let mut f = self.0.lock().unwrap();
            let body = if request.body.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&request.body).unwrap()
            };
            f.requests.push((request.url.to_string(), body));
            let response = |v: Value| {
                Ok(HttpResponse {
                    status: 200,
                    headers: HeaderMap::new(),
                    body: serde_json::to_vec(&v).unwrap(),
                })
            };
            if request.url.host_str() == Some("fixture.blob.invalid") {
                assert!(!request.headers.contains_key("authorization"));
                return Ok(HttpResponse {
                    status: 200,
                    headers: HeaderMap::new(),
                    body: f.archive.clone(),
                });
            }
            let path = request.url.path();
            let env = json!({"id":f.environment,"name":"production","url":"https://github.invalid/repos/fixture/repo/environments/production","protection_rules":[],"deployment_branch_policy":null});
            let run = json!({"id":50,"repository":{"id":42},"run_attempt":f.attempt,"workflow_id":3,"actor":{"id":7},"triggering_actor":{"id":7},"event":"workflow_dispatch","head_sha":"1".repeat(40),"status":"completed","conclusion":"success","path":".github/workflows/deploy.yml","html_url":"https://github.invalid/fixture/repo/actions/runs/50"});
            let deployment = |n: u64| json!({"id":n,"sha":"a".repeat(40),"environment":"production","production_environment":true,"transient_environment":false,"creator":{"id":99},"created_at":"2026-01-03T00:00:00Z","updated_at":"2026-01-03T00:00:00Z"});
            let value = match path {
                "/user" => json!({"id":7,"login":"fixture","type":"User"}),
                "/repos/fixture/repo" => {
                    json!({"id":42,"owner":{"id":7,"login":"fixture","type":"User"},"name":"repo","default_branch":"main","private":true,"archived":false,"disabled":false,"allow_squash_merge":true,"permissions":{"pull":true,"push":true}})
                }
                "/repos/fixture/repo/git/ref/heads/main" => {
                    f.main_reads += 1;
                    if f.move_main && f.main_reads > 1 {
                        f.main = "c".repeat(40)
                    }
                    json!({"ref":"refs/heads/main","object":{"type":"commit","sha":f.main}})
                }
                "/repos/fixture/repo/git/ref/heads/trusted" => {
                    json!({"ref":"refs/heads/trusted","object":{"type":"commit","sha":"1".repeat(40)}})
                }
                p if p.starts_with("/repos/fixture/repo/git/commits/") => {
                    json!({"sha":p.rsplit('/').next().unwrap(),"tree":{"sha":"b".repeat(40)},"parents":[]})
                }
                "/repos/fixture/repo/environments" => json!({"total_count":1,"environments":[env]}),
                "/repos/fixture/repo/environments/production" => env,
                "/repos/fixture/repo/deployments" => {
                    assert!(!request.url.query_pairs().any(|(k, _)| k == "sha"));
                    assert!(
                        request
                            .url
                            .query_pairs()
                            .any(|(k, v)| k == "environment" && v == "production")
                    );
                    if f.fail_last_page {
                        if request
                            .url
                            .query_pairs()
                            .any(|(k, v)| k == "page" && v == "2")
                        {
                            return Err(Error::new(
                                "fixture_read_failure",
                                "last page unavailable",
                            ));
                        }
                        let mut headers = HeaderMap::new();
                        headers.insert("link",HeaderValue::from_static("<https://github.invalid/repos/fixture/repo/deployments?environment=production&per_page=100&page=2>; rel=\"next\""));
                        return Ok(HttpResponse {
                            status: 200,
                            headers,
                            body: serde_json::to_vec(&json!([deployment(f.deployment_id)]))
                                .unwrap(),
                        });
                    }
                    if f.older_failure.is_some() {
                        let mut old = deployment(90);
                        old["created_at"] = json!("2026-01-01T00:00:00Z");
                        json!([deployment(f.deployment_id), old])
                    } else if f.extra_target {
                        json!([deployment(f.deployment_id), deployment(92)])
                    } else {
                        json!([deployment(f.deployment_id)])
                    }
                }
                "/repos/fixture/repo/deployments/90/statuses" => {
                    json!([{"id":94,"state":"failure","environment":"production","creator":{"id":99},"created_at":f.older_failure.unwrap(),"updated_at":f.older_failure.unwrap()}])
                }
                p if p.ends_with("/statuses") => {
                    json!([{"id":93,"state":f.status,"environment":"production","creator":{"id":99},"created_at":"2026-01-04T00:00:00Z","updated_at":"2026-01-04T00:00:00Z"}])
                }
                "/repos/fixture/repo/actions/workflows" => {
                    json!({"total_count":1,"workflows":[{"id":3,"path":".github/workflows/deploy.yml","state":"active"}]})
                }
                "/repos/fixture/repo/actions/workflows/3/dispatches" => {
                    json!({"workflow_run_id":50})
                }
                "/repos/fixture/repo/actions/runs/50"
                | "/repos/fixture/repo/actions/runs/50/attempts/1"
                | "/repos/fixture/repo/actions/runs/50/attempts/2" => run,
                p if p.ends_with("/jobs") => {
                    let job = |id: u64| json!({"id":id,"run_id":50,"run_attempt":f.attempt,"name":"deploy","head_sha":"1".repeat(40),"status":"completed","conclusion":"success","check_run_url":format!("https://github.invalid/repos/fixture/repo/check-runs/{id}"),"steps":[]});
                    let mut jobs = vec![job(60)];
                    if f.extra_job {
                        jobs.push(job(61))
                    }
                    json!({"total_count":jobs.len(),"jobs":jobs})
                }
                "/repos/fixture/repo/actions/runs/50/artifacts" => {
                    json!({"total_count":1,"artifacts":[{"id":80,"name":f.artifact_name}]})
                }
                "/repos/fixture/repo/actions/artifacts/80" => {
                    json!({"id":80,"workflow_run":{"id":50,"repository_id":42,"head_sha":"1".repeat(40)},"expired":false,"size_in_bytes":f.archive.len(),"digest":bytes_digest(&f.archive)})
                }
                "/repos/fixture/repo/actions/artifacts/80/zip" => {
                    let mut headers = HeaderMap::new();
                    headers.insert(
                        "location",
                        HeaderValue::from_static(
                            "https://fixture.blob.invalid/archive?grant=fixture",
                        ),
                    );
                    return Ok(HttpResponse {
                        status: 302,
                        headers,
                        body: vec![],
                    });
                }
                _ => panic!("unexpected fixture request {path}"),
            };
            response(value)
        })
    }
}
struct Fixture {
    _temp: tempfile::TempDir,
    observer: ProductionObserver,
    host: Arc<ControlHost>,
    github: GithubClient,
    state: Arc<Mutex<State>>,
    records: std::path::PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let host = Arc::new(
            ControlHost::setup(
                PrivateDir::create(&base.join("control")).unwrap(),
                id(1),
                id(2),
            )
            .unwrap(),
        );
        let state = Arc::new(Mutex::new(State::default()));
        let github = GithubClient::with_transport(
            HttpPolicy::new(
                "https://github.invalid",
                "7".into(),
                StdDuration::from_secs(2),
            )
            .unwrap(),
            GithubSelection {
                repositories: vec!["42".into()],
                organizations: vec![],
                pr_authors: vec!["7".into()],
            },
            Credential::new("fixture-only".into()).unwrap(),
            Some(Arc::new(Wire(state.clone()))),
        )
        .unwrap();
        let records = base.join("records");
        let observer = ProductionObserver::new(
            github.clone(),
            host.clone(),
            Arc::new(AttestationKey::new("fixture".into(), vec![7; 32]).unwrap()),
            PrivateDir::create(&records).unwrap(),
            Arc::new(|_| Ok(())),
        )
        .unwrap();
        Self {
            _temp: temp,
            observer,
            host,
            github,
            state,
            records,
        }
    }
    fn admission(&self, c: &RepositoryContract) -> (DeliveryAdmission, Permit) {
        self.host
            .lifecycle(request(id(3)), Lifecycle::Start, 0)
            .unwrap();
        let lease = Lease {
            runtime_id: id(1),
            owner_generation: 1,
            owner_id: id(4),
        };
        self.host.attach_reconciled(lease.clone()).unwrap();
        let digest = bytes_digest(b"fixture");
        let work = WorkIdentity {
            linear_endpoint: "https://linear.invalid/graphql".into(),
            linear_workspace: id(7),
            parent: id(8),
            child: None,
            generation: Generation {
                authorization: digest.clone(),
                definition: digest.clone(),
                correction: digest.clone(),
                scope: digest.clone(),
            },
        };
        let profile = Profile {
            tool: "codex".into(),
            model: "fixture".into(),
            reasoning: "high".into(),
            speed: Speed::Standard,
        };
        let parent = ParentProjection {
            workspace_id: id(6),
            workspace_name: "fixture".into(),
            parent_reference: "fixture/8".into(),
            key: IssueKey::new("https://linear.invalid/graphql", id(7), id(8)).unwrap(),
            issue: None,
            included: true,
            observed_at: Utc::now(),
            required_children: BTreeSet::new(),
            children: BTreeMap::new(),
            observations: BTreeMap::new(),
            metadata: None,
            authorization_candidate: None,
            authorization_published: true,
            corrections: Corrections::default(),
            observation_revision: 1,
            disappeared_children: BTreeMap::new(),
            work: Some(work.clone()),
            blockers: vec![],
            prerequisites: BTreeSet::new(),
            source_digest: digest.clone(),
            config_revision: digest.clone(),
            drain_config_revision: digest.clone(),
            workflow_digest: digest.clone(),
            coordination_profile: profile,
            release_mode: ReleaseMode::Manual,
        };
        let gate = parent
            .fresh_gate(
                None,
                GatePurpose::NewOperation,
                &DependencyOutcomes::default(),
                Utc::now(),
                Duration::seconds(60),
            )
            .unwrap();
        let scope = DeliveryScope {
            schema_version: 1,
            runtime_id: id(1),
            phase: DeliveryPhase::Release,
            work,
            repository: c.repository.clone(),
            revision: RevisionBinding {
                repository: c.repository.clone(),
                head: source().commit.clone(),
                base: source().commit.clone(),
                tested_commit: source().commit,
                tested_tree: source().tree,
            },
            fresh_gate_digest: gate.digest().unwrap(),
            contract_digest: c.digest().unwrap(),
            candidate_vector_digest: Some(digest),
        };
        let permit = self
            .host
            .admit(
                &lease,
                Uuid::new_v4(),
                gate.work_key().unwrap(),
                scope.digest().unwrap(),
                || Ok(()),
            )
            .unwrap();
        (
            DeliveryAdmission::bind(permit.clone(), gate, scope).unwrap(),
            permit,
        )
    }
    async fn prepare(
        &self,
        c: &RepositoryContract,
    ) -> (
        DeliveryAdmission,
        Permit,
        TargetDeploymentChallenge,
        kegari_v0::control::Receipt,
    ) {
        let (admission, permit) = self.admission(c);
        let challenge = self
            .observer
            .prepare_target(c, &admission, &permit, "deploy", source())
            .await
            .unwrap();
        let broker = ProviderBroker {
            linear: None,
            github: Some(self.github.clone()),
        };
        let receipt = broker
            .mutate(
                self.host.clone(),
                request(challenge.dispatch_request()),
                Some(permit.clone()),
                None,
                self.observer.dispatch_command(c, &challenge).unwrap(),
                || Ok(()),
            )
            .await
            .unwrap();
        (admission, permit, challenge, receipt)
    }
    fn artifact(
        &self,
        c: &RepositoryContract,
        ch: &TargetDeploymentChallenge,
        mutate: impl FnOnce(&mut Value),
    ) {
        let step = match &c.configuration.deployment {
            kegari_v0::config::Deployment::Steps { steps } => &steps[0],
            _ => panic!(),
        };
        let artifact = bytes_digest(b"actual target artifact bytes");
        let expected = HostedProofExpectation {
            context_digest: Some(
                deployment_context_digest(42, &step.target_id, 88, 91, &source(), &artifact)
                    .unwrap(),
            ),
            runtime_id: id(1),
            operation_id: ch.operation_id(),
            challenge: ch.challenge(),
            repository_id: 42,
            repository_owner_id: 7,
            run_id: 50,
            run_attempt: 1,
            check_run_id: 60,
            actor_id: 7,
            trigger_sha: "1".repeat(40),
            tested_source_sha: source().commit,
            workflow_ref: step.workflow.proof.workflow_ref.clone(),
            workflow_sha: step.workflow.proof.workflow_sha.clone(),
            job_workflow_ref: step.workflow.proof.job_workflow_ref.clone(),
            job_workflow_sha: step.workflow.proof.job_workflow_sha.clone(),
            event_name: "workflow_dispatch".into(),
        };
        let at = Utc::now().timestamp();
        let claims = json!({"iss":GITHUB_OIDC_ISSUER,"aud":expected.audience().unwrap(),"sub":"fixture","jti":ch.challenge().to_string(),"iat":at,"nbf":at-1,"exp":at+300,"repository_id":"42","repository_owner_id":"7","run_id":"50","run_attempt":"1","check_run_id":"60","actor_id":"7","sha":expected.trigger_sha,"workflow_ref":expected.workflow_ref,"workflow_sha":expected.workflow_sha,"job_workflow_ref":expected.job_workflow_ref,"job_workflow_sha":expected.job_workflow_sha,"runner_environment":"github-hosted","event_name":"workflow_dispatch"});
        let mut value = json!({"schema_version":1,"runtime_id":id(1),"phase_operation":ch.operation_id(),"dispatch_request":ch.dispatch_request(),"challenge":ch.challenge(),"repository_id":42,"step_id":"deploy","target_id":step.target_id,"environment_id":88,"deployment_id":91,"run_id":50,"run_attempt":1,"source":source(),"outputs":{"source_sha":source().commit},"artifact_digest":artifact,"proofs":[{"check_run_id":60,"token":token(claims)}]});
        mutate(&mut value);
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        zip.start_file(
            "kegari-deployment.json",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(&serde_json::to_vec(&value).unwrap()).unwrap();
        let bytes = zip.finish().unwrap().into_inner();
        let mut state = self.state.lock().unwrap();
        state.artifact_name = ch.artifact_name(1);
        state.archive = bytes;
    }
}
fn request(request_id: Uuid) -> MutationRequest {
    MutationRequest {
        request_id,
        runtime_id: id(1),
        actor: Actor {
            principal: None,
            identity: "fixture".into(),
            roles: BTreeSet::from(["lifecycle".into()]),
        },
        kind: "fixture".into(),
        canonical_targets: vec![id(1).to_string()],
        expected: BTreeMap::new(),
        payload: Value::Null,
    }
}
#[tokio::test]
async fn deployment_free_reads_actual_main_tree_and_rejects_moving_baseline() {
    let f = Fixture::new();
    let c = contract(false);
    let baseline = f.observer.observe(&c).await.unwrap();
    assert_eq!(baseline.baseline().production, source());
    assert!(baseline.targets().is_empty());
    assert!(baseline.baseline().target_artifacts.is_empty());
    f.state.lock().unwrap().main = "c".repeat(40);
    assert!(f.observer.reobserve(&c, &baseline).await.is_err());
    let f = Fixture::new();
    f.state.lock().unwrap().move_main = true;
    assert!(f.observer.observe(&c).await.is_err());
}
#[tokio::test]
async fn historical_unattested_target_is_explicit_preparation_blocker() {
    let f = Fixture::new();
    let e = f.observer.observe(&contract(true)).await.unwrap_err();
    assert_eq!(e.code, "production_baseline");
    assert!(e.next_action.unwrap().contains("trusted"));
    assert!(
        f.state
            .lock()
            .unwrap()
            .requests
            .iter()
            .all(|(_, b)| b.is_null())
    );
}
#[tokio::test]
async fn actual_artifact_oidc_and_owned_receipt_produce_complete_recoverable_baseline() {
    let f = Fixture::new();
    let c = contract(true);
    let (a, p, ch, r) = f.prepare(&c).await;
    let repeated = f
        .observer
        .prepare_target(&c, &a, &p, "deploy", source())
        .await
        .unwrap();
    assert_eq!(repeated.dispatch_request(), ch.dispatch_request());
    assert_eq!(repeated.challenge(), ch.challenge());
    f.artifact(&c, &ch, |_| {});
    let proof = f
        .observer
        .observe_target(&c, &a, &p, &ch, &r, &verifier())
        .await
        .unwrap();
    assert_eq!(proof.deployment_id(), 91);
    let baseline = f.observer.observe(&c).await.unwrap();
    assert_eq!(baseline.targets().len(), 1);
    assert_eq!(
        baseline.baseline().target_artifacts["service-production"],
        bytes_digest(b"actual target artifact bytes")
    );
    assert_eq!(
        baseline.baseline().deployed_artifact.as_ref(),
        Some(&bytes_digest(b"actual target artifact bytes"))
    );
    f.observer.reobserve(&c, &baseline).await.unwrap();
    let existing = f
        .observer
        .verify_existing_target(
            &c,
            "service-production",
            &source(),
            &bytes_digest(b"actual target artifact bytes"),
        )
        .await
        .unwrap();
    assert_eq!(existing.deployment_id(), proof.deployment_id());
    assert_eq!(existing.digest().unwrap(), proof.digest().unwrap());
    assert!(
        f.observer
            .verify_existing_target(
                &c,
                "service-production",
                &source(),
                &bytes_digest(b"wrong artifact")
            )
            .await
            .is_err()
    );
    let mut stale_source = source();
    stale_source.commit = "c".repeat(40);
    assert!(
        f.observer
            .verify_existing_target(
                &c,
                "service-production",
                &stale_source,
                &bytes_digest(b"actual target artifact bytes")
            )
            .await
            .is_err()
    );
    assert!(
        f.observer
            .verify_existing_target(
                &c,
                "unconfigured",
                &source(),
                &bytes_digest(b"actual target artifact bytes")
            )
            .await
            .is_err()
    );

    assert!(
        std::fs::read_dir(&f.records)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
            .all(
                |e| !String::from_utf8_lossy(&std::fs::read(e.path()).unwrap())
                    .contains("eyJhbGci")
            )
    );
}
#[tokio::test]
async fn mutated_target_artifact_cannot_pair_with_source_only_or_other_artifact_token() {
    for change in ["artifact", "environment", "source", "dispatch", "job"] {
        let f = Fixture::new();
        let c = contract(true);
        let (a, p, ch, r) = f.prepare(&c).await;
        f.artifact(&c, &ch, |v| match change {
            "artifact" => v["artifact_digest"] = json!(bytes_digest(b"other artifact")),
            "environment" => v["environment_id"] = json!(89),
            "source" => v["source"]["tree"] = json!("c".repeat(40)),
            "dispatch" => v["dispatch_request"] = json!(id(999)),
            "job" => v["proofs"][0]["check_run_id"] = json!(61),
            _ => unreachable!(),
        });
        assert!(
            f.observer
                .observe_target(&c, &a, &p, &ch, &r, &verifier())
                .await
                .is_err(),
            "{change}"
        );
        assert!(f.observer.recover_target(&c, "deploy").await.is_err());
    }
}
#[tokio::test]
async fn recovery_rechecks_full_unfiltered_target_status_and_run_attempt() {
    for change in [
        "pending",
        "failure",
        "deployment",
        "attempt",
        "job",
        "environment",
        "ambiguous",
        "last_page",
        "main",
    ] {
        let f = Fixture::new();
        let c = contract(true);
        let (a, p, ch, r) = f.prepare(&c).await;
        f.artifact(&c, &ch, |_| {});
        f.observer
            .observe_target(&c, &a, &p, &ch, &r, &verifier())
            .await
            .unwrap();
        {
            let mut s = f.state.lock().unwrap();
            match change {
                "pending" => s.status = "pending".into(),
                "failure" => s.status = "failure".into(),
                "deployment" => s.deployment_id = 92,
                "attempt" => s.attempt = 2,
                "job" => s.extra_job = true,
                "environment" => s.environment = 89,
                "ambiguous" => s.extra_target = true,
                "last_page" => s.fail_last_page = true,
                "main" => s.main = "c".repeat(40),
                _ => unreachable!(),
            }
        }
        assert!(
            f.observer.recover_target(&c, "deploy").await.is_err(),
            "{change}"
        );
    }
}

// Disposable offline signing fixture, never a production trust root.
const FIXTURE_KEY: &str = "MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQDFTEX6ChIN+tc0MHn6ey3eu77bUMu7G62F77UjloREcMWQSyO2wSvUf6lXqZ3j7QLOgOjIEUrSpalLDe9uTDnHsJ4qe+QfFiGDdFIVtsTpFGRAtJu0SONWAWZeSVnQe79ssv9UXGRHCQwgvLbUHCtDp6fFZTEnNnuAWknStUhNHJjQ5pog5V1aFTzvJWDVy8/tYTrLY0WJtB8j3wxaATRnZKh6LQvaiqPKvjuMpqjOuPBWL7Sfw6EtsIUsbdKN3JCwwaVBw8SUtxA+pqCUohOPO7rJUvAAk+WHI6AOIrYvgOzX8WBFpc/FwucT/7CnvDOS3QRBTLRRn1ifeU41K9pzAgMBAAECggEATn7MEHDVQg6cdD0dKpS7aTRU+LiqdaTLYtA9xwzpTrl3QLNEYD92irLH0wK+uKHhTftnocGPfUM/mYbPzZgyHMqeAtULq3qIlt4+kjkHGpxsTnoMwKuHFv6zlaulOhchz8NfZlbj5oco9hVewAD9DYu0agqahbldZXGGkXhibZrR+JX1qLn6zMYxdRjAgRwBvC4YmRKi5CVKanetswwheOblGEXJMfl+9PbCJpG4DiDuPe4q8S7iczCVMnrBrzgLtor+Zzgue5uc48nR9QuuE1t5u5ioP3E04YCAsLj31lOnH2ycrVufGalY0tFujDlpkYYiuQi3s7vAn2MEqerA0QKBgQDsoGOn9Pm6yL2Q5GQQEJlbYayGCWgUuT59JdFzMUkA7G8bPDXU+3Y5+1JtNp+1itV9hAYwao4iGtBXBEVpEwUNZkyxO9wJvXsZOHAZL2RFCZlcQSVpacFXcm+/JNPuDGIlvJT1WycEKbq5/kURYY6OfN9zLP+1AEuRag8p33D+XQKBgQDVc5IQSX8W7P7ar3umP74zCWt+bpK+swrYK/FH6oNVskp/uIIHh8T5kF8SFppQwXQlSbRlcabNWAFhtjBD1kqSuCOSExNTW+CXf7xfi3AVp5IFZEdUW7PcMAXg3Cyg6RmAP4OmdDPbt+lIg+0TCMGHXebnY7dINCIdOYClD+OPDwKBgB0MD0+13dYVAFhCARqLvxedpN0LWJB6sPYR52U5npmTbIwuVvlfKQMzG8o2U2KouU5gH5Zf7jS3j+2uo4qjgBJGBZIYJAm6DdtptzEQRUHITOArSxlmlwpSDO+s6KsnMAEr5ik8WEzFhpuAYCAjFqndIFz2ICmq8JvYJiZgvHEtAoGAElmHGwNWe6AQILipRpeAQF708L+MufwmWIMekPImLD3Rjb1M4FzZoCK7KZDr3zliRLpbWcU1c723ELFa4NoROx+9Xxdot+fUHPIMPaJ9Fj97nB8MysG/apRuTPBgncysNow/QxYGR0M+OM+ul85O5NY7vXSjEv/4Vs7Uslv1Uh0CgYEAiEa8RJbaWjOi3vxlhsjVhlUBNR6LK7s8oQYjutn0oeJLZn4kSrYEtkhXjkjkntvY+5BvhAq5ZVCXiGhS5W+gw19cdVSl6yN33QNbBluq3Lj/ZLJYbOwtLr2PgAp+Hu8GktAOhRuMI4YoOkoS5xK9ZjKvS4BnJCnm0wF+7c1+08k=";

// Disposable offline signing fixture, never a production trust root.
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

#[tokio::test]
async fn every_configured_target_needs_its_own_authenticated_current_proof() {
    let f = Fixture::new();
    let mut c = contract(true);
    if let kegari_v0::config::Deployment::Steps { steps } = &mut c.configuration.deployment {
        let mut second = steps[0].clone();
        second.id = "deploy-two".into();
        second.target_id = "second-production-target".into();
        second.environment_id = "89".into();
        steps.push(second);
    }
    let (a, p, ch, r) = f.prepare(&c).await;
    f.artifact(&c, &ch, |_| {});
    f.observer
        .observe_target(&c, &a, &p, &ch, &r, &verifier())
        .await
        .unwrap();
    let error = f.observer.observe(&c).await.unwrap_err();
    assert!(
        error
            .message
            .contains("no retained authenticated deployment proof")
    );
}

#[tokio::test]
async fn target_proof_requires_actual_receipt_and_private_authenticated_recovery_record() {
    let f = Fixture::new();
    let c = contract(true);
    let (a, p, ch, r) = f.prepare(&c).await;
    f.artifact(&c, &ch, |_| {});
    let mut wrong = r.clone();
    wrong.request_id = id(999);
    assert!(
        f.observer
            .observe_target(&c, &a, &p, &ch, &wrong, &verifier())
            .await
            .is_err()
    );
    f.observer
        .observe_target(&c, &a, &p, &ch, &r, &verifier())
        .await
        .unwrap();
    let records = PrivateDir::open(&f.records).unwrap();
    let leaf = records
        .records()
        .unwrap()
        .into_iter()
        .find(|n| n.starts_with("target-proof-"))
        .unwrap();
    let mut record: Value = records.read(&leaf).unwrap();
    record["payload"]["target"]["artifact_digest"] = json!(bytes_digest(b"tampered"));
    records.write(&leaf, &record).unwrap();
    assert_eq!(
        f.observer
            .recover_target(&c, "deploy")
            .await
            .unwrap_err()
            .code,
        "invalid_attestation"
    );
}

#[tokio::test]
async fn archive_member_path_count_and_expansion_are_bounded_before_acceptance() {
    for variant in ["traversal", "multiple", "expanded"] {
        let f = Fixture::new();
        let c = contract(true);
        let (a, p, ch, r) = f.prepare(&c).await;
        f.artifact(&c, &ch, |_| {});
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        zip.start_file(
            if variant == "traversal" {
                "../kegari-deployment.json"
            } else {
                "kegari-deployment.json"
            },
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated),
        )
        .unwrap();
        zip.write_all(
            if variant == "expanded" {
                vec![b' '; 1_048_577]
            } else {
                b"{}".to_vec()
            }
            .as_slice(),
        )
        .unwrap();
        if variant == "multiple" {
            zip.start_file("extra.json", zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"{}").unwrap();
        }
        f.state.lock().unwrap().archive = zip.finish().unwrap().into_inner();
        assert!(
            f.observer
                .observe_target(&c, &a, &p, &ch, &r, &verifier())
                .await
                .is_err(),
            "{variant}"
        );
        assert!(
            !f.records
                .parent()
                .unwrap()
                .join("kegari-deployment.json")
                .exists()
        );
    }
}

#[tokio::test]
async fn deployment_source_matches_candidate_tree_and_is_rechecked_at_dispatch() {
    let f = Fixture::new();
    let c = contract(true);
    let (admission, permit) = f.admission(&c);
    let mut wrong = source();
    wrong.tree = "c".repeat(40);
    assert!(
        f.observer
            .prepare_target(&c, &admission, &permit, "deploy", wrong)
            .await
            .is_err()
    );
    // A provider squash SHA may differ while preserving the exact accepted tree.
    f.state.lock().unwrap().main = "c".repeat(40);
    let squash = Revision {
        commit: "c".repeat(40),
        tree: source().tree,
    };
    let challenge = f
        .observer
        .prepare_target(&c, &admission, &permit, "deploy", squash.clone())
        .await
        .unwrap();
    assert_eq!(challenge.source(), &squash);
    let command = f.observer.dispatch_command(&c, &challenge).unwrap();
    let ProviderMutation::GithubDispatch {
        ref expected_production,
        ..
    } = command
    else {
        panic!()
    };
    assert_eq!(expected_production.as_ref(), Some(&squash));
    f.state.lock().unwrap().main = "d".repeat(40);
    let broker = ProviderBroker {
        linear: None,
        github: Some(f.github.clone()),
    };
    let failure = broker
        .mutate(
            f.host.clone(),
            request(challenge.dispatch_request()),
            Some(permit),
            None,
            command,
            || Ok(()),
        )
        .await
        .unwrap_err();
    assert_eq!(failure.code, "stale_generation");
    let receipt = f.host.operation(challenge.dispatch_request()).unwrap();
    assert_eq!(
        receipt.observed_result.as_ref().unwrap()["effect"],
        "not_sent"
    );
    assert!(
        !f.state
            .lock()
            .unwrap()
            .requests
            .iter()
            .any(|(url, _)| url.ends_with("/dispatches"))
    );
}

#[tokio::test]
async fn late_failure_status_on_older_deployment_blocks_current_target() {
    let f = Fixture::new();
    let c = contract(true);
    let (a, p, ch, r) = f.prepare(&c).await;
    f.artifact(&c, &ch, |_| {});
    f.observer
        .observe_target(&c, &a, &p, &ch, &r, &verifier())
        .await
        .unwrap();
    f.state.lock().unwrap().older_failure = Some("2026-01-02T00:00:00Z");
    f.observer.recover_target(&c, "deploy").await.unwrap();
    // Old deployment T1, successful deployment/status T2, old failed status T3.
    f.state.lock().unwrap().older_failure = Some("2026-01-05T00:00:00Z");
    assert!(f.observer.recover_target(&c, "deploy").await.is_err());
    f.state.lock().unwrap().older_failure = Some("2026-01-04T00:00:00Z");
    assert!(f.observer.recover_target(&c, "deploy").await.is_err());
}
