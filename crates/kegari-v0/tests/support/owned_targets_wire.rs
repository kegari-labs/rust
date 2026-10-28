//! Two independent offline target observations, including distinct artifact bytes.
//! Only provider data is constructed here; target capabilities use the real adapter.
use super::*;

pub(super) struct Targets {
    values: Vec<Target>,
}
struct Target {
    step: &'static str,
    target: &'static str,
    environment: u64,
    name: &'static str,
    run: u64,
    deployment: u64,
    source: Revision,
    artifact: String,
    archive: Vec<u8>,
    artifact_name: String,
}
impl Targets {
    pub(super) fn new(source: Revision) -> Self {
        Self {
            values: [
                ("deploy", "service-production", 88, "production", 150, 191),
                (
                    "deploy-secondary",
                    "service-secondary",
                    89,
                    "secondary",
                    250,
                    291,
                ),
            ]
            .into_iter()
            .map(
                |(step, target, environment, name, run, deployment)| Target {
                    step,
                    target,
                    environment,
                    name,
                    run,
                    deployment,
                    source: source.clone(),
                    artifact: crate::digest::bytes_digest(
                        format!("distinct actual fixture artifact for {target}").as_bytes(),
                    ),
                    archive: vec![],
                    artifact_name: String::new(),
                },
            )
            .collect(),
        }
    }
    pub(super) fn artifact(
        &mut self,
        contract: &RepositoryContract,
        challenge: &TargetDeploymentChallenge,
    ) {
        let name = challenge.artifact_name(1);
        let target = self
            .values
            .iter_mut()
            .find(|target| {
                name == format!(
                    "kegari-deployment-{}-1-{}",
                    challenge.operation_id(),
                    target.environment
                )
            })
            .unwrap();
        assert_eq!(challenge.source(), &target.source);
        let Some(crate::config::Deployment::Steps { steps }) = &contract.configuration.deployment
        else {
            panic!()
        };
        let step = steps.iter().find(|step| step.id == target.step).unwrap();
        assert_eq!(step.target_id, target.target);
        assert_eq!(step.environment_id, target.environment.to_string());
        let expectation = HostedProofExpectation {
            context_digest: Some(
                deployment_context_digest(
                    42,
                    target.target,
                    target.environment,
                    target.deployment,
                    &target.source,
                    &target.artifact,
                )
                .unwrap(),
            ),
            runtime_id: id(1),
            operation_id: challenge.operation_id(),
            challenge: challenge.challenge(),
            repository_id: 42,
            repository_owner_id: 7,
            run_id: target.run,
            run_attempt: 1,
            check_run_id: target.run + 10,
            actor_id: 7,
            trigger_sha: "1".repeat(40),
            tested_source_sha: target.source.commit.clone(),
            workflow_ref: step.workflow.proof.workflow_ref.clone(),
            workflow_sha: step.workflow.proof.workflow_sha.clone(),
            job_workflow_ref: step.workflow.proof.job_workflow_ref.clone(),
            job_workflow_sha: step.workflow.proof.job_workflow_sha.clone(),
            event_name: "workflow_dispatch".into(),
        };
        let now = Utc::now().timestamp();
        let claims = json!({"iss":GITHUB_OIDC_ISSUER,"aud":expectation.audience().unwrap(),"sub":"fixture","jti":challenge.challenge().to_string(),"iat":now,"nbf":now-1,"exp":now+300,"repository_id":"42","repository_owner_id":"7","run_id":target.run.to_string(),"run_attempt":"1","check_run_id":(target.run+10).to_string(),"actor_id":"7","sha":expectation.trigger_sha,"workflow_ref":expectation.workflow_ref,"workflow_sha":expectation.workflow_sha,"job_workflow_ref":expectation.job_workflow_ref,"job_workflow_sha":expectation.job_workflow_sha,"runner_environment":"github-hosted","event_name":"workflow_dispatch"});
        let manifest = json!({"schema_version":1,"runtime_id":id(1),"phase_operation":challenge.operation_id(),"dispatch_request":challenge.dispatch_request(),"challenge":challenge.challenge(),"repository_id":42,"step_id":target.step,"target_id":target.target,"environment_id":target.environment,"deployment_id":target.deployment,"run_id":target.run,"run_attempt":1,"source":target.source,"outputs":{"source_sha":target.source.commit},"artifact_digest":target.artifact,"proofs":[{"check_run_id":target.run+10,"token":token(claims)}]});
        target.archive = zip_fixture(
            "kegari-deployment.json",
            &serde_json::to_vec(&manifest).unwrap(),
        );
        target.artifact_name = name;
    }
    pub(super) fn send(
        &self,
        request: &HttpRequest,
        base: &Arc<Mutex<Fixture>>,
    ) -> Option<Result<HttpResponse>> {
        let response = |value| {
            Ok(HttpResponse {
                status: 200,
                headers: HeaderMap::new(),
                body: serde_json::to_vec(&value).unwrap(),
            })
        };
        let path = request.url.path();
        if request.url.host_str() == Some("fixture.blob.invalid")
            && path.starts_with("/owned-target/")
        {
            assert!(!request.headers.contains_key("authorization"));
            let target = self
                .values
                .iter()
                .find(|target| path == format!("/owned-target/{}", target.run))
                .unwrap();
            return Some(Ok(HttpResponse {
                status: 200,
                headers: HeaderMap::new(),
                body: target.archive.clone(),
            }));
        }
        if request.url.host_str() != Some("api.github.com") {
            return None;
        }
        let prefix = "/repos/fixture/repo";
        let environment = |target: &Target| json!({"id":target.environment,"name":target.name,"url":format!("https://api.github.com{prefix}/environments/{}", target.name),"protection_rules":[],"deployment_branch_policy":null});
        if path == format!("{prefix}/environments") {
            return Some(response(
                json!({"total_count":self.values.len(),"environments":self.values.iter().map(environment).collect::<Vec<_>>()}),
            ));
        }
        if path == format!("{prefix}/actions/workflows/3/dispatches") {
            assert_eq!(request.method, Method::Post);
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let target = self
                .values
                .iter()
                .find(|target| body["inputs"]["kegari_target_id"] == target.target)
                .unwrap();
            assert_eq!(
                body["inputs"]["kegari_environment_id"],
                target.environment.to_string()
            );
            base.lock().unwrap().dispatch_count += 1;
            return Some(response(json!({"workflow_run_id":target.run})));
        }
        if path == format!("{prefix}/deployments") {
            let name = request
                .url
                .query_pairs()
                .find(|(key, _)| key == "environment")
                .unwrap()
                .1;
            let target = self
                .values
                .iter()
                .find(|target| target.name == name)
                .unwrap();
            return Some(response(json!([target.deployment()])));
        }
        for target in &self.values {
            let run = target.run;
            if path == format!("{prefix}/environments/{}", target.name) {
                return Some(response(environment(target)));
            }
            if path == format!("{prefix}/deployments/{}", target.deployment) {
                return Some(response(target.deployment()));
            }
            if path == format!("{prefix}/deployments/{}/statuses", target.deployment) {
                return Some(response(
                    json!([{"id":target.deployment+2,"state":"success","environment":target.name,"creator":{"id":99},"created_at":"2026-01-04T00:00:00Z","updated_at":"2026-01-04T00:00:00Z"}]),
                ));
            }
            if path == format!("{prefix}/actions/runs/{run}")
                || path == format!("{prefix}/actions/runs/{run}/attempts/1")
            {
                return Some(response(
                    json!({"id":run,"repository":{"id":42},"run_attempt":1,"workflow_id":3,"actor":{"id":7},"triggering_actor":{"id":7},"event":"workflow_dispatch","head_sha":"1".repeat(40),"status":"completed","conclusion":"success","path":".github/workflows/caller.yml","html_url":format!("https://github.com/fixture/repo/actions/runs/{run}")}),
                ));
            }
            if path == format!("{prefix}/actions/runs/{run}/attempts/1/jobs") {
                return Some(response(
                    json!({"total_count":1,"jobs":[{"id":run+20,"run_id":run,"run_attempt":1,"name":"suite","head_sha":"1".repeat(40),"status":"completed","conclusion":"success","check_run_url":format!("https://api.github.com{prefix}/check-runs/{}",run+10),"steps":[]}]}),
                ));
            }
            if path == format!("{prefix}/actions/runs/{run}/artifacts") {
                return Some(response(
                    json!({"total_count":1,"artifacts":[{"id":run+30,"name":target.artifact_name}]}),
                ));
            }
            if path == format!("{prefix}/actions/artifacts/{}", run + 30) {
                return Some(response(
                    json!({"id":run+30,"workflow_run":{"id":run,"repository_id":42,"head_sha":"1".repeat(40)},"expired":false,"size_in_bytes":target.archive.len(),"digest":crate::digest::bytes_digest(&target.archive)}),
                ));
            }
            if path == format!("{prefix}/actions/artifacts/{}/zip", run + 30) {
                return Some(Ok(HttpResponse {
                    status: 302,
                    headers: HeaderMap::from_iter([(
                        "location".parse().unwrap(),
                        format!("https://fixture.blob.invalid/owned-target/{run}?grant=fixture")
                            .parse()
                            .unwrap(),
                    )]),
                    body: vec![],
                }));
            }
        }
        None
    }
}
impl Target {
    fn deployment(&self) -> Value {
        json!({"id":self.deployment,"sha":self.source.commit,"environment":self.name,"production_environment":true,"transient_environment":false,"creator":{"id":99},"created_at":"2026-01-03T00:00:00Z","updated_at":"2026-01-03T00:00:00Z"})
    }
}
