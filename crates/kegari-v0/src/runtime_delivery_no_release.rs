//! Already-production completion requires an admitted publication and exact
//! native readback. A signed local intent alone never satisfies a dependency.
use super::*;
use crate::{
    control::{Actor, MutationRequest, Receipt},
    digest::{bytes_digest, canonical_digest},
    evidence::{NoReleaseRequired, NoReleaseRequirements, ProductionTargetBaseline, Publication},
    providers::{Comment, ProviderMutation},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NoReleaseIntent {
    schema_version: u32,
    publication_operation: Uuid,
    attempt: crate::control::MetadataReference,
    workspace: Uuid,
    config_revision: String,
    workflow_digest: String,
    parent_publication: PublicationRef,
    acceptance_publication: PublicationRef,
    contracts: Vec<(RepositoryId, String)>,
    record: Signed<NoReleaseRequired>,
}
impl NoReleaseIntent {
    fn scope(&self) -> Result<String> {
        canonical_digest(1, self)
    }
    fn effect(&self) -> Result<Uuid> {
        if self.publication_operation.is_nil() || self.attempt.purpose != "no_release_required" {
            return Err(Error::new(
                "record_corrupt",
                "already-production publication attempt is invalid",
            ));
        }
        Ok(self.publication_operation)
    }
}
fn intent_leaf(operation: Uuid) -> String {
    format!("no-release-{operation}.json")
}
fn eligible_shape(parent: &VerifiedParent) -> bool {
    parent.manifest().candidates.iter().all(|candidate| {
        candidate.ordered_delta_digests.is_empty()
            && candidate.result.as_ref() == Some(&candidate.production)
    })
}
fn target_inventory(
    workspace: &WorkspaceDelivery,
    parent: &VerifiedParent,
) -> Result<BTreeMap<String, (String, RepositoryId)>> {
    let mut inventory = BTreeMap::new();
    for baseline in &parent.manifest().baselines {
        let contract = workspace
            .contracts
            .iter()
            .find(|c| c.repository == baseline.repository)
            .ok_or_else(|| {
                Error::new(
                    "scope_violation",
                    "already-production repository is no longer configured",
                )
            })?;
        match &contract.configuration.deployment {
            crate::config::Deployment::None(value) if value == "none" => (),
            crate::config::Deployment::Steps { steps } => {
                for step in steps {
                    if inventory
                        .insert(
                            step.target_id.clone(),
                            ("github".into(), contract.repository.clone()),
                        )
                        .is_some()
                    {
                        return Err(Error::new(
                            "no_release_gate",
                            "already-production target identity is ambiguous",
                        ));
                    }
                }
            }
            _ => {
                return Err(Error::new(
                    "no_release_gate",
                    "deployment inventory is incomplete",
                ));
            }
        }
    }
    Ok(inventory)
}
fn selected_contracts(
    workspace: &WorkspaceDelivery,
    parent: &VerifiedParent,
) -> Result<Vec<(RepositoryId, String)>> {
    let mut selected = BTreeMap::new();
    for baseline in &parent.manifest().baselines {
        let contract = workspace
            .contracts
            .iter()
            .find(|c| c.repository == baseline.repository)
            .ok_or_else(|| {
                Error::new(
                    "scope_violation",
                    "already-production baseline repository removed",
                )
            })?;
        if selected
            .insert(contract.repository.clone(), contract.digest()?)
            .is_some()
        {
            return Err(Error::new(
                "record_corrupt",
                "duplicate already-production repository",
            ));
        }
    }
    Ok(selected.into_iter().collect())
}
fn provider_target(record: &NoReleaseRequired) -> String {
    format!(
        "linear:{}:{}:{}",
        record.work.linear_endpoint, record.work.linear_workspace, record.work.parent
    )
}

fn same_targets(
    original: &[ProductionTargetBaseline],
    current: &[ProductionTargetBaseline],
) -> bool {
    let normalized = |targets: &[ProductionTargetBaseline]| {
        let mut targets = targets.to_vec();
        for target in &mut targets {
            target.observed_at = chrono::DateTime::UNIX_EPOCH;
            target.provider_record.clear();
        }
        targets.sort_by(|a, b| a.target.cmp(&b.target));
        targets
    };
    normalized(original) == normalized(current)
}

impl RuntimeDelivery {
    /// Rebuild dependency cache entries from this pass's native publications.
    /// Historical UI rows are retained separately.
    pub(in crate::runtime_delivery) fn clear_no_release_observations(&self) -> Result<()> {
        self.restore_releases()?;
        let mut state = self.releases.state.lock().map_err(|_| unavailable())?;
        state.no_release.clear();
        state.queue = rebuild_queue(&state.attempts, &state.no_release)?;
        Ok(())
    }
    async fn observe_no_release_targets(
        &self,
        workspace: &WorkspaceDelivery,
        parent: &VerifiedParent,
    ) -> Result<Vec<ProductionTargetBaseline>> {
        let mut targets = Vec::new();
        let mut seen = BTreeSet::new();
        for baseline in &parent.manifest().baselines {
            if !seen.insert(baseline.repository.clone()) {
                return Err(Error::new(
                    "no_release_gate",
                    "duplicate production baseline",
                ));
            }
            let contract = workspace
                .contracts
                .iter()
                .find(|c| c.repository == baseline.repository)
                .ok_or_else(|| {
                    Error::new(
                        "scope_violation",
                        "production baseline repository is not selected",
                    )
                })?;
            let current = workspace.observer.observe(contract).await?;
            if current.contract_digest() != contract.digest()?
                || current.baseline() != baseline
                || Utc::now() - current.observed_at() > Duration::seconds(30)
            {
                return Err(Error::new(
                    "stale_result",
                    "already-production baseline changed after accepted integrated evidence",
                ));
            }
            targets.extend_from_slice(current.targets());
        }
        Ok(targets)
    }
    fn persist_unadmitted_no_release_intent(
        &self,
        intent: &NoReleaseIntent,
        previous_digest: Option<&str>,
    ) -> Result<()> {
        let operation = intent.record.payload.operation_id;
        match self.runtime.control.operation(operation) {
            Ok(_) => {
                return Err(Error::new(
                    "operation_conflict",
                    "an admitted already-production intent cannot be replaced",
                ));
            }
            Err(error) if error.code == "record_missing" => (),
            Err(error) => return Err(error),
        }
        if self.runtime.control.operations()?.iter().any(|receipt| {
            receipt.operation_id == intent.publication_operation
                || receipt.expected.get("admission_operation") == Some(&operation.to_string())
        }) {
            return Err(Error::new(
                "operation_conflict",
                "an existing effect prevents already-production replanning",
            ));
        }
        let previous = match self.load_no_release_intent(operation) {
            Ok(previous) => Some(previous),
            Err(error) if error.code == "record_missing" => None,
            Err(error) => return Err(error),
        };
        let observed = previous
            .as_ref()
            .map(|previous| canonical_digest(1, previous))
            .transpose()?;
        if observed.as_deref() != previous_digest {
            return Err(Error::new(
                "ownership_conflict",
                "unadmitted already-production intent changed during replanning",
            ));
        }
        if let Some(previous) = previous
            && observed.as_deref() != Some(canonical_digest(1, intent)?.as_str())
        {
            let digest = observed.as_ref().unwrap().rsplit(':').next().unwrap();
            self.records.write(
                &format!("no-release-unadmitted-{operation}-{digest}.json"),
                &self
                    .runtime
                    .services
                    .key
                    .sign("runtime_no_release_intent", &previous)?,
            )?;
        }
        self.records.write(
            &intent_leaf(operation),
            &self
                .runtime
                .services
                .key
                .sign("runtime_no_release_intent", intent)?,
        )
    }

    fn load_no_release_intent(&self, operation: Uuid) -> Result<NoReleaseIntent> {
        let signed: Signed<NoReleaseIntent> = self.records.read(&intent_leaf(operation))?;
        self.runtime
            .services
            .key
            .verify("runtime_no_release_intent", &signed)?;
        let intent = signed.payload;
        self.runtime
            .services
            .key
            .verify("no_release_required", &intent.record)?;
        if intent.schema_version != 1 || intent.record.payload.operation_id != operation {
            return Err(Error::new(
                "record_corrupt",
                "already-production intent identity changed",
            ));
        }
        Ok(intent)
    }
}

impl RuntimeDelivery {
    fn check_no_release_intent(
        runtime: &RuntimeCoordinator,
        intent: &NoReleaseIntent,
        gate: &FreshGate,
        parent: &VerifiedParent,
        acceptance: &VerifiedAcceptance,
    ) -> Result<()> {
        let config = runtime.configuration.snapshot()?;
        runtime
            .configuration
            .authority_fences()?
            .check_workspace(intent.workspace)?;
        if config.revision()? != intent.config_revision
            || gate.config_revision() != intent.config_revision
            || gate.workflow_digest() != intent.workflow_digest
            || gate.workspace_id() != intent.workspace
            || gate.work() != &intent.record.payload.work
            || config.runtime.id != intent.record.payload.runtime_id
            || parent.token() != intent.record.payload.evidence_token
            || acceptance.digest() != intent.record.payload.acceptance_digest
            || acceptance.acceptance().runtime_id != config.runtime.id
        {
            return Err(Error::new(
                "stale_result",
                "already-production intent no longer binds current accepted work",
            ));
        }
        let workspace = config
            .workspaces
            .values()
            .find(|w| w.id == intent.workspace)
            .ok_or_else(|| Error::new("scope_violation", "already-production workspace removed"))?;
        let actor: Uuid =
            acceptance.acceptance().linear_actor.parse().map_err(|_| {
                Error::new("identity_mismatch", "acceptance actor is not canonical")
            })?;
        if !workspace.actors.acceptors.contains(&actor) {
            return Err(Error::new(
                "access_denied",
                "already-production acceptance actor was removed",
            ));
        }
        let projection = runtime.observations.admission_snapshot()?;
        let current = projection.parent(intent.workspace, gate.work().parent)?;
        let fresh = current.fresh_gate(
            None,
            GatePurpose::NewOperation,
            &runtime.dependency_outcomes()?,
            Utc::now(),
            Duration::seconds(30),
        )?;
        gate.compare_current(&fresh, Utc::now(), Duration::seconds(30))?;
        let comments = &current
            .observations
            .get(&gate.work().parent)
            .ok_or_else(|| {
                Error::new("incompletely_observed", "already-production parent missing")
            })?
            .comments;
        for reference in [&intent.parent_publication, &intent.acceptance_publication] {
            if comments
                .iter()
                .filter(|c| {
                    c.id == reference.id
                        && c.body_digest == reference.body_digest
                        && c.user_id == Some(reference.writer)
                        && !c.bot
                        && !c.delegated
                        && c.issue_id == gate.work().parent
                })
                .count()
                != 1
            {
                return Err(Error::new(
                    "stale_result",
                    "accepted already-production publication changed or disappeared",
                ));
            }
        }
        current_parent_publication(
            &runtime.services.key,
            config.runtime.id,
            intent.parent_publication.writer,
            current,
            parent,
        )?;
        for other in comments {
            if other.id != intent.acceptance_publication.id
                && other.user_id == Some(intent.acceptance_publication.writer)
                && !other.bot
                && !other.delegated
                && other.issue_id == gate.work().parent
                && let Ok(signed) = evidence::decode_signed_publication::<evidence::Acceptance>(
                    &other.body,
                    "human_acceptance",
                    &runtime.services.key,
                )
                && signed.payload.work == *gate.work()
                && signed.payload.reviewed_token == parent.token()
            {
                return Err(Error::new(
                    "stale_result",
                    "already-production acceptance is ambiguous",
                ));
            }
        }
        Ok(())
    }
}

impl RuntimeDelivery {
    fn no_release_publication(
        &self,
        intent: &NoReleaseIntent,
        comment: &Comment,
        receipt: &Receipt,
    ) -> Result<Publication> {
        let record = &intent.record.payload;
        let body = evidence::encode_signed_publication("no_release_required", &intent.record)?;
        let admission = self.runtime.control.operation(record.operation_id)?;
        if comment.body != body
            || comment.body_digest != bytes_digest(body.as_bytes())
            || comment.issue_id != record.work.parent
            || comment.bot
            || comment.delegated
            || comment.user_id != Some(intent.parent_publication.writer)
            || receipt.runtime_id != record.runtime_id
            || receipt.operation_id != intent.effect()?
            || receipt.kind != "linear_comment"
            || !matches!(
                receipt.phase,
                ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
            )
            || receipt.remote_ids.get("comment_id") != Some(&comment.id.to_string())
            || receipt.expected.get("provider_body_digest") != Some(&comment.body_digest)
            || receipt.expected.get("provider_writer")
                != Some(&intent.parent_publication.writer.to_string())
            || receipt.expected.get("no_release_digest") != Some(&intent.record.digest()?)
            || receipt.expected.get("admission_operation") != Some(&record.operation_id.to_string())
            || receipt.canonical_targets != vec![provider_target(record)]
            || admission.canonical_targets
                != vec![
                    IssueKey::new(
                        &record.work.linear_endpoint,
                        record.work.linear_workspace,
                        record.work.parent,
                    )?
                    .identity()?,
                ]
            || receipt.actor != admission.actor
            || receipt.expected.get("admitted_scope") != admission.expected.get("scope")
            || admission.kind != "admission"
            || admission.phase != ReceiptPhase::Reconciled
            || admission
                .observed_result
                .as_ref()
                .is_none_or(|result| result["local_execution"] != "exited")
            || admission.runtime_id != record.runtime_id
            || admission.expected.get("scope") != Some(&intent.scope()?)
            || admission.owner_generation != receipt.owner_generation
            || crate::control::MetadataReference::from_receipt(&admission)?
                .as_ref()
                .map(|reference| canonical_digest(1, reference))
                .transpose()?
                != Some(canonical_digest(1, &intent.attempt)?)
            || receipt.expected.get("snapshot") != Some(&intent.attempt.generation)
        {
            return Err(Error::new(
                "reconciliation_required",
                "already-production publication lacks its exact actual host receipt and admission",
            ));
        }
        let current = self.runtime.control.snapshot()?;
        let work = IssueKey::new(
            &record.work.linear_endpoint,
            record.work.linear_workspace,
            record.work.parent,
        )?
        .identity()?;
        if current.unresolved_operations.values().any(|targets| {
            targets.contains(&work) || targets.contains(&record.runtime_id.to_string())
        }) {
            return Err(Error::new(
                "reconciliation_required",
                "already-production work retains unresolved effects",
            ));
        }
        Ok(Publication {
            provider: "linear".into(),
            record_id: comment.id.to_string(),
            writer_account_id: intent.parent_publication.writer.to_string(),
            payload_digest: canonical_digest(1, &intent.record)?,
            observed_at: Utc::now(),
            url: comment.url.clone(),
        })
    }
}

impl RuntimeDelivery {
    pub(in crate::runtime_delivery) async fn hydrate_no_release(
        &self,
        workspace: &WorkspaceDelivery,
        projection: &ParentProjection,
        snapshot: &mut VerifiedDeliverySnapshot,
    ) -> Result<()> {
        let (Some(parent), Some(acceptance)) = (
            snapshot.parent_delivery().verified_parent(),
            snapshot.parent_delivery().verified_acceptance(),
        ) else {
            return Ok(());
        };
        if !eligible_shape(parent) {
            return Ok(());
        }
        let comments = &projection
            .observations
            .get(&projection.key.issue)
            .ok_or_else(|| {
                Error::new(
                    "incompletely_observed",
                    "already-production parent not observed",
                )
            })?
            .comments;
        let selected = workspace
            .configuration
            .workspaces
            .values()
            .find(|selected| selected.id == projection.workspace_id)
            .ok_or_else(|| Error::new("scope_violation", "already-production workspace removed"))?;
        let writer: Uuid = workspace.configuration.connections[&selected.linear_connection]
            .account_id
            .parse()
            .map_err(|_| {
                Error::new(
                    "identity_mismatch",
                    "already-production writer is not canonical",
                )
            })?;
        let mut matching = Vec::new();
        for comment in comments {
            if comment.user_id != Some(writer)
                || comment.bot
                || comment.delegated
                || comment.issue_id != projection.key.issue
            {
                continue;
            }
            let Ok(record) = evidence::decode_signed_publication::<NoReleaseRequired>(
                &comment.body,
                "no_release_required",
                &self.runtime.services.key,
            ) else {
                continue;
            };
            if record.payload.runtime_id == workspace.configuration.runtime.id
                && &record.payload.work == snapshot.work()
                && record.payload.evidence_token == parent.token()
                && record.payload.acceptance_digest == acceptance.digest()
            {
                matching.push((comment, record));
            }
        }
        if matching.is_empty() {
            return Ok(());
        }
        if matching.len() != 1 {
            return Err(Error::new(
                "reconciliation_required",
                "already-production publications are ambiguous",
            ));
        }
        let (comment, record) = matching.remove(0);
        let intent = self.load_no_release_intent(record.payload.operation_id)?;
        if intent.record.digest()? != record.digest()? {
            return Err(Error::new(
                "record_corrupt",
                "published already-production record differs from immutable intent",
            ));
        }
        Self::check_no_release_intent(
            &self.runtime,
            &intent,
            snapshot.authority(),
            parent,
            acceptance,
        )?;
        if intent.contracts != selected_contracts(workspace, parent)? {
            return Err(Error::new(
                "stale_config",
                "already-production repository contracts changed",
            ));
        }
        let current = self.observe_no_release_targets(workspace, parent).await?;
        if !same_targets(&record.payload.targets, &current) {
            return Err(Error::new(
                "stale_result",
                "already-production target source/artifact/provider identity changed",
            ));
        }
        let publication = self.no_release_publication(
            &intent,
            comment,
            &self.runtime.control.operation(intent.effect()?)?,
        )?;
        let inventory = target_inventory(workspace, parent)?;
        let proof = evidence::verify_no_release_required(
            &record,
            parent,
            acceptance,
            NoReleaseRequirements {
                current_work: snapshot.work(),
                pending_correction: false,
                targets: &inventory,
            },
            &publication,
            &intent.parent_publication.writer.to_string(),
            &self.runtime.services.key,
        )?;
        Self::check_no_release_intent(
            &self.runtime,
            &intent,
            snapshot.authority(),
            parent,
            acceptance,
        )?;
        {
            let mut state = self.releases.state.lock().map_err(|_| unavailable())?;
            state.queue.record_no_release_required(&proof)?;
            state
                .no_release
                .insert(proof.record().operation_id, proof.clone());
        }
        snapshot.no_release_required(proof, publication.url)?;
        Ok(())
    }
}

impl RuntimeDelivery {
    /// An accepted unchanged parent must not manufacture an empty release graph.
    pub(super) async fn advance_no_release(
        &self,
        lease: &Lease,
        projection: &RuntimeProjection,
        workspace_id: Uuid,
        parent_id: Uuid,
    ) -> Result<bool> {
        let projection_parent = projection.parent(workspace_id, parent_id)?;
        let delivery = self.runtime.current_delivery(&projection_parent.key)?;
        let (Some(parent), Some(acceptance)) =
            (delivery.verified_parent(), delivery.verified_acceptance())
        else {
            return Ok(false);
        };
        if !eligible_shape(parent) {
            return Ok(false);
        }
        if delivery.no_release_required().is_some() {
            return Ok(true);
        }
        let workspace = self.workspace(projection, workspace_id)?;
        let gate = projection_parent.fresh_gate(
            None,
            GatePurpose::NewOperation,
            &self.runtime.dependency_outcomes()?,
            Utc::now(),
            Duration::seconds(30),
        )?;
        let (parent_publication, acceptance_publication) = self.publication_refs(
            &workspace.configuration,
            projection_parent,
            parent,
            acceptance,
        )?;
        let identity = canonical_digest(
            1,
            &(
                workspace_id,
                gate.work(),
                parent.token(),
                acceptance.digest(),
            ),
        )?;
        let mut plan = crate::preparation::metadata_publication_attempt(
            "no_release_required",
            lease.runtime_id,
            &identity,
            &gate.work_key()?,
            &self.runtime.control.operations()?,
            &workspace.configuration.runtime,
            Utc::now(),
        )?;
        let operation = plan.admission;
        let targets = self.observe_no_release_targets(&workspace, parent).await?;
        let inventory = target_inventory(&workspace, parent)?;
        let previous = match self.load_no_release_intent(operation) {
            Ok(intent) => Some(intent),
            Err(error) if error.code == "record_missing" => None,
            Err(error) => return Err(error),
        };
        if let Some(previous) = &previous {
            if previous.attempt.generation != plan.reference.generation
                || previous.attempt.purpose != plan.reference.purpose
                || previous.attempt.attempt != plan.reference.attempt
            {
                return Err(Error::new(
                    "record_corrupt",
                    "unadmitted publication budget lineage changed",
                ));
            }
            plan.reference.max_attempts = plan
                .reference
                .max_attempts
                .min(previous.attempt.max_attempts);
            if plan.reference.attempt > plan.reference.max_attempts {
                return Err(Error::new(
                    "attempts_exhausted",
                    "unadmitted publication retains its original finite cap",
                ));
            }
        }
        let previous_digest = previous
            .as_ref()
            .map(|intent| canonical_digest(1, intent))
            .transpose()?;
        let reusable = previous.as_ref().filter(|intent| {
            intent.config_revision == gate.config_revision()
                && intent.workflow_digest == gate.workflow_digest()
                && intent.parent_publication == parent_publication
                && intent.acceptance_publication == acceptance_publication
                && canonical_digest(1, &intent.attempt) == canonical_digest(1, &plan.reference)
        });
        let intent = if let Some(intent) = reusable {
            intent.clone()
        } else {
            let record = evidence::prepare_no_release_required(
                NoReleaseRequired {
                    schema_version: 1,
                    runtime_id: lease.runtime_id,
                    operation_id: operation,
                    work: gate.work().clone(),
                    evidence_token: parent.token().into(),
                    acceptance_digest: acceptance.digest().into(),
                    baselines: parent.manifest().baselines.clone(),
                    targets: targets.clone(),
                    recorded_at: Utc::now(),
                },
                parent,
                acceptance,
                NoReleaseRequirements {
                    current_work: gate.work(),
                    pending_correction: false,
                    targets: &inventory,
                },
                &self.runtime.services.key,
            )?;
            NoReleaseIntent {
                schema_version: 1,
                publication_operation: plan.operation,
                attempt: plan.reference.clone(),
                workspace: workspace_id,
                config_revision: gate.config_revision().into(),
                workflow_digest: gate.workflow_digest().into(),
                parent_publication,
                acceptance_publication,
                contracts: selected_contracts(&workspace, parent)?,
                record,
            }
        };
        Self::check_no_release_intent(&self.runtime, &intent, &gate, parent, acceptance)?;
        if intent.contracts != selected_contracts(&workspace, parent)? {
            return Err(Error::new(
                "stale_config",
                "already-production repository contracts changed",
            ));
        }
        if !same_targets(&intent.record.payload.targets, &targets) {
            return Err(Error::new(
                "stale_result",
                "already-production intent targets changed",
            ));
        }
        match self.runtime.control.operation(operation) {
            Ok(_) => {
                return Err(Error::new(
                    "reconciliation_required",
                    "prior already-production publication requires exact readback or explicit operation reconciliation; it is not resent",
                ));
            }
            Err(error) if error.code == "record_missing" => (),
            Err(error) => return Err(error),
        }
        let broker = self.provider_broker(&workspace.configuration, workspace_id)?;
        let body = evidence::encode_signed_publication("no_release_required", &intent.record)?;
        let effect = intent.effect()?;
        let record_digest = intent.record.digest()?;
        let permit = self.runtime.control.admit_metadata(
            lease,
            crate::control::MetadataAdmission {
                request_id: operation,
                work_key: gate.work_key()?,
                scope_digest: intent.scope()?,
                reference: plan.reference.clone(),
            },
            || {
                self.runtime.registration.verify_active_owner(lease)?;
                Self::check_no_release_intent(&self.runtime, &intent, &gate, parent, acceptance)?;
                self.releases
                    .state
                    .lock()
                    .map_err(|_| unavailable())?
                    .queue
                    .check_no_release_resources(&intent.record.payload)?;
                self.persist_unadmitted_no_release_intent(&intent, previous_digest.as_deref())
            },
        )?;
        let runtime = self.runtime.clone();
        let state = self.releases.state.clone();
        let checked_intent = intent.clone();
        let checked_gate = gate.clone();
        let checked_parent = parent.clone();
        let checked_acceptance = acceptance.clone();
        let checked_lease = lease.clone();
        let request = MutationRequest {
            request_id: effect,
            runtime_id: lease.runtime_id,
            actor: Actor {
                identity: format!("owner:{}", lease.owner_id),
                roles: BTreeSet::new(),
                principal: None,
            },
            kind: "linear_comment".into(),
            canonical_targets: vec![gate.work_key()?],
            expected: BTreeMap::from([
                ("no_release_digest".into(), record_digest),
                ("snapshot".into(), identity.clone()),
                (
                    "metadata_attempt".into(),
                    plan.reference.attempt.to_string(),
                ),
                (
                    "metadata_max_attempts".into(),
                    plan.reference.max_attempts.to_string(),
                ),
            ]),
            payload: json!({}),
        };
        let effect_host = self.runtime.control.clone();
        let effect_broker = broker.clone();
        // This owned finite task survives its caller being cancelled. It cannot
        // send a second effect, and retires only after the broker's task joins.
        let owned = tokio::spawn(async move {
            let result = effect_broker
                .mutate_observed(
                    effect_host.clone(),
                    request,
                    Some(permit),
                    None,
                    ProviderMutation::LinearComment {
                        issue: parent_id,
                        body,
                    },
                    move |_| {
                        runtime.registration.verify_active_owner(&checked_lease)?;
                        Self::check_no_release_intent(
                            &runtime,
                            &checked_intent,
                            &checked_gate,
                            &checked_parent,
                            &checked_acceptance,
                        )?;
                        state
                            .lock()
                            .map_err(|_| unavailable())?
                            .queue
                            .check_no_release_resources(&checked_intent.record.payload)
                    },
                )
                .await;
            effect_host.retire_local(
                operation,
                json!({"no_release_publication_task":"joined",
                "effect":effect, "acknowledged":result.is_ok(), "error":result.as_ref().err()}),
            )?;
            result
        });
        let receipt = owned.await.map_err(|_| {
            Error::new(
                "operation_uncertain",
                "already-production publication task did not join",
            )
        })??;
        let linear = broker
            .linear
            .as_ref()
            .ok_or_else(|| Error::new("prerequisite", "Linear writer unavailable"))?;
        let comments = linear.comments(parent_id).await?;
        let id = receipt.remote_ids.get("comment_id").ok_or_else(|| {
            Error::new(
                "reconciliation_required",
                "already-production acknowledgement has no comment identity",
            )
        })?;
        let comment = comments
            .iter()
            .find(|c| c.id.to_string() == *id)
            .ok_or_else(|| {
                Error::new(
                    "evidence_unpublished",
                    "already-production publication has not been observed",
                )
            })?;
        self.no_release_publication(&intent, comment, &receipt)?;
        // The next complete refresh reconstructs the current snapshot,
        // dependency outcome and history from this exact native publication.
        Ok(true)
    }
}

impl RuntimeDelivery {
    pub(super) async fn reconcile_no_release(
        &self,
        decision: &crate::decisions::AuthorizedOperationRecovery,
    ) -> Result<Option<Receipt>> {
        let target = decision.operation();
        let admission_id = if target.kind == "linear_comment"
            && target.expected.contains_key("no_release_digest")
        {
            target
                .expected
                .get("admission_operation")
                .and_then(|value| value.parse().ok())
                .ok_or_else(|| {
                    Error::new(
                        "record_corrupt",
                        "already-production receipt has no exact admission",
                    )
                })?
        } else if target.kind == "admission"
            && crate::control::MetadataReference::from_receipt(target)?
                .is_some_and(|reference| reference.purpose == "no_release_required")
        {
            target.operation_id
        } else {
            return Ok(None);
        };
        let intent = self.load_no_release_intent(admission_id)?;
        let record = &intent.record.payload;
        let admission = self.runtime.control.operation(admission_id)?;
        let effect = self.runtime.control.operation(intent.effect()?)?;
        let work = IssueKey::new(
            &record.work.linear_endpoint,
            record.work.linear_workspace,
            record.work.parent,
        )?
        .identity()?;
        let body = evidence::encode_signed_publication("no_release_required", &intent.record)?;
        if admission.kind != "admission"
            || admission.runtime_id != record.runtime_id
            || admission.expected.get("scope") != Some(&intent.scope()?)
            || admission.canonical_targets != vec![work.clone()]
            || crate::control::MetadataReference::from_receipt(&admission)?
                .as_ref()
                .map(|value| canonical_digest(1, value))
                .transpose()?
                != Some(canonical_digest(1, &intent.attempt)?)
            || effect.kind != "linear_comment"
            || effect.runtime_id != record.runtime_id
            || effect.owner_generation != admission.owner_generation
            || effect.actor != admission.actor
            || effect.expected.get("admission_operation") != Some(&admission_id.to_string())
            || effect.expected.get("admitted_scope") != admission.expected.get("scope")
            || effect.expected.get("no_release_digest") != Some(&intent.record.digest()?)
            || effect.expected.get("provider_body_digest") != Some(&bytes_digest(body.as_bytes()))
            || effect.expected.get("provider_writer")
                != Some(&intent.parent_publication.writer.to_string())
            || effect.expected.get("snapshot") != Some(&intent.attempt.generation)
            || effect.canonical_targets != vec![provider_target(record)]
            || ![admission_id, effect.operation_id].contains(&target.operation_id)
        {
            return Err(Error::new(
                "ownership_conflict",
                "already-production recovery differs from immutable effect lineage",
            ));
        }
        let config = self.runtime.configuration.snapshot()?;
        let workspace = config
            .workspaces
            .values()
            .find(|workspace| workspace.id == intent.workspace)
            .ok_or_else(|| {
                Error::new(
                    "scope_violation",
                    "original publication workspace is not configured",
                )
            })?;
        let broker = self.provider_broker(&config, intent.workspace)?;
        let linear = broker.linear.as_ref().ok_or_else(|| {
            Error::new(
                "prerequisite",
                "original publication connection unavailable",
            )
        })?;
        if IssueKey::new(
            config.connections[&workspace.linear_connection]
                .endpoint
                .as_deref()
                .ok_or_else(|| {
                    Error::new("scope_violation", "original Linear endpoint unavailable")
                })?,
            workspace.linear_workspace_id,
            record.work.parent,
        )?
        .identity()?
            != IssueKey::new(
                &record.work.linear_endpoint,
                record.work.linear_workspace,
                record.work.parent,
            )?
            .identity()?
            || workspace.linear_workspace_id != record.work.linear_workspace
            || config.connections[&workspace.linear_connection].account_id
                != intent.parent_publication.writer.to_string()
            || linear.identity().await?.account_id != intent.parent_publication.writer.to_string()
        {
            return Err(Error::new(
                "identity_mismatch",
                "original publication account/workspace is not selected",
            ));
        }
        let comments = linear.comments(record.work.parent).await?;
        let matching = comments
            .iter()
            .filter(|comment| {
                comment.body == body
                    && comment.body_digest == bytes_digest(body.as_bytes())
                    && comment.issue_id == record.work.parent
                    && !comment.bot
                    && !comment.delegated
                    && comment.user_id == Some(intent.parent_publication.writer)
                    && effect
                        .remote_ids
                        .get("comment_id")
                        .is_none_or(|id| *id == comment.id.to_string())
            })
            .collect::<Vec<_>>();
        let [comment] = matching.as_slice() else {
            return Err(Error::new(
                "operation_uncertain",
                "recovery needs one exact existing already-production publication; absence never authorizes replay",
            ));
        };
        let publication = Publication {
            provider: "linear".into(),
            record_id: comment.id.to_string(),
            writer_account_id: intent.parent_publication.writer.to_string(),
            payload_digest: canonical_digest(1, &intent.record)?,
            observed_at: Utc::now(),
            url: comment.url.clone(),
        };
        // Recovery records the old publication fact only. It does not reissue a
        // current no-release capability, renew acceptance or change a generation.
        let target_digest = canonical_digest(1, target)?;
        let actual = self.runtime.control.reconcile_operation_observed(decision.actor(), target.operation_id, |current, snapshot| {
            self.operation_recovery_gate(decision, snapshot)?;
            if self.runtime.configuration.snapshot()?.revision()? != config.revision()?
                || canonical_digest(1, current)? != target_digest
                || snapshot.active_operations.iter().any(|permit| permit.operation_id == admission_id)
                || snapshot.pending_external_effects.contains(&effect.operation_id)
            { return Err(Error::new("operation_active", "original already-production task remains active or recovery observation changed")); }
            let result = if current.operation_id == admission_id {
                let actual_effect = self.runtime.control.operation(effect.operation_id)?;
                if actual_effect.phase != ReceiptPhase::Reconciled
                    || actual_effect.remote_ids.get("comment_id") != Some(&comment.id.to_string())
                    || self.runtime.control.operations()?.iter().any(|receipt| {
                        receipt.operation_id != effect.operation_id && receipt.expected.get("admission_operation") == Some(&admission_id.to_string())
                    })
                { return Err(Error::new("reconciliation_required", "reconcile the sole exact publication effect before retiring its original admission")); }
                json!({"local_execution":"exited","unresolved_effects":[],"observation":{"publication":publication,"recovery":"original_metadata_task_verified"}})
            } else {
                json!({"publication":publication,"no_release_digest":intent.record.digest()?,"recovery":"observed_existing_native_record"})
            };
            Ok(crate::control::Reconciliation::UniqueVerified {
                remote_ids: BTreeMap::from([("comment_id".into(), comment.id.to_string())]),
                result, payload_digest: current.payload_digest.clone(), actor: current.actor.clone(),
            })
        })?;
        Ok(Some(actual))
    }
}

#[cfg(test)]
#[path = "runtime_delivery_no_release_tests.rs"]
mod tests;
