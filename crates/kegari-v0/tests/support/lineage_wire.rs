//! Offline native-endpoint protocol, actual owned Git bytes, and signed fixture
//! OIDC. This cannot execute a confined repository check or qualify live GitHub.
use super::*;
use crate::deployment::{ProductionObserver, TargetDeploymentChallenge, deployment_context_digest};
use crate::git::Revision;
use std::path::Path;

pub(crate) struct RuntimeLineageWire {
    base: Arc<Mutex<Fixture>>,
    objects: Mutex<BTreeMap<String, Value>>,
    pack: Mutex<(Vec<u8>, Vec<u8>)>,
    deployed: Revision,
    merge: Mutex<Option<Revision>>,
}
impl RuntimeLineageWire {
    pub(crate) fn new(seed: &RuntimeSeedWire, deployed: Revision) -> Arc<Self> {
        Arc::new(Self {
            base: seed.0.clone(),
            objects: Mutex::new(BTreeMap::new()),
            pack: Mutex::new((vec![], vec![])),
            deployed,
            merge: Mutex::new(None),
        })
    }
    pub(crate) fn configure(config: &mut Config) {
        for connection in config.connections.values_mut() {
            if connection.kind == crate::config::ProviderKind::Github {
                connection.endpoint = Some("https://api.github.com/".into());
            }
        }
        let repository = config
            .workspaces
            .get_mut("example")
            .unwrap()
            .repositories
            .get_mut("42")
            .unwrap();
        repository.hostname = "github.com".into();
        let route = serde_json::to_value(&repository.no_change_validation).unwrap();
        repository.deployment = serde_json::from_value(json!({"steps":[{"id":"deploy","target_id":"service-production","environment_id":"88","workflow":route}]})).unwrap();
    }
    pub(crate) fn prepare_merge(
        &self,
        binary: &Path,
        source: &Path,
        slice: &IsolatedSlice,
    ) -> Revision {
        let candidate = slice.manifest().result.as_ref().unwrap();
        // Populate the owned synthetic remote repository from the actual
        // candidate fixture before serving its immutable upload-pack response.
        git_bytes(
            binary,
            source,
            &[
                "fetch",
                "--no-tags",
                "--no-write-fetch-head",
                slice.path().to_str().unwrap(),
                &candidate.commit,
            ],
            None,
        );
        let bytes = git_bytes(
            binary,
            source,
            &[
                "commit-tree",
                &candidate.tree,
                "-p",
                &slice.manifest().production.commit,
                "-m",
                "test: owned production squash",
            ],
            None,
        );
        let merged = Revision {
            commit: String::from_utf8(bytes).unwrap().trim().into(),
            tree: candidate.tree.clone(),
        };
        {
            let mut wire = self.base.lock().unwrap();
            wire.prs.retain(|p| p["number"] != 17);
            wire.prs.push(json!({"id":170,"number":17,"user":{"id":7},"state":"open","draft":false,"head":{"sha":candidate.commit,"ref":"kegari-release/fixture","repo":{"id":42}},"base":{"sha":slice.manifest().production.commit,"ref":"main","repo":{"id":42}},"merged":false,"merge_commit_sha":null,"html_url":"https://github.com/fixture/repo/pull/17"}));
        }
        *self.merge.lock().unwrap() = Some(merged.clone());
        self.capture_objects(
            binary,
            source,
            &[
                candidate.clone(),
                merged.clone(),
                slice.manifest().production.clone(),
            ],
        );
        merged
    }
    pub(crate) fn capture_objects(&self, binary: &Path, source: &Path, revisions: &[Revision]) {
        let mut commits = self.objects.lock().unwrap();
        for revision in revisions {
            let parents = String::from_utf8(git_bytes(
                binary,
                source,
                &["show", "-s", "--format=%P", &revision.commit],
                None,
            ))
            .unwrap();
            commits.insert(revision.commit.clone(), json!({"sha":revision.commit,"tree":{"sha":revision.tree},"parents":parents.split_whitespace().map(|id|json!({"sha":id})).collect::<Vec<_>>()}));
        }
        let advertisement = git_bytes(
            binary,
            source,
            &[
                "upload-pack",
                "--stateless-rpc",
                "--advertise-refs",
                source.to_str().unwrap(),
            ],
            None,
        );
        let mut request = Vec::new();
        let append = |out: &mut Vec<u8>, line: &str| {
            out.extend_from_slice(format!("{:04x}{line}", line.len() + 4).as_bytes());
        };
        append(&mut request, "command=fetch\n");
        request.extend_from_slice(b"0001");
        for oid in commits.keys() {
            append(&mut request, &format!("want {oid}\n"));
        }
        append(&mut request, "done\n");
        append(&mut request, "no-progress\n");
        request.extend_from_slice(b"0000");
        let response = git_bytes(
            binary,
            source,
            &["upload-pack", "--stateless-rpc", source.to_str().unwrap()],
            Some(&request),
        );
        *self.pack.lock().unwrap() = (advertisement, response);
    }
    pub(crate) async fn seed_target(
        &self,
        coordinator: &DeliveryCoordinator,
        observer: &ProductionObserver,
        admission: &DeliveryAdmission,
    ) {
        let contract = coordinator.contract(admission).unwrap();
        let challenge = observer
            .prepare_target(
                contract,
                admission,
                &admission.permit,
                "deploy",
                self.deployed.clone(),
            )
            .await
            .unwrap();
        let receipt = coordinator
            .broker
            .mutate(
                coordinator.host.clone(),
                MutationRequest {
                    request_id: challenge.dispatch_request(),
                    runtime_id: admission.scope().runtime_id,
                    actor: Actor {
                        identity: format!("owner:{}", admission.permit.lease.owner_id),
                        principal: None,
                        roles: BTreeSet::new(),
                    },
                    kind: String::new(),
                    canonical_targets: vec![],
                    expected: BTreeMap::new(),
                    payload: Value::Null,
                },
                Some(admission.permit.clone()),
                None,
                observer.dispatch_command(contract, &challenge).unwrap(),
                || Ok(()),
            )
            .await
            .unwrap();
        self.artifact(contract, &challenge);
        observer
            .observe_target(
                contract,
                admission,
                &admission.permit,
                &challenge,
                &receipt,
                &verifier(),
            )
            .await
            .unwrap();
    }
    pub(crate) async fn send_merge(
        &self,
        coordinator: &DeliveryCoordinator,
        scope: DeliveryScope,
        permit: &crate::control::Permit,
        plan: &ReleasePlan,
        gate: &FreshGate,
    ) -> Receipt {
        let step = plan
            .steps
            .iter()
            .find(|step| matches!(step.kind, StepKind::Merge { .. }))
            .unwrap();
        let StepKind::Merge {
            pull_request,
            production_base,
            candidate,
            ..
        } = &step.kind
        else {
            unreachable!()
        };
        let request_id = Uuid::new_v4();
        let record = ReleaseEffectRecord {
            schema_version: 1,
            plan_digest: plan.digest().unwrap(),
            payload: crate::release::StepPayload {
                operation_id: plan.operation_id,
                step_id: step.id.clone(),
                attempt: 1,
                prerequisite_attempts: BTreeMap::new(),
                plan_digest: plan.digest().unwrap(),
                kind: step.kind.clone(),
                exact_source_commit: None,
            },
            scope: scope.clone(),
            admission_operation: permit.operation_id,
            admission_scope_digest: permit.scope_digest.clone(),
            request_id: Some(request_id),
            command: Some(ProviderMutation::GithubMerge {
                repository: coordinator.contracts[&scope.repository].provider.clone(),
                pr: *pull_request,
                expected_head: candidate.commit.clone(),
                expected_base: production_base.commit.clone(),
            }),
            deployment_step: None,
            challenge: None,
        };
        coordinator
            .records
            .write(
                &release_effect_leaf(plan.operation_id, &step.id, 1),
                &coordinator.key.sign("release_effect", &record).unwrap(),
            )
            .unwrap();
        let current_gate = coordinator.current_gate.clone();
        let gate = gate.clone();
        coordinator
            .broker
            .mutate(
                coordinator.host.clone(),
                MutationRequest {
                    request_id,
                    runtime_id: scope.runtime_id,
                    actor: Actor {
                        identity: format!("owner:{}", permit.lease.owner_id),
                        principal: None,
                        roles: BTreeSet::new(),
                    },
                    kind: String::new(),
                    canonical_targets: vec![],
                    expected: BTreeMap::from([
                        ("release_plan".into(), record.plan_digest),
                        ("release_step".into(), step.id.clone()),
                        ("release_attempt".into(), "1".into()),
                        ("delivery_scope".into(), scope.digest().unwrap()),
                    ]),
                    payload: Value::Null,
                },
                Some(permit.clone()),
                None,
                record.command.unwrap(),
                move || current_gate(&gate),
            )
            .await
            .unwrap()
    }
    fn artifact(&self, contract: &RepositoryContract, challenge: &TargetDeploymentChallenge) {
        let crate::config::Deployment::Steps { steps } = &contract.configuration.deployment else {
            panic!()
        };
        let step = &steps[0];
        let artifact = crate::digest::bytes_digest(b"fixture initial serving artifact");
        let expectation = HostedProofExpectation {
            context_digest: Some(
                deployment_context_digest(42, &step.target_id, 88, 91, &self.deployed, &artifact)
                    .unwrap(),
            ),
            runtime_id: id(1),
            operation_id: challenge.operation_id(),
            challenge: challenge.challenge(),
            repository_id: 42,
            repository_owner_id: 7,
            run_id: 50,
            run_attempt: 1,
            check_run_id: 60,
            actor_id: 7,
            trigger_sha: "1".repeat(40),
            tested_source_sha: self.deployed.commit.clone(),
            workflow_ref: step.workflow.proof.workflow_ref.clone(),
            workflow_sha: step.workflow.proof.workflow_sha.clone(),
            job_workflow_ref: step.workflow.proof.job_workflow_ref.clone(),
            job_workflow_sha: step.workflow.proof.job_workflow_sha.clone(),
            event_name: "workflow_dispatch".into(),
        };
        let now = Utc::now().timestamp();
        let claims = json!({"iss":GITHUB_OIDC_ISSUER,"aud":expectation.audience().unwrap(),"sub":"fixture","jti":challenge.challenge().to_string(),"iat":now,"nbf":now-1,"exp":now+300,"repository_id":"42","repository_owner_id":"7","run_id":"50","run_attempt":"1","check_run_id":"60","actor_id":"7","sha":expectation.trigger_sha,"workflow_ref":expectation.workflow_ref,"workflow_sha":expectation.workflow_sha,"job_workflow_ref":expectation.job_workflow_ref,"job_workflow_sha":expectation.job_workflow_sha,"runner_environment":"github-hosted","event_name":"workflow_dispatch"});
        let manifest = json!({"schema_version":1,"runtime_id":id(1),"phase_operation":challenge.operation_id(),"dispatch_request":challenge.dispatch_request(),"challenge":challenge.challenge(),"repository_id":42,"step_id":"deploy","target_id":step.target_id,"environment_id":88,"deployment_id":91,"run_id":50,"run_attempt":1,"source":self.deployed,"outputs":{"source_sha":self.deployed.commit},"artifact_digest":artifact,"proofs":[{"check_run_id":60,"token":token(claims)}]});
        let mut wire = self.base.lock().unwrap();
        wire.artifact_name = challenge.artifact_name(1);
        wire.archive = zip_fixture(
            "kegari-deployment.json",
            &serde_json::to_vec(&manifest).unwrap(),
        );
    }
}
impl HttpTransport for RuntimeLineageWire {
    fn send(
        &self,
        mut request: HttpRequest,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<HttpResponse>> + Send + '_>>
    {
        Box::pin(async move {
            let path = request.url.path().to_owned();
            let response = |value| HttpResponse {
                status: 200,
                headers: HeaderMap::new(),
                body: serde_json::to_vec(&value).unwrap(),
            };
            if request.url.host_str() == Some("github.com") {
                assert!(request.headers["authorization"].is_sensitive());
                let mut headers = HeaderMap::new();
                let data = self.pack.lock().unwrap();
                let body = if path.ends_with("/info/refs") {
                    headers.insert(
                        "content-type",
                        "application/x-git-upload-pack-advertisement"
                            .parse()
                            .unwrap(),
                    );
                    data.0.clone()
                } else {
                    assert_eq!(path, "/fixture/repo.git/git-upload-pack");
                    headers.insert(
                        "content-type",
                        "application/x-git-upload-pack-result".parse().unwrap(),
                    );
                    data.1.clone()
                };
                return Ok(HttpResponse {
                    status: 200,
                    headers,
                    body,
                });
            }
            if request.url.host_str() == Some("api.github.com") {
                if let Some(value) = path
                    .strip_prefix("/repos/fixture/repo/git/commits/")
                    .and_then(|id| self.objects.lock().unwrap().get(id).cloned())
                {
                    return Ok(response(value));
                }
                if path == "/repos/fixture/repo/deployments" {
                    return Ok(response(
                        json!([{"id":91,"sha":self.deployed.commit,"environment":"production","production_environment":true,"transient_environment":false,"creator":{"id":99},"created_at":"2026-01-03T00:00:00Z","updated_at":"2026-01-03T00:00:00Z"}]),
                    ));
                }
                if path == "/repos/fixture/repo/deployments/91/statuses" {
                    return Ok(response(
                        json!([{"id":93,"state":"success","environment":"production","creator":{"id":99},"created_at":"2026-01-04T00:00:00Z","updated_at":"2026-01-04T00:00:00Z"}]),
                    ));
                }
                if path == "/repos/fixture/repo/pulls/17/merge" && request.method == Method::Put {
                    let merged = self.merge.lock().unwrap().clone().unwrap();
                    let body: Value = serde_json::from_slice(&request.body).unwrap();
                    let mut wire = self.base.lock().unwrap();
                    let pr = wire.prs.iter_mut().find(|p| p["number"] == 17).unwrap();
                    assert_eq!(body["sha"], pr["head"]["sha"]);
                    pr["merged"] = json!(true);
                    pr["state"] = json!("closed");
                    pr["merge_commit_sha"] = json!(merged.commit);
                    wire.merge_parent = Some(wire.production.clone());
                    wire.production = merged.commit.clone();
                    wire.production_tree = merged.tree;
                    return Ok(response(
                        json!({"merged":true,"sha":merged.commit,"message":"actual fixture squash"}),
                    ));
                }
                request.url.set_host(Some("github.invalid")).unwrap();
            }
            let mut result = Wire(self.base.clone()).send(request).await?;
            if result.headers.get("content-type").is_none()
                && let Ok(value) = std::str::from_utf8(&result.body)
            {
                result.body = value
                    .replace(
                        "https://github.invalid/repos/",
                        "https://api.github.com/repos/",
                    )
                    .replace("https://github.invalid/", "https://github.com/")
                    .into_bytes();
            }
            Ok(result)
        })
    }
}
fn git_bytes(binary: &Path, root: &Path, args: &[&str], input: Option<&[u8]>) -> Vec<u8> {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let mut child = Command::new(binary)
        .current_dir(root)
        .args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_PROTOCOL", "version=2")
        .env("GIT_AUTHOR_NAME", "Fixture Human")
        .env("GIT_COMMITTER_NAME", "Fixture Human")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.test")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.test")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(bytes) = input {
        child.stdin.take().unwrap().write_all(bytes).unwrap();
    } else {
        drop(child.stdin.take());
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}
