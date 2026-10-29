//! Retained replacement data becomes usable only after current publication and
//! source readback. No worker, process ownership or completed run is recovered here.
use super::*;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplacementCheckpoint {
    pub(crate) schema_version: u32,
    pub(crate) runtime: Uuid,
    pub(crate) operation: Uuid,
    pub(crate) workspace: Uuid,
    pub(crate) binding: String,
    pub(crate) original: String,
    pub(crate) context: Value,
    gate_digest: String,
    contract_digest: String,
    source_digest: String,
    production: kegari_git::Revision,
    selected: Signed<ChildResultRecord>,
    publication: Publication,
    merge_receipt: Uuid,
    protection: evidence::RepositoryProtection,
    reason: String,
}

impl ReplacementWork {
    pub(crate) fn checkpoint(
        &self,
        runtime: Uuid,
        operation: Uuid,
    ) -> Result<ReplacementCheckpoint> {
        Ok(ReplacementCheckpoint {
            schema_version: 2,
            runtime,
            operation,
            workspace: self.gate.workspace_id(),
            binding: self.digest.clone(),
            original: self.original_digest()?,
            context: self.context()?,
            gate_digest: self.gate.digest()?,
            contract_digest: self.contract_digest.clone(),
            source_digest: self.source.digest()?,
            production: self.source.revision().clone(),
            selected: self.published.signed.clone(),
            publication: self.published.publication.clone(),
            merge_receipt: self.staged.merge_receipt,
            protection: self.staged.protection.clone(),
            reason: self.reason.clone(),
        })
    }

    pub(crate) fn revalidate_retained(&self) -> Result<()> {
        self.source.revalidate_filesystem()
    }

    pub(crate) fn verify_current_child(&self, current: &evidence::VerifiedChild) -> Result<()> {
        if current.digest() != self.published.verified.digest()
            || current.evidence().work != *self.gate.work()
        {
            return Err(Error::new(
                "stale_result",
                "replacement's original child contribution changed",
            ));
        }
        Ok(())
    }
}

impl ReplacementCheckpoint {
    pub(crate) fn work(&self) -> &WorkIdentity {
        &self.selected.payload.evidence.work
    }
}

impl DeliveryCoordinator {
    pub(crate) async fn restore_replacement_work(
        &self,
        gate: &FreshGate,
        saved: &ReplacementCheckpoint,
        workflow: &Workflow,
    ) -> Result<Arc<ReplacementWork>> {
        (self.current_gate)(gate)?;
        if saved.schema_version != 2
            || saved.runtime != self.host.snapshot()?.intent.runtime_id
            || saved.operation.is_nil()
            || saved.workspace != gate.workspace_id()
            || saved.gate_digest != gate.digest()?
            || saved.work() != gate.work()
            || saved.reason.trim().is_empty()
            || saved.reason.len() > 4096
        {
            return Err(Error::new(
                "replacement_mapping",
                "retained replacement scope changed",
            ));
        }
        self.key.verify("child_result", &saved.selected)?;
        let observation = self.observation(
            gate.clone(),
            saved.selected.payload.evidence.revision.clone(),
        )?;
        let contract = self.contract(&observation)?;
        if contract.digest()? != saved.contract_digest {
            return Err(Error::new(
                "stale_config",
                "replacement repository contract changed",
            ));
        }
        let published = self
            .hydrate_child_result(
                &observation,
                workflow,
                &saved.selected.payload.evidence.fulfilled_criteria,
                saved.publication.record_id.parse().map_err(|_| {
                    Error::new("record_corrupt", "replacement publication ID is invalid")
                })?,
            )
            .await?;
        if canonical_digest(1, &published.signed)? != canonical_digest(1, &saved.selected)?
            || published.publication.provider != saved.publication.provider
            || published.publication.record_id != saved.publication.record_id
            || published.publication.writer_account_id != saved.publication.writer_account_id
            || published.publication.payload_digest != saved.publication.payload_digest
            || published.publication.url != saved.publication.url
        {
            return Err(Error::new(
                "stale_result",
                "replacement's original publication changed",
            ));
        }
        let (delta, pull_request) = match &published.verified.evidence().outcome {
            evidence::ChildOutcome::Changed {
                delta,
                pull_request,
            } => (delta.as_ref(), *pull_request),
            evidence::ChildOutcome::Replacement {
                delta,
                related_pull_request,
            } => (delta.as_ref(), *related_pull_request),
            _ => {
                return Err(Error::new(
                    "replacement_mapping",
                    "replacement no longer selects a changed contribution",
                ));
            }
        };
        if delta.digest()? != saved.original || delta.repository != contract.repository {
            return Err(Error::new(
                "replacement_mapping",
                "replacement contribution changed",
            ));
        }
        let scope = gate
            .child_scope()
            .ok_or_else(|| Error::new("scope_violation", "replacement needs child scope"))?;
        if scope.write_repository.as_deref() != Some(contract.configuration_name.as_str())
            || scope.write_paths != delta.scope.paths
        {
            return Err(Error::new(
                "scope_violation",
                "replacement write scope changed",
            ));
        }
        let staging_result = if delta.replacement.is_some() {
            let (record, verified) = self.replacement_staged(&observation, workflow, delta)?;
            PublishedChild {
                verified,
                signed: record.staged,
                publication: record.staged_publication,
            }
        } else {
            published.clone()
        };
        let staged_original = replacement_staging_original(delta)?;
        let signed_delta: Signed<Value> = self
            .records
            .read(&format!("delta-{}.json", &staged_original.digest()?[7..]))?;
        self.key.verify("staged_delta", &signed_delta)?;
        if signed_delta.payload.get("manifest") != Some(&json!(staged_original))
            || signed_delta.payload.get("merge_request") != Some(&json!(saved.merge_receipt))
            || staging_result.signed.payload.protection.as_ref() != Some(&saved.protection)
        {
            return Err(Error::new(
                "replacement_mapping",
                "replacement lost its original staged merge evidence",
            ));
        }
        let contracts: Vec<_> = self.contracts.values().cloned().collect();
        let sources = crate::remote_objects::retained_sources(
            &self.records,
            &self.host,
            &self.key,
            self.github()?,
            &contracts,
        )?;
        let mut source = None;
        for candidate in sources {
            if candidate.digest()? == saved.source_digest {
                if source.is_some()
                    || candidate.repository() != &contract.repository
                    || candidate.revision() != &saved.production
                {
                    return Err(Error::new(
                        "replacement_mapping",
                        "replacement source identity is inconsistent",
                    ));
                }
                candidate.verify_for_clone(&self.git)?;
                source = Some(Arc::new(candidate));
            }
        }
        let source = source.ok_or_else(|| {
            Error::new(
                "source_unavailable",
                "replacement's original imported production source is unavailable",
            )
        })?;
        if self.main_revision(&observation).await? != saved.production {
            return Err(Error::new(
                "stale_result",
                "production advanced during interrupted replacement",
            ));
        }
        let staged = StagedDelta {
            delta: self.retained_delta(delta)?,
            pull_request,
            merge_receipt: saved.merge_receipt,
            protection: saved.protection.clone(),
        };
        let digest = canonical_digest(
            1,
            &json!({"gate":gate.digest()?,"contract":contract.digest()?,
            "staged":published.verified.digest(),"original":delta.digest()?,"source":source.digest()?,"reason":saved.reason}),
        )?;
        let work = Arc::new(ReplacementWork {
            gate: gate.clone(),
            contract_digest: contract.digest()?,
            staged,
            published,
            staging_result,
            source,
            reason: saved.reason.clone(),
            digest,
        });
        if work.digest() != saved.binding || work.context()? != saved.context {
            return Err(Error::new(
                "replacement_mapping",
                "restored replacement differs from its exact original assignment",
            ));
        }
        (self.current_gate)(gate)?;
        Ok(work)
    }
}
