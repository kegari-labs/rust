//! Read-only establishment of a pre-existing target's source/artifact baseline.
//! These records never establish a Kegari-owned deployment, dispatch or release.
use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExternalManifest {
    schema_version: u32,
    purpose: String,
    runtime_id: Uuid,
    repository_id: u64,
    step_id: String,
    target_id: String,
    environment_id: u64,
    deployment_id: u64,
    status_id: u64,
    run_id: u64,
    run_attempt: u64,
    source: Revision,
    #[serde(deserialize_with = "unique_outputs")]
    outputs: BTreeMap<String, String>,
    artifact_digest: String,
    proofs: Vec<JobToken>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExternalRecord {
    schema_version: u32,
    runtime: Uuid,
    contract_digest: String,
    step: String,
    facts: NativeTargetFacts,
    run: WorkflowRun,
    jobs_digest: String,
    artifact_id: u64,
    archive_digest: String,
    manifest_digest: String,
    proofs: Vec<VerifiedHostedJob>,
    target: ProductionTargetBaseline,
}

/// A purpose-separated audience component for a trusted observation workflow.
/// The workflow must measure the current deployed artifact, not copy a digest
/// from a deployment request or from model output.
pub fn external_baseline_context_digest(
    repository_id: u64,
    target: &str,
    environment_id: u64,
    deployment_id: u64,
    status_id: u64,
    source: &Revision,
    artifact_digest: &str,
) -> Result<String> {
    if status_id == 0 {
        return Err(blocked(
            "external observation requires the exact native status",
        ));
    }
    canonical_digest(
        1,
        &json!({
            "purpose": "external_target_observation",
            "status_id": status_id,
            "target_context": deployment_context_digest(
                repository_id, target, environment_id, deployment_id, source, artifact_digest
            )?
        }),
    )
}

fn external_leaf(
    runtime: Uuid,
    contract_digest: &str,
    step: &str,
    facts: &NativeTargetFacts,
) -> Result<String> {
    Ok(format!(
        "external-baseline-{}.json",
        canonical_digest(1, &(runtime, contract_digest, step, facts))?
            .rsplit(':')
            .next()
            .unwrap()
    ))
}

fn expected_proof(
    runtime: Uuid,
    contract: &RepositoryContract,
    step: &DeploymentStep,
    facts: &NativeTargetFacts,
    run: &WorkflowRun,
    artifact_digest: &str,
    check_run_id: u64,
) -> Result<HostedProofExpectation> {
    let route = observation_route(step)?;
    Ok(HostedProofExpectation {
        context_digest: Some(external_baseline_context_digest(
            contract.provider.id,
            &step.target_id,
            facts.environment_id,
            facts.deployment_id,
            facts.status_id,
            &facts.source,
            artifact_digest,
        )?),
        runtime_id: runtime,
        // These are deterministic observation correlations, not admitted control
        // operations. They are never supplied to ControlHost or owned history.
        operation_id: Uuid::from_u128(
            (u128::from(contract.provider.id) << 64) | u128::from(run.id),
        ),
        challenge: Uuid::from_u128(
            (u128::from(facts.environment_id) << 64) | u128::from(facts.status_id),
        ),
        repository_id: contract.provider.id,
        repository_owner_id: contract.provider.owner_id,
        run_id: run.id,
        run_attempt: run.attempt,
        check_run_id,
        actor_id: run.actor_id,
        trigger_sha: run.head_sha.clone(),
        tested_source_sha: facts.source.commit.clone(),
        workflow_ref: route.proof.workflow_ref.clone(),
        workflow_sha: route.proof.workflow_sha.clone(),
        job_workflow_ref: route.proof.job_workflow_ref.clone(),
        job_workflow_sha: route.proof.job_workflow_sha.clone(),
        event_name: route.proof.event_name.clone(),
    })
}

fn observation_route(step: &DeploymentStep) -> Result<&ExactRevisionWorkflow> {
    step.baseline_observation.as_ref().ok_or_else(|| {
        blocked("current external target needs a configured trusted baseline observation route")
            .next("Configure deployment.baseline_observation and run that reviewed read-only workflow for the current runtime, target and main source. Kegari will consume its fresh proof without deploying or dispatching a workflow.")
    })
}

impl ProductionObserver {
    /// Read a configured external observation using independently trusted signing
    /// keys. This only retains evidence: no permit, workflow dispatch or target
    /// mutation is created. Ordinary observe() fetches GitHub's fixed trust root
    /// when an external target has not previously been authenticated.
    pub async fn establish_external_target(
        &self,
        contract: &RepositoryContract,
        step_id: &str,
        verifier: &HostedProofVerifier,
    ) -> Result<ProductionTargetBaseline> {
        let step = step(contract, step_id)?;
        let main = self.main(contract).await?;
        let observed = self.external_target(contract, step, Some(verifier)).await?;
        let target = observed.target();
        if target.source != main || self.main(contract).await? != main {
            return Err(blocked(
                "external target and current main changed or diverge",
            ));
        }
        Ok(target)
    }

    pub(super) async fn external_target(
        &self,
        contract: &RepositoryContract,
        step: &DeploymentStep,
        verifier: Option<&HostedProofVerifier>,
    ) -> Result<VerifiedExternalTarget> {
        let route = observation_route(step)?;
        let runtime = self.control.snapshot()?.intent.runtime_id;
        let contract_digest = contract.digest()?;
        let facts = self.current_target(contract, step).await?;
        let leaf = external_leaf(runtime, &contract_digest, &step.id, &facts)?;
        if let Some(record) = self.read_external(&leaf)? {
            self.reobserve_external(contract, step, &record).await?;
            return Ok(VerifiedExternalTarget {
                record,
                observed_at: Utc::now(),
            });
        }
        // Complete run inventory, followed by each exact run read, prevents a
        // guessed ID or unrelated artifact from becoming the observation source.
        let runs = self
            .github
            .workflow_runs(
                &contract.provider,
                route.workflow_id,
                Some(&route.workflow_revision),
            )
            .await?;
        if runs.len() > 256 {
            return Err(blocked("external observation run inventory exceeds bound"));
        }
        let artifact_name = format!(
            "kegari-baseline-{runtime}-{}-{}",
            facts.environment_id, facts.status_id
        );
        let mut candidates = Vec::new();
        for run in runs {
            if run.status != "completed" || run.conclusion.as_deref() != Some("success") {
                continue;
            }
            let artifacts = self.github.artifacts(&contract.provider, run.id).await?;
            let matching: Vec<_> = artifacts
                .iter()
                .filter(|a| {
                    a.data.get("name").and_then(Value::as_str) == Some(artifact_name.as_str())
                })
                .collect();
            if matching.len() > 1 {
                return Err(blocked(
                    "external observation has duplicate named artifacts",
                ));
            }
            if let Some(artifact) = matching.first() {
                candidates.push((run, artifact.id));
            }
        }
        if candidates.len() != 1 {
            return Err(blocked("external target lacks one unique fresh observation artifact")
                .detail(json!({"artifact_name": artifact_name, "workflow_id": route.workflow_id,
                    "runtime_id": runtime,"target": step.target_id,"source": facts.source}))
                .next("Run the configured read-only baseline observation workflow for this exact runtime, current target status and source. Do not redeploy to invent a baseline."));
        }
        let (listed, artifact_id) = candidates.pop().unwrap();
        let run = self
            .github
            .workflow_run(&contract.provider, listed.id, None)
            .await?;
        if canonical_digest(1, &listed)? != canonical_digest(1, &run)? {
            return Err(blocked("external observation run changed during inventory"));
        }
        let jobs = self.verify_route_run(contract, route, &run).await?;
        let archive = self
            .github
            .artifact_archive(&contract.provider, artifact_id, run.id, &run.head_sha)
            .await?;
        let (manifest, manifest_digest): (ExternalManifest, _) =
            read_named_manifest(&archive.bytes, "kegari-baseline.json")?;
        crate::evidence::validate_digest(&manifest.artifact_digest)?;
        if manifest.schema_version != 1
            || manifest.purpose != "external_target_observation"
            || manifest.runtime_id != runtime
            || manifest.repository_id != contract.provider.id
            || manifest.step_id != step.id
            || manifest.target_id != step.target_id
            || manifest.environment_id != facts.environment_id
            || manifest.deployment_id != facts.deployment_id
            || manifest.status_id != facts.status_id
            || manifest.run_id != run.id
            || manifest.run_attempt != run.attempt
            || manifest.source != facts.source
            || manifest.outputs.get(&route.observed_source_output) != Some(&facts.source.commit)
            || manifest.proofs.is_empty()
            || manifest.proofs.len() > 256
        {
            return Err(blocked(
                "external observation artifact differs from current native target/source",
            ));
        }
        let fetched;
        let verifier = match verifier {
            Some(verifier) => verifier,
            None => {
                fetched = HostedProofVerifier::fetch(std::time::Duration::from_secs(15)).await?;
                &fetched
            }
        };
        let now = Utc::now();
        let mut checks = BTreeSet::new();
        let mut proofs = Vec::new();
        for proof in manifest.proofs {
            if !checks.insert(proof.check_run_id)
                || !jobs
                    .iter()
                    .any(|job| job.check_run_id == proof.check_run_id)
            {
                return Err(blocked(
                    "external observation proof job is duplicate or unrelated",
                ));
            }
            let expected = expected_proof(
                runtime,
                contract,
                step,
                &facts,
                &run,
                &manifest.artifact_digest,
                proof.check_run_id,
            )?;
            proofs.push(verifier.verify_and_record(&proof.token, &expected, now, &self.records)?);
        }
        if checks != jobs.iter().map(|job| job.check_run_id).collect() {
            return Err(blocked(
                "external observation omits a successful job's trusted proof",
            ));
        }
        let record = ExternalRecord {
            schema_version: 1,
            runtime,
            contract_digest,
            step: step.id.clone(),
            facts: facts.clone(),
            jobs_digest: canonical_digest(1, &jobs)?,
            artifact_id: archive.artifact_id,
            archive_digest: archive.archive_digest,
            manifest_digest,
            proofs,
            target: ProductionTargetBaseline {
                provider: "github".into(),
                target: step.target_id.clone(),
                repository: contract.repository.clone(),
                source: facts.source.clone(),
                artifact_digest: manifest.artifact_digest,
                provider_record: format!(
                    "github-external-observation:{}/{}/{}/{}/{}",
                    contract.repository.hostname,
                    contract.provider.id,
                    facts.deployment_id,
                    facts.status_id,
                    run.id
                ),
                observed_at: now,
            },
            run,
        };
        self.reobserve_external(contract, step, &record).await?;
        let _lock = self.records.section_lock("external-baseline.lock")?;
        if let Some(prior) = self.read_external(&leaf)? {
            if canonical_digest(1, &prior)? != canonical_digest(1, &record)? {
                return Err(blocked(
                    "external observation was concurrently established; reobserve it",
                ));
            }
        } else {
            self.records
                .write(&leaf, &self.key.sign("external_target_baseline", &record)?)?;
        }
        Ok(VerifiedExternalTarget {
            record,
            observed_at: Utc::now(),
        })
    }

    fn read_external(&self, leaf: &str) -> Result<Option<ExternalRecord>> {
        let signed: Signed<ExternalRecord> = match self.records.read_bounded(leaf, 1_048_576) {
            Ok(value) => value,
            Err(error) if error.code == "record_missing" => return Ok(None),
            Err(error) => return Err(error),
        };
        self.key.verify("external_target_baseline", &signed)?;
        let record = signed.payload;
        if record.schema_version != 1
            || external_leaf(
                record.runtime,
                &record.contract_digest,
                &record.step,
                &record.facts,
            )? != leaf
        {
            return Err(blocked("external baseline record identity changed"));
        }
        Ok(Some(record))
    }

    async fn reobserve_external(
        &self,
        contract: &RepositoryContract,
        step: &DeploymentStep,
        record: &ExternalRecord,
    ) -> Result<()> {
        let runtime = self.control.snapshot()?.intent.runtime_id;
        let run = self
            .github
            .workflow_run(&contract.provider, record.run.id, None)
            .await?;
        let jobs = self
            .verify_route_run(contract, observation_route(step)?, &run)
            .await?;
        let facts = self.current_target(contract, step).await?;
        if record.runtime != runtime
            || record.contract_digest != contract.digest()?
            || record.step != step.id
            || record.facts != facts
            || record.target.repository != contract.repository
            || record.target.target != step.target_id
            || record.target.source != facts.source
            || canonical_digest(1, &run)? != canonical_digest(1, &record.run)?
            || canonical_digest(1, &jobs)? != record.jobs_digest
            || record.proofs.len() != jobs.len()
        {
            return Err(blocked(
                "external baseline no longer matches the configured native target/run",
            ));
        }
        let mut checks = BTreeSet::new();
        for proof in &record.proofs {
            let expected = expected_proof(
                runtime,
                contract,
                step,
                &facts,
                &run,
                &record.target.artifact_digest,
                proof.expectation.check_run_id,
            )?;
            if proof.schema_version != 1
                || proof.expectation != expected
                || !checks.insert(proof.expectation.check_run_id)
                || !jobs
                    .iter()
                    .any(|job| job.check_run_id == proof.expectation.check_run_id)
            {
                return Err(blocked(
                    "external baseline lost its original trusted job binding",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub(super) struct VerifiedExternalTarget {
    record: ExternalRecord,
    observed_at: DateTime<Utc>,
}
impl VerifiedExternalTarget {
    pub(super) fn target(&self) -> ProductionTargetBaseline {
        let mut target = self.record.target.clone();
        target.observed_at = self.observed_at;
        target
    }
    pub(super) fn digest(&self) -> Result<String> {
        canonical_digest(1, &self.record)
    }
}

/// Explicit external origin. An enclosing authenticated completion may retain
/// this locator, but it never becomes a deployment receipt or release permit.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExternalTargetObservationRef {
    runtime: Uuid,
    contract_digest: String,
    target: ProductionTargetBaseline,
    leaf: String,
    signed_digest: String,
    record_digest: String,
}
impl ExternalTargetObservationRef {
    pub(crate) fn target(&self) -> &ProductionTargetBaseline {
        &self.target
    }
    pub(crate) fn runtime_id(&self) -> Uuid {
        self.runtime
    }
    pub(crate) fn contract_digest(&self) -> &str {
        &self.contract_digest
    }
    pub(crate) fn validate_retained(
        &self,
        records: &PrivateDir,
        key: &AttestationKey,
    ) -> Result<()> {
        let signed: Signed<ExternalRecord> = records.read_bounded(&self.leaf, 1_048_576)?;
        key.verify("external_target_baseline", &signed)?;
        let record = &signed.payload;
        if record.schema_version != 1
            || record.runtime != self.runtime
            || self.runtime.is_nil()
            || record.contract_digest != self.contract_digest
            || external_leaf(
                record.runtime,
                &record.contract_digest,
                &record.step,
                &record.facts,
            )? != self.leaf
            || signed.digest()? != self.signed_digest
            || canonical_digest(1, record)? != self.record_digest
            || !baseline_history::same_target(&record.target, &self.target)
            || record.proofs.is_empty()
            || record.proofs.len() > 256
            || record.target.source != record.facts.source
        {
            return Err(blocked(
                "retained external baseline differs from its authenticated origin",
            ));
        }
        self.target.source.validate()?;
        crate::evidence::validate_digest(&self.target.artifact_digest)?;
        Ok(())
    }
    pub(crate) fn proof_file(&self) -> (String, String, String) {
        (
            self.leaf.clone(),
            "external_target_baseline".into(),
            self.signed_digest.clone(),
        )
    }
}

#[derive(Clone, Debug)]
pub(crate) struct HistoricalExternalTarget {
    reference: ExternalTargetObservationRef,
}
impl HistoricalExternalTarget {
    pub(crate) fn target(&self) -> &ProductionTargetBaseline {
        self.reference.target()
    }
    pub(crate) fn reference(&self) -> &ExternalTargetObservationRef {
        &self.reference
    }
    pub(crate) fn verification_effect(&self) -> crate::release::ObservedEffect {
        let target = self.target();
        crate::release::ObservedEffect::Verification {
            repository: target.repository.clone(),
            target: crate::release::ResourceKey::Target {
                provider: target.provider.clone(),
                id: target.target.clone(),
            },
            source_commit: target.source.commit.clone(),
            tree: target.source.tree.clone(),
            artifact_digest: target.artifact_digest.clone(),
            evidence_digest: self.reference.record_digest.clone(),
        }
    }
}

impl ProductionObserver {
    fn external_reference(
        &self,
        record: &ExternalRecord,
        target: ProductionTargetBaseline,
    ) -> Result<ExternalTargetObservationRef> {
        let signed = self.key.sign("external_target_baseline", record)?;
        let reference = ExternalTargetObservationRef {
            runtime: record.runtime,
            contract_digest: record.contract_digest.clone(),
            target,
            leaf: external_leaf(
                record.runtime,
                &record.contract_digest,
                &record.step,
                &record.facts,
            )?,
            signed_digest: signed.digest()?,
            record_digest: canonical_digest(1, record)?,
        };
        reference.validate_retained(&self.records, &self.key)?;
        Ok(reference)
    }
    pub(crate) fn external_baseline_references(
        &self,
        contract: &RepositoryContract,
        baseline: &VerifiedProductionBaseline,
    ) -> Result<Vec<ExternalTargetObservationRef>> {
        if baseline.contract_digest != contract.digest()?
            || baseline.baseline.repository != contract.repository
            || baseline.baseline.successor_recovery_operation.is_some()
            || Utc::now() - baseline.observed_at > chrono::Duration::seconds(30)
            || baseline.observed_at > Utc::now() + chrono::Duration::seconds(5)
        {
            return Err(blocked(
                "external baseline references require exact recent ordinary observation",
            ));
        }
        let mut seen = BTreeSet::new();
        baseline
            .external_targets
            .iter()
            .map(|proof| {
                let target = proof.target();
                let configured = step(contract, &proof.record.step)?;
                if !seen.insert(target.target.clone())
                    || configured.target_id != target.target
                    || !baseline
                        .targets
                        .iter()
                        .any(|current| baseline_history::same_target(current, &target))
                    || baseline
                        .owned_targets
                        .iter()
                        .any(|owned| owned.target().target == target.target)
                    || target.source != baseline.baseline.production
                    || baseline.baseline.artifact_for_target(&target.target)
                        != Some(target.artifact_digest.as_str())
                {
                    return Err(blocked(
                        "external baseline proof differs from complete source/artifact inventory",
                    ));
                }
                self.external_reference(&proof.record, target)
            })
            .collect()
    }
    /// Original external observation provenance, never ownership of a deployment.
    pub(crate) async fn historical_external(
        &self,
        contract: &RepositoryContract,
        digest: &str,
    ) -> Result<Option<HistoricalExternalTarget>> {
        crate::evidence::validate_digest(digest)?;
        let mut selected = None;
        let mut count = 0;
        let mut bytes = 0;
        for leaf in self.records.records_bounded(65_536)? {
            if !leaf.starts_with("external-baseline-") || !leaf.ends_with(".json") {
                continue;
            }
            count += 1;
            if count > 1024 {
                return Err(blocked("external baseline history exceeds bound"));
            }
            let record = self
                .read_external(&leaf)?
                .ok_or_else(|| blocked("external baseline disappeared during history lookup"))?;
            bytes += serde_json::to_vec(&record)
                .map_err(|_| blocked("external baseline encoding failed"))?
                .len();
            if bytes > 64 * 1024 * 1024 {
                return Err(blocked("external baseline history bytes exceed bound"));
            }
            if canonical_digest(1, &record)? != digest {
                continue;
            }
            if selected.is_some() {
                return Err(blocked("external baseline digest is ambiguous"));
            }
            let configured = step(contract, &record.step)?;
            if record.runtime != self.control.snapshot()?.intent.runtime_id
                || record.contract_digest != contract.digest()?
            {
                return Err(blocked(
                    "external baseline history belongs to another runtime or contract",
                ));
            }
            self.reobserve_historical_facts(contract, configured, &record.facts)
                .await?;
            let run = self
                .github
                .workflow_run(&contract.provider, record.run.id, Some(record.run.attempt))
                .await?;
            let jobs = self
                .verify_route_run(contract, observation_route(configured)?, &run)
                .await?;
            if canonical_digest(1, &run)? != canonical_digest(1, &record.run)?
                || canonical_digest(1, &jobs)? != record.jobs_digest
            {
                return Err(blocked("original external observation run or jobs changed"));
            }
            let reference = self.external_reference(&record, record.target.clone())?;
            selected = Some(HistoricalExternalTarget { reference });
        }
        Ok(selected)
    }
}

#[cfg(test)]
use crate as fixture_api;
#[cfg(test)]
#[allow(dead_code, unused_imports)]
#[path = "../tests/support/deployment_wire.rs"]
mod wire;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn external_baseline_reference_remains_external_through_current_and_historical_reads() {
        let fixture = wire::Fixture::new();
        let mut contract = wire::contract(true);
        let Deployment::Steps { steps } = contract.configuration.deployment.as_mut().unwrap()
        else {
            panic!()
        };
        steps[0].baseline_observation = Some(steps[0].workflow.clone());
        fixture.external_artifact(&contract, |_| {});
        let target = fixture
            .observer
            .establish_external_target(&contract, "deploy", &wire::verifier())
            .await
            .unwrap();
        let baseline = fixture.observer.observe(&contract).await.unwrap();
        assert!(
            fixture
                .observer
                .owned_baseline_references(&contract, &baseline)
                .unwrap()
                .is_empty()
        );
        let refs = fixture
            .observer
            .external_baseline_references(&contract, &baseline)
            .unwrap();
        assert_eq!(refs.len(), 1);
        let reference = &refs[0];
        let records = PrivateDir::open(&fixture.records).unwrap();
        let key = AttestationKey::new("fixture".into(), vec![7; 32]).unwrap();
        reference.validate_retained(&records, &key).unwrap();
        let existing = fixture
            .observer
            .verify_existing_target(
                &contract,
                &target.target,
                &target.source,
                &target.artifact_digest,
            )
            .await
            .unwrap();
        assert_eq!(existing.digest().unwrap(), reference.record_digest);
        let history = fixture
            .observer
            .historical_external(&contract, &existing.digest().unwrap())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            history.verification_effect(),
            crate::release::ObservedEffect::Verification { .. }
        ));
        assert!(
            fixture
                .host
                .snapshot()
                .unwrap()
                .active_operations
                .is_empty()
        );
        // These exact external references can enter no-release seals, but raw
        // references retain no authority and a different target cannot borrow them.
        let mut forged = serde_json::to_value(reference).unwrap();
        forged["target"]["artifact_digest"] = json!(bytes_digest(b"different"));
        let forged: ExternalTargetObservationRef = serde_json::from_value(forged).unwrap();
        assert_eq!(
            forged.validate_retained(&records, &key).unwrap_err().code,
            "production_baseline"
        );
        let (leaf, purpose, _) = reference.proof_file();
        assert_eq!(purpose, "external_target_baseline");
        std::fs::remove_file(records.path().join(leaf)).unwrap();
        assert_eq!(
            reference
                .validate_retained(&records, &key)
                .unwrap_err()
                .code,
            "record_missing"
        );
    }
}
