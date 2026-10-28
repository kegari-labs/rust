//! Production baselines from complete provider reads and authenticated target
//! evidence. Deployment requests/status text never establish artifact identity.
use crate::{
    control::{ControlHost, Permit, Receipt, ReceiptPhase},
    delivery::{DeliveryAdmission, DeliveryPhase, RepositoryContract},
    evidence::{AttestationKey, ProductionBaseline, ProductionTargetBaseline, Signed},
    git::Revision,
    providers::{HostedProofExpectation, HostedProofVerifier, ProviderMutation, VerifiedHostedJob},
    reconcile::FreshGate,
};
use chrono::{DateTime, Utc};
use kegari_config::{Deployment, DeploymentStep};
use kegari_digest::{bytes_digest, canonical_digest};
use kegari_providers::{GithubClient, GithubRecord, WorkflowJob, WorkflowRun};
use kegari_storage::PrivateDir;
use kegari_types::error::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    sync::Arc,
};
use uuid::Uuid;

#[path = "deployment_history.rs"]
mod history;
pub use history::{HistoricalTargetDeployment, TargetProofReference};
#[path = "deployment_baseline_history.rs"]
mod baseline_history;
pub(crate) use baseline_history::OwnedTargetObservationRef;

pub type CurrentAuthority = Arc<dyn Fn(&FreshGate) -> Result<()> + Send + Sync>;

pub struct ProductionObserver {
    github: GithubClient,
    control: Arc<ControlHost>,
    key: Arc<AttestationKey>,
    records: PrivateDir,
    current: CurrentAuthority,
    successor: Option<Arc<crate::runtime_delivery::KnownPredecessorBaseline>>,
}
#[derive(Clone, Debug)]
pub struct VerifiedProductionBaseline {
    // Actual current owned target proofs; successor baselines cannot mint history.
    owned_targets: Vec<VerifiedTargetDeployment>,
    baseline: ProductionBaseline,
    targets: Vec<ProductionTargetBaseline>,
    contract_digest: String,
    observed_at: DateTime<Utc>,
}
impl VerifiedProductionBaseline {
    pub fn baseline(&self) -> &ProductionBaseline {
        &self.baseline
    }
    pub fn targets(&self) -> &[ProductionTargetBaseline] {
        &self.targets
    }
    pub fn contract_digest(&self) -> &str {
        &self.contract_digest
    }
    pub fn observed_at(&self) -> DateTime<Utc> {
        self.observed_at
    }
}
/// Complete current native production inventory for an explicit recovery
/// decision. Divergence is retained as observed state, never promoted to a
/// successful deployment or an ordinary assembly baseline.
#[derive(Clone, Debug)]
pub struct VerifiedRecoveryBaseline {
    repository: crate::git::RepositoryId,
    production: Revision,
    targets: Vec<ProductionTargetBaseline>,
    contract_digest: String,
    observed_at: DateTime<Utc>,
}
impl VerifiedRecoveryBaseline {
    pub fn repository(&self) -> &crate::git::RepositoryId {
        &self.repository
    }
    pub fn production(&self) -> &Revision {
        &self.production
    }
    pub fn targets(&self) -> &[ProductionTargetBaseline] {
        &self.targets
    }
    pub fn contract_digest(&self) -> &str {
        &self.contract_digest
    }
    pub fn observed_at(&self) -> DateTime<Utc> {
        self.observed_at
    }
    pub fn coherent(&self) -> bool {
        self.targets
            .iter()
            .all(|target| target.source == self.production)
    }
    pub fn observations(&self) -> Vec<crate::release::BaselineObservation> {
        use crate::release::{BaselineObservation, ResourceKey};
        let mut observations = vec![BaselineObservation {
            resource: ResourceKey::Repository {
                repository: self.repository.clone(),
            },
            production_commit: self.production.commit.clone(),
            deployed_commit: None,
            artifact_digest: None,
            provider_record: format!(
                "github-repository:{}/{}@{}",
                self.repository.hostname, self.repository.id, self.production.commit
            ),
        }];
        observations.extend(self.targets.iter().map(|target| BaselineObservation {
            resource: ResourceKey::Target {
                provider: target.provider.clone(),
                id: target.target.clone(),
            },
            production_commit: self.production.commit.clone(),
            deployed_commit: Some(target.source.commit.clone()),
            artifact_digest: Some(target.artifact_digest.clone()),
            provider_record: target.provider_record.clone(),
        }));
        observations
    }
}
#[derive(Clone, Debug)]
pub struct VerifiedTargetDeployment {
    record: TargetRecord,
    observed_at: DateTime<Utc>,
}
impl VerifiedTargetDeployment {
    pub fn digest(&self) -> Result<String> {
        canonical_digest(1, &self.record)
    }
    pub fn operation_id(&self) -> Uuid {
        self.record.challenge.operation
    }
    pub fn dispatch_request(&self) -> Uuid {
        self.record.challenge.dispatch
    }
    pub fn actor_account_id(&self) -> String {
        self.record.run.actor_id.to_string()
    }
    pub fn origin_digest(&self) -> Result<String> {
        canonical_digest(1, &self.record.challenge)
    }
    pub fn target(&self) -> &ProductionTargetBaseline {
        &self.record.target
    }
    pub fn deployment_id(&self) -> u64 {
        self.record.facts.deployment_id
    }
    pub fn run_id(&self) -> u64 {
        self.record.run.id
    }
    pub fn run_attempt(&self) -> u64 {
        self.record.run.attempt
    }
    pub fn observed_at(&self) -> DateTime<Utc> {
        self.observed_at
    }
    pub fn contract_digest(&self) -> &str {
        &self.record.challenge.contract_digest
    }
}
#[derive(Clone, Debug)]
pub struct TargetDeploymentChallenge {
    signed: Signed<ChallengeRecord>,
}
impl TargetDeploymentChallenge {
    pub fn operation_id(&self) -> Uuid {
        self.signed.payload.operation
    }
    pub fn dispatch_request(&self) -> Uuid {
        self.signed.payload.dispatch
    }
    pub fn challenge(&self) -> Uuid {
        self.signed.payload.challenge
    }
    pub fn source(&self) -> &Revision {
        &self.signed.payload.source
    }
    pub fn artifact_name(&self, attempt: u64) -> String {
        format!(
            "kegari-deployment-{}-{}-{}",
            self.operation_id(),
            attempt,
            self.signed.payload.environment_id
        )
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChallengeRecord {
    schema_version: u32,
    runtime: Uuid,
    operation: Uuid,
    dispatch: Uuid,
    challenge: Uuid,
    admission_digest: String,
    contract_digest: String,
    repository_id: u64,
    step: String,
    target: String,
    environment_id: u64,
    source: Revision,
    created_at: DateTime<Utc>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeTargetFacts {
    repository_id: u64,
    environment_id: u64,
    environment_name: String,
    environment_policy_digest: String,
    deployment_id: u64,
    deployment_creator: u64,
    deployment_created: DateTime<Utc>,
    deployment_updated: DateTime<Utc>,
    status_id: u64,
    status_creator: u64,
    status_created: DateTime<Utc>,
    status_updated: DateTime<Utc>,
    source: Revision,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetRecord {
    challenge: ChallengeRecord,
    dispatch_payload: String,
    facts: NativeTargetFacts,
    run: WorkflowRun,
    jobs_digest: String,
    artifact_id: u64,
    archive_digest: String,
    manifest_digest: String,
    proofs: Vec<VerifiedHostedJob>,
    target: ProductionTargetBaseline,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetIndex {
    contract_digest: String,
    step: String,
    proof_digest: String,
    proof_file: String,
}

/// Untrusted, bounded workflow output. Raw JWTs never implement Debug or Serialize.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeploymentManifest {
    schema_version: u32,
    runtime_id: Uuid,
    phase_operation: Uuid,
    dispatch_request: Uuid,
    challenge: Uuid,
    repository_id: u64,
    step_id: String,
    target_id: String,
    environment_id: u64,
    deployment_id: u64,
    run_id: u64,
    run_attempt: u64,
    source: Revision,
    #[serde(deserialize_with = "unique_outputs")]
    outputs: BTreeMap<String, String>,
    artifact_digest: String,
    proofs: Vec<JobToken>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JobToken {
    check_run_id: u64,
    token: String,
}

impl ProductionObserver {
    /// Actual canonical main observation for a finite release step. Targets may
    /// temporarily differ after this plan's merge, so this does not manufacture
    /// a coherent full deployment baseline.
    pub async fn observe_main(&self, contract: &RepositoryContract) -> Result<Revision> {
        let first = self.main(contract).await?;
        if self.main(contract).await? != first {
            return Err(blocked("main changed during release source observation"));
        }
        Ok(first)
    }
    /// Establish an unchanged release target from the complete current target
    /// vector. This is a read, never an instruction to merge or redeploy it.
    pub async fn verify_existing_target(
        &self,
        contract: &RepositoryContract,
        target: &str,
        source: &Revision,
        artifact: &str,
    ) -> Result<VerifiedTargetDeployment> {
        source.validate()?;
        crate::evidence::validate_digest(artifact)?;
        let Deployment::Steps { steps } = contract.configuration.deployment_policy()? else {
            return Err(blocked(
                "existing target verification needs a configured deployment target",
            ));
        };
        let matching: Vec<_> = steps.iter().filter(|s| s.target_id == target).collect();
        let [configured] = matching.as_slice() else {
            return Err(blocked("existing target is missing or ambiguous"));
        };
        let baseline = self.observe_recovery(contract).await?;
        if baseline.production != *source
            || !baseline.targets.iter().any(|current| {
                current.target == target
                    && current.source == *source
                    && current.artifact_digest == artifact
            })
        {
            return Err(blocked(
                "existing target no longer serves the accepted production source/artifact",
            ));
        }
        let proof = self.recover_target(contract, &configured.id).await?;
        if proof.target().source != *source
            || proof.target().artifact_digest != artifact
            || !baseline.targets.iter().any(|target| {
                let mut current = proof.target().clone();
                // Observation time changes on each complete read; every native
                // source, artifact, target and provider record must stay exact.
                current.observed_at = target.observed_at;
                target == &current
            })
        {
            return Err(blocked(
                "existing target changed during complete baseline observation",
            ));
        }
        Ok(proof)
    }
    /// Recover only an already persisted exact challenge. This does not admit,
    /// dispatch, or renew a deployment operation.
    pub fn recover_challenge(
        &self,
        contract: &RepositoryContract,
        operation: Uuid,
        step_id: &str,
    ) -> Result<TargetDeploymentChallenge> {
        step(contract, step_id)?;
        let index=format!("deployment-admission-{}.json", canonical_digest(1,
            &json!({"runtime":self.control.snapshot()?.intent.runtime_id,"operation":operation,"step":step_id}))?
            .rsplit(':').next().unwrap());
        let challenge = TargetDeploymentChallenge {
            signed: self.records.read(&index)?,
        };
        self.check_challenge(contract, &challenge)?;
        if challenge.operation_id() != operation {
            return Err(blocked("deployment challenge operation differs"));
        }
        Ok(challenge)
    }
    pub fn new(
        github: GithubClient,
        control: Arc<ControlHost>,
        key: Arc<AttestationKey>,
        records: PrivateDir,
        current: CurrentAuthority,
    ) -> Result<Self> {
        PrivateDir::open(records.path())?;
        Ok(Self {
            github,
            control,
            key,
            records,
            current,
            successor: None,
        })
    }
    /// Retain an authenticated, fully observed predecessor for this preparation.
    /// A raw recovery identifier is never sufficient to select this path.
    pub fn with_successor(
        &self,
        successor: Arc<crate::runtime_delivery::KnownPredecessorBaseline>,
    ) -> Result<Self> {
        successor.revalidate()?;
        (self.current)(successor.gate())?;
        Ok(Self {
            github: self.github.clone(),
            control: self.control.clone(),
            key: self.key.clone(),
            records: PrivateDir::open(self.records.path())?,
            current: self.current.clone(),
            successor: Some(successor),
        })
    }
    async fn observe_successor(
        &self,
        contract: &RepositoryContract,
        successor: &crate::runtime_delivery::KnownPredecessorBaseline,
    ) -> Result<VerifiedProductionBaseline> {
        successor.revalidate()?;
        (self.current)(successor.gate())?;
        let expected = successor.baseline(&contract.repository)?;
        let current = self.observe_recovery(contract).await?;
        if current.contract_digest != expected.contract_digest
            || current.production != expected.production
            || target_identity(&current.targets)? != target_identity(&expected.targets)?
        {
            return Err(blocked(
                "production changed after known predecessor observation",
            ));
        }
        successor.revalidate()?;
        (self.current)(successor.gate())?;
        let sources: BTreeMap<_, _> = current
            .targets
            .iter()
            .map(|target| (target.target.clone(), target.source.commit.clone()))
            .collect();
        let artifacts: BTreeMap<_, _> = current
            .targets
            .iter()
            .map(|target| (target.target.clone(), target.artifact_digest.clone()))
            .collect();
        let common = |values: &BTreeMap<String, String>| {
            values
                .values()
                .next()
                .filter(|first| values.values().all(|value| value == *first))
                .cloned()
        };
        let baseline = ProductionBaseline {
            repository: current.repository,
            production: current.production,
            deployed_revision: common(&sources),
            target_revisions: sources,
            deployed_artifact: common(&artifacts),
            target_artifacts: artifacts,
            deployment_free: matches!(contract.configuration.deployment_policy()?, Deployment::None(value) if value == "none"),
            successor_recovery_operation: Some(successor.predecessor()),
        };
        baseline.validate()?;
        Ok(VerifiedProductionBaseline {
            owned_targets: vec![],
            baseline,
            targets: current.targets,
            contract_digest: current.contract_digest,
            observed_at: current.observed_at,
        })
    }
    /// Capture an exact source challenge before the permitted configured workflow
    /// dispatch. This function performs reads and private bookkeeping only.
    pub async fn prepare_target(
        &self,
        contract: &RepositoryContract,
        admission: &DeliveryAdmission,
        permit: &Permit,
        step_id: &str,
        source: Revision,
    ) -> Result<TargetDeploymentChallenge> {
        let step = step(contract, step_id)?;
        self.check_admission(contract, admission, permit)?;
        source.validate()?;
        if source.tree != admission.scope().revision.tested_tree
            || self.main(contract).await? != source
        {
            return Err(blocked(
                "deployment source is not the admitted tree at current main",
            ));
        }
        let environment_id = environment_id(step)?;
        self.github
            .environment_by_id(&contract.provider, environment_id)
            .await?;
        let record = ChallengeRecord {
            schema_version: 1,
            runtime: admission.scope().runtime_id,
            operation: admission.operation_id(),
            dispatch: Uuid::new_v4(),
            challenge: Uuid::new_v4(),
            admission_digest: permit.scope_digest.clone(),
            contract_digest: contract.digest()?,
            repository_id: contract.provider.id,
            step: step.id.clone(),
            target: step.target_id.clone(),
            environment_id,
            source,
            created_at: Utc::now(),
        };
        let mut signed = self.key.sign("deployment_challenge", &record)?;
        let index = format!(
            "deployment-admission-{}.json",
            canonical_digest(
                1,
                &json!({
                    "runtime":record.runtime,"operation":record.operation,"step":record.step
                })
            )?
            .rsplit(':')
            .next()
            .unwrap()
        );
        self.control.validate_permit(permit, || {
            (self.current)(admission.fresh_gate())?;
            let _lock = self.records.section_lock("deployment-challenges.lock")?;
            match self.records.read::<Signed<ChallengeRecord>>(&index) {
                Ok(prior) => {
                    self.key.verify("deployment_challenge", &prior)?;
                    let mut normalized = record.clone();
                    normalized.dispatch = prior.payload.dispatch;
                    normalized.challenge = prior.payload.challenge;
                    normalized.created_at = prior.payload.created_at;
                    if normalized != prior.payload {
                        return Err(blocked("deployment step was already prepared with another exact source or contract"));
                    }
                    signed = prior;
                }
                Err(error) if error.code == "record_missing" => self.records.write(&index, &signed)?,
                Err(error) => return Err(error),
            }
            // A crash between index and challenge writes is repaired with the
            // same signed nonce/dispatch, never a fresh external request ID.
            let leaf = challenge_file(signed.payload.challenge);
            match self.records.read::<Signed<ChallengeRecord>>(&leaf) {
                Ok(prior) if prior == signed => Ok(()),
                Ok(_) => Err(blocked("retained deployment challenge was replaced")),
                Err(error) if error.code == "record_missing" => self.records.write(&leaf, &signed),
                Err(error) => Err(error),
            }
        })?;
        Ok(TargetDeploymentChallenge { signed })
    }
    /// Host-only sealed mutation vocabulary; the caller must transmit through
    /// ProviderBroker with the challenge's request ID and the same live permit.
    pub fn dispatch_command(
        &self,
        contract: &RepositoryContract,
        challenge: &TargetDeploymentChallenge,
    ) -> Result<ProviderMutation> {
        self.check_challenge(contract, challenge)?;
        let c = &challenge.signed.payload;
        let route = &step(contract, &c.step)?.workflow;
        let (locator, reference) = route
            .proof
            .workflow_ref
            .rsplit_once('@')
            .ok_or_else(|| blocked("deployment workflow ref is missing"))?;
        let path = locator
            .splitn(3, '/')
            .nth(2)
            .ok_or_else(|| blocked("deployment workflow path is missing"))?;
        let reference = reference
            .strip_prefix("refs/")
            .filter(|r| r.starts_with("heads/") || r.starts_with("tags/"))
            .ok_or_else(|| {
                blocked("deployment dispatch requires a pinned branch/tag workflow ref")
            })?;
        Ok(ProviderMutation::GithubDispatch {
            repository: contract.provider.clone(),
            workflow_id: route.workflow_id,
            workflow_path: path.into(),
            reference: reference.into(),
            workflow_sha: route.workflow_revision.clone(),
            source_sha: c.source.commit.clone(),
            expected_production: Some(c.source.clone()),
            source_input: route.source_input.clone(),
            operation_input: route.operation_input.clone(),
            inputs: BTreeMap::from([
                ("kegari_runtime_id".into(), c.runtime.to_string()),
                ("kegari_phase_operation".into(), c.operation.to_string()),
                ("kegari_challenge".into(), c.challenge.to_string()),
                ("kegari_step_id".into(), c.step.clone()),
                ("kegari_target_id".into(), c.target.clone()),
                ("kegari_environment_id".into(), c.environment_id.to_string()),
                ("kegari_source_tree".into(), c.source.tree.clone()),
            ]),
        })
    }
    /// Mint from a complete artifact download and a currently valid signature;
    /// neither caller-provided VerifiedHostedJob nor deployment payload is trusted.
    pub async fn observe_target(
        &self,
        contract: &RepositoryContract,
        admission: &DeliveryAdmission,
        permit: &Permit,
        challenge: &TargetDeploymentChallenge,
        dispatch: &Receipt,
        verifier: &HostedProofVerifier,
    ) -> Result<VerifiedTargetDeployment> {
        self.check_admission(contract, admission, permit)?;
        self.check_challenge(contract, challenge)?;
        let c = &challenge.signed.payload;
        if c.operation != admission.operation_id() || c.admission_digest != permit.scope_digest {
            return Err(blocked(
                "target challenge belongs to another admitted phase",
            ));
        }
        // Re-observing the same owned dispatch retains the original immutable
        // proof and its original verification time; it does not reuse a JWT.
        if let Some(mut record) =
            self.retained_dispatch_proof(contract, &c.step, c.challenge, c.dispatch)?
        {
            let actual = self.retained_proof_receipt(contract, &record)?;
            if dispatch.request_id != actual.request_id
                || dispatch.operation_id != actual.operation_id
                || dispatch.runtime_id != actual.runtime_id
                || dispatch.payload_digest != actual.payload_digest
            {
                return Err(blocked(
                    "supplied dispatch differs from the original immutable target receipt",
                ));
            }
            self.reobserve_record(contract, &record).await?;
            self.control.validate_permit(permit, || {
                (self.current)(admission.fresh_gate())?;
                let _lock = self.records.section_lock("deployment-proofs.lock")?;
                record = self.persist_target_proof(contract, record.clone())?;
                Ok(())
            })?;
            return Ok(VerifiedTargetDeployment {
                record,
                observed_at: Utc::now(),
            });
        }
        // A new proof still requires the original successful dispatch ACK.
        // A reconciled historical effect alone cannot mint fresh evidence.
        let dispatch = self.dispatch_receipt(contract, c, dispatch)?;
        let run_id = number(
            dispatch
                .remote_ids
                .get("workflow_run_id")
                .map(String::as_str)
                .ok_or_else(|| blocked("owned dispatch has no exact observed run identity"))?,
        )?;
        let step = step(contract, &c.step)?;
        let run = self
            .github
            .workflow_run(&contract.provider, run_id, None)
            .await?;
        let jobs = self.verify_run(contract, step, &run).await?;
        let name = challenge.artifact_name(run.attempt);
        let artifacts = self.github.artifacts(&contract.provider, run.id).await?;
        let matches: Vec<_> = artifacts
            .iter()
            .filter(|a| a.data.get("name").and_then(Value::as_str) == Some(name.as_str()))
            .collect();
        if matches.len() != 1 {
            return Err(blocked(
                "owned attempt lacks one unique deployment proof artifact",
            ));
        }
        let archive = self
            .github
            .artifact_archive(&contract.provider, matches[0].id, run.id, &run.head_sha)
            .await?;
        let (manifest, manifest_digest) = read_manifest(&archive.bytes)?;
        let facts = self.current_target(contract, step).await?;
        check_manifest(&manifest, c, &facts, &run, step)?;
        let context = deployment_context_digest(
            contract.provider.id,
            &c.target,
            c.environment_id,
            facts.deployment_id,
            &c.source,
            &manifest.artifact_digest,
        )?;
        let mut checks = BTreeSet::new();
        let mut proofs = Vec::new();
        let now = Utc::now();
        for proof in manifest.proofs {
            if !checks.insert(proof.check_run_id)
                || !jobs.iter().any(|j| j.check_run_id == proof.check_run_id)
            {
                return Err(blocked(
                    "deployment proof contains a duplicate or unrelated exact job",
                ));
            }
            let expected = HostedProofExpectation {
                context_digest: Some(context.clone()),
                runtime_id: c.runtime,
                operation_id: c.operation,
                challenge: c.challenge,
                repository_id: contract.provider.id,
                repository_owner_id: contract.provider.owner_id,
                run_id: run.id,
                run_attempt: run.attempt,
                check_run_id: proof.check_run_id,
                actor_id: run.actor_id,
                trigger_sha: run.head_sha.clone(),
                tested_source_sha: c.source.commit.clone(),
                workflow_ref: step.workflow.proof.workflow_ref.clone(),
                workflow_sha: step.workflow.proof.workflow_sha.clone(),
                job_workflow_ref: step.workflow.proof.job_workflow_ref.clone(),
                job_workflow_sha: step.workflow.proof.job_workflow_sha.clone(),
                event_name: step.workflow.proof.event_name.clone(),
            };
            proofs.push(verifier.verify_and_record(&proof.token, &expected, now, &self.records)?);
        }
        if checks != jobs.iter().map(|j| j.check_run_id).collect() {
            return Err(blocked(
                "deployment proof does not cover the complete successful job inventory",
            ));
        }
        let target = ProductionTargetBaseline {
            provider: "github".into(),
            target: c.target.clone(),
            repository: contract.repository.clone(),
            source: c.source.clone(),
            artifact_digest: manifest.artifact_digest,
            provider_record: format!(
                "github-deployment:{}/{}/{}",
                contract.repository.hostname, contract.provider.id, facts.deployment_id
            ),
            observed_at: now,
        };
        let mut record = TargetRecord {
            challenge: c.clone(),
            dispatch_payload: dispatch.payload_digest,
            facts,
            run,
            jobs_digest: canonical_digest(1, &jobs)?,
            artifact_id: archive.artifact_id,
            archive_digest: archive.archive_digest,
            manifest_digest,
            proofs,
            target,
        };
        self.reobserve_record(contract, &record).await?;
        self.control.validate_permit(permit, || {
            (self.current)(admission.fresh_gate())?;
            let _lock = self.records.section_lock("deployment-proofs.lock")?;
            record = self.persist_target_proof(contract, record.clone())?;
            Ok(())
        })?;
        Ok(VerifiedTargetDeployment {
            record,
            observed_at: Utc::now(),
        })
    }
    /// Historical proof recovery validates the protected HMAC record and fresh
    /// native provider facts; expired bearer tokens are never accepted anew.
    pub async fn recover_target(
        &self,
        contract: &RepositoryContract,
        step_id: &str,
    ) -> Result<VerifiedTargetDeployment> {
        let record = self.retained_target_record(contract, step_id).await?;
        self.reobserve_record(contract, &record).await?;
        Ok(VerifiedTargetDeployment {
            record,
            observed_at: Utc::now(),
        })
    }
    /// Observe every configured environment without hiding a known main/target
    /// mismatch. Missing or uncertain native proof still fails closed. This is
    /// read-only and grants no permission to deploy the old source.
    pub async fn observe_recovery(
        &self,
        contract: &RepositoryContract,
    ) -> Result<VerifiedRecoveryBaseline> {
        let deployment = contract.configuration.deployment_policy()?;
        let production = self.main(contract).await?;
        let mut targets = Vec::new();
        let mut ids = BTreeSet::new();
        let mut environments = BTreeSet::new();
        match deployment {
            Deployment::None(value) if value == "none" => {}
            Deployment::Steps { steps } if !steps.is_empty() => {
                for step in steps {
                    if !ids.insert(step.target_id.clone())
                        || !environments.insert(environment_id(step)?)
                    {
                        return Err(blocked(
                            "configured recovery target/environment inventory is ambiguous",
                        ));
                    }
                    let record = self.retained_target_record(contract, &step.id).await?;
                    self.reobserve_target_record(contract, &record).await?;
                    let mut target = record.target;
                    target.observed_at = Utc::now();
                    targets.push(target);
                }
            }
            _ => {
                return Err(blocked(
                    "recovery requires complete configured deployment behavior",
                ));
            }
        }
        if self.main(contract).await? != production {
            return Err(blocked("main changed during recovery baseline observation"));
        }
        Ok(VerifiedRecoveryBaseline {
            repository: contract.repository.clone(),
            production,
            targets,
            contract_digest: contract.digest()?,
            observed_at: Utc::now(),
        })
    }
    /// All configured targets are mandatory. Unknown, failed, divergent or
    /// unattested targets block ordinary assembly instead of choosing one artifact.
    pub async fn observe(
        &self,
        contract: &RepositoryContract,
    ) -> Result<VerifiedProductionBaseline> {
        let deployment = contract.configuration.deployment_policy()?;
        if let Some(successor) = &self.successor {
            return self.observe_successor(contract, successor).await;
        }
        let contract_digest = contract.digest()?;
        let production = self.main(contract).await?;
        let mut targets = Vec::new();
        let mut target_artifacts = BTreeMap::new();
        let mut owned_targets = Vec::new();
        let deployment_free = match deployment {
            Deployment::None(value) if value == "none" => true,
            Deployment::Steps { steps } if !steps.is_empty() => {
                let mut environments = BTreeSet::new();
                for s in steps {
                    if !environments.insert(environment_id(s)?)
                        || target_artifacts.contains_key(&s.target_id)
                    {
                        return Err(blocked(
                            "configured deployment targets/environments are ambiguous",
                        ));
                    }
                    let observed = self.recover_target(contract, &s.id).await?;
                    if observed.target().source != production {
                        return Err(blocked("deployed target and current main diverge"));
                    }
                    target_artifacts.insert(
                        s.target_id.clone(),
                        observed.target().artifact_digest.clone(),
                    );
                    let mut target = observed.target().clone();
                    target.observed_at = observed.observed_at();
                    targets.push(target);
                    owned_targets.push(observed);
                }
                false
            }
            _ => {
                return Err(blocked(
                    "repository deployment behavior is not completely configured",
                ));
            }
        };
        if self.main(contract).await? != production {
            return Err(blocked(
                "main changed during complete production observation",
            ));
        }
        let common = target_artifacts
            .values()
            .next()
            .filter(|first| target_artifacts.values().all(|v| v == *first))
            .cloned();
        let baseline = ProductionBaseline {
            repository: contract.repository.clone(),
            production: production.clone(),
            deployed_revision: (!deployment_free).then_some(production.commit.clone()),
            target_revisions: targets
                .iter()
                .map(|target| (target.target.clone(), target.source.commit.clone()))
                .collect(),
            deployed_artifact: common,
            target_artifacts,
            deployment_free,
            successor_recovery_operation: None,
        };
        baseline.validate()?;
        Ok(VerifiedProductionBaseline {
            owned_targets,
            baseline,
            targets,
            contract_digest,
            observed_at: Utc::now(),
        })
    }
    pub async fn reobserve(
        &self,
        contract: &RepositoryContract,
        prior: &VerifiedProductionBaseline,
    ) -> Result<VerifiedProductionBaseline> {
        let current = self.observe(contract).await?;
        if prior.contract_digest != current.contract_digest
            || prior.baseline != current.baseline
            || target_identity(&prior.targets)? != target_identity(&current.targets)?
        {
            return Err(blocked(
                "production baseline changed after candidate admission",
            ));
        }
        Ok(current)
    }
    fn check_admission(
        &self,
        contract: &RepositoryContract,
        admission: &DeliveryAdmission,
        permit: &Permit,
    ) -> Result<()> {
        admission.verify_bound_permit(permit)?;
        let scope = admission.scope();
        if !matches!(
            scope.phase,
            DeliveryPhase::Release | DeliveryPhase::Recovery
        ) || scope.repository != contract.repository
            || scope.contract_digest != contract.digest()?
            || admission.operation_id() != permit.operation_id
        {
            return Err(blocked(
                "deployment observation requires the exact admitted release/recovery scope",
            ));
        }
        self.control
            .validate_permit(permit, || (self.current)(admission.fresh_gate()))
    }
    fn check_challenge(
        &self,
        contract: &RepositoryContract,
        challenge: &TargetDeploymentChallenge,
    ) -> Result<()> {
        self.key.verify("deployment_challenge", &challenge.signed)?;
        let c = &challenge.signed.payload;
        let stored: Signed<ChallengeRecord> = self.records.read(&challenge_file(c.challenge))?;
        self.key.verify("deployment_challenge", &stored)?;
        let s = step(contract, &c.step)?;
        if stored != challenge.signed
            || c.schema_version != 1
            || c.contract_digest != contract.digest()?
            || c.repository_id != contract.provider.id
            || c.target != s.target_id
            || c.environment_id != environment_id(s)?
        {
            return Err(blocked(
                "deployment challenge does not bind this exact current contract",
            ));
        }
        Ok(())
    }
    fn dispatch_receipt(
        &self,
        contract: &RepositoryContract,
        c: &ChallengeRecord,
        supplied: &Receipt,
    ) -> Result<Receipt> {
        let actual = self.control.operation(c.dispatch)?;
        let route = &step(contract, &c.step)?.workflow;
        let result = actual
            .observed_result
            .as_ref()
            .ok_or_else(|| blocked("deployment dispatch outcome is unknown"))?;
        if actual.request_id != c.dispatch
            || actual.operation_id != c.dispatch
            || actual.runtime_id != c.runtime
            || actual.kind != "github_dispatch"
            || actual.payload_digest != supplied.payload_digest
            || actual.request_id != supplied.request_id
            || !matches!(
                actual.phase,
                ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
            )
            || actual.expected.get("admitted_scope") != Some(&c.admission_digest)
            || result.get("repository_id").and_then(Value::as_u64) != Some(contract.provider.id)
            || result.get("workflow_id").and_then(Value::as_u64) != Some(route.workflow_id)
            || result.get("workflow_sha").and_then(Value::as_str)
                != Some(route.workflow_revision.as_str())
            || result.get("source_sha").and_then(Value::as_str) != Some(c.source.commit.as_str())
            || result.get("operation_id").and_then(Value::as_str)
                != Some(c.dispatch.to_string().as_str())
        {
            return Err(blocked(
                "deployment dispatch lacks exact durable owned scope/source lineage",
            ));
        }
        Ok(actual)
    }
    async fn main(&self, contract: &RepositoryContract) -> Result<Revision> {
        contract.digest()?;
        self.github.identity().await?;
        let commit = self
            .github
            .reference(&contract.provider, "heads/main")
            .await?;
        let object = self.github.commit(&contract.provider, &commit).await?;
        let revision = Revision {
            commit,
            tree: text(&object, "/tree/sha")?.into(),
        };
        revision.validate()?;
        Ok(revision)
    }
    async fn current_target(
        &self,
        contract: &RepositoryContract,
        step: &DeploymentStep,
    ) -> Result<NativeTargetFacts> {
        let environment = self
            .github
            .environment_by_id(&contract.provider, environment_id(step)?)
            .await?;
        let deployments = self
            .github
            .environment_deployments(&contract.provider, &environment)
            .await?;
        let mut selected: Option<(DateTime<Utc>, NativeTargetFacts)> = None;
        let mut failures = Vec::new();
        for deployment in &deployments {
            if deployment.repository_id != contract.provider.id
                || deployment.id == 0
                || text(&deployment.data, "/environment")? != environment.name
                || !boolean(&deployment.data, "/production_environment")?
                || boolean(&deployment.data, "/transient_environment")?
            {
                return Err(blocked(
                    "deployment target inventory is not the configured production environment",
                ));
            }
            let created = date(&deployment.data, "/created_at")?;
            let statuses = self
                .github
                .deployment_statuses(&contract.provider, deployment.id)
                .await?;
            let status = newest(&statuses)?;
            let state = text(&status.data, "/state")?;
            let status_time = date(&status.data, "/created_at")?;
            if !matches!(
                state,
                "success" | "inactive" | "error" | "failure" | "in_progress" | "queued" | "pending"
            ) {
                return Err(blocked("deployment status is unknown"));
            }
            // Current state can be changed by a later status on an older deployment.
            // Earlier known failures may be superseded by this later verified success.
            // Concurrent pending work or multiple active successes stay ambiguous.
            if state == "inactive" {
                continue;
            }
            if matches!(state, "error" | "failure") {
                failures.push(status_time);
                continue;
            }
            if state != "success" {
                return Err(blocked("a target deployment remains pending or unknown"));
            }
            if text(&status.data, "/environment")? != environment.name {
                return Err(blocked("deployment status environment differs"));
            }
            let commit = text(&deployment.data, "/sha")?.to_owned();
            let object = self.github.commit(&contract.provider, &commit).await?;
            let source = Revision {
                commit,
                tree: text(&object, "/tree/sha")?.into(),
            };
            source.validate()?;
            let facts = NativeTargetFacts {
                repository_id: contract.provider.id,
                environment_id: environment.id,
                environment_name: environment.name.clone(),
                environment_policy_digest: canonical_digest(
                    1,
                    &json!({"rules":environment.protection_rules,"policy":environment.deployment_branch_policy}),
                )?,
                deployment_id: deployment.id,
                deployment_creator: positive(&deployment.data, "/creator/id")?,
                deployment_created: created,
                deployment_updated: date(&deployment.data, "/updated_at")?,
                status_id: status.id,
                status_creator: positive(&status.data, "/creator/id")?,
                status_created: date(&status.data, "/created_at")?,
                status_updated: date(&status.data, "/updated_at")?,
                source,
            };
            if selected.replace((status_time, facts)).is_some() {
                return Err(blocked(
                    "multiple non-inactive successful deployments make current target ambiguous",
                ));
            }
        }
        let (created, facts) = selected
            .ok_or_else(|| blocked("configured target has no current successful deployment"))?;
        if failures.iter().any(|failed| *failed >= created) {
            return Err(blocked(
                "a newer failed deployment makes the target outcome unresolved",
            ));
        }
        Ok(facts)
    }
    async fn verify_run(
        &self,
        contract: &RepositoryContract,
        step: &DeploymentStep,
        run: &WorkflowRun,
    ) -> Result<Vec<WorkflowJob>> {
        let actor = self.github.identity().await?;
        let route = &step.workflow;
        route.proof.validate()?;
        let locator = route.proof.workflow_ref.split('@').next().unwrap_or("");
        let workflow_path = locator.splitn(3, '/').nth(2).unwrap_or("");
        if run.repository_id != contract.provider.id
            || run.attempt != 1
            || run.workflow_id != route.workflow_id
            || run.head_sha != route.workflow_revision
            || run.head_sha != route.proof.workflow_sha
            || run.event != "workflow_dispatch"
            || run.event != route.proof.event_name
            || run.workflow_path != workflow_path
            || run.actor_id.to_string() != actor.account_id
            || run.triggering_actor_id != run.actor_id
            || run.status != "completed"
            || run.conclusion.as_deref() != Some("success")
        {
            return Err(blocked(
                "owned pinned deployment run is not a successful exact attempt",
            ));
        }
        let mut jobs = self
            .github
            .jobs(&contract.provider, run.id, run.attempt)
            .await?;
        jobs.sort_by_key(|job| job.id);
        let mut checks = BTreeSet::new();
        if jobs.is_empty()
            || jobs.len() > 256
            || jobs.iter().any(|j| {
                j.run_id != run.id
                    || j.attempt != run.attempt
                    || j.head_sha != run.head_sha
                    || j.status != "completed"
                    || j.conclusion.as_deref() != Some("success")
                    || !checks.insert(j.check_run_id)
            })
        {
            return Err(blocked("deployment jobs are incomplete or unsuccessful"));
        }
        Ok(jobs)
    }
    async fn reobserve_record(
        &self,
        contract: &RepositoryContract,
        record: &TargetRecord,
    ) -> Result<()> {
        self.reobserve_target_record(contract, record).await?;
        if self.main(contract).await? != record.challenge.source {
            return Err(blocked(
                "current main differs from retained successful deployment source",
            ));
        }
        Ok(())
    }
    async fn reobserve_target_record(
        &self,
        contract: &RepositoryContract,
        record: &TargetRecord,
    ) -> Result<()> {
        self.check_challenge(
            contract,
            &TargetDeploymentChallenge {
                signed: self.key.sign("deployment_challenge", &record.challenge)?,
            },
        )?;
        let receipt = self.retained_proof_receipt(contract, record)?;
        if receipt.payload_digest != record.dispatch_payload
            || record.target.source != record.challenge.source
            || record.target.target != record.challenge.target
            || record.target.repository != contract.repository
        {
            return Err(blocked(
                "retained deployment proof lost exact scope lineage",
            ));
        }
        let step = step(contract, &record.challenge.step)?;
        let facts = self.current_target(contract, step).await?;
        let run = self
            .github
            .workflow_run(&contract.provider, record.run.id, None)
            .await?;
        let jobs = self.verify_run(contract, step, &run).await?;
        if facts != record.facts
            || canonical_digest(1, &run)? != canonical_digest(1, &record.run)?
            || canonical_digest(1, &jobs)? != record.jobs_digest
        {
            return Err(blocked(
                "current deployment/run differs from retained successful proof",
            ));
        }
        Ok(())
    }
}

/// Host/workflow shared audience component. The workflow computes this only after
/// verifying the actual target deployment and deployed artifact bytes/provenance.
pub fn deployment_context_digest(
    repository_id: u64,
    target: &str,
    environment_id: u64,
    deployment_id: u64,
    source: &Revision,
    artifact_digest: &str,
) -> Result<String> {
    source.validate()?;
    crate::evidence::validate_digest(artifact_digest)?;
    if [repository_id, environment_id, deployment_id].contains(&0) || target.trim().is_empty() {
        return Err(blocked("deployment context lacks exact target identities"));
    }
    canonical_digest(
        1,
        &json!({"purpose":"deployment_target","repository_id":repository_id,
        "target_id":target,"environment_id":environment_id,"deployment_id":deployment_id,
        "source":source,"artifact_digest":artifact_digest}),
    )
}
fn step<'a>(contract: &'a RepositoryContract, id: &str) -> Result<&'a DeploymentStep> {
    contract.digest()?;
    let Deployment::Steps { steps } = contract.configuration.deployment_policy()? else {
        return Err(blocked("repository has no deployment steps"));
    };
    let matches: Vec<_> = steps.iter().filter(|s| s.id == id).collect();
    if matches.len() != 1 {
        return Err(blocked("deployment step identity is absent or ambiguous"));
    }
    let selected = matches[0];
    selected.workflow.proof.validate()?;
    if selected.target_id.trim().is_empty()
        || selected.workflow.workflow_revision != selected.workflow.proof.workflow_sha
    {
        return Err(blocked(
            "deployment step lacks a pinned exact workflow contract",
        ));
    }
    Ok(selected)
}
fn environment_id(step: &DeploymentStep) -> Result<u64> {
    number(&step.environment_id)
}
fn number(value: &str) -> Result<u64> {
    value
        .parse::<u64>()
        .ok()
        .filter(|n| *n > 0 && n.to_string() == value)
        .ok_or_else(|| blocked("canonical numeric provider identity is invalid"))
}
fn text<'a>(value: &'a Value, path: &str) -> Result<&'a str> {
    value
        .pointer(path)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| blocked("required deployment field is missing"))
}
fn positive(value: &Value, path: &str) -> Result<u64> {
    value
        .pointer(path)
        .and_then(Value::as_u64)
        .filter(|v| *v > 0)
        .ok_or_else(|| blocked("required numeric deployment field is missing"))
}
fn boolean(value: &Value, path: &str) -> Result<bool> {
    value
        .pointer(path)
        .and_then(Value::as_bool)
        .ok_or_else(|| blocked("required deployment flag is missing"))
}
fn date(value: &Value, path: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text(value, path)?)
        .map(|v| v.with_timezone(&Utc))
        .map_err(|_| blocked("deployment timestamp is malformed"))
}
fn newest(records: &[GithubRecord]) -> Result<&GithubRecord> {
    let mut ordered = records
        .iter()
        .map(|r| Ok((date(&r.data, "/created_at")?, r)))
        .collect::<Result<Vec<_>>>()?;
    ordered.sort_by_key(|(at, _)| *at);
    let Some((last, record)) = ordered.last() else {
        return Err(blocked("deployment has no observed status"));
    };
    if ordered.iter().filter(|(at, _)| at == last).count() != 1 {
        return Err(blocked("latest deployment status timestamp is ambiguous"));
    }
    Ok(record)
}
fn challenge_file(id: Uuid) -> String {
    format!("deployment-challenge-{id}.json")
}
fn index_file(contract: &RepositoryContract, step: &str) -> Result<String> {
    Ok(format!(
        "target-index-{}.json",
        canonical_digest(1, &json!({"repository":contract.repository,"step":step}))?
            .rsplit(':')
            .next()
            .unwrap()
    ))
}
fn target_identity(targets: &[ProductionTargetBaseline]) -> Result<String> {
    canonical_digest(
        1,
        &targets
            .iter()
            .map(|t| {
                json!({"provider":t.provider,"target":t.target,"repository":t.repository,
        "source":t.source,"artifact":t.artifact_digest,"record":t.provider_record})
            })
            .collect::<Vec<_>>(),
    )
}
fn blocked(message: &str) -> Error {
    Error::new("production_baseline", message)
}
fn check_manifest(
    manifest: &DeploymentManifest,
    c: &ChallengeRecord,
    facts: &NativeTargetFacts,
    run: &WorkflowRun,
    step: &DeploymentStep,
) -> Result<()> {
    crate::evidence::validate_digest(&manifest.artifact_digest)?;
    if manifest.schema_version != 1
        || manifest.runtime_id != c.runtime
        || manifest.phase_operation != c.operation
        || manifest.dispatch_request != c.dispatch
        || manifest.challenge != c.challenge
        || manifest.repository_id != c.repository_id
        || manifest.step_id != c.step
        || manifest.target_id != c.target
        || manifest.environment_id != c.environment_id
        || manifest.deployment_id != facts.deployment_id
        || manifest.run_id != run.id
        || manifest.run_attempt != run.attempt
        || manifest.source != c.source
        || facts.source != c.source
        || manifest.outputs.get(&step.workflow.observed_source_output) != Some(&c.source.commit)
        || manifest.proofs.is_empty()
        || manifest.proofs.len() > 256
    {
        return Err(blocked(
            "deployment artifact differs from exact admitted target/source/result contract",
        ));
    }
    Ok(())
}
fn read_manifest(bytes: &[u8]) -> Result<(DeploymentManifest, String)> {
    const LIMIT: u64 = 1_048_576;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err(blocked("deployment archive exceeds bound"));
    }
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|_| blocked("deployment archive is malformed"))?;
    if archive.len() != 1 || archive.metadata().has_duplicate_names() {
        return Err(blocked(
            "deployment archive must contain exactly its one manifest",
        ));
    }
    let mut file = archive
        .by_index(0)
        .map_err(|_| blocked("deployment archive member is unreadable"))?;
    if file.name().ok().as_deref() != Some("kegari-deployment.json")
        || file.encrypted()
        || file.is_dir()
        || file.is_symlink()
        || file.enclosed_name().is_none()
        || file.size() > LIMIT
        || !matches!(
            file.compression(),
            zip::CompressionMethod::Stored | zip::CompressionMethod::Deflated
        )
        || file
            .unix_mode()
            .is_some_and(|m| m & 0o170000 != 0 && m & 0o170000 != 0o100000)
    {
        return Err(blocked(
            "deployment archive member violates safe manifest contract",
        ));
    }
    let expected = file.size();
    let mut data = Vec::new();
    file.by_ref()
        .take(LIMIT + 1)
        .read_to_end(&mut data)
        .map_err(|_| blocked("deployment manifest CRC/content validation failed"))?;
    if data.len() as u64 != expected || data.len() as u64 > LIMIT {
        return Err(blocked("deployment manifest size is inconsistent"));
    }
    let manifest = serde_json::from_slice(&data)
        .map_err(|_| blocked("deployment manifest schema is invalid"))?;
    Ok((manifest, bytes_digest(&data)))
}

fn unique_outputs<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<BTreeMap<String, String>, D::Error> {
    struct Unique;
    impl<'de> serde::de::Visitor<'de> for Unique {
        type Value = BTreeMap<String, String>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("unique workflow output fields")
        }
        fn visit_map<M: serde::de::MapAccess<'de>>(
            self,
            mut map: M,
        ) -> std::result::Result<Self::Value, M::Error> {
            let mut result = BTreeMap::new();
            while let Some((key, value)) = map.next_entry::<String, String>()? {
                if result.insert(key, value).is_some() {
                    return Err(serde::de::Error::custom("duplicate workflow output"));
                }
            }
            Ok(result)
        }
    }
    deserializer.deserialize_map(Unique)
}
