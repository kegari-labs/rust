//! Serialized release ownership and durable observations. Retained records are
//! recovery facts; only the live finite plan permit can dispatch another step.
use super::child::{ActivePhase, PhaseRequest};
use super::parent::ParentSources;
use super::*;
use crate::{
    authority::IssueKey,
    config::ReleaseMode,
    control::{ControlSnapshot, Intent, Lease, Permit, ReceiptPhase},
    delivery::*,
    evidence::{
        self, Signed, VerifiedAcceptance, VerifiedParent, VerifiedProduction, WorkIdentity,
    },
    release::{
        self as domain, AdmissionFacts, DeliveryEvidence, FreshReleaseFacts, ReleaseAttempt,
        ReleasePlan, ReleaseQueue, StepState,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeSet, sync::Mutex};

#[path = "runtime_delivery_no_release.rs"]
mod no_release;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicationRef {
    id: Uuid,
    body_digest: String,
    writer: Uuid,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ManualRole {
    Release,
    Recover,
}
impl ManualRole {
    fn as_str(self) -> &'static str {
        match self {
            Self::Release => "release",
            Self::Recover => "recover",
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AttemptRecord {
    schema_version: u32,
    runtime: Uuid,
    workspace: Uuid,
    work: WorkIdentity,
    config_revision: String,
    workflow_digest: String,
    parent_publication: PublicationRef,
    acceptance_publication: PublicationRef,
    manual_intent: Option<Uuid>,
    manual_actor: Option<crate::control::Actor>,
    manual_role: Option<ManualRole>,
    acceptance_actor: Uuid,
    #[serde(with = "repository_operations")]
    preparations: BTreeMap<RepositoryId, Uuid>,
    attempt: ReleaseAttempt,
    admission_confirmed: bool,
    admission_operation: Uuid,
    recovery_receipts: BTreeMap<Uuid, String>,
    local_retired: bool,
    pending_receipts: BTreeMap<Uuid, String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PreparationRef {
    schema_version: u32,
    workspace: Uuid,
    work: WorkIdentity,
    evidence_token: String,
    operation: Uuid,
    #[serde(with = "repository_operations")]
    repositories: BTreeMap<RepositoryId, Uuid>,
}
struct RepoPreparation {
    operation: Uuid,
    phase: Option<ActivePhase>,
    candidate: Option<PreparedProductionCandidate>,
    production: Option<VerifiedProduction>,
}
struct Preparation {
    reference: PreparationRef,
    sources: ParentSources,
    repositories: BTreeMap<RepositoryId, RepoPreparation>,
    error: Option<Error>,
}
struct LiveRelease {
    permit: Permit,
    gate: FreshGate,
    parent: VerifiedParent,
    acceptance: VerifiedAcceptance,
    eligibility: domain::VerifiedReleaseEligibility,
    preparation: Arc<tokio::sync::Mutex<Preparation>>,
    admissions: BTreeMap<String, DeliveryAdmission>,
    workspace: WorkspaceDelivery,
    _active: crate::observations::ActiveSnapshot,
}
#[derive(Default)]
struct ReleaseState {
    restored: bool,
    attempts: BTreeMap<Uuid, AttemptRecord>,
    no_release: BTreeMap<Uuid, evidence::VerifiedNoReleaseRequired>,
    queue: ReleaseQueue,
}
// JSON object keys cannot encode RepositoryId structs. Preserve canonical
// repository ordering as strict tuple arrays in authenticated records.
mod repository_operations {
    use super::*;
    use serde::{Deserializer, Serializer, de::Error as _, ser::Error as _};
    pub fn serialize<S: Serializer>(
        map: &BTreeMap<RepositoryId, Uuid>,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        if map.len() > 1024 {
            return Err(S::Error::custom("too many repository operations"));
        }
        for (repository, operation) in map {
            repository.validate().map_err(S::Error::custom)?;
            if operation.is_nil() {
                return Err(S::Error::custom("nil repository operation"));
            }
        }
        map.iter().collect::<Vec<_>>().serialize(serializer)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<BTreeMap<RepositoryId, Uuid>, D::Error> {
        let entries = Vec::<(RepositoryId, Uuid)>::deserialize(deserializer)?;
        if entries.len() > 1024 {
            return Err(D::Error::custom("too many repository operations"));
        }
        let mut result = BTreeMap::new();
        for (repository, operation) in entries {
            repository.validate().map_err(D::Error::custom)?;
            if operation.is_nil()
                || result
                    .last_key_value()
                    .is_some_and(|(prior, _)| prior >= &repository)
            {
                return Err(D::Error::custom(
                    "repository operations must be unique, sorted and nonnil",
                ));
            }
            result.insert(repository, operation);
        }
        Ok(result)
    }
}
type PreparationKey = (Uuid, Uuid, String);
fn preparation_key(workspace: Uuid, work: &WorkIdentity, token: &str) -> PreparationKey {
    (workspace, work.parent, token.to_owned())
}
type Preparations = BTreeMap<PreparationKey, Arc<tokio::sync::Mutex<Preparation>>>;
#[derive(Default)]
pub(super) struct Releases {
    state: Arc<Mutex<ReleaseState>>,
    preparations: Mutex<Preparations>,
    live: Mutex<BTreeMap<Uuid, Arc<tokio::sync::Mutex<LiveRelease>>>>,
    progressing: tokio::sync::Mutex<()>,
}
fn unavailable() -> Error {
    Error::new("runtime_unavailable", "release ownership lock failed")
}
fn attempt_leaf(operation: Uuid) -> String {
    format!("release-attempt-{operation}.json")
}
fn preparation_leaf(workspace: Uuid, token: &str) -> Result<String> {
    evidence::validate_digest(token)?;
    Ok(format!(
        "release-preparation-{workspace}-{}.json",
        token.rsplit(':').next().unwrap()
    ))
}
fn admission_scope(receipt: &crate::control::Receipt, plan: &ReleasePlan) -> Result<String> {
    if receipt.operation_id == plan.operation_id {
        if [
            "run",
            "metadata",
            "continuation",
            "source_advanced",
            "worker_preparation",
            "release_resume",
        ]
        .iter()
        .any(|key| receipt.expected.contains_key(*key))
        {
            return Err(Error::new(
                "ownership_conflict",
                "release origin is another admission kind",
            ));
        }
        return plan.digest();
    }
    let reference =
        crate::control::ReleaseResumeReference::from_receipt(receipt)?.ok_or_else(|| {
            Error::new(
                "ownership_conflict",
                "release admission has no exact resume lineage",
            )
        })?;
    if reference.plan_operation_id() != plan.operation_id
        || reference.plan_digest() != plan.digest()?
    {
        return Err(Error::new(
            "ownership_conflict",
            "resumed admission belongs to another plan",
        ));
    }
    reference.scope_digest()
}
fn issue(record: &AttemptRecord) -> Result<IssueKey> {
    IssueKey::new(
        &record.work.linear_endpoint,
        record.work.linear_workspace,
        record.work.parent,
    )
}
// A held plan cannot dispatch another step. Once every owned effect is known,
// its finite local claim can retire while the resource hold remains intact.
fn ready_for_local_retirement(record: &AttemptRecord) -> bool {
    record.pending_receipts.is_empty()
        && !record.attempt.has_unknown()
        && (record.attempt.completed()
            || record.attempt.resolved
            || record.attempt.abandoned
            || record.attempt.hold_reason.is_some())
}
fn rebuild_queue(
    attempts: &BTreeMap<Uuid, AttemptRecord>,
    no_release: &BTreeMap<Uuid, evidence::VerifiedNoReleaseRequired>,
) -> Result<ReleaseQueue> {
    let mut queue = ReleaseQueue::default();
    // These proofs were admitted against the then-current queue and reobserved
    // from native publications. Reconstruct history before later owners/holds;
    // recording history never releases a resource or consumes spacing.
    for proof in no_release.values() {
        queue.record_no_release_required(proof)?;
    }
    // Resource locks and unresolved holds are reconstructed before eligibility
    // is considered. A completed old release remains spacing/dependency history.
    for record in attempts.values() {
        record.attempt.validate_recovered()?;
        let mut ownership = record.attempt.clone();
        if !record.pending_receipts.is_empty()
            || ((record.attempt.completed() || record.attempt.resolved) && !record.local_retired)
        {
            ownership.resolved = false;
            ownership.hold_reason = Some(
                "observed effects still need exact receipt reconciliation and local retirement"
                    .into(),
            );
        }
        queue.reconcile_attempt(&ownership)?;
    }
    for record in attempts
        .values()
        .filter(|r| r.attempt.completed() && r.pending_receipts.is_empty() && r.local_retired)
    {
        if record.attempt.hold_reason.is_none() {
            queue.record_success(&record.attempt)?;
        }
    }
    Ok(queue)
}
impl RuntimeDelivery {
    fn save_release(&self, record: &AttemptRecord) -> Result<()> {
        self.records.write(
            &attempt_leaf(record.attempt.plan.operation_id),
            &self
                .runtime
                .services
                .key
                .sign("runtime_release_attempt", record)?,
        )
    }
    fn restore_releases(&self) -> Result<()> {
        let mut state = self.releases.state.lock().map_err(|_| unavailable())?;
        if state.restored {
            return Ok(());
        }
        let config = self.runtime.configuration.snapshot()?;
        let mut records = BTreeMap::new();
        for name in self.records.records_bounded(65536)? {
            let Some(id) = name
                .strip_prefix("release-attempt-")
                .and_then(|s| s.strip_suffix(".json"))
            else {
                continue;
            };
            let signed: Signed<AttemptRecord> = self.records.read(&name)?;
            self.runtime
                .services
                .key
                .verify("runtime_release_attempt", &signed)?;
            let mut record = signed.payload;
            let plan = &record.attempt.plan;
            if record.schema_version != 1
                || record.runtime != config.runtime.id
                || record.work.child.is_some()
                || plan.runtime_id != record.runtime
                || plan.parent_id != record.work.parent
                || plan.linear_workspace != record.work.linear_workspace
                || plan.generation != record.work.generation
                || plan.operation_id.to_string() != id
                || record.preparations.len() > 1024
            {
                return Err(Error::new(
                    "record_corrupt",
                    "release attempt identity differs from retained filename/runtime",
                ));
            }
            record.attempt.validate_recovered()?;
            if record.admission_confirmed {
                let receipt = self.runtime.control.operation(record.admission_operation)?;
                let expected_scope = admission_scope(&receipt, plan)?;
                if receipt.kind != "admission"
                    || receipt.runtime_id != record.runtime
                    || receipt.expected.get("scope") != Some(&expected_scope)
                    || receipt.owner_generation != record.attempt.owner_generation
                    || receipt.canonical_targets != vec![issue(&record)?.identity()?]
                {
                    return Err(Error::new(
                        "ownership_conflict",
                        "release record lacks exact actual plan admission",
                    ));
                }
            }
            if !record.attempt.completed() && !record.attempt.resolved {
                // Never adopt a prior host's live permit or assume its network
                // tasks stopped. Metadata recovery may later establish outcomes.
                record.attempt.hold_reason.get_or_insert_with(|| {
                    "recovered release requires explicit known-outcome reconciliation".into()
                });
            }
            records.insert(record.attempt.plan.operation_id, record);
        }
        state.queue = rebuild_queue(&records, &state.no_release)?;
        state.attempts = records;
        state.restored = true;
        Ok(())
    }
    fn update_release<T>(
        &self,
        operation: Uuid,
        update: impl FnOnce(&mut AttemptRecord, &ControlSnapshot) -> Result<T>,
    ) -> Result<T> {
        self.runtime.control.serialize_observation(|snapshot| {
            let mut state = self.releases.state.lock().map_err(|_| unavailable())?;
            let mut record =
                state.attempts.get(&operation).cloned().ok_or_else(|| {
                    Error::new("operation_missing", "release attempt is unavailable")
                })?;
            let result = update(&mut record, snapshot)?;
            record.attempt.validate_recovered()?;
            let mut proposed = state.attempts.clone();
            proposed.insert(operation, record.clone());
            let queue = rebuild_queue(&proposed, &state.no_release)?;
            self.save_release(&record)?;
            state.attempts = proposed;
            state.queue = queue;
            Ok(result)
        })
    }
    fn publication_refs(
        &self,
        config: &Config,
        projection: &ParentProjection,
        parent: &VerifiedParent,
        acceptance: &VerifiedAcceptance,
    ) -> Result<(PublicationRef, PublicationRef)> {
        let workspace = config
            .workspaces
            .values()
            .find(|w| w.id == projection.workspace_id)
            .ok_or_else(|| Error::new("scope_violation", "release workspace removed"))?;
        let writer: Uuid = config.connections[&workspace.linear_connection]
            .account_id
            .parse()
            .map_err(|_| Error::new("identity_mismatch", "Linear writer is not canonical"))?;
        let observed = projection
            .observations
            .get(&projection.key.issue)
            .ok_or_else(|| {
                Error::new(
                    "incompletely_observed",
                    "release parent is not currently visible",
                )
            })?;
        let mut parents = Vec::new();
        let mut acceptances = Vec::new();
        for comment in &observed.comments {
            if comment.user_id != Some(writer)
                || comment.bot
                || comment.delegated
                || comment.issue_id != projection.key.issue
            {
                continue;
            }
            let reference = PublicationRef {
                id: comment.id,
                body_digest: comment.body_digest.clone(),
                writer,
            };
            if comment
                .body
                .starts_with("<!-- kegari:v0:parent_result -->\n")
            {
                let signed = match evidence::decode_signed_publication::<Value>(
                    &comment.body,
                    "parent_result",
                    &self.runtime.services.key,
                ) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if signed.payload["runtime_id"] == json!(config.runtime.id)
                    && signed.payload["evidence"]
                        == serde_json::to_value(parent.manifest())
                            .map_err(|_| Error::invalid("parent evidence encoding"))?
                {
                    parents.push(reference);
                }
            } else if comment
                .body
                .starts_with("<!-- kegari:v0:human_acceptance -->\n")
            {
                let signed = match evidence::decode_signed_publication::<evidence::Acceptance>(
                    &comment.body,
                    "human_acceptance",
                    &self.runtime.services.key,
                ) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if &signed.payload == acceptance.acceptance()
                    && workspace
                        .actors
                        .acceptors
                        .iter()
                        .any(|id| id.to_string() == signed.payload.linear_actor)
                {
                    acceptances.push(reference);
                }
            }
        }
        if parents.len() != 1 || acceptances.len() != 1 {
            return Err(Error::new(
                "stale_result",
                "release requires unique current native parent and acceptance publications",
            ));
        }
        let receipts = self.runtime.control.operations()?;
        for reference in parents.iter().chain(&acceptances) {
            if !receipts.iter().any(|r| {
                r.runtime_id == config.runtime.id
                    && r.kind == "linear_comment"
                    && matches!(
                        r.phase,
                        ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
                    )
                    && r.remote_ids.get("comment_id") == Some(&reference.id.to_string())
                    && r.expected.get("provider_body_digest") == Some(&reference.body_digest)
                    && r.expected.get("provider_writer") == Some(&writer.to_string())
                    && r.canonical_targets.contains(&format!(
                        "linear:{}:{}:{}",
                        projection.key.endpoint, projection.key.workspace, projection.key.issue
                    ))
            }) {
                return Err(Error::new(
                    "reconciliation_required",
                    "release publication lacks immutable acknowledged host receipt",
                ));
            }
        }
        Ok((parents.remove(0), acceptances.remove(0)))
    }
    fn check_release_publications(
        runtime: &RuntimeCoordinator,
        record: &AttemptRecord,
        active: Option<&FreshGate>,
    ) -> Result<()> {
        let config = runtime.configuration.snapshot()?;
        runtime
            .configuration
            .authority_fences()?
            .check_workspace(record.workspace)?;
        if active.is_none() && config.revision()? != record.config_revision {
            return Err(Error::new("stale_config", "release configuration changed"));
        }
        let workspace = config
            .workspaces
            .values()
            .find(|w| w.id == record.workspace)
            .ok_or_else(|| Error::new("scope_violation", "release workspace was removed"))?;
        if !workspace
            .actors
            .acceptors
            .contains(&record.acceptance_actor)
        {
            return Err(Error::new(
                "access_denied",
                "release acceptance actor was removed",
            ));
        }
        if record.manual_actor.is_some() != record.manual_role.is_some() {
            return Err(Error::new(
                "record_corrupt",
                "release command role is missing",
            ));
        }
        if let Some(actor) = &record.manual_actor {
            let role = record.manual_role.unwrap().as_str();
            crate::app::verify_current_actor(&config, actor, role)?;
            runtime
                .configuration
                .authority_fences()?
                .check_actor(actor, role)?;
        }
        let parent = if let Some(expected) = active {
            runtime
                .observations
                .active_parent(expected, &runtime.dependency_outcomes()?)?
        } else {
            runtime
                .observations
                .admission_snapshot()?
                .parent(record.workspace, record.work.parent)?
                .clone()
        };
        if parent.work.as_ref() != Some(&record.work)
            || parent.workflow_digest != record.workflow_digest
        {
            return Err(Error::new(
                "stale_result",
                "release work generation or workflow changed",
            ));
        }
        let comments = &parent
            .observations
            .get(&parent.key.issue)
            .ok_or_else(|| Error::new("incompletely_observed", "release parent is unavailable"))?
            .comments;
        for reference in [&record.parent_publication, &record.acceptance_publication] {
            if comments
                .iter()
                .filter(|c| {
                    c.id == reference.id
                        && c.body_digest == reference.body_digest
                        && c.user_id == Some(reference.writer)
                        && !c.bot
                        && !c.delegated
                        && c.issue_id == record.work.parent
                })
                .count()
                != 1
            {
                return Err(Error::new(
                    "stale_result",
                    "accepted release publication was removed, edited or replaced",
                ));
            }
        }
        for comment in comments {
            if comment.id == record.acceptance_publication.id
                || comment.bot
                || comment.delegated
                || comment.user_id != Some(record.acceptance_publication.writer)
                || comment.issue_id != record.work.parent
            {
                continue;
            }
            if let Ok(signed) = evidence::decode_signed_publication::<evidence::Acceptance>(
                &comment.body,
                "human_acceptance",
                &runtime.services.key,
            ) && signed.payload.work == record.work
                && signed.payload.reviewed_token == record.attempt.plan.evidence_token
            {
                return Err(Error::new(
                    "stale_result",
                    "conflicting current authenticated acceptance makes release selection ambiguous",
                ));
            }
        }
        let selected = comments
            .iter()
            .find(|c| c.id == record.parent_publication.id)
            .ok_or_else(|| Error::new("stale_result", "release parent publication disappeared"))?;
        reject_superseding_parent(&runtime.services.key, record.runtime, comments, selected)?;
        Ok(())
    }
    pub fn release_attempts(&self) -> Result<Vec<ReleaseAttempt>> {
        self.restore_releases()?;
        Ok(self
            .releases
            .state
            .lock()
            .map_err(|_| unavailable())?
            .attempts
            .values()
            .map(|r| r.attempt.clone())
            .collect())
    }
}
impl RuntimeDelivery {
    async fn prepare_release(
        &self,
        lease: &Lease,
        projection: &RuntimeProjection,
        workspace_id: Uuid,
        parent_id: Uuid,
    ) -> Result<Option<Arc<tokio::sync::Mutex<Preparation>>>> {
        let Some(sources) = self.parent_sources(workspace_id, parent_id).await? else {
            return Ok(None);
        };
        let key = preparation_key(
            workspace_id,
            &sources.parent.manifest().work,
            sources.parent.token(),
        );
        let retained = self
            .releases
            .preparations
            .lock()
            .map_err(|_| unavailable())?
            .get(&key)
            .cloned();
        let pipeline = if let Some(pipeline) = retained {
            pipeline
        } else {
            let leaf = preparation_leaf(workspace_id, sources.parent.token())?;
            let reference = match self.records.read::<Signed<PreparationRef>>(&leaf) {
                Ok(signed) => {
                    self.runtime
                        .services
                        .key
                        .verify("runtime_release_preparation", &signed)?;
                    if signed.payload.schema_version != 1
                        || signed.payload.workspace != workspace_id
                        || signed.payload.work != sources.parent.manifest().work
                        || signed.payload.evidence_token != sources.parent.token()
                        || signed
                            .payload
                            .repositories
                            .keys()
                            .cloned()
                            .collect::<BTreeSet<_>>()
                            != sources
                                .parent
                                .manifest()
                                .candidates
                                .iter()
                                .map(|c| c.repository.clone())
                                .collect()
                    {
                        return Err(Error::new(
                            "record_corrupt",
                            "retained release preparation differs from the exact parent vector",
                        ));
                    }
                    signed.payload
                }
                Err(e) if e.code == "record_missing" => {
                    let reference = PreparationRef {
                        schema_version: 1,
                        workspace: workspace_id,
                        work: sources.parent.manifest().work.clone(),
                        evidence_token: sources.parent.token().into(),
                        operation: Uuid::new_v4(),
                        repositories: sources
                            .parent
                            .manifest()
                            .candidates
                            .iter()
                            .map(|c| (c.repository.clone(), Uuid::new_v4()))
                            .collect(),
                    };
                    self.records.write(
                        &leaf,
                        &self
                            .runtime
                            .services
                            .key
                            .sign("runtime_release_preparation", &reference)?,
                    )?;
                    reference
                }
                Err(e) => return Err(e),
            };
            let repositories = reference
                .repositories
                .iter()
                .map(|(repository, operation)| {
                    (
                        repository.clone(),
                        RepoPreparation {
                            operation: *operation,
                            phase: None,
                            candidate: None,
                            production: None,
                        },
                    )
                })
                .collect();
            let pipeline = Arc::new(tokio::sync::Mutex::new(Preparation {
                reference,
                sources,
                repositories,
                error: None,
            }));
            self.releases
                .preparations
                .lock()
                .map_err(|_| unavailable())?
                .insert(key, pipeline.clone());
            pipeline
        };
        self.progress_release_preparation(Some(lease), projection, pipeline.clone())
            .await?;
        Ok(Some(pipeline))
    }
    async fn progress_release_preparation(
        &self,
        lease: Option<&Lease>,
        projection: &RuntimeProjection,
        pipeline: Arc<tokio::sync::Mutex<Preparation>>,
    ) -> Result<()> {
        let result = self
            .progress_release_preparation_inner(lease, projection, pipeline.clone())
            .await;
        if let Err(error) = &result
            && super::child::terminal_gate_failure(error)
        {
            let mut prepared = pipeline.lock().await;
            prepared.error = Some(error.clone());
            for repo in prepared.repositories.values_mut() {
                if let Some(phase) = &repo.phase {
                    self.retire_delivery_phase(
                        phase,
                        json!({"production_preparation":"terminal_failure","error":error.code}),
                    )?;
                    repo.phase = None;
                }
            }
        }
        result
    }
    async fn progress_release_preparation_inner(
        &self,
        lease: Option<&Lease>,
        projection: &RuntimeProjection,
        pipeline: Arc<tokio::sync::Mutex<Preparation>>,
    ) -> Result<()> {
        let mut prepared = pipeline.lock().await;
        if prepared
            .repositories
            .values()
            .all(|repo| repo.phase.is_none())
            && let Some(error) = &prepared.error
            && super::child::terminal_gate_failure(error)
        {
            return Err(error.clone());
        }
        let active = prepared.repositories.values().find_map(|r| {
            r.phase.as_ref().map(|phase| {
                (
                    phase.workspace.clone(),
                    phase.admission.fresh_gate().clone(),
                )
            })
        });
        // A retained phase drains with its exact admitted settings. Maintenance
        // cannot select another repository or mint a replacement admission.
        if active.is_none() && lease.is_none() {
            return Ok(());
        }
        let active_repository = prepared
            .repositories
            .iter()
            .find(|(_, r)| r.phase.is_some())
            .map(|(id, _)| id.clone());
        let (workspace, gate, parent) = if let Some((workspace, gate)) = active {
            let parent = self
                .runtime
                .observations
                .active_parent(&gate, &self.runtime.dependency_outcomes()?)?;
            (workspace.as_ref().clone(), gate, parent)
        } else {
            let parent = projection
                .parent(prepared.reference.workspace, prepared.reference.work.parent)?
                .clone();
            let workspace = self.workspace(projection, prepared.reference.workspace)?;
            let gate = parent.fresh_gate(
                None,
                GatePurpose::NewOperation,
                &self.runtime.dependency_outcomes()?,
                Utc::now(),
                Duration::seconds(30),
            )?;
            (workspace, gate, parent)
        };
        if parent.work.as_ref() != Some(&prepared.reference.work) {
            return Err(Error::new(
                "stale_result",
                "release preparation generation changed",
            ));
        }
        if active_repository.is_none() {
            let current = self.runtime.current_delivery(&parent.key)?;
            if current
                .verified_parent()
                .is_none_or(|p| p.token() != prepared.reference.evidence_token)
            {
                return Err(Error::new(
                    "stale_result",
                    "release preparation parent evidence changed",
                ));
            }
        } else {
            let configured = workspace
                .configuration
                .workspaces
                .values()
                .find(|w| w.id == prepared.reference.workspace)
                .ok_or_else(|| Error::new("scope_violation", "release workspace unavailable"))?;
            let writer = workspace.configuration.connections[&configured.linear_connection]
                .account_id
                .parse()
                .map_err(|_| Error::new("identity_mismatch", "Linear writer is not canonical"))?;
            current_parent_publication(
                &self.runtime.services.key,
                workspace.configuration.runtime.id,
                writer,
                &parent,
                &prepared.sources.parent,
            )?;
        }
        let verifier = self.hosted_verifier(&workspace.configuration).await?;
        let sources = &prepared.sources;
        let parent_proof = sources.parent.clone();
        let slices = sources.slices.clone();
        let children = sources.children.clone();
        let integrations = sources.integrations.clone();
        let vector_digest = sources.vector_digest.clone();
        for contract in &workspace.contracts {
            if active_repository
                .as_ref()
                .is_some_and(|id| id != &contract.repository)
            {
                continue;
            }
            let candidate = parent_proof
                .manifest()
                .candidates
                .iter()
                .find(|c| c.repository == contract.repository)
                .ok_or_else(|| {
                    Error::new(
                        "candidate_provenance",
                        "release misses a configured repository candidate",
                    )
                })?;
            let revision = candidate_revision(candidate)?;
            let repo = prepared
                .repositories
                .get_mut(&contract.repository)
                .ok_or_else(|| {
                    Error::new("record_corrupt", "release repository preparation missing")
                })?;
            if repo.production.is_some() {
                if let Some(phase) = &repo.phase {
                    self.retire_delivery_phase(phase, json!({"production_preparation":"verified","repository":contract.repository}))?;
                    repo.phase = None;
                }
                continue;
            }
            if repo.phase.is_none() {
                let lease = lease.ok_or_else(|| {
                    Error::new(
                        "admission_closed",
                        "maintenance cannot admit another production preparation",
                    )
                })?;
                let observation = workspace
                    .coordinator
                    .observation(gate.clone(), revision.clone())?;
                // A durable earlier admission is never silently restarted. If it
                // sealed all evidence, read it; otherwise keep exact prior holds.
                match self.runtime.control.operation(repo.operation) {
                    Ok(_) => {
                        let proof = if candidate.ordered_delta_digests.is_empty() {
                            workspace
                                .coordinator
                                .hydrate_existing_production_validation(
                                    &observation,
                                    &parent_proof,
                                    &integrations,
                                    repo.operation,
                                )
                                .await?
                        } else {
                            let native = workspace
                                .coordinator
                                .hydrate_production_candidate(
                                    &observation,
                                    &parent_proof,
                                    repo.operation,
                                )
                                .await?;
                            let proof = workspace
                                .coordinator
                                .hydrate_production_validation(&observation, &native, &parent_proof)
                                .await?;
                            repo.candidate = Some(native);
                            proof
                        };
                        repo.production = Some(proof);
                        continue;
                    }
                    Err(e) if e.code == "operation_missing" || e.code == "record_missing" => {}
                    Err(e) => return Err(e),
                }
                let scope = DeliveryScope {
                    schema_version: 1,
                    runtime_id: lease.runtime_id,
                    phase: DeliveryPhase::Release,
                    work: gate.work().clone(),
                    repository: contract.repository.clone(),
                    revision,
                    fresh_gate_digest: gate.digest()?,
                    contract_digest: contract.digest()?,
                    candidate_vector_digest: Some(vector_digest.clone()),
                };
                repo.phase = Some(
                    self.admit_delivery_phase(
                        &workspace,
                        &gate,
                        lease,
                        PhaseRequest {
                            operation: repo.operation,
                            scope,
                        },
                    )
                    .await?,
                );
            }
            let phase = repo.phase.as_ref().unwrap();
            if candidate.ordered_delta_digests.is_empty() {
                repo.production = Some(
                    phase
                        .coordinator
                        .acquire_existing_production_validation(
                            &phase.admission,
                            &parent_proof,
                            &integrations,
                            &verifier,
                        )
                        .await?,
                );
            } else {
                if repo.candidate.is_none() {
                    let slice = slices
                        .iter()
                        .find(|s| s.manifest().repository == contract.repository)
                        .ok_or_else(|| {
                            Error::new(
                                "candidate_provenance",
                                "release actual isolated source unavailable",
                            )
                        })?;
                    let selected = children
                        .iter()
                        .filter(|c| {
                            c.verified().evidence().revision.repository == contract.repository
                        })
                        .cloned()
                        .collect::<Vec<_>>();
                    repo.candidate = Some(
                        phase
                            .coordinator
                            .prepare_production_candidate(
                                &phase.admission,
                                &parent_proof,
                                slice,
                                &selected,
                            )
                            .await?,
                    );
                }
                repo.production = Some(
                    phase
                        .coordinator
                        .acquire_production_validation(
                            &phase.admission,
                            repo.candidate.as_ref().unwrap(),
                            &parent_proof,
                            &verifier,
                        )
                        .await?,
                );
            }
            self.retire_delivery_phase(
                phase,
                json!({"production_preparation":"verified","repository":contract.repository}),
            )?;
            repo.phase = None;
            break; // One finite repository phase per fair driver cycle.
        }
        prepared.error = None;
        Ok(())
    }
    fn release_requirements(
        &self,
        parent: &ParentProjection,
    ) -> Result<Vec<domain::DependencyRequirement>> {
        let outcomes = self.runtime.dependency_outcomes()?;
        let authorization = parent.authorization_candidate.as_ref().ok_or_else(|| {
            Error::new("authority_missing", "release parent authorization missing")
        })?;
        let mut dependencies = Vec::new();
        for id in &parent.prerequisites {
            let observation = parent.observations.get(id).ok_or_else(|| {
                Error::new(
                    "dependency_unsatisfied",
                    "release prerequisite native identity missing",
                )
            })?;
            if observation.issue.parent_id.is_some() {
                continue;
            }
            let key = IssueKey::new(&parent.key.endpoint, parent.key.workspace, *id)?;
            let generation = authorization
                .dependency_generations
                .get(id)
                .ok_or_else(|| {
                    Error::new(
                        "dependency_unsatisfied",
                        "release prerequisite required generation missing",
                    )
                })?;
            let digest = outcomes.parent_outcome(&key, generation).ok_or_else(|| {
                Error::new(
                    "dependency_unsatisfied",
                    "release prerequisite has no verified production outcome",
                )
            })?;
            dependencies.push(domain::DependencyRequirement {
                parent_id: *id,
                generation: generation.clone(),
                outcome_digest: digest.into(),
            });
        }
        Ok(dependencies)
    }
}

fn candidate_revision(
    candidate: &crate::git::CandidateManifest,
) -> Result<evidence::RevisionBinding> {
    let result = candidate
        .result
        .as_ref()
        .ok_or_else(|| Error::new("candidate_provenance", "release candidate result missing"))?;
    Ok(evidence::RevisionBinding {
        repository: candidate.repository.clone(),
        head: result.commit.clone(),
        base: candidate.production.commit.clone(),
        tested_commit: result.commit.clone(),
        tested_tree: result.tree.clone(),
    })
}
fn repository_evidence<'a>(
    prepared: &'a Preparation,
    contracts: &'a [RepositoryContract],
) -> Result<Vec<ReleaseRepositoryEvidence<'a>>> {
    contracts
        .iter()
        .map(|contract| {
            let repo = prepared
                .repositories
                .get(&contract.repository)
                .ok_or_else(|| {
                    Error::new("candidate_provenance", "repository preparation missing")
                })?;
            let production = repo.production.as_ref().ok_or_else(|| {
                Error::new("operation_pending", "production validation incomplete")
            })?;
            let evidence = if let Some(candidate) = &repo.candidate {
                ReleaseStepEvidence::Changed {
                    prepared: candidate,
                    production,
                }
            } else {
                let baseline = prepared
                    .sources
                    .baselines
                    .iter()
                    .find(|b| b.baseline().repository == contract.repository)
                    .ok_or_else(|| {
                        Error::new(
                            "production_baseline",
                            "unchanged release baseline unavailable",
                        )
                    })?;
                ReleaseStepEvidence::Existing {
                    baseline,
                    production,
                }
            };
            Ok(ReleaseRepositoryEvidence { contract, evidence })
        })
        .collect()
}
impl RuntimeDelivery {
    async fn admit_release(
        &self,
        lease: &Lease,
        projection: &RuntimeProjection,
        prepared: Arc<tokio::sync::Mutex<Preparation>>,
        sorted: &ReleaseCandidate,
    ) -> Result<()> {
        let preparation = prepared.lock().await;
        if preparation
            .repositories
            .values()
            .any(|r| r.production.is_none() || r.phase.is_some())
        {
            return Ok(());
        }
        let workspace = self.workspace(projection, preparation.reference.workspace)?;
        let selected = workspace
            .configuration
            .workspaces
            .values()
            .find(|w| w.id == preparation.reference.workspace)
            .unwrap();
        let parent = projection.parent(selected.id, preparation.reference.work.parent)?;
        let current = self.runtime.current_delivery(&parent.key)?;
        let Some(acceptance) = current.verified_acceptance().cloned() else {
            return Ok(());
        };
        let (selected_digest, selected_at) = sorted
            .acceptance_digest
            .as_deref()
            .zip(sorted.accepted)
            .ok_or_else(|| {
                Error::new(
                    "stale_result",
                    "acceptance arrived after release ordering; defer to a fresh sorted pass",
                )
            })?;
        current.require_release_selection(
            &preparation.reference.evidence_token,
            selected_digest,
            selected_at,
        )?;
        if current
            .verified_parent()
            .is_none_or(|p| p.token() != preparation.reference.evidence_token)
        {
            return Err(Error::new(
                "stale_result",
                "release candidate evidence changed",
            ));
        }
        let gate = parent.fresh_gate(
            None,
            GatePurpose::NewOperation,
            &self.runtime.dependency_outcomes()?,
            Utc::now(),
            Duration::seconds(30),
        )?;
        let predecessor = self.known_predecessor_baseline(projection, &gate).await?;
        let observer = predecessor
            .as_ref()
            .map(|proof| workspace.observer.with_successor(proof.clone()))
            .transpose()?;
        let observer = observer.as_ref().unwrap_or(workspace.observer.as_ref());
        let repository_evidence = repository_evidence(&preparation, &workspace.contracts)?;
        // Before queue admission, recover every complete production gate from
        // current provider observations rather than relying on its creation time.
        for item in &repository_evidence {
            let revision = item.evidence.production().revision().clone();
            let observation = workspace.coordinator.observation(gate.clone(), revision)?;
            let current = match item.evidence {
                ReleaseStepEvidence::Changed {
                    prepared,
                    production: _,
                } => {
                    workspace
                        .coordinator
                        .hydrate_production_validation(
                            &observation,
                            prepared,
                            &preparation.sources.parent,
                        )
                        .await?
                }
                ReleaseStepEvidence::Existing {
                    baseline,
                    production: _,
                } => {
                    observer.reobserve(item.contract, baseline).await?;
                    workspace
                        .coordinator
                        .hydrate_existing_production_validation(
                            &observation,
                            &preparation.sources.parent,
                            &preparation.sources.integrations,
                            preparation.repositories[&item.contract.repository].operation,
                        )
                        .await?
                }
            };
            if current.digest()? != item.evidence.production().digest()? {
                return Err(Error::new(
                    "stale_result",
                    "production validation changed before release admission",
                ));
            }
        }
        let plan = build_release_plan(ReleasePlanInput {
            runtime_id: lease.runtime_id,
            operation_id: preparation.reference.operation,
            workspace: selected,
            parent: &preparation.sources.parent,
            acceptance: &acceptance,
            carried: None,
            repositories: &repository_evidence,
            dependencies: self.release_requirements(parent)?,
        })?;
        if self
            .releases
            .state
            .lock()
            .map_err(|_| unavailable())?
            .attempts
            .contains_key(&plan.operation_id)
        {
            return Ok(());
        }
        let manual = self.pending_release_requests()?.into_iter().find(|r| {
            r.workspace_id() == selected.id
                && r.record.payload.work == preparation.reference.work
                && r.evidence_token() == plan.evidence_token
                && r.record.payload.config_revision
                    == workspace.configuration.revision().unwrap_or_default()
        });
        let successor = self.successor_intent(&plan, gate.work(), &acceptance)?;
        if let Some(manual) = &manual {
            crate::app::verify_current_actor(
                &workspace.configuration,
                &manual.record.payload.actor,
                "release",
            )?;
            self.runtime
                .configuration
                .authority_fences()?
                .check_actor(&manual.record.payload.actor, "release")?;
        }
        let snapshot = self.runtime.control.snapshot()?;
        let production = preparation
            .repositories
            .values()
            .map(|r| r.production.clone().unwrap())
            .collect::<Vec<_>>();
        let evidence = || DeliveryEvidence {
            parent: &preparation.sources.parent,
            acceptance: &acceptance,
            carried: None,
            production: &production,
        };
        let facts = |snapshot: &ControlSnapshot| AdmissionFacts {
            running: snapshot.intent.desired == Intent::Running,
            included: parent.included,
            authorized: true,
            no_pending_correction: true,
            preparation_complete: true,
            provider_reads_complete: true,
            repository_policy_usable: true,
            credentials_usable: true,
            manual_mode: selected.release.mode == ReleaseMode::Manual,
            explicit_manual_request: successor
                .as_ref()
                .map(|s| s.command())
                .or_else(|| manual.as_ref().map(|r| r.request_id())),
            actor_may_release: successor.is_some() || manual.is_some(),
            owner_generation: lease.owner_generation,
            control_generation: snapshot.intent.generation,
            required_step_ids: plan.steps.iter().map(|s| s.id.clone()).collect(),
        };
        let eligibility = {
            let state = self.releases.state.lock().map_err(|_| unavailable())?;
            domain::verify_release_eligibility(
                &plan,
                evidence(),
                facts(&snapshot),
                &state.queue,
                Utc::now(),
            )?
        };
        self.runtime
            .retain_release_eligibility(&parent.key, eligibility.clone())?;
        if selected.release.mode == ReleaseMode::Manual && manual.is_none() && successor.is_none() {
            return Ok(());
        }
        let (parent_publication, acceptance_publication) = self.publication_refs(
            &workspace.configuration,
            parent,
            &preparation.sources.parent,
            &acceptance,
        )?;
        let active = self.runtime.observations.capture_active(
            gate.workspace_id(),
            gate.config_revision(),
            gate.workflow_digest(),
        )?;
        let retained_workspace = self.release_workspace(&workspace, gate.workspace_id())?;
        let check = self.current_gate();
        let operation = plan.operation_id;
        let admitted = self.runtime.control.admit_observed(
            lease,
            operation,
            gate.work_key()?,
            plan.digest()?,
            |admission_snapshot| {
                self.runtime.registration.verify_active_owner(lease)?;
                check(&gate)?;
                if let Some(predecessor) = &predecessor {
                    predecessor.revalidate()?;
                }
                let current_config = self.runtime.configuration.snapshot()?;
                if current_config.revision()? != workspace.configuration.revision()? {
                    return Err(Error::new(
                        "stale_config",
                        "release configuration changed during admission",
                    ));
                }
                if let Some(manual) = &manual {
                    crate::app::verify_current_actor(
                        &current_config,
                        &manual.record.payload.actor,
                        "release",
                    )?;
                    self.runtime
                        .configuration
                        .authority_fences()?
                        .check_actor(&manual.record.payload.actor, "release")?;
                }
                self.runtime
                    .current_delivery(&parent.key)?
                    .require_release_selection(
                        preparation.sources.parent.token(),
                        selected_digest,
                        selected_at,
                    )?;
                if successor.is_some()
                    && self
                        .successor_intent(&plan, gate.work(), &acceptance)?
                        .is_none()
                {
                    return Err(Error::new(
                        "stale_result",
                        "successor authorization disappeared",
                    ));
                }
                let mut state = self.releases.state.lock().map_err(|_| unavailable())?;
                state.queue.eligible(&plan, Utc::now())?;
                let attempt = ReleaseAttempt::admit(
                    plan.clone(),
                    evidence(),
                    facts(admission_snapshot),
                    Utc::now(),
                )?;
                let record = AttemptRecord {
                    schema_version: 1,
                    runtime: lease.runtime_id,
                    workspace: selected.id,
                    work: gate.work().clone(),
                    config_revision: gate.config_revision().into(),
                    workflow_digest: gate.workflow_digest().into(),
                    parent_publication: parent_publication.clone(),
                    acceptance_publication: acceptance_publication.clone(),
                    manual_intent: successor
                        .as_ref()
                        .map(|s| s.command())
                        .or_else(|| manual.as_ref().map(|r| r.request_id())),
                    manual_actor: successor
                        .as_ref()
                        .map(|s| s.actor().clone())
                        .or_else(|| manual.as_ref().map(|r| r.record.payload.actor.clone())),
                    manual_role: successor
                        .as_ref()
                        .map(|_| ManualRole::Recover)
                        .or_else(|| manual.as_ref().map(|_| ManualRole::Release)),
                    acceptance_actor: acceptance.acceptance().linear_actor.parse().map_err(
                        |_| Error::new("identity_mismatch", "acceptance actor is not canonical"),
                    )?,
                    preparations: preparation.reference.repositories.clone(),
                    attempt,
                    admission_confirmed: false,
                    admission_operation: operation,
                    recovery_receipts: BTreeMap::new(),
                    local_retired: false,
                    pending_receipts: BTreeMap::new(),
                };
                Self::check_release_publications(&self.runtime, &record, None)?;
                let mut proposed = state.attempts.clone();
                proposed.insert(operation, record.clone());
                let queue = rebuild_queue(&proposed, &state.no_release)?;
                self.save_release(&record)?;
                state.queue = queue;
                state.attempts = proposed;
                Ok(())
            },
        );
        let permit = match admitted {
            Ok(permit) => permit,
            Err(error) => {
                let reserved = self
                    .releases
                    .state
                    .lock()
                    .map_err(|_| unavailable())?
                    .attempts
                    .contains_key(&operation);
                if reserved {
                    self.update_release(operation, |record, _| {
                        record
                            .attempt
                            .fence(format!("admission not confirmed: {}", error.code), |_| {
                                Ok(())
                            })
                    })?;
                }
                return Err(error);
            }
        };
        // Keep the actual capability before any fallible persistence/reporting.
        let live = Arc::new(tokio::sync::Mutex::new(LiveRelease {
            permit: permit.clone(),
            gate,
            parent: preparation.sources.parent.clone(),
            acceptance,
            eligibility,
            preparation: prepared.clone(),
            admissions: BTreeMap::new(),
            workspace: retained_workspace,
            _active: active,
        }));
        self.releases
            .live
            .lock()
            .map_err(|_| unavailable())?
            .insert(operation, live);
        self.update_release(operation, |record, _| {
            record.admission_confirmed = true;
            Ok(())
        })?;
        Ok(())
    }
}
impl RuntimeDelivery {
    fn retire_release(&self, operation: Uuid, record: &AttemptRecord) -> Result<()> {
        self.runtime.registration.verify_control_owner()?;
        let work = issue(record)?.identity()?;
        let snapshot = self.runtime.control.snapshot()?;
        if snapshot
            .unresolved_operations
            .iter()
            .any(|(id, targets)| *id == operation || targets.contains(&work))
        {
            return Err(Error::new(
                "reconciliation_required",
                "release still has unresolved original effects",
            ));
        }
        let prior = self.runtime.control.operation(record.admission_operation)?;
        let scope = admission_scope(&prior, &record.attempt.plan)?;
        let receipt = if prior.phase == ReceiptPhase::Reconciled {
            prior
        } else {
            self.runtime.control.retire_local(
                record.admission_operation,
                json!({"release":"terminal","plan_digest":record.attempt.plan.digest()?}),
            )?
        };
        if receipt.kind != "admission"
            || receipt.expected.get("scope") != Some(&scope)
            || receipt.observed_result.as_ref().is_none_or(|v| {
                v["local_execution"] != "exited"
                    || v["unresolved_effects"]
                        .as_array()
                        .is_none_or(|a| !a.is_empty())
            })
        {
            return Err(Error::new(
                "reconciliation_required",
                "local release retirement retains unresolved effects",
            ));
        }
        self.update_release(operation, |current, snapshot| {
            if snapshot
                .unresolved_operations
                .iter()
                .any(|(id, targets)| *id == operation || targets.contains(&work))
            {
                return Err(Error::new(
                    "reconciliation_required",
                    "release effect hold changed during retirement",
                ));
            }
            if !current.pending_receipts.is_empty() {
                return Err(Error::new(
                    "reconciliation_required",
                    "release receipt reconciliation is incomplete",
                ));
            }
            current.local_retired = true;
            Ok(())
        })
    }
    fn release_final_gate(
        &self,
        operation: Uuid,
        gate: FreshGate,
        permit: Permit,
    ) -> ReleaseStepCheck {
        let state = self.releases.state.clone();
        let runtime = self.runtime.clone();
        let authority = self.retained_gate();
        Arc::new(move |plan, step| {
            runtime.registration.verify_active_owner(&permit.lease)?;
            authority(&gate)?;
            let state = state.lock().map_err(|_| unavailable())?;
            let record = state
                .attempts
                .get(&operation)
                .ok_or_else(|| Error::new("operation_missing", "admitted release state missing"))?;
            if !record.admission_confirmed
                || record.attempt.plan != *plan
                || !state.queue.owns_all(plan)
                || record.attempt.hold_reason.is_some()
                || !record.pending_receipts.is_empty()
                || record.attempt.abandoned
                || record.attempt.resolved
                || !matches!(
                    record.attempt.steps.get(step),
                    Some(StepState::SentUnknown { .. } | StepState::Acknowledged { .. })
                )
            {
                return Err(Error::new(
                    "release_fenced",
                    "release queue or exact sent step no longer authorizes dispatch",
                ));
            }
            Self::check_release_publications(&runtime, record, Some(&gate))
        })
    }
    fn record_release_observation(
        &self,
        operation: Uuid,
        observed: &ObservedReleaseStep,
    ) -> Result<()> {
        self.commit_release_observation(operation, observed, |receipt| {
            self.runtime.control.reconcile_owned_checked(receipt, crate::control::Reconciliation::UniqueVerified {
                remote_ids: receipt.remote_ids.clone(), result: json!({"release_plan":observed.plan_digest(),"step":observed.step_id(),"outcome":observed.outcome(),"effect":observed.effect()}),
                payload_digest: receipt.payload_digest.clone(), actor: receipt.actor.clone(),
            }, || self.runtime.registration.verify_control_owner()).map(|_| ())
        })
    }
    fn commit_release_observation(
        &self,
        operation: Uuid,
        observed: &ObservedReleaseStep,
        reconcile: impl FnOnce(&crate::control::Receipt) -> Result<()>,
    ) -> Result<()> {
        self.update_release(operation, |record, _| {
            if record.attempt.plan.digest() != Ok(observed.plan_digest().into()) {
                return Err(Error::new(
                    "ownership_conflict",
                    "observed effect belongs to another release plan",
                ));
            }
            let step = observed.step_id();
            if record.attempt.step_attempt(step)? != observed.attempt() {
                return Err(Error::new(
                    "ownership_conflict",
                    "release observation belongs to another step attempt",
                ));
            }
            if !matches!(observed.outcome(), ReleaseStepOutcome::Pending)
                && let Some(receipt) = observed.receipt()
            {
                record
                    .pending_receipts
                    .insert(receipt.operation_id, step.into());
            }
            let remote = observed
                .receipt()
                .map(|r| serde_json::to_string(&r.remote_ids).unwrap_or_default())
                .filter(|s| s != "{}" && !s.is_empty());
            match observed.outcome() {
                ReleaseStepOutcome::Pending => {
                    if let Some(remote) = remote
                        && matches!(
                            record.attempt.steps.get(step),
                            Some(StepState::SentUnknown { .. })
                        )
                    {
                        record.attempt.acknowledge(step, remote, |_| Ok(()))?;
                    }
                }
                ReleaseStepOutcome::Succeeded => record.attempt.observe_success(
                    step,
                    observed.effect().cloned().ok_or_else(|| {
                        Error::new(
                            "release_provenance",
                            "successful adapter result lacks effect",
                        )
                    })?,
                    observed.observed_at(),
                    |_| Ok(()),
                )?,
                ReleaseStepOutcome::Failed(reason) => record.attempt.observe_failure(
                    step,
                    reason.clone(),
                    remote,
                    observed.observed_at(),
                    |_| Ok(()),
                )?,
            }
            Ok(())
        })?;
        if !matches!(observed.outcome(), ReleaseStepOutcome::Pending)
            && let Some(receipt) = observed.receipt()
        {
            reconcile(receipt)?;
            self.update_release(operation, |record, _| {
                record.pending_receipts.remove(&receipt.operation_id);
                Ok(())
            })?;
        }
        Ok(())
    }
    async fn advance_live_release(
        &self,
        operation: Uuid,
        live: &mut LiveRelease,
        _projection: &RuntimeProjection,
    ) -> Result<()> {
        let record = self
            .releases
            .state
            .lock()
            .map_err(|_| unavailable())?
            .attempts
            .get(&operation)
            .cloned()
            .ok_or_else(|| Error::new("operation_missing", "release attempt vanished"))?;
        let plan = &record.attempt.plan;
        let workspace = &live.workspace;
        let verifier = self.hosted_verifier(&workspace.configuration).await?;
        // Poll sent effects first, even if the original command acknowledgement
        // was lost. The adapter never creates a new request in this path.
        for (step, state) in &record.attempt.steps {
            if !state.is_unknown()
                && !record
                    .pending_receipts
                    .values()
                    .any(|pending| pending == step)
            {
                continue;
            }
            let admission = live.admissions.get(step).ok_or_else(|| {
                Error::new(
                    "reconciliation_required",
                    "sent step lost its live bounded dispatch context",
                )
            })?;
            let observed = workspace
                .coordinator
                .observe_release_step(
                    admission,
                    plan,
                    step,
                    record.attempt.step_attempt(step)?,
                    &workspace.observer,
                    &verifier,
                )
                .await?;
            self.record_release_observation(operation, &observed)?;
        }
        let record = self
            .releases
            .state
            .lock()
            .map_err(|_| unavailable())?
            .attempts[&operation]
            .clone();
        if ready_for_local_retirement(&record) {
            if !record.local_retired {
                self.retire_release(operation, &record)?;
            }
            return Ok(());
        }
        if record.attempt.hold_reason.is_some() || !record.pending_receipts.is_empty() {
            return Ok(());
        }
        Self::check_release_publications(&self.runtime, &record, Some(&live.gate))?;
        self.runtime.control.validate_permit(&live.permit, || {
            self.runtime
                .registration
                .verify_active_owner(&live.permit.lease)?;
            (self.retained_gate())(&live.gate)
        })?;
        let Some(step) = record.attempt.next_steps().first().map(|s| (*s).clone()) else {
            return Ok(());
        };
        let contract = workspace
            .contracts
            .iter()
            .find(|c| &c.repository == step.kind.repository())
            .ok_or_else(|| Error::new("scope_violation", "release repository removed"))?;
        let current = workspace.observer.observe_main(contract).await?;
        let revisions = BTreeMap::from([(contract.repository.clone(), current)]);
        let dispatch = self.runtime.control.serialize_observation(|snapshot| {
            Self::check_release_publications(&self.runtime, &record, Some(&live.gate))?;
            self.runtime
                .registration
                .verify_active_owner(&live.permit.lease)?;
            (self.retained_gate())(&live.gate)?;
            let mut state = self.releases.state.lock().map_err(|_| unavailable())?;
            let mut current =
                state.attempts.get(&operation).cloned().ok_or_else(|| {
                    Error::new("operation_missing", "release queue entry missing")
                })?;
            if current.attempt.plan != record.attempt.plan || !state.queue.owns_all(plan) {
                return Err(Error::new(
                    "release_locked",
                    "release plan lost exact resource ownership",
                ));
            }
            let facts = FreshReleaseFacts {
                generation: &record.work.generation,
                evidence_token: &plan.evidence_token,
                owner_generation: live.permit.lease.owner_generation,
                authority_current: true,
                pending_correction: false,
                provider_reads_complete: true,
                resource_lock_owned: true,
                credentials_usable: true,
                repository_policy_current: true,
                force_stopped: snapshot
                    .fenced_operations
                    .contains(&live.permit.operation_id),
                current_revisions: &revisions,
            };
            let prepared_record = current.clone();
            let payload = current.attempt.prepare_step(&step.id, &facts, |next| {
                let mut saved = prepared_record.clone();
                saved.attempt = next.clone();
                self.save_release(&saved)
            })?;
            // Preserve each successful persistence even if the following one
            // fails. Recovery sees Prepared and will not claim dispatch happened.
            state.attempts.insert(operation, current.clone());
            let sent_record = current.clone();
            let dispatch = current.attempt.mark_sent(&payload, &facts, |next| {
                let mut saved = sent_record.clone();
                saved.attempt = next.clone();
                self.save_release(&saved)
            })?;
            state.attempts.insert(operation, current);
            Ok(dispatch)
        })?;
        let prepared = live.preparation.lock().await;
        let repository = repository_evidence(&prepared, &workspace.contracts)?
            .into_iter()
            .find(|r| &r.contract.repository == step.kind.repository())
            .ok_or_else(|| {
                Error::new(
                    "candidate_provenance",
                    "release candidate preparation missing",
                )
            })?;
        let scope = DeliveryScope {
            schema_version: 1,
            runtime_id: plan.runtime_id,
            phase: DeliveryPhase::Release,
            work: record.work.clone(),
            repository: contract.repository.clone(),
            revision: repository.evidence.production().revision().clone(),
            fresh_gate_digest: live.gate.digest()?,
            contract_digest: contract.digest()?,
            candidate_vector_digest: Some(live.parent.manifest().candidate_vector_digest()?),
        };
        let admission = workspace.coordinator.bind_release_step(
            &live.permit,
            &live.gate,
            scope,
            plan,
            &dispatch,
            &live.eligibility,
        )?;
        live.admissions.insert(step.id.clone(), admission.clone());
        let observed = workspace
            .coordinator
            .dispatch_release_step(ReleaseStepRequest {
                admission: &admission,
                dispatch: &dispatch,
                plan,
                parent: &live.parent,
                acceptance: &live.acceptance,
                carried: None,
                evidence: repository.evidence,
                observer: &workspace.observer,
                verifier: &verifier,
                final_gate: self.release_final_gate(
                    operation,
                    live.gate.clone(),
                    live.permit.clone(),
                ),
            })
            .await?;
        self.record_release_observation(operation, &observed)?;
        Ok(())
    }
    pub async fn advance_releases(
        &self,
        lease: &Lease,
        projection: &RuntimeProjection,
    ) -> Result<()> {
        let _serial = self.releases.progressing.lock().await;
        self.restore_releases()?;
        self.drain_releases(projection).await?;
        if self.runtime.control.snapshot()?.intent.desired != Intent::Running {
            return Ok(());
        }
        self.runtime.registration.verify_active_owner(lease)?;
        let mut first = None;
        let mut candidates = Vec::new();
        for workspace in projection.workspaces.values() {
            for parent in workspace.parents.values() {
                let current = self.runtime.current_delivery(&parent.key)?;
                let accepted = current
                    .verified_acceptance()
                    .map(|a| a.acceptance().accepted_at);
                let acceptance_digest =
                    current.verified_acceptance().map(|a| a.digest().to_owned());
                candidates.push(ReleaseCandidate {
                    workspace: workspace.workspace_id,
                    parent: parent.key.clone(),
                    accepted,
                    acceptance_digest,
                });
            }
        }
        order_candidates(&mut candidates);
        for candidate in candidates {
            let result = async {
                if self
                    .advance_no_release(
                        lease,
                        projection,
                        candidate.workspace,
                        candidate.parent.issue,
                    )
                    .await?
                {
                    return Ok(());
                }
                let Some(prepared) = self
                    .prepare_release(
                        lease,
                        projection,
                        candidate.workspace,
                        candidate.parent.issue,
                    )
                    .await?
                else {
                    return Ok(());
                };
                self.admit_release(lease, projection, prepared, &candidate)
                    .await
            }
            .await;
            if let Err(error) = result
                && let Err(error) = self.runtime.invalidate_delivery(&candidate.parent, error)
            {
                first.get_or_insert(error);
            }
        }
        first.map_or(Ok(()), Err)
    }
    pub(super) async fn drain_releases(&self, projection: &RuntimeProjection) -> Result<()> {
        let preparations = self
            .releases
            .preparations
            .lock()
            .map_err(|_| unavailable())?
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for preparation in preparations {
            if let Err(error) = self
                .progress_release_preparation(None, projection, preparation.clone())
                .await
            {
                let mut prepared = preparation.lock().await;
                prepared.error = Some(error.clone());
                let key = IssueKey::new(
                    &prepared.reference.work.linear_endpoint,
                    prepared.reference.work.linear_workspace,
                    prepared.reference.work.parent,
                )?;
                self.runtime.invalidate_delivery(&key, error)?;
            }
        }
        let retained = self
            .releases
            .live
            .lock()
            .map_err(|_| unavailable())?
            .iter()
            .map(|(id, live)| (*id, live.clone()))
            .collect::<Vec<_>>();
        let mut first = None;
        for (id, live) in retained {
            let mut live = live.lock().await;
            let advanced = self.advance_live_release(id, &mut live, projection).await;
            if advanced.is_ok()
                && self
                    .releases
                    .state
                    .lock()
                    .map_err(|_| unavailable())?
                    .attempts
                    .get(&id)
                    .is_some_and(|r| ready_for_local_retirement(r) && r.local_retired)
            {
                self.releases
                    .live
                    .lock()
                    .map_err(|_| unavailable())?
                    .remove(&id);
            }
            if let Err(error) = advanced {
                let held = self.update_release(id, |record, _| {
                    record
                        .attempt
                        .fence(format!("{}: {}", error.code, error.message), |_| Ok(()))
                });
                if let Err(error) = held {
                    first.get_or_insert(error);
                }
                let key = IssueKey::new(
                    &live.parent.manifest().work.linear_endpoint,
                    live.parent.manifest().work.linear_workspace,
                    live.parent.manifest().work.parent,
                )?;
                if let Err(error) = self.runtime.invalidate_delivery(&key, error) {
                    first.get_or_insert(error);
                }
            }
        }
        first.map_or(Ok(()), Err)
    }
}
impl RuntimeDelivery {
    /// Reads existing native effects while stopped. This never reconstructs a
    /// dispatch capability or clears a partial-release hold by assertion.
    pub async fn reconcile_releases(&self, projection: &RuntimeProjection) -> Result<()> {
        let _serial = self.releases.progressing.lock().await;
        self.restore_releases()?;
        let live = self
            .releases
            .live
            .lock()
            .map_err(|_| unavailable())?
            .keys()
            .copied()
            .collect::<BTreeSet<_>>();
        let records = self
            .releases
            .state
            .lock()
            .map_err(|_| unavailable())?
            .attempts
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let mut first = None;
        for record in records {
            if live.contains(&record.attempt.plan.operation_id)
                || ((record.attempt.completed() || record.attempt.resolved)
                    && record.pending_receipts.is_empty()
                    && record.local_retired)
            {
                continue;
            }
            let result = async {
                let workspace = self.workspace(projection, record.workspace)?;
                for (id, state) in &record.attempt.steps {
                    if !state.is_unknown()
                        && !record
                            .pending_receipts
                            .values()
                            .any(|pending| pending == id)
                    {
                        continue;
                    }
                    let _step = record
                        .attempt
                        .plan
                        .steps
                        .iter()
                        .find(|s| &s.id == id)
                        .ok_or_else(|| {
                            Error::new("record_corrupt", "retained release step is undeclared")
                        })?;
                    let observed = workspace
                        .coordinator
                        .recover_owned_release_step(
                            &record.attempt.plan,
                            id,
                            record.attempt.step_attempt(id)?,
                            &workspace.observer,
                        )
                        .await?;
                    self.record_release_observation(record.attempt.plan.operation_id, &observed)?;
                }
                let current = self
                    .releases
                    .state
                    .lock()
                    .map_err(|_| unavailable())?
                    .attempts[&record.attempt.plan.operation_id]
                    .clone();
                if ready_for_local_retirement(&current) && !current.local_retired {
                    self.retire_release(current.attempt.plan.operation_id, &current)?;
                }
                Ok(())
            }
            .await;
            if let Err(error) = result
                && let Err(error) = self.runtime.invalidate_delivery(&issue(&record)?, error)
            {
                first.get_or_insert(error);
            }
        }
        first.map_or(Ok(()), Err)
    }
    /// Maintenance may drain an existing finite plan; it never creates another
    /// preparation, admission or dispatch capability from a retained JSON record.
    pub async fn maintain_releases(&self, projection: &RuntimeProjection) -> Result<()> {
        let _serial = self.releases.progressing.lock().await;
        self.restore_releases()?;
        self.drain_releases(projection).await
    }
}
fn release_revision(
    plan: &ReleasePlan,
    repository: &RepositoryId,
) -> Result<evidence::RevisionBinding> {
    for step in &plan.steps {
        match &step.kind {
            domain::StepKind::Merge {
                repository: actual,
                production_base,
                candidate,
                ..
            } if actual == repository => {
                return Ok(evidence::RevisionBinding {
                    repository: repository.clone(),
                    head: candidate.commit.clone(),
                    base: production_base.commit.clone(),
                    tested_commit: candidate.commit.clone(),
                    tested_tree: candidate.tree.clone(),
                });
            }
            domain::StepKind::VerifyExisting {
                repository: actual,
                source,
                ..
            }
            | domain::StepKind::DeployExisting {
                repository: actual,
                source,
                ..
            }
            | domain::StepKind::VerifyDeployed {
                repository: actual,
                source,
                ..
            } if actual == repository => {
                return Ok(evidence::RevisionBinding {
                    repository: repository.clone(),
                    head: source.commit.clone(),
                    base: source.commit.clone(),
                    tested_commit: source.commit.clone(),
                    tested_tree: source.tree.clone(),
                });
            }
            _ => {}
        }
    }
    Err(Error::new(
        "release_provenance",
        "release repository has no exact admitted candidate/source",
    ))
}
impl RuntimeDelivery {
    fn release_workspace(
        &self,
        admitted: &WorkspaceDelivery,
        workspace: Uuid,
    ) -> Result<WorkspaceDelivery> {
        let broker = self.provider_broker(&admitted.configuration, workspace)?;
        let github = broker
            .github
            .clone()
            .ok_or_else(|| Error::new("missing_secret", "release GitHub connection unavailable"))?;
        let current = self.retained_gate();
        let coordinator = Arc::new(DeliveryCoordinator::new(
            DeliveryServices {
                host: self.runtime.control.clone(),
                broker,
                records: admitted.records.clone(),
                key: self.runtime.services.key.clone(),
                git: admitted.git.clone(),
                retained_git_sources: admitted
                    .contracts
                    .iter()
                    .map(|c| (c.repository.clone(), PathBuf::from(&c.configuration.source)))
                    .collect(),
                current_gate: current.clone(),
                executions: self.runtime.executions.clone(),
                processes: self.processes.clone(),
            },
            admitted.contracts.clone(),
        )?);
        let observer = Arc::new(ProductionObserver::new(
            github,
            self.runtime.control.clone(),
            self.runtime.services.key.clone(),
            admitted.records.child("deployment")?,
            current,
        )?);
        Ok(WorkspaceDelivery {
            configuration: admitted.configuration.clone(),
            coordinator,
            observer,
            contracts: admitted.contracts.clone(),
            workflow: admitted.workflow.clone(),
            git: admitted.git.clone(),
            records: admitted.records.clone(),
        })
    }
}

struct ReleaseCandidate {
    workspace: Uuid,
    parent: IssueKey,
    accepted: Option<chrono::DateTime<Utc>>,
    acceptance_digest: Option<String>,
}
fn order_candidates(candidates: &mut [ReleaseCandidate]) {
    candidates.sort_by(|a, b| {
        (a.accepted.is_none(), a.accepted, &a.parent, a.workspace).cmp(&(
            b.accepted.is_none(),
            b.accepted,
            &b.parent,
            b.workspace,
        ))
    });
}
fn current_parent_publication(
    key: &evidence::AttestationKey,
    runtime: Uuid,
    writer: Uuid,
    projection: &ParentProjection,
    parent: &VerifiedParent,
) -> Result<()> {
    let comments = &projection
        .observations
        .get(&projection.key.issue)
        .ok_or_else(|| Error::new("incompletely_observed", "parent publication is not visible"))?
        .comments;
    let expected = serde_json::to_value(parent.manifest())
        .map_err(|_| Error::invalid("parent manifest encoding"))?;
    let matches = comments
        .iter()
        .filter(|comment| {
            !comment.bot
                && !comment.delegated
                && comment.user_id == Some(writer)
                && comment.issue_id == projection.key.issue
        })
        .filter(|comment| {
            evidence::decode_signed_publication::<Value>(&comment.body, "parent_result", key)
                .is_ok_and(|signed| {
                    signed.payload["runtime_id"] == json!(runtime)
                        && signed.payload["evidence"] == expected
                })
        })
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(Error::new(
            "stale_result",
            "retained preparation parent publication was removed or changed",
        ));
    }
    let selected = matches[0];
    reject_superseding_parent(key, runtime, comments, selected)
}
fn reject_superseding_parent(
    key: &evidence::AttestationKey,
    runtime: Uuid,
    comments: &[crate::providers::Comment],
    selected: &crate::providers::Comment,
) -> Result<()> {
    let expected =
        evidence::decode_signed_publication::<Value>(&selected.body, "parent_result", key)?;
    let selected_time = selected.created_at.ok_or_else(|| {
        Error::new(
            "incompletely_observed",
            "parent publication timestamp is unavailable",
        )
    })?;
    for other in comments {
        if other.id == selected.id
            || other.bot
            || other.delegated
            || other.user_id != selected.user_id
            || other.issue_id != selected.issue_id
        {
            continue;
        }
        if let Ok(signed) =
            evidence::decode_signed_publication::<Value>(&other.body, "parent_result", key)
            && signed.payload["runtime_id"] == json!(runtime)
            && signed.payload["evidence"]["work"] == expected.payload["evidence"]["work"]
            && other.created_at.is_none_or(|time| time >= selected_time)
        {
            return Err(Error::new(
                "stale_result",
                "a newer or ambiguous parent result supersedes the selected publication",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::digest::canonical_digest;

    pub(super) fn record(plan: ReleasePlan, work: WorkIdentity) -> AttemptRecord {
        let reference = PublicationRef {
            id: Uuid::new_v4(),
            body_digest: canonical_digest(1, &json!("fixture native publication")).unwrap(),
            writer: Uuid::from_u128(5),
        };
        // This is a retained-state fixture, not a release admission proof.
        // Actual observed effects are obtained from the production wire adapter.
        let attempt = ReleaseAttempt {
            steps: plan
                .steps
                .iter()
                .map(|s| {
                    (
                        s.id.clone(),
                        StepState::SentUnknown {
                            payload_digest: canonical_digest(
                                1,
                                &json!("fixture dispatched payload"),
                            )
                            .unwrap(),
                        },
                    )
                })
                .collect(),
            plan,
            admitted_at: Utc::now(),
            control_generation: 1,
            owner_generation: 1,
            step_history: BTreeMap::new(),
            cancellations: BTreeMap::new(),
            hold_reason: None,
            abandoned: false,
            resolved: false,
            assigned_successor: None,
            recovery_records: vec![],
        };
        attempt.validate_recovered().unwrap();
        AttemptRecord {
            schema_version: 1,
            runtime: attempt.plan.runtime_id,
            workspace: Uuid::from_u128(2),
            work,
            config_revision: canonical_digest(1, &json!("fixture config")).unwrap(),
            workflow_digest: canonical_digest(1, &json!("fixture workflow")).unwrap(),
            parent_publication: reference.clone(),
            acceptance_publication: reference,
            manual_intent: None,
            manual_actor: None,
            manual_role: None,
            acceptance_actor: Uuid::from_u128(5),
            preparations: BTreeMap::new(),
            admission_operation: attempt.plan.operation_id,
            recovery_receipts: BTreeMap::new(),
            attempt,
            admission_confirmed: true,
            local_retired: false,
            pending_receipts: BTreeMap::new(),
        }
    }

    #[tokio::test]
    async fn nonempty_repository_operation_maps_roundtrip_through_signed_private_json() {
        let (plan, work, _) = crate::delivery::observed_merge_for_runtime().await;
        let mut attempt = record(plan, work);
        attempt.preparations = BTreeMap::from([
            (
                RepositoryId {
                    hostname: "github.invalid".into(),
                    id: "42".into(),
                },
                Uuid::new_v4(),
            ),
            (
                RepositoryId {
                    hostname: "github.invalid".into(),
                    id: "7".into(),
                },
                Uuid::new_v4(),
            ),
        ]);
        let reference = PreparationRef {
            schema_version: 1,
            workspace: attempt.workspace,
            work: attempt.work.clone(),
            evidence_token: attempt.attempt.plan.evidence_token.clone(),
            operation: attempt.attempt.plan.operation_id,
            repositories: attempt.preparations.clone(),
        };
        let root = tempfile::TempDir::new().unwrap();
        let directory = crate::storage::PrivateDir::create(
            &root.path().canonicalize().unwrap().join("records"),
        )
        .unwrap();
        let key = evidence::AttestationKey::new("fixture".into(), vec![7; 32]).unwrap();
        let signed = key.sign("runtime_release_attempt", &attempt).unwrap();
        directory.write("attempt.json", &signed).unwrap();
        let restored: Signed<AttemptRecord> = directory.read("attempt.json").unwrap();
        key.verify("runtime_release_attempt", &restored).unwrap();
        assert_eq!(restored.payload.preparations, attempt.preparations);
        assert_eq!(restored.digest().unwrap(), signed.digest().unwrap());
        let prepared = key.sign("runtime_release_preparation", &reference).unwrap();
        directory.write("preparation.json", &prepared).unwrap();
        let restored: Signed<PreparationRef> = directory.read("preparation.json").unwrap();
        key.verify("runtime_release_preparation", &restored)
            .unwrap();
        assert_eq!(restored.payload.repositories, reference.repositories);
        assert_eq!(restored.digest().unwrap(), prepared.digest().unwrap());
        let mut duplicated = serde_json::to_value(&reference).unwrap();
        let entries = duplicated["repositories"].as_array_mut().unwrap();
        entries.push(entries[0].clone());
        assert!(serde_json::from_value::<PreparationRef>(duplicated).is_err());
        let mut unsorted = serde_json::to_value(&attempt).unwrap();
        unsorted["preparations"].as_array_mut().unwrap().reverse();
        assert!(serde_json::from_value::<AttemptRecord>(unsorted).is_err());
    }

    #[tokio::test]
    async fn failed_receipt_reconciliation_retains_resource_hold_across_restart_and_retirement() {
        let (_root, built, delivery) = super::super::tests::fixture().await;
        let (plan, work, observed) = crate::delivery::observed_merge_for_runtime().await;
        assert_eq!(observed.outcome(), &ReleaseStepOutcome::Succeeded);
        let operation = plan.operation_id;
        let lease = super::super::tests::start_control(&built.runtime);
        built
            .runtime
            .control
            .admit(
                &lease,
                operation,
                IssueKey::new(&work.linear_endpoint, work.linear_workspace, work.parent)
                    .unwrap()
                    .identity()
                    .unwrap(),
                plan.digest().unwrap(),
                || Ok(()),
            )
            .unwrap();
        let initial = record(plan.clone(), work);
        delivery.save_release(&initial).unwrap();
        delivery.restore_releases().unwrap();
        // This fixture starts the admitted operation in this host. Restore is
        // separately tested below after the durable observation was written.
        delivery
            .update_release(operation, |r, _| {
                r.attempt.hold_reason = None;
                Ok(())
            })
            .unwrap();
        let mut competitor = plan.clone();
        competitor.operation_id = Uuid::new_v4();
        competitor.parent_id = Uuid::new_v4();
        let error = delivery
            .commit_release_observation(operation, &observed, |_| {
                Err(Error::new(
                    "storage_failed",
                    "injected receipt persistence failure",
                ))
            })
            .unwrap_err();
        assert_eq!(error.code, "storage_failed");
        let signed: Signed<AttemptRecord> =
            delivery.records.read(&attempt_leaf(operation)).unwrap();
        built
            .runtime
            .services
            .key
            .verify("runtime_release_attempt", &signed)
            .unwrap();
        assert!(signed.payload.attempt.completed());
        assert!(!signed.payload.local_retired);
        assert_eq!(signed.payload.pending_receipts.len(), 1);
        assert!(
            delivery
                .releases
                .state
                .lock()
                .unwrap()
                .queue
                .eligible(&competitor, Utc::now())
                .is_err()
        );

        let recovered = RuntimeDelivery::with_fixture(
            built.runtime.clone(),
            delivery.processes.clone(),
            delivery.git_path().unwrap(),
        )
        .unwrap();
        recovered.restore_releases().unwrap();
        assert!(
            recovered
                .releases
                .state
                .lock()
                .unwrap()
                .queue
                .eligible(&competitor, Utc::now())
                .is_err()
        );
        // Reconciliation succeeds later; the actual local admission still owns
        // retirement and must be observed before the resource can be reused.
        recovered
            .commit_release_observation(operation, &observed, |_| Ok(()))
            .unwrap();
        assert!(
            recovered
                .releases
                .state
                .lock()
                .unwrap()
                .queue
                .eligible(&competitor, Utc::now())
                .is_err()
        );
        let current = recovered.releases.state.lock().unwrap().attempts[&operation].clone();
        recovered.retire_release(operation, &current).unwrap();
        assert!(recovered.releases.state.lock().unwrap().attempts[&operation].local_retired);
        assert!(
            recovered
                .releases
                .state
                .lock()
                .unwrap()
                .queue
                .eligible(&competitor, Utc::now())
                .is_ok()
        );
    }

    #[tokio::test]
    async fn known_failed_release_retires_local_claim_but_keeps_resource_hold() {
        let (_root, built, delivery) = super::super::tests::fixture().await;
        let (plan, work, _) = crate::delivery::observed_merge_for_runtime().await;
        let operation = plan.operation_id;
        let lease = super::super::tests::start_control(&built.runtime);
        built
            .runtime
            .control
            .admit(
                &lease,
                operation,
                IssueKey::new(&work.linear_endpoint, work.linear_workspace, work.parent)
                    .unwrap()
                    .identity()
                    .unwrap(),
                plan.digest().unwrap(),
                || Ok(()),
            )
            .unwrap();
        let mut initial = record(plan.clone(), work);
        let step = plan.steps[0].id.clone();
        initial.attempt.steps.insert(
            step.clone(),
            StepState::Failed {
                reason: "actual provider validation failed".into(),
                remote_id: Some("owned:1".into()),
                observed_at: Utc::now(),
            },
        );
        initial.attempt.hold_reason = Some("terminal release failure".into());
        assert!(ready_for_local_retirement(&initial));
        initial.pending_receipts.insert(Uuid::new_v4(), step);
        assert!(!ready_for_local_retirement(&initial));
        initial.pending_receipts.clear();
        delivery.save_release(&initial).unwrap();
        delivery.restore_releases().unwrap();
        delivery.retire_release(operation, &initial).unwrap();
        let receipt = built.runtime.control.operation(operation).unwrap();
        assert_eq!(receipt.phase, ReceiptPhase::Reconciled);
        assert_eq!(
            receipt.observed_result.as_ref().unwrap()["local_execution"],
            "exited"
        );
        assert_eq!(
            receipt.observed_result.as_ref().unwrap()["unresolved_effects"],
            json!([])
        );
        assert!(
            built
                .runtime
                .control
                .snapshot()
                .unwrap()
                .active_operations
                .is_empty()
        );
        let current = delivery.releases.state.lock().unwrap().attempts[&operation].clone();
        assert!(current.local_retired);
        assert!(current.attempt.hold_reason.is_some());
        let mut competitor = plan;
        competitor.operation_id = Uuid::new_v4();
        competitor.parent_id = Uuid::new_v4();
        assert!(
            delivery
                .releases
                .state
                .lock()
                .unwrap()
                .queue
                .eligible(&competitor, Utc::now())
                .is_err()
        );
    }

    #[tokio::test]
    async fn global_release_order_uses_acceptance_before_uuid_and_skips_ineligible_work() {
        let (plan, _, _) = crate::delivery::observed_merge_for_runtime().await;
        let early = Utc::now() - Duration::hours(1);
        let old = IssueKey::new(
            "https://linear.invalid/graphql",
            Uuid::from_u128(30),
            Uuid::from_u128(900),
        )
        .unwrap();
        let recent = IssueKey::new(
            "https://linear.invalid/graphql",
            Uuid::from_u128(20),
            Uuid::from_u128(100),
        )
        .unwrap();
        let mut candidates = vec![
            ReleaseCandidate {
                workspace: Uuid::from_u128(1),
                parent: recent.clone(),
                accepted: Some(early + Duration::minutes(1)),
                acceptance_digest: Some(plan.acceptance_digest.clone()),
            },
            ReleaseCandidate {
                workspace: Uuid::from_u128(99),
                parent: old.clone(),
                accepted: Some(early),
                acceptance_digest: Some(plan.acceptance_digest.clone()),
            },
        ];
        order_candidates(&mut candidates);
        assert_eq!(candidates[0].parent, old);
        let plans = candidates
            .iter()
            .map(|candidate| {
                let mut p = plan.clone();
                p.parent_id = candidate.parent.issue;
                p.operation_id = Uuid::new_v4();
                p
            })
            .collect::<Vec<_>>();
        let mut queue = ReleaseQueue::default();
        let admitted = plans
            .iter()
            .filter_map(|p| queue.reserve(p, Utc::now()).is_ok().then_some(p.parent_id))
            .collect::<Vec<_>>();
        assert_eq!(admitted, vec![old.issue]);

        // The older overlapping plan is ineligible, but it cannot starve a
        // younger parent's configured disjoint repository.
        let mut disjoint = plans[1].clone();
        let mut repository = disjoint.steps[0].kind.repository().clone();
        repository.id = "9001".into();
        if let domain::StepKind::Merge {
            repository: actual, ..
        } = &mut disjoint.steps[0].kind
        {
            *actual = repository.clone();
        } else {
            panic!("fixture merge expected");
        }
        disjoint.resources = [domain::ResourceKey::Repository {
            repository: repository.clone(),
        }]
        .into();
        disjoint.deployment_free = [repository].into();
        let mut blocked_old = plans[0].clone();
        blocked_old.operation_id = Uuid::new_v4();
        let selected = [&blocked_old, &disjoint]
            .into_iter()
            .filter_map(|p| queue.reserve(p, Utc::now()).is_ok().then_some(p.parent_id))
            .collect::<Vec<_>>();
        assert_eq!(selected, vec![recent.issue]);
        assert!(queue.eligible(&blocked_old, Utc::now()).is_err());
        assert!(queue.owns_all(&disjoint));
        candidates[1].accepted = Some(early);
        order_candidates(&mut candidates);
        assert_eq!(candidates[0].parent, recent); // canonical parent tie-break
    }

    #[test]
    fn later_native_signed_parent_publication_invalidates_same_generation_selection() {
        let key = evidence::AttestationKey::new("fixture".into(), vec![7; 32]).unwrap();
        let now = Utc::now();
        let payload = json!({"runtime_id":Uuid::from_u128(1),"evidence":{"work":{"parent":Uuid::from_u128(100),"generation":"same"},"content":"first"}});
        let make = |id, value: Value, time| {
            let signed = key.sign("parent_result", &value).unwrap();
            let body = evidence::encode_signed_publication("parent_result", &signed).unwrap();
            crate::providers::Comment {
                id: Uuid::from_u128(id),
                issue_id: Uuid::from_u128(100),
                body_digest: crate::digest::bytes_digest(body.as_bytes()),
                body,
                user_id: Some(Uuid::from_u128(5)),
                bot: false,
                delegated: false,
                created_at: Some(time),
                updated_at: Some(time),
                edited_at: None,
                url: "https://linear.invalid/comment".into(),
            }
        };
        let first = make(50, payload.clone(), now);
        assert!(
            reject_superseding_parent(
                &key,
                Uuid::from_u128(1),
                std::slice::from_ref(&first),
                &first
            )
            .is_ok()
        );
        let mut changed = payload;
        changed["evidence"]["content"] = json!("replacement");
        let second = make(51, changed, now + Duration::seconds(1));
        assert_eq!(
            reject_superseding_parent(
                &key,
                Uuid::from_u128(1),
                &[first.clone(), second.clone()],
                &first
            )
            .unwrap_err()
            .code,
            "stale_result"
        );
        assert!(
            reject_superseding_parent(&key, Uuid::from_u128(1), &[first, second.clone()], &second)
                .is_ok()
        );
    }

    #[test]
    fn preparation_identity_selects_new_parent_result_and_keeps_old_generation() {
        let digest = |v| canonical_digest(1, &json!(v)).unwrap();
        let mut work = WorkIdentity {
            linear_endpoint: "https://linear.invalid/graphql".into(),
            linear_workspace: Uuid::from_u128(3),
            parent: Uuid::from_u128(100),
            child: None,
            generation: evidence::Generation {
                authorization: digest("authorization"),
                definition: digest("definition"),
                correction: digest("old"),
                scope: digest("scope"),
            },
        };
        let workspace = Uuid::from_u128(2);
        let old = preparation_key(workspace, &work, &digest("old parent token"));
        let mut retained = BTreeMap::from([(old.clone(), "old unknown or completed preparation")]);
        work.generation.correction = digest("new correction");
        let current = preparation_key(workspace, &work, &digest("new parent token"));
        assert!(!retained.contains_key(&current));
        retained.insert(current.clone(), "new accepted preparation");
        assert_eq!(retained[&old], "old unknown or completed preparation");
        assert_eq!(retained[&current], "new accepted preparation");
        assert_ne!(
            preparation_leaf(workspace, &old.2).unwrap(),
            preparation_leaf(workspace, &current.2).unwrap()
        );
    }
}

#[path = "runtime_delivery_recovery.rs"]
mod recovery;

#[path = "runtime_delivery_baseline.rs"]
mod baseline;
pub use baseline::KnownPredecessorBaseline;

#[cfg(test)]
#[path = "runtime_delivery_successor_fixture.rs"]
mod successor_fixture;
