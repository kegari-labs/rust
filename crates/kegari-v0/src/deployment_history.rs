//! Immutable evidence of a previously observed target success. This capability
//! is deliberately distinct from a current target or production baseline.
use super::*;

const MAX_PROOF_FILES: usize = 1024;
const MAX_PROOF_BYTES: u64 = 1024 * 1024;
const MAX_SCAN_BYTES: usize = 64 * 1024 * 1024;

/// A locator, not authority. Recovery authenticates the complete retained record
/// and actual native identities before returning historical evidence.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetProofReference {
    record_digest: String,
}
impl VerifiedTargetDeployment {
    pub fn historical_reference(&self) -> Result<TargetProofReference> {
        Ok(TargetProofReference {
            record_digest: self.digest()?,
        })
    }
}

#[derive(Clone, Debug)]
pub struct HistoricalTargetDeployment {
    record: TargetRecord,
}
impl HistoricalTargetDeployment {
    pub fn target(&self) -> &ProductionTargetBaseline {
        &self.record.target
    }
    pub fn digest(&self) -> Result<String> {
        canonical_digest(1, &self.record)
    }
    pub(crate) fn operation_id(&self) -> Uuid {
        self.record.challenge.operation
    }
    pub(crate) fn dispatch_request(&self) -> Uuid {
        self.record.challenge.dispatch
    }
    pub(crate) fn challenge(&self) -> Uuid {
        self.record.challenge.challenge
    }
    pub(crate) fn deployment_effect(&self, plan: Uuid) -> Result<crate::release::ObservedEffect> {
        let record = &self.record;
        Ok(crate::release::ObservedEffect::Deployment {
            repository: record.target.repository.clone(),
            target: self.resource(),
            source_commit: record.target.source.commit.clone(),
            tree: record.target.source.tree.clone(),
            artifact_digest: record.target.artifact_digest.clone(),
            deployment_id: record.facts.deployment_id.to_string(),
            run: crate::release::OwnedRun {
                repository: record.target.repository.clone(),
                run_id: record.run.id,
                attempt: u32::try_from(record.run.attempt)
                    .map_err(|_| blocked("historical run attempt exceeds bound"))?,
                operation_id: plan,
                actor_account_id: record.run.actor_id.to_string(),
                origin_digest: canonical_digest(1, &record.challenge)?,
            },
        })
    }
    pub(crate) fn verification_effect(&self) -> Result<crate::release::ObservedEffect> {
        Ok(crate::release::ObservedEffect::Verification {
            repository: self.record.target.repository.clone(),
            target: self.resource(),
            source_commit: self.record.target.source.commit.clone(),
            tree: self.record.target.source.tree.clone(),
            artifact_digest: self.record.target.artifact_digest.clone(),
            evidence_digest: self.digest()?,
        })
    }
    fn resource(&self) -> crate::release::ResourceKey {
        crate::release::ResourceKey::Target {
            provider: self.record.target.provider.clone(),
            id: self.record.target.target.clone(),
        }
    }
}

fn digest_leaf(digest: &str) -> Result<String> {
    crate::evidence::validate_digest(digest)?;
    Ok(format!(
        "target-digest-{}.json",
        digest.rsplit(':').next().unwrap()
    ))
}
fn dispatch_leaf(dispatch: Uuid) -> String {
    format!("target-dispatch-{dispatch}.json")
}
fn proof_leaf(signed: &Signed<TargetRecord>) -> Result<String> {
    Ok(format!(
        "target-proof-{}.json",
        signed.digest()?.rsplit(':').next().unwrap()
    ))
}

pub(crate) struct TargetHistoryReferences {
    pub files: Vec<(String, String, String)>,
    pub receipts: Vec<Receipt>,
}

impl ProductionObserver {
    /// Retain the exact immutable local inputs of an already observed historical
    /// target. The mutable current-target pointer is deliberately not a dependency.
    pub(crate) fn historical_references(
        &self,
        contract: &RepositoryContract,
        historical: &HistoricalTargetDeployment,
    ) -> Result<TargetHistoryReferences> {
        let record = &historical.record;
        let dispatch = self.retained_proof_receipt(contract, record)?;
        let mut receipts = self
            .control
            .closed_release_resume_receipts(record.challenge.operation)?;
        receipts.push(dispatch);
        let expected = self.key.sign("target_deployment", record)?;
        let proof = proof_leaf(&expected)?;
        let actual: Signed<TargetRecord> = self.records.read(&proof)?;
        self.key.verify("target_deployment", &actual)?;
        if canonical_digest(1, &actual)? != canonical_digest(1, &expected)? {
            return Err(blocked("historical target immutable proof changed"));
        }
        let challenge = challenge_file(record.challenge.challenge);
        let actual_challenge: Signed<ChallengeRecord> = self.records.read(&challenge)?;
        self.key.verify("deployment_challenge", &actual_challenge)?;
        if actual_challenge.payload != record.challenge {
            return Err(blocked("historical target challenge changed"));
        }
        let mut files = vec![
            (proof.clone(), "target_deployment".into(), actual.digest()?),
            (
                challenge,
                "deployment_challenge".into(),
                actual_challenge.digest()?,
            ),
        ];
        for leaf in [
            dispatch_leaf(record.challenge.dispatch),
            digest_leaf(&canonical_digest(1, record)?)?,
        ] {
            let index: Signed<TargetIndex> = match self.records.read(&leaf) {
                Ok(value) => value,
                Err(error) if error.code == "record_missing" => continue,
                Err(error) => return Err(error),
            };
            self.key.verify("target_deployment_index", &index)?;
            if index.payload.proof_file != proof
                || index.payload.proof_digest != actual.digest()?
                || index.payload.contract_digest != record.challenge.contract_digest
                || index.payload.step != record.challenge.step
            {
                return Err(blocked("historical target immutable locator changed"));
            }
            files.push((leaf, "target_deployment_index".into(), index.digest()?));
        }
        Ok(TargetHistoryReferences { files, receipts })
    }
    // A crash can leave the immutable proof complete but its current pointer
    // absent or pointing to an older authenticated proof. This read-only lookup
    // grants no new observation or execution authority: callers still revalidate
    // the full current run/target (and main
    // for ordinary baseline recovery) after a unique authenticated match.
    pub(super) async fn retained_target_record(
        &self,
        contract: &RepositoryContract,
        step_id: &str,
    ) -> Result<TargetRecord> {
        let configured = step(contract, step_id)?;
        let contract_digest = contract.digest()?;
        let indexed = self.indexed_proof(&index_file(contract, step_id)?)?;
        if let Some(record) = &indexed
            && (record.challenge.contract_digest != contract_digest
                || record.challenge.step != step_id)
        {
            return Err(blocked("retained target index belongs to another contract"));
        }
        // Authenticate any present pointer and referenced proof before reading
        // provider state. Malformed, dangling or invalid signed pointers cannot
        // fall through to another record. An older valid proof may be replaced
        // only by one exact current-facts match; this never rewrites the pointer.
        let current = self.current_target(contract, configured).await?;
        if let Some(record) = indexed
            && record.facts == current
        {
            return Ok(record);
        }
        self.scan_proof(|record| {
            Ok(record.challenge.contract_digest == contract_digest
                && record.challenge.step == step_id
                && record.facts == current)
        })?
        .ok_or_else(|| {
            blocked("current target has no retained authenticated deployment proof")
                .next("Run the configured trusted deployment or validation route under explicit release/recovery authority.")
        })
    }
    fn indexed_proof(&self, leaf: &str) -> Result<Option<TargetRecord>> {
        let index: Signed<TargetIndex> = match self.records.read(leaf) {
            Ok(value) => value,
            Err(error) if error.code == "record_missing" => return Ok(None),
            Err(error) => return Err(error),
        };
        self.key.verify("target_deployment_index", &index)?;
        let signed: Signed<TargetRecord> = self
            .records
            .read_bounded(&index.payload.proof_file, MAX_PROOF_BYTES)?;
        self.key.verify("target_deployment", &signed)?;
        if proof_leaf(&signed)? != index.payload.proof_file
            || signed.digest()? != index.payload.proof_digest
            || signed.payload.challenge.contract_digest != index.payload.contract_digest
            || signed.payload.challenge.step != index.payload.step
        {
            return Err(blocked(
                "immutable target index differs from its content-addressed proof",
            ));
        }
        Ok(Some(signed.payload))
    }
    // Legacy records predate immutable lookup indexes. Only a unique exact
    // authenticated match is usable; scans have entry, per-record and total caps.
    fn scan_proof(
        &self,
        matches: impl Fn(&TargetRecord) -> Result<bool>,
    ) -> Result<Option<TargetRecord>> {
        let mut count = 0;
        let mut bytes = 0;
        let mut selected: Option<TargetRecord> = None;
        for leaf in self.records.records_bounded(65536)? {
            let Some(hex) = leaf
                .strip_prefix("target-proof-")
                .and_then(|v| v.strip_suffix(".json"))
            else {
                continue;
            };
            if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(blocked("malformed retained target proof filename"));
            }
            count += 1;
            if count > MAX_PROOF_FILES {
                return Err(blocked("retained target proof inventory exceeds bound"));
            }
            let signed: Signed<TargetRecord> = self.records.read_bounded(&leaf, MAX_PROOF_BYTES)?;
            bytes += serde_json::to_vec(&signed)
                .map_err(|_| blocked("target proof serialization failed"))?
                .len();
            if bytes > MAX_SCAN_BYTES {
                return Err(blocked("retained target proof bytes exceed bound"));
            }
            self.key.verify("target_deployment", &signed)?;
            if proof_leaf(&signed)? != leaf {
                return Err(blocked("retained target proof content address changed"));
            }
            if matches(&signed.payload)? {
                if let Some(prior) = &selected
                    && canonical_digest(1, prior)? != canonical_digest(1, &signed.payload)?
                {
                    return Err(blocked(
                        "multiple immutable proofs match the historical target identity",
                    ));
                }
                selected = Some(signed.payload);
            }
        }
        Ok(selected)
    }
    pub(super) fn retained_dispatch_proof(
        &self,
        contract: &RepositoryContract,
        step: &str,
        challenge: Uuid,
        dispatch: Uuid,
    ) -> Result<Option<TargetRecord>> {
        let matches = |r: &TargetRecord| {
            Ok(r.challenge.dispatch == dispatch && r.challenge.challenge == challenge)
        };
        let record = match self.indexed_proof(&dispatch_leaf(dispatch))? {
            Some(record) => Some(record),
            None => self.scan_proof(matches)?,
        };
        if let Some(record) = &record
            && (!matches(record)?
                || record.challenge.contract_digest != contract.digest()?
                || record.challenge.step != step)
        {
            return Err(blocked(
                "historical dispatch proof belongs to another exact target contract",
            ));
        }
        Ok(record)
    }
    pub(super) fn persist_target_proof(
        &self,
        contract: &RepositoryContract,
        mut record: TargetRecord,
    ) -> Result<TargetRecord> {
        let c = &record.challenge;
        if let Some(prior) =
            self.retained_dispatch_proof(contract, &c.step, c.challenge, c.dispatch)?
        {
            let mut normalized = record.clone();
            normalized.target.observed_at = prior.target.observed_at;
            if canonical_digest(1, &normalized)? != canonical_digest(1, &prior)? {
                return Err(blocked(
                    "owned deployment already has different immutable successful evidence",
                ));
            }
            record = prior;
        }
        let signed = self.key.sign("target_deployment", &record)?;
        let index = TargetIndex {
            contract_digest: record.challenge.contract_digest.clone(),
            step: record.challenge.step.clone(),
            proof_digest: signed.digest()?,
            proof_file: proof_leaf(&signed)?,
        };
        match self.records.read::<Signed<TargetRecord>>(&index.proof_file) {
            Ok(prior) if canonical_digest(1, &prior)? == canonical_digest(1, &signed)? => {}
            Ok(_) => return Err(blocked("immutable target proof was replaced")),
            Err(error) if error.code == "record_missing" => {
                self.records.write(&index.proof_file, &signed)?
            }
            Err(error) => return Err(error),
        }
        let signed_index = self.key.sign("target_deployment_index", &index)?;
        for leaf in [
            dispatch_leaf(record.challenge.dispatch),
            digest_leaf(&canonical_digest(1, &record)?)?,
        ] {
            match self.records.read::<Signed<TargetIndex>>(&leaf) {
                Ok(prior)
                    if canonical_digest(1, &prior)? == canonical_digest(1, &signed_index)? => {}
                Ok(_) => return Err(blocked("immutable target proof index was replaced")),
                Err(error) if error.code == "record_missing" => {
                    self.records.write(&leaf, &signed_index)?
                }
                Err(error) => return Err(error),
            }
        }
        self.records.write(
            &index_file(contract, &record.challenge.step)?,
            &signed_index,
        )?;
        Ok(record)
    }
    /// Recover one historical event, never current serving state or readiness.
    pub async fn recover_historical_target(
        &self,
        contract: &RepositoryContract,
        reference: &TargetProofReference,
    ) -> Result<HistoricalTargetDeployment> {
        let leaf = digest_leaf(&reference.record_digest)?;
        let record = match self.indexed_proof(&leaf)? {
            Some(record) => Some(record),
            None => self.scan_proof(|r| Ok(canonical_digest(1, r)? == reference.record_digest))?,
        }
        .ok_or_else(|| blocked("exact immutable historical target proof is unavailable"))?;
        if canonical_digest(1, &record)? != reference.record_digest {
            return Err(blocked("historical target digest index changed"));
        }
        self.reobserve_historical_record(contract, &record).await?;
        Ok(HistoricalTargetDeployment { record })
    }
    pub(crate) async fn historical_dispatch(
        &self,
        contract: &RepositoryContract,
        step: &str,
        challenge: Uuid,
        dispatch: Uuid,
    ) -> Result<HistoricalTargetDeployment> {
        let record = self
            .retained_dispatch_proof(contract, step, challenge, dispatch)?
            .ok_or_else(|| {
                blocked("owned terminal dispatch has no exact immutable target proof")
            })?;
        self.reobserve_historical_record(contract, &record).await?;
        Ok(HistoricalTargetDeployment { record })
    }
    pub(crate) async fn historical_digest(
        &self,
        contract: &RepositoryContract,
        digest: &str,
    ) -> Result<HistoricalTargetDeployment> {
        self.recover_historical_target(
            contract,
            &TargetProofReference {
                record_digest: digest.into(),
            },
        )
        .await
    }
    pub(super) fn retained_proof_receipt(
        &self,
        contract: &RepositoryContract,
        record: &TargetRecord,
    ) -> Result<Receipt> {
        let c = &record.challenge;
        self.check_challenge(
            contract,
            &TargetDeploymentChallenge {
                signed: self.key.sign("deployment_challenge", c)?,
            },
        )?;
        let owner = self.control.operation(c.operation)?;
        // Active admissions are durably Prepared until their owned task exits.
        // Only the control host's actual live registry can disambiguate that
        // state from an interrupted/recovered Prepared admission. The receipt's
        // serialized Permit is never adopted as authority.
        let active = owner.phase == ReceiptPhase::Prepared
            && self
                .control
                .snapshot()?
                .active_operations
                .iter()
                .any(|permit| {
                    permit.operation_id == c.operation
                        && permit.lease.runtime_id == c.runtime
                        && permit.scope_digest == c.admission_digest
                });
        if owner.kind != "admission"
            || owner.runtime_id != c.runtime
            || (!active
                && !matches!(
                    owner.phase,
                    ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
                ))
            || owner.expected.get("scope") != Some(&c.admission_digest)
            || [
                "run",
                "metadata",
                "continuation",
                "source_advanced",
                "worker_preparation",
                "session_resume",
                "session_recovery_preparation",
            ]
            .iter()
            .any(|key| owner.expected.contains_key(*key))
        {
            return Err(blocked(
                "target proof lost its exact original release admission",
            ));
        }
        let (plan_operation, plan_digest) = if let Some(resume) =
            crate::control::ReleaseResumeReference::from_receipt(&owner)?
        {
            let original = self.control.operation(resume.plan_operation_id())?;
            if resume.request_id()? != c.operation
                || resume.scope_digest()? != c.admission_digest
                || original.kind != "admission"
                || original.phase != ReceiptPhase::Reconciled
                || original.runtime_id != c.runtime
                || original.canonical_targets != owner.canonical_targets
                || original.expected.get("scope").map(String::as_str) != Some(resume.plan_digest())
                || [
                    "run",
                    "metadata",
                    "continuation",
                    "source_advanced",
                    "worker_preparation",
                    "session_resume",
                    "session_recovery_preparation",
                    "release_resume",
                ]
                .iter()
                .any(|key| original.expected.contains_key(*key))
            {
                return Err(blocked(
                    "target proof resume no longer binds the exact original plan admission",
                ));
            }
            (resume.plan_operation_id(), resume.plan_digest().to_owned())
        } else {
            (c.operation, c.admission_digest.clone())
        };
        let receipt = self.control.operation(c.dispatch)?;
        if receipt.request_id != c.dispatch
            || receipt.operation_id != c.dispatch
            || receipt.runtime_id != c.runtime
            || receipt.kind != "github_dispatch"
            || receipt.payload_digest != record.dispatch_payload
            || !matches!(
                receipt.phase,
                ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
            )
            || receipt.expected.get("admitted_scope") != Some(&c.admission_digest)
            || receipt.expected.get("admission_operation") != Some(&c.operation.to_string())
            || receipt
                .expected
                .get("release_plan")
                .is_some_and(|digest| digest != &plan_digest)
            || receipt.remote_ids.get("workflow_run_id") != Some(&record.run.id.to_string())
            || record.target.source != c.source
            || record.target.target != c.target
            || record.target.repository != contract.repository
        {
            return Err(blocked(
                "historical target lost exact immutable dispatch ownership",
            ));
        }
        let result = receipt
            .observed_result
            .as_ref()
            .ok_or_else(|| blocked("target dispatch has no observed result"))?;
        if result.get("repository_id").is_some() {
            self.dispatch_receipt(contract, c, &receipt)?;
        } else {
            // Runtime reconciliation replaces ACK details with an exact effect.
            // The signed original proof retains the ACK-bound payload; the new
            // result must bind this same target, run, source and original plan.
            let plan = receipt
                .expected
                .get("release_plan")
                .ok_or_else(|| blocked("reconciled target lacks release plan binding"))?;
            let step = receipt
                .expected
                .get("release_step")
                .ok_or_else(|| blocked("reconciled target lacks release step binding"))?;
            let effect: crate::release::ObservedEffect =
                serde_json::from_value(result["effect"].clone())
                    .map_err(|_| blocked("reconciled target effect is malformed"))?;
            let crate::release::ObservedEffect::Deployment { run, .. } = &effect else {
                return Err(blocked(
                    "target dispatch was not reconciled as a deployment",
                ));
            };
            let historical = HistoricalTargetDeployment {
                record: record.clone(),
            };
            if receipt.phase != ReceiptPhase::Reconciled
                || result["release_plan"].as_str() != Some(plan)
                || result["step"].as_str() != Some(step)
                || result["outcome"] != json!("succeeded")
                || run.operation_id != plan_operation
                || effect != historical.deployment_effect(plan_operation)?
            {
                return Err(blocked(
                    "reconciled target effect differs from its immutable original proof",
                ));
            }
        }
        Ok(receipt)
    }
    async fn reobserve_historical_record(
        &self,
        contract: &RepositoryContract,
        record: &TargetRecord,
    ) -> Result<()> {
        self.retained_proof_receipt(contract, record)?;
        let configured = step(contract, &record.challenge.step)?;
        let facts = &record.facts;
        let environment = self
            .github
            .environment_by_id(&contract.provider, facts.environment_id)
            .await?;
        if facts.repository_id != contract.provider.id
            || facts.environment_id != environment_id(configured)?
            || environment.name != facts.environment_name
            || canonical_digest(
                1,
                &json!({"rules":environment.protection_rules,"policy":environment.deployment_branch_policy}),
            )? != facts.environment_policy_digest
        {
            return Err(blocked(
                "historical deployment environment identity or policy changed",
            ));
        }
        let deployment = self
            .github
            .deployment(&contract.provider, facts.deployment_id)
            .await?;
        if deployment.repository_id != facts.repository_id
            || deployment.id != facts.deployment_id
            || text(&deployment.data, "/sha")? != facts.source.commit
            || text(&deployment.data, "/environment")? != facts.environment_name
            || !boolean(&deployment.data, "/production_environment")?
            || boolean(&deployment.data, "/transient_environment")?
            || positive(&deployment.data, "/creator/id")? != facts.deployment_creator
            || date(&deployment.data, "/created_at")? != facts.deployment_created
        {
            return Err(blocked("original native deployment identity changed"));
        }
        let statuses = self
            .github
            .deployment_statuses(&contract.provider, facts.deployment_id)
            .await?;
        let matching: Vec<_> = statuses
            .iter()
            .filter(|s| s.id == facts.status_id)
            .collect();
        let [status] = matching.as_slice() else {
            return Err(blocked(
                "original successful deployment status is absent or ambiguous",
            ));
        };
        if status.repository_id != facts.repository_id
            || text(&status.data, "/state")? != "success"
            || text(&status.data, "/environment")? != facts.environment_name
            || positive(&status.data, "/creator/id")? != facts.status_creator
            || date(&status.data, "/created_at")? != facts.status_created
            || date(&status.data, "/updated_at")? != facts.status_updated
        {
            return Err(blocked("original successful deployment status changed"));
        }
        let object = self
            .github
            .commit(&contract.provider, &facts.source.commit)
            .await?;
        if text(&object, "/tree/sha")? != facts.source.tree
            || facts.source != record.challenge.source
        {
            return Err(blocked("historical deployment source object differs"));
        }
        // Exact attempt endpoint, never a later human rerun of the same run ID.
        let run = self
            .github
            .workflow_run(&contract.provider, record.run.id, Some(record.run.attempt))
            .await?;
        let jobs = self.verify_run(contract, configured, &run).await?;
        if canonical_digest(1, &run)? != canonical_digest(1, &record.run)?
            || canonical_digest(1, &jobs)? != record.jobs_digest
        {
            return Err(blocked(
                "original successful deployment run or jobs changed",
            ));
        }
        self.retained_proof_receipt(contract, record)?;
        Ok(())
    }
}
