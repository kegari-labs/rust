//! Original accepted release facts. This seal never becomes current readiness.
use super::*;

#[derive(Clone, Debug)]
pub struct VerifiedReleaseOrigin {
    plan: ReleasePlan,
    manifest: evidence::ParentEvidence,
    vector: ParentCandidateVector,
    original_admission_digest: String,
    observed_at: DateTime<Utc>,
}
impl VerifiedReleaseOrigin {
    pub fn plan(&self) -> &ReleasePlan {
        &self.plan
    }
    pub fn manifest(&self) -> &evidence::ParentEvidence {
        &self.manifest
    }
    pub fn targets(&self) -> &[ProductionTargetBaseline] {
        &self.vector.signed.payload.targets
    }
    pub fn observed_at(&self) -> DateTime<Utc> {
        self.observed_at
    }
    pub fn admission_digest(&self) -> &str {
        &self.original_admission_digest
    }
    pub fn slice_operation(&self, repository: &RepositoryId) -> Result<Uuid> {
        self.vector
            .signed
            .payload
            .slices
            .iter()
            .find(|(repo, _)| repo == repository)
            .map(|(_, id)| *id)
            .ok_or_else(|| {
                Error::new(
                    "candidate_provenance",
                    "original release candidate slice is missing",
                )
            })
    }
}
impl DeliveryCoordinator {
    /// Read exact historical publications and the original actual admission.
    /// Old child generations are provenance; they do not become fresh gates.
    pub async fn release_origin(
        &self,
        plan: &ReleasePlan,
        parent_comment: Uuid,
        acceptance_comment: Uuid,
    ) -> Result<VerifiedReleaseOrigin> {
        plan.validate()?;
        let admission = self.host.operation(plan.operation_id)?;
        if admission.kind != "admission"
            || admission.runtime_id != plan.runtime_id
            || admission.expected.get("scope") != Some(&plan.digest()?)
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
            .any(|key| admission.expected.contains_key(*key))
        {
            return Err(Error::new(
                "ownership_conflict",
                "original plan has no exact actual release admission",
            ));
        }
        let linear =
            self.broker.linear.as_ref().ok_or_else(|| {
                Error::new("prerequisite", "original Linear connection unavailable")
            })?;
        let identity = linear.identity().await?;
        let writer: Uuid = identity
            .account_id
            .parse()
            .map_err(|_| Error::new("identity_mismatch", "Linear account ID is not canonical"))?;
        let key = linear.issue_key(plan.parent_id)?;
        if key.workspace != plan.linear_workspace
            || admission.canonical_targets != vec![key.identity()?]
        {
            return Err(Error::new(
                "identity_mismatch",
                "original plan belongs to another selected Linear scope",
            ));
        }
        let comments = linear.comments(plan.parent_id).await?;
        let select = |id| {
            let matches = comments
                .iter()
                .filter(|comment| comment.id == id)
                .collect::<Vec<_>>();
            let [comment] = matches.as_slice() else {
                return Err(Error::new(
                    "evidence_unpublished",
                    "exact original native publication is missing or duplicated",
                ));
            };
            if comment.issue_id != plan.parent_id
                || comment.user_id != Some(writer)
                || comment.bot
                || comment.delegated
            {
                return Err(Error::new(
                    "evidence_unpublished",
                    "original native publication writer or scope differs",
                ));
            }
            Ok(*comment)
        };
        let parent = select(parent_comment)?;
        let acceptance = select(acceptance_comment)?;
        let signed: Signed<ParentResultRecord> =
            evidence::decode_signed_publication(&parent.body, "parent_result", &self.key)?;
        let accepted: Signed<evidence::Acceptance> =
            evidence::decode_signed_publication(&acceptance.body, "human_acceptance", &self.key)?;
        let manifest = &signed.payload.evidence;
        if signed.payload.schema_version != 1
            || signed.payload.runtime_id != plan.runtime_id
            || manifest.work.parent != plan.parent_id
            || manifest.work.child.is_some()
            || manifest.work.linear_endpoint != key.endpoint
            || manifest.work.linear_workspace != plan.linear_workspace
            || manifest.work.generation != plan.generation
            || manifest.token()? != plan.evidence_token
            || accepted.digest()? != plan.acceptance_digest
            || accepted.payload.runtime_id != plan.runtime_id
            || accepted.payload.work != manifest.work
            || accepted.payload.reviewed_token != plan.evidence_token
            || accepted.payload.provider_writer != writer.to_string()
            || accepted.payload.accepted_at < manifest.created_at
            || plan.carried_acceptance.is_some()
        {
            return Err(Error::new(
                "candidate_provenance",
                "original parent and acceptance do not bind the admitted plan",
            ));
        }
        let receipts = self.host.operations()?;
        for comment in [parent, acceptance] {
            if receipts
                .iter()
                .filter(|receipt| {
                    receipt.kind == "linear_comment"
                        && receipt.runtime_id == plan.runtime_id
                        && matches!(
                            receipt.phase,
                            crate::control::ReceiptPhase::Acknowledged
                                | crate::control::ReceiptPhase::Reconciled
                        )
                        && if comment.id == parent.id {
                            receipt.expected.get("admission_operation")
                                == Some(&manifest.publication_operation.to_string())
                        } else {
                            receipt.operation_id == accepted.payload.operation_id
                                && receipt.actor == accepted.payload.control_actor
                        }
                        && receipt.remote_ids.get("comment_id") == Some(&comment.id.to_string())
                        && receipt.expected.get("provider_body_digest")
                            == Some(&comment.body_digest)
                        && receipt.expected.get("provider_writer") == Some(&writer.to_string())
                        && receipt.canonical_targets
                            == vec![format!(
                                "linear:{}:{}:{}",
                                key.endpoint, key.workspace, key.issue
                            )]
                })
                .count()
                != 1
            {
                return Err(Error::new(
                    "reconciliation_required",
                    "original publication has no unique immutable host receipt",
                ));
            }
        }
        evidence::validate_digest(&signed.payload.vector_digest)?;
        let vector: Signed<VectorRecord> = self.records.read(&format!(
            "parent-vector-{}.json",
            &signed.payload.vector_digest[7..]
        ))?;
        self.key.verify("parent_candidate_vector", &vector)?;
        let v = &vector.payload;
        let planned_repos: BTreeSet<_> = plan
            .resources
            .iter()
            .filter_map(|r| match r {
                ResourceKey::Repository { repository } => Some(repository.clone()),
                _ => None,
            })
            .collect();
        let repos = manifest
            .candidates
            .iter()
            .map(|candidate| candidate.repository.clone())
            .collect::<BTreeSet<_>>();
        if !planned_repos.is_subset(&repos)
            || repos.difference(&planned_repos).any(|repository| {
                !manifest.candidates.iter().any(|candidate| {
                    &candidate.repository == repository
                        && candidate.ordered_delta_digests.is_empty()
                }) || !manifest
                    .baselines
                    .iter()
                    .any(|baseline| &baseline.repository == repository && baseline.deployment_free)
            })
        {
            return Err(Error::new(
                "candidate_provenance",
                "original plan omits an effectful accepted repository",
            ));
        }
        let targets: BTreeSet<_> = plan
            .resources
            .iter()
            .filter_map(|r| match r {
                ResourceKey::Target { id, .. } => Some(id.clone()),
                _ => None,
            })
            .collect();
        if v.schema_version != 1
            || v.runtime_id != plan.runtime_id
            || v.work != manifest.work
            || v.contract_digest != signed.payload.contract_digest
            || vector_digest(v)? != signed.payload.vector_digest
            || v.baselines != manifest.baselines
            || v.candidates != manifest.candidates
            || v.baselines.len() != repos.len()
            || v.candidates.len() != repos.len()
            || v.slices.len() != repos.len()
            || v.baselines
                .iter()
                .map(|b| b.repository.clone())
                .collect::<BTreeSet<_>>()
                != repos
            || v.candidates
                .iter()
                .map(|c| c.repository.clone())
                .collect::<BTreeSet<_>>()
                != repos
            || v.slices
                .iter()
                .map(|(r, _)| r.clone())
                .collect::<BTreeSet<_>>()
                != repos
            || v.targets.len() != targets.len()
            || v.targets
                .iter()
                .map(|t| t.target.clone())
                .collect::<BTreeSet<_>>()
                != targets
            || manifest.production_targets != targets
        {
            return Err(Error::new(
                "candidate_provenance",
                "original signed vector is incomplete or differs from accepted scope",
            ));
        }
        for baseline in &v.baselines {
            baseline.validate()?;
        }
        for step in &plan.steps {
            if let crate::release::StepKind::Merge {
                repository,
                production_base,
                candidate,
                selected_delta_digests,
                ..
            } = &step.kind
                && !v.candidates.iter().any(|c| {
                    &c.repository == repository
                        && &c.production == production_base
                        && c.result.as_ref() == Some(candidate)
                        && &c.ordered_delta_digests == selected_delta_digests
                })
            {
                return Err(Error::new(
                    "candidate_provenance",
                    "original merge was not this accepted candidate",
                ));
            }
        }
        Ok(VerifiedReleaseOrigin {
            plan: plan.clone(),
            manifest: manifest.clone(),
            vector: ParentCandidateVector {
                signed: vector,
                digest: signed.payload.vector_digest,
            },
            original_admission_digest: admission.payload_digest,
            observed_at: Utc::now(),
        })
    }
}

impl DeliveryCoordinator {
    pub(in crate::delivery) fn successor_slice(
        &self,
        admission: &DeliveryObservation,
        predecessor: &crate::runtime_delivery::KnownPredecessorBaseline,
        origin: &VerifiedReleaseOrigin,
    ) -> Result<IsolatedSlice> {
        self.current(
            admission,
            &[DeliveryPhase::CandidateAssembly, DeliveryPhase::Recovery],
        )?;
        let record = self.successor_slice_record(
            admission.fresh_gate(),
            &admission.scope.repository,
            predecessor,
            origin,
        )?;
        let candidate = self.git.inspect_candidate(
            &record.payload.intent.execution_root,
            record.payload.intent.operation_id,
        )?;
        if candidate.manifest != record.payload.candidate {
            return Err(Error::new(
                "candidate_provenance",
                "retained predecessor objects changed",
            ));
        }
        predecessor.revalidate_lineage()?;
        self.current(
            admission,
            &[DeliveryPhase::CandidateAssembly, DeliveryPhase::Recovery],
        )?;
        Ok(IsolatedSlice { record, candidate })
    }
    pub(crate) fn retained_successor_host_roots(
        &self,
        gate: &FreshGate,
        predecessor: &crate::runtime_delivery::KnownPredecessorBaseline,
    ) -> Result<Vec<crate::git::VerifiedHostReadRoot>> {
        let mut roots = Vec::new();
        let mut origins = BTreeMap::from([(
            predecessor.origin().plan().operation_id,
            predecessor.origin(),
        )]);
        for (origin, _) in predecessor.integration_sources() {
            origins.insert(origin.plan().operation_id, origin);
        }
        for origin in origins.values() {
            for candidate in &origin.manifest().candidates {
                let record =
                    self.successor_slice_record(gate, &candidate.repository, predecessor, origin)?;
                let operation = record.payload.intent.operation_id;
                roots.push(crate::git::retained_candidate_read_root(
                    candidate.repository.clone(),
                    &record
                        .payload
                        .intent
                        .execution_root
                        .join(format!("candidate-{operation}")),
                    gate.config_revision().into(),
                    record.digest()?,
                    candidate,
                )?);
            }
        }
        Ok(roots)
    }
    fn successor_slice_record(
        &self,
        gate: &FreshGate,
        repository: &RepositoryId,
        predecessor: &crate::runtime_delivery::KnownPredecessorBaseline,
        origin: &VerifiedReleaseOrigin,
    ) -> Result<Signed<SliceRecord>> {
        (self.current_gate)(gate)?;
        predecessor.require_historical_gate(gate)?;
        predecessor.require_origin(origin)?;
        let operation = origin.slice_operation(repository)?;
        let contract = self.contracts.get(repository).ok_or_else(|| {
            Error::new(
                "scope_violation",
                "predecessor repository is no longer configured",
            )
        })?;
        let record: Signed<SliceRecord> = self
            .records
            .read(&format!("slice-result-{operation}.json"))?;
        self.key.verify("isolated_slice", &record)?;
        let r = &record.payload;
        if r.intent.schema_version != 1
            || r.intent.operation_id != operation
            || r.intent.scope.runtime_id != origin.plan().runtime_id
            || r.intent.scope.work != origin.manifest().work
            || r.intent.scope.repository != *repository
            || r.intent.scope.contract_digest != contract.digest()?
            || r.candidate.operation_id != operation
            || !origin.manifest().candidates.contains(&r.candidate)
        {
            return Err(Error::new(
                "scope_violation",
                "retained predecessor slice differs from exact accepted source",
            ));
        }
        let receipt = self.host.operation(r.receipt_id)?;
        let expected = canonical_digest(
            1,
            &json!({"kind":"candidate_assembly","targets":[r.intent.scope.repository.id],
            "expected":{"delivery_scope":r.intent.scope.digest()?,"admitted_scope":r.intent.scope.digest()?,
            "admission_operation":r.intent.operation_id.to_string(),"admitted_work":crate::authority::IssueKey::new(
                &r.intent.scope.work.linear_endpoint,r.intent.scope.work.linear_workspace,r.intent.scope.work.parent)?.identity()?},
            "payload":r.intent,"principal":null}),
        )?;
        if !matches!(
            receipt.phase,
            crate::control::ReceiptPhase::Acknowledged | crate::control::ReceiptPhase::Reconciled
        ) || receipt.runtime_id != origin.plan().runtime_id
            || receipt.kind != "candidate_assembly"
            || receipt.payload_digest != expected
        {
            return Err(Error::new(
                "operation_uncertain",
                "predecessor slice lacks exact acknowledged preparation",
            ));
        }
        Ok(record)
    }
}

#[cfg(test)]
impl DeliveryCoordinator {
    /// Synthetic upstream execution envelope used only to exercise the actual
    /// historical provider/receipt reader. It does not mint a historical proof.
    pub(in crate::delivery) fn fixture_original_parent_body(
        &self,
        parent: &evidence::VerifiedParent,
        slice: &IsolatedSlice,
        runtime: Uuid,
    ) -> Result<String> {
        self.fixture_original_manifest_body(parent.manifest(), slice, &[], runtime)
    }
    pub(crate) fn fixture_original_manifest_body(
        &self,
        parent: &evidence::ParentEvidence,
        slice: &IsolatedSlice,
        targets: &[ProductionTargetBaseline],
        runtime: Uuid,
    ) -> Result<String> {
        let contract_digest = canonical_digest(1, &"explicit synthetic parent contract")?;
        let vector = VectorRecord {
            schema_version: 1,
            runtime_id: runtime,
            work: parent.work.clone(),
            contract_digest: contract_digest.clone(),
            slices: vec![(slice.manifest().repository.clone(), slice.operation_id())],
            baselines: parent.baselines.clone(),
            candidates: parent.candidates.clone(),
            targets: targets.to_vec(),
            prepared_at: Utc::now(),
        };
        let digest = vector_digest(&vector)?;
        self.records.write(
            &format!("parent-vector-{}.json", &digest[7..]),
            &self.key.sign("parent_candidate_vector", &vector)?,
        )?;
        let parent = self.key.sign(
            "parent_result",
            &ParentResultRecord {
                schema_version: 1,
                runtime_id: runtime,
                contract_digest,
                vector_digest: digest,
                integrated_digest: canonical_digest(
                    1,
                    &"explicit synthetic integrated validator facts",
                )?,
                child_records: vec![],
                coordinator_claim_digest: canonical_digest(
                    1,
                    &"explicit synthetic completed coordination",
                )?,
                evidence: parent.clone(),
            },
        )?;
        record_body("parent_result", &parent)
    }
}
