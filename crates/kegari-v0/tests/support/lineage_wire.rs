//! Offline native-endpoint protocol, actual owned Git bytes, and signed fixture
//! OIDC. This cannot execute a confined repository check or qualify live GitHub.
use super::*;
use crate::deployment::{ProductionObserver, TargetDeploymentChallenge, deployment_context_digest};
use kegari_git::Revision;
use std::path::Path;

#[path = "owned_targets_wire.rs"]
mod owned_targets;

pub(crate) struct TargetReleaseFixture<'a> {
    pub coordinator: &'a DeliveryCoordinator,
    pub observer: &'a ProductionObserver,
    pub gate: &'a FreshGate,
    pub permit: &'a crate::control::Permit,
    pub plan: &'a ReleasePlan,
    pub vector_digest: &'a str,
    pub attempt: u32,
}
pub(crate) struct RuntimeLineageWire {
    base: Arc<Mutex<Fixture>>,
    objects: Mutex<BTreeMap<String, Value>>,
    pack: Mutex<(Vec<u8>, Vec<u8>)>,
    deployed: Revision,
    serving: Mutex<(Revision, u64, u64)>,
    merge: Mutex<Option<Revision>>,
    auxiliary: Mutex<Option<(Revision, bool)>>,
    distinct_targets: Mutex<Option<owned_targets::Targets>>,
}
impl RuntimeLineageWire {
    pub(crate) fn new(seed: &RuntimeSeedWire, deployed: Revision) -> Arc<Self> {
        Arc::new(Self {
            base: seed.0.clone(),
            objects: Mutex::new(BTreeMap::new()),
            pack: Mutex::new((vec![], vec![])),
            serving: Mutex::new((deployed.clone(), 50, 91)),
            deployed,
            merge: Mutex::new(None),
            auxiliary: Mutex::new(None),
            distinct_targets: Mutex::new(None),
        })
    }
    pub(crate) fn configure_distinct_targets(config: &mut Config) {
        let repository = config
            .workspaces
            .get_mut("example")
            .unwrap()
            .repositories
            .get_mut("42")
            .unwrap();
        let route = serde_json::to_value(&repository.no_change_validation).unwrap();
        repository.deployment = serde_json::from_value(json!({"steps":[
            {"id":"deploy","target_id":"service-production","environment_id":"88","workflow":route},
            {"id":"deploy-secondary","target_id":"service-secondary","environment_id":"89","workflow":route}
        ]})).unwrap();
    }
    pub(crate) fn enable_distinct_targets(&self) {
        assert!(
            self.distinct_targets
                .lock()
                .unwrap()
                .replace(owned_targets::Targets::new(self.deployed.clone()))
                .is_none()
        );
    }
    /// Explicit external fixture state only; this does not produce a deployment
    /// receipt or proof for the changed native target.
    pub(crate) fn move_fixture_target(&self, source: Revision) {
        *self.serving.lock().unwrap() = (source, 52, 93);
    }
    pub(crate) fn auxiliary_repository(pull: bool) -> Value {
        json!({"id":84,"owner":{"id":7,"login":"fixture","type":"User"},"name":"aux","default_branch":"main","private":true,"archived":false,"disabled":false,"allow_squash_merge":true,"permissions":{"pull":pull,"push":true}})
    }
    pub(crate) fn add_auxiliary(&self, revision: Revision) {
        assert!(
            self.auxiliary
                .lock()
                .unwrap()
                .replace((revision, true))
                .is_none()
        );
    }
    pub(crate) fn auxiliary_pull(&self, pull: bool) {
        self.auxiliary.lock().unwrap().as_mut().unwrap().1 = pull;
    }
    pub(crate) fn configure(config: &mut Config) {
        for connection in config.connections.values_mut() {
            if connection.kind == kegari_config::ProviderKind::Github {
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
        self.seed_named_target(coordinator, observer, admission, "deploy")
            .await;
    }
    pub(crate) async fn seed_all_targets(
        &self,
        coordinator: &DeliveryCoordinator,
        observer: &ProductionObserver,
        admission: &DeliveryAdmission,
    ) {
        let contract = coordinator.contract(admission).unwrap();
        let Some(kegari_config::Deployment::Steps { steps }) = &contract.configuration.deployment
        else {
            panic!("target fixture needs configured steps")
        };
        for step in steps {
            self.seed_named_target(coordinator, observer, admission, &step.id)
                .await;
        }
    }
    async fn seed_named_target(
        &self,
        coordinator: &DeliveryCoordinator,
        observer: &ProductionObserver,
        admission: &DeliveryAdmission,
        step: &str,
    ) {
        let contract = coordinator.contract(admission).unwrap();
        let challenge = observer
            .prepare_target(
                contract,
                admission,
                &admission.permit,
                step,
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
    /// Actual broker final-gate refusal after its read preflight, followed by the
    /// production no-dispatch reader. No remote write or hosted proof is made.
    pub(crate) async fn failed_target_release(
        &self,
        input: TargetReleaseFixture<'_>,
    ) -> ObservedReleaseStep {
        let TargetReleaseFixture {
            coordinator,
            observer,
            gate,
            permit,
            plan,
            vector_digest,
            attempt,
        } = input;
        let StepKind::DeployExisting {
            repository, source, ..
        } = &plan.steps[0].kind
        else {
            panic!("deployment-only fixture");
        };
        let contract = &coordinator.contracts[repository];
        let scope = DeliveryScope {
            schema_version: 1,
            runtime_id: plan.runtime_id,
            phase: DeliveryPhase::Release,
            work: gate.work().clone(),
            repository: repository.clone(),
            revision: RevisionBinding {
                repository: repository.clone(),
                head: source.commit.clone(),
                base: source.commit.clone(),
                tested_commit: source.commit.clone(),
                tested_tree: source.tree.clone(),
            },
            fresh_gate_digest: gate.digest().unwrap(),
            contract_digest: contract.digest().unwrap(),
            candidate_vector_digest: Some(vector_digest.into()),
        };
        let admission = DeliveryAdmission {
            permit: permit.clone(),
            observation: DeliveryObservation {
                gate: gate.clone(),
                scope: scope.clone(),
                admitted: Some(permit.clone()),
            },
        };
        let challenge = observer
            .prepare_target(contract, &admission, permit, "deploy", source.clone())
            .await
            .unwrap();
        let command = observer.dispatch_command(contract, &challenge).unwrap();
        let record = ReleaseEffectRecord {
            schema_version: 1,
            plan_digest: plan.digest().unwrap(),
            payload: crate::release::StepPayload {
                operation_id: plan.operation_id,
                step_id: "deploy".into(),
                attempt,
                prerequisite_attempts: BTreeMap::new(),
                plan_digest: plan.digest().unwrap(),
                kind: plan.steps[0].kind.clone(),
                exact_source_commit: Some(source.commit.clone()),
            },
            scope,
            admission_operation: permit.operation_id,
            admission_scope_digest: permit.scope_digest.clone(),
            request_id: Some(challenge.dispatch_request()),
            command: Some(command.clone()),
            deployment_step: Some("deploy".into()),
            challenge: Some(challenge.challenge()),
        };
        coordinator
            .records
            .write(
                &release_effect_leaf(plan.operation_id, "deploy", attempt),
                &coordinator.key.sign("release_effect", &record).unwrap(),
            )
            .unwrap();
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let current = coordinator.current_gate.clone();
        let selected = gate.clone();
        let error = coordinator
            .broker
            .mutate(
                coordinator.host.clone(),
                MutationRequest {
                    request_id: challenge.dispatch_request(),
                    runtime_id: plan.runtime_id,
                    actor: Actor {
                        identity: format!("owner:{}", permit.lease.owner_id),
                        principal: None,
                        roles: BTreeSet::new(),
                    },
                    kind: String::new(),
                    canonical_targets: vec![],
                    payload: Value::Null,
                    expected: BTreeMap::from([
                        ("release_plan".into(), plan.digest().unwrap()),
                        ("release_step".into(), "deploy".into()),
                        ("release_attempt".into(), attempt.to_string()),
                    ]),
                },
                Some(permit.clone()),
                None,
                command,
                move || {
                    current(&selected)?;
                    if calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) > 0 {
                        return Err(Error::new(
                            "fixture_preflight_refused",
                            "actual owned preflight has not dispatched",
                        ));
                    }
                    Ok(())
                },
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, "fixture_preflight_refused");
        let observed = coordinator
            .recover_owned_release_step(plan, "deploy", attempt, observer)
            .await
            .unwrap();
        assert!(matches!(observed.outcome(), ReleaseStepOutcome::Failed(_)));
        observed
    }
    /// Explicit synthetic accepted input; real Control/HTTP/proof/step readers
    /// below do not assert that the runtime crossed its integrated-check gate.
    pub(crate) async fn complete_target_release(
        &self,
        input: TargetReleaseFixture<'_>,
    ) -> Vec<ObservedReleaseStep> {
        let TargetReleaseFixture {
            coordinator,
            observer,
            gate,
            permit,
            plan,
            vector_digest,
            attempt: deploy_attempt,
        } = input;
        let StepKind::DeployExisting {
            repository, source, ..
        } = &plan.steps[0].kind
        else {
            panic!("fixture requires deployment-only C")
        };
        let contract = &coordinator.contracts[repository];
        let scope = DeliveryScope {
            schema_version: 1,
            runtime_id: plan.runtime_id,
            phase: DeliveryPhase::Release,
            work: gate.work().clone(),
            repository: repository.clone(),
            revision: RevisionBinding {
                repository: repository.clone(),
                head: source.commit.clone(),
                base: source.commit.clone(),
                tested_commit: source.commit.clone(),
                tested_tree: source.tree.clone(),
            },
            fresh_gate_digest: gate.digest().unwrap(),
            contract_digest: contract.digest().unwrap(),
            candidate_vector_digest: Some(vector_digest.into()),
        };
        let admission = DeliveryAdmission {
            permit: permit.clone(),
            observation: DeliveryObservation {
                gate: gate.clone(),
                scope: scope.clone(),
                admitted: Some(permit.clone()),
            },
        };
        let challenge = observer
            .prepare_target(contract, &admission, permit, "deploy", source.clone())
            .await
            .unwrap();
        *self.serving.lock().unwrap() = (source.clone(), 51, 92);
        let command = observer.dispatch_command(contract, &challenge).unwrap();
        let current_gate = coordinator.current_gate.clone();
        let admitted_gate = gate.clone();
        let receipt = coordinator
            .broker
            .mutate(
                coordinator.host.clone(),
                MutationRequest {
                    request_id: challenge.dispatch_request(),
                    runtime_id: plan.runtime_id,
                    actor: Actor {
                        identity: format!("owner:{}", permit.lease.owner_id),
                        principal: None,
                        roles: BTreeSet::new(),
                    },
                    kind: String::new(),
                    canonical_targets: vec![],
                    expected: BTreeMap::from([
                        ("release_plan".into(), plan.digest().unwrap()),
                        ("release_step".into(), "deploy".into()),
                        ("release_attempt".into(), deploy_attempt.to_string()),
                    ]),
                    payload: Value::Null,
                },
                Some(permit.clone()),
                None,
                command.clone(),
                move || current_gate(&admitted_gate),
            )
            .await
            .unwrap();
        self.artifact(contract, &challenge);
        observer
            .observe_target(
                contract,
                &admission,
                permit,
                &challenge,
                &receipt,
                &verifier(),
            )
            .await
            .unwrap();
        let mut observed = Vec::new();
        for step in &plan.steps {
            let deploy = step.id == "deploy";
            let attempt = if deploy { deploy_attempt } else { 1 };
            let record = ReleaseEffectRecord {
                schema_version: 1,
                plan_digest: plan.digest().unwrap(),
                payload: crate::release::StepPayload {
                    operation_id: plan.operation_id,
                    step_id: step.id.clone(),
                    attempt,
                    prerequisite_attempts: step
                        .depends_on
                        .iter()
                        .map(|id| (id.clone(), deploy_attempt))
                        .collect(),
                    plan_digest: plan.digest().unwrap(),
                    kind: step.kind.clone(),
                    exact_source_commit: Some(source.commit.clone()),
                },
                scope: scope.clone(),
                admission_operation: permit.operation_id,
                admission_scope_digest: permit.scope_digest.clone(),
                request_id: deploy.then_some(challenge.dispatch_request()),
                command: deploy.then_some(command.clone()),
                deployment_step: Some("deploy".into()),
                challenge: deploy.then_some(challenge.challenge()),
            };
            coordinator
                .records
                .write(
                    &release_effect_leaf(plan.operation_id, &step.id, attempt),
                    &coordinator.key.sign("release_effect", &record).unwrap(),
                )
                .unwrap();
            let result = coordinator
                .recover_owned_release_step(plan, &step.id, attempt, observer)
                .await
                .unwrap();
            assert_eq!(result.outcome(), &ReleaseStepOutcome::Succeeded);
            observed.push(result);
        }
        observed
    }
    fn artifact(&self, contract: &RepositoryContract, challenge: &TargetDeploymentChallenge) {
        if let Some(targets) = self.distinct_targets.lock().unwrap().as_mut() {
            targets.artifact(contract, challenge);
            return;
        }
        let (source, run, deployment) = self.serving.lock().unwrap().clone();
        let check = run + 10;
        let Some(kegari_config::Deployment::Steps { steps }) = &contract.configuration.deployment
        else {
            panic!()
        };
        let step = &steps[0];
        let artifact = kegari_digest::bytes_digest(b"fixture initial serving artifact");
        let expectation = HostedProofExpectation {
            context_digest: Some(
                deployment_context_digest(42, &step.target_id, 88, deployment, &source, &artifact)
                    .unwrap(),
            ),
            runtime_id: id(1),
            operation_id: challenge.operation_id(),
            challenge: challenge.challenge(),
            repository_id: 42,
            repository_owner_id: 7,
            run_id: run,
            run_attempt: 1,
            check_run_id: check,
            actor_id: 7,
            trigger_sha: "1".repeat(40),
            tested_source_sha: source.commit.clone(),
            workflow_ref: step.workflow.proof.workflow_ref.clone(),
            workflow_sha: step.workflow.proof.workflow_sha.clone(),
            job_workflow_ref: step.workflow.proof.job_workflow_ref.clone(),
            job_workflow_sha: step.workflow.proof.job_workflow_sha.clone(),
            event_name: "workflow_dispatch".into(),
        };
        let now = Utc::now().timestamp();
        let claims = json!({"iss":GITHUB_OIDC_ISSUER,"aud":expectation.audience().unwrap(),"sub":"fixture","jti":challenge.challenge().to_string(),"iat":now,"nbf":now-1,"exp":now+300,"repository_id":"42","repository_owner_id":"7","run_id":run.to_string(),"run_attempt":"1","check_run_id":check.to_string(),"actor_id":"7","sha":expectation.trigger_sha,"workflow_ref":expectation.workflow_ref,"workflow_sha":expectation.workflow_sha,"job_workflow_ref":expectation.job_workflow_ref,"job_workflow_sha":expectation.job_workflow_sha,"runner_environment":"github-hosted","event_name":"workflow_dispatch"});
        let manifest = json!({"schema_version":1,"runtime_id":id(1),"phase_operation":challenge.operation_id(),"dispatch_request":challenge.dispatch_request(),"challenge":challenge.challenge(),"repository_id":42,"step_id":"deploy","target_id":step.target_id,"environment_id":88,"deployment_id":deployment,"run_id":run,"run_attempt":1,"source":source,"outputs":{"source_sha":source.commit},"artifact_digest":artifact,"proofs":[{"check_run_id":check,"token":token(claims)}]});
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
            if let Some(response) = self
                .distinct_targets
                .lock()
                .unwrap()
                .as_mut()
                .and_then(|targets| targets.send(&request, &self.base))
            {
                return response;
            }
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
                if path.starts_with("/repos/fixture/aux") {
                    assert_eq!(request.method, Method::Get);
                    let (revision, pull) = self.auxiliary.lock().unwrap().clone().unwrap();
                    let value = match path.as_str() {
                        "/repos/fixture/aux" => Self::auxiliary_repository(pull),
                        "/repos/fixture/aux/pulls" => json!([]),
                        "/repos/fixture/aux/git/ref/heads/main" => {
                            json!({"ref":"refs/heads/main","object":{"type":"commit","sha":revision.commit}})
                        }
                        "/repos/fixture/aux/git/ref/heads/staging" => {
                            json!({"ref":"refs/heads/staging","object":{"type":"commit","sha":revision.commit}})
                        }
                        _ if path
                            == format!("/repos/fixture/aux/git/commits/{}", revision.commit) =>
                        {
                            json!({"sha":revision.commit,"tree":{"sha":revision.tree},"parents":[]})
                        }
                        _ => panic!("unexpected auxiliary repository request: {path}"),
                    };
                    return Ok(response(value));
                }
                let (source, run, deployment) = self.serving.lock().unwrap().clone();
                let deployment_value = json!({"id":deployment,"sha":source.commit,"environment":"production","production_environment":true,"transient_environment":false,"creator":{"id":99},"created_at":"2026-01-03T00:00:00Z","updated_at":"2026-01-03T00:00:00Z"});
                if run > 50 {
                    let base = "/repos/fixture/repo";
                    if path == format!("{base}/actions/workflows/3/dispatches") {
                        self.base.lock().unwrap().dispatch_count += 1;
                        return Ok(response(json!({"workflow_run_id":run})));
                    }
                    if path == format!("{base}/actions/runs/{run}")
                        || path == format!("{base}/actions/runs/{run}/attempts/1")
                    {
                        return Ok(response(
                            json!({"id":run,"repository":{"id":42},"run_attempt":1,"workflow_id":3,"actor":{"id":7},"triggering_actor":{"id":7},"event":"workflow_dispatch","head_sha":"1".repeat(40),"status":"completed","conclusion":"success","path":".github/workflows/caller.yml","html_url":format!("https://github.com/fixture/repo/actions/runs/{run}")}),
                        ));
                    }
                    if path == format!("{base}/actions/runs/{run}/attempts/1/jobs") {
                        return Ok(response(
                            json!({"total_count":1,"jobs":[{"id":run+20,"run_id":run,"run_attempt":1,"name":"suite","head_sha":"1".repeat(40),"status":"completed","conclusion":"success","check_run_url":format!("https://api.github.com/repos/fixture/repo/check-runs/{}",run+10),"steps":[]}]}),
                        ));
                    }
                    if path == format!("{base}/actions/runs/{run}/artifacts") {
                        return Ok(response(
                            json!({"total_count":1,"artifacts":[{"id":run+30,"name":self.base.lock().unwrap().artifact_name}]}),
                        ));
                    }
                    if path == format!("{base}/actions/artifacts/{}", run + 30) {
                        let wire = self.base.lock().unwrap();
                        return Ok(response(
                            json!({"id":run+30,"workflow_run":{"id":run,"repository_id":42,"head_sha":"1".repeat(40)},"expired":false,"size_in_bytes":wire.archive.len(),"digest":kegari_digest::bytes_digest(&wire.archive)}),
                        ));
                    }
                    if path == format!("{base}/actions/artifacts/{}/zip", run + 30) {
                        return Ok(HttpResponse {
                            status: 302,
                            headers: HeaderMap::from_iter([(
                                "location".parse().unwrap(),
                                "https://fixture.blob.invalid/archive?grant=fixture"
                                    .parse()
                                    .unwrap(),
                            )]),
                            body: vec![],
                        });
                    }
                }
                if let Some(value) = path
                    .strip_prefix("/repos/fixture/repo/git/commits/")
                    .and_then(|id| self.objects.lock().unwrap().get(id).cloned())
                {
                    return Ok(response(value));
                }
                if path == "/repos/fixture/repo/deployments" {
                    return Ok(response(json!([deployment_value])));
                }
                if path == format!("/repos/fixture/repo/deployments/{deployment}") {
                    return Ok(response(deployment_value));
                }
                if path == format!("/repos/fixture/repo/deployments/{deployment}/statuses") {
                    return Ok(response(
                        json!([{"id":deployment+2,"state":"success","environment":"production","creator":{"id":99},"created_at":"2026-01-04T00:00:00Z","updated_at":"2026-01-04T00:00:00Z"}]),
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
            if path == "/user/repos"
                && let Some((_, pull)) = self.auxiliary.lock().unwrap().as_ref()
            {
                let mut repositories: Vec<Value> = serde_json::from_slice(&result.body).unwrap();
                repositories.push(Self::auxiliary_repository(*pull));
                result.body = serde_json::to_vec(&repositories).unwrap();
            }
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
    let (mut child, _group) = kegari_process::group::OwnedGroup::spawn_std(
        Command::new(binary)
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
            .stderr(Stdio::piped()),
    )
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
