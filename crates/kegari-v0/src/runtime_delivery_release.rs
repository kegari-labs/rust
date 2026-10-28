//! Serialized release ownership and durable observations. Retained records are
//! recovery facts; only the live finite plan permit can dispatch another step.
use super::child::{ActivePhase, PhaseRequest};
use super::parent::ParentSources;
use super::*;
use crate::authority::IssueKey;
use crate::control::ControlSnapshot;
use crate::control::Intent;
use crate::control::Permit;
use crate::control::ReceiptPhase;
use crate::delivery::*;
use crate::evidence;
use crate::evidence::VerifiedAcceptance;
use crate::evidence::VerifiedParent;
use crate::evidence::VerifiedProduction;
use crate::evidence::WorkIdentity;
use crate::release as domain;
use crate::release::AdmissionFacts;
use crate::release::DeliveryEvidence;
use crate::release::FreshReleaseFacts;
use crate::release::ReleaseAttempt;
use crate::release::ReleasePlan;
use crate::release::ReleaseQueue;
use crate::release::StepState;
use kegari_config::ReleaseMode;
use kegari_digest::canonical_digest;
use kegari_secrets::Signed;
use kegari_types::control::Lease;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeSet, sync::Mutex};

#[path = "runtime_delivery_no_release.rs"]
mod no_release;
pub(crate) use no_release::history::VerifiedNoReleaseCompletion;
#[path = "runtime_delivery_no_release_cache.rs"]
mod no_release_cache;
#[path = "runtime_delivery_release_cache.rs"]
mod release_cache;

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    carry_publication: Option<PublicationRef>,
    manual_intent: Option<Uuid>,
    manual_actor: Option<kegari_control_api::identity::Actor>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    display: Option<ReleaseDisplay>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReleaseDisplay {
    workspace_name: String,
    identifier: String,
    title: String,
    url: String,
}
impl ReleaseDisplay {
    fn capture(config: &Config, parent: &ParentProjection) -> Result<Self> {
        let (name, workspace) = config
            .workspaces
            .iter()
            .find(|(_, w)| w.id == parent.workspace_id)
            .ok_or_else(|| {
                Error::new(
                    "identity_mismatch",
                    "release display workspace is unavailable",
                )
            })?;
        let issue = parent.issue.as_ref().ok_or_else(|| {
            Error::new(
                "incomplete_snapshot",
                "release display has no native parent observation",
            )
        })?;
        if IssueKey::new(&issue.endpoint, issue.workspace_id, issue.id)? != parent.key
            || workspace.linear_workspace_id != issue.workspace_id
        {
            return Err(Error::new(
                "identity_mismatch",
                "release display native parent identity differs",
            ));
        }
        Ok(Self {
            workspace_name: name.clone(),
            identifier: issue.identifier.clone(),
            title: issue.title.clone(),
            url: issue.url.clone(),
        })
    }
    fn inspection(&self) -> crate::inspection::ParentDisplay {
        crate::inspection::ParentDisplay {
            workspace_name: Some(self.workspace_name.clone()),
            identifier: Some(self.identifier.clone()),
            title: Some(self.title.clone()),
            url: Some(self.url.clone()),
        }
    }
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
    carry_phase: Option<ActivePhase>,
    carry: Option<PublishedCarry>,
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
    carried: Option<PublishedCarry>,
    eligibility: domain::VerifiedReleaseEligibility,
    preparation: Arc<tokio::sync::Mutex<Preparation>>,
    admissions: BTreeMap<String, DeliveryAdmission>,
    workspace: WorkspaceDelivery,
    _active: crate::observations::ActiveSnapshot,
}
/// Authenticated no-release history plus its last complete inventory status.
/// Only a successful inventory can clear an error; unrelated queue rebuilds
/// must preserve the refusal and must not fall back to ephemeral current facts.
#[derive(Default)]
struct NoReleaseQueueHistory {
    proofs: BTreeMap<Uuid, VerifiedNoReleaseCompletion>,
    anchors: BTreeMap<Uuid, VerifiedNoReleaseCompletion>,
    unavailable_sealed: BTreeMap<Uuid, Error>,
    fatal_error: Option<Error>,
}
impl NoReleaseQueueHistory {
    fn is_sealed(&self, operation: &Uuid) -> bool {
        self.anchors.contains_key(operation)
            || self.proofs.contains_key(operation)
            || self.unavailable_sealed.contains_key(operation)
    }
    fn require_available(&self) -> Result<()> {
        if let Some(error) = &self.fatal_error {
            return Err(error.clone());
        }
        for (operation, error) in &self.unavailable_sealed {
            if error.code == "unsafe_storage" || !self.anchors.contains_key(operation) {
                return Err(error.clone());
            }
        }
        Ok(())
    }
    /// Import shared immutable identity as protective scope only. A new facade
    /// cannot use an empty queue to forget a known seal before its first full
    /// no-release inventory. Existing verified facts and diagnostics survive.
    fn retain_shared_anchors(
        &mut self,
        runtime: Uuid,
        shared: Vec<VerifiedNoReleaseCompletion>,
    ) -> Result<bool> {
        let mut anchors = self.anchors.clone();
        let mut unavailable = self.unavailable_sealed.clone();
        let mut learned = false;
        for anchor in shared {
            let operation = anchor.operation();
            if anchor.runtime_id() != runtime {
                return Err(Error::new(
                    "identity_mismatch",
                    "shared no-release anchor belongs to another runtime",
                ));
            }
            domain::CompletionScope::no_release(&anchor)?;
            if anchors
                .get(&operation)
                .is_some_and(|prior| prior.digest() != anchor.digest())
                || self
                    .proofs
                    .get(&operation)
                    .is_some_and(|prior| prior.digest() != anchor.digest())
            {
                return Err(Error::new(
                    "no_release_completion",
                    "shared no-release anchor changed its immutable identity",
                ));
            }
            anchors.entry(operation).or_insert_with(|| {
                learned = true;
                anchor
            });
            if !self.proofs.contains_key(&operation) && !unavailable.contains_key(&operation) {
                learned = true;
                unavailable.insert(
                    operation,
                    Error::new(
                        "record_missing",
                        "shared no-release seal has not been verified in this queue inventory",
                    ),
                );
            }
        }
        if anchors.len() > 4096 || unavailable.len() > 4096 {
            return Err(Error::new(
                "no_release_completion",
                "shared no-release scope inventory exceeds bound",
            ));
        }
        self.anchors = anchors;
        self.unavailable_sealed = unavailable;
        Ok(learned)
    }
    fn observed(
        &self,
        proofs: Vec<VerifiedNoReleaseCompletion>,
        anchors: Vec<VerifiedNoReleaseCompletion>,
        mut errors: BTreeMap<Uuid, Error>,
        mut sealed_operations: BTreeSet<Uuid>,
    ) -> Result<Self> {
        sealed_operations.extend(self.anchors.keys().copied());
        sealed_operations.extend(self.proofs.keys().copied());
        sealed_operations.extend(self.unavailable_sealed.keys().copied());
        if sealed_operations.len() > 4096
            || proofs.len() > 4096
            || anchors.len() > 4096
            || errors.len() > 4096
        {
            return Err(Error::new(
                "no_release_completion",
                "no-release queue history exceeds its finite bound",
            ));
        }
        let mut observed = Self {
            anchors: self.anchors.clone(),
            ..Self::default()
        };
        for anchor in anchors.into_iter().chain(self.proofs.values().cloned()) {
            let operation = anchor.operation();
            if !sealed_operations.contains(&operation) {
                return Err(Error::new(
                    "no_release_completion",
                    "no-release anchor is absent from authenticated seal inventory",
                ));
            }
            domain::CompletionScope::no_release(&anchor)?;
            if observed
                .anchors
                .get(&operation)
                .is_some_and(|prior| prior.digest() != anchor.digest())
            {
                errors.insert(
                    operation,
                    Error::new(
                        "no_release_completion",
                        "previously observed no-release seal changed",
                    ),
                );
            } else {
                observed.anchors.insert(operation, anchor);
            }
        }
        for proof in proofs {
            let operation = proof.operation();
            let anchor = observed.anchors.get(&operation).ok_or_else(|| {
                Error::new(
                    "no_release_completion",
                    "no-release proof has no authenticated anchor",
                )
            })?;
            if anchor.digest() != proof.digest() {
                errors.insert(
                    operation,
                    Error::new(
                        "no_release_completion",
                        "previously observed no-release seal changed",
                    ),
                );
            } else if !errors.contains_key(&operation) {
                observed.proofs.insert(operation, proof);
            }
        }
        for operation in sealed_operations {
            if !observed.anchors.contains_key(&operation) {
                return Err(Error::new(
                    "no_release_completion",
                    "sealed no-release operation has no authenticated protective scope",
                ));
            }
            if let Some(error) = errors.get(&operation) {
                observed.unavailable_sealed.insert(operation, error.clone());
            } else if !observed.proofs.contains_key(&operation) {
                observed.unavailable_sealed.insert(
                    operation,
                    Error::new(
                        "record_missing",
                        "previously observed no-release completion is absent from inventory",
                    ),
                );
            }
        }
        observed.require_available()?;
        Ok(observed)
    }
}

#[derive(Default)]
struct ReleaseState {
    restored: bool,
    attempts: BTreeMap<Uuid, AttemptRecord>,
    no_release: BTreeMap<Uuid, evidence::VerifiedNoReleaseRequired>,
    completions: BTreeMap<Uuid, VerifiedReleaseCompletion>,
    completion_anchors: BTreeMap<Uuid, ReleaseCompletionAnchor>,
    completion_inventory_error: Option<Error>,
    historical_no_release: NoReleaseQueueHistory,
    completion_errors: BTreeMap<Uuid, Error>,
    queue: ReleaseQueue,
}
impl ReleaseState {
    // Replacement inside impl ReleaseState; root-owned runtime_delivery_release.rs.
    fn completed_ancestors(
        &self,
        work: &WorkIdentity,
        resources: &BTreeSet<domain::ResourceKey>,
    ) -> Result<BTreeMap<Uuid, Uuid>> {
        if let Some(error) = &self.completion_inventory_error {
            return Err(error.clone());
        }
        work.validate()?;
        let terminals: BTreeSet<_> = self
            .completion_anchors
            .keys()
            .chain(self.completions.keys())
            .copied()
            .collect();
        if terminals.len() > 4096 {
            return Err(Error::new(
                "release_completion",
                "completion scope inventory exceeds bound",
            ));
        }
        let mut consumed = BTreeMap::new();
        for terminal in terminals {
            let observed = self.completions.get(&terminal);
            let anchor = match self.completion_anchors.get(&terminal) {
                Some(anchor) => anchor.clone(),
                None => observed
                    .ok_or_else(|| {
                        Error::new("release_completion", "completion scope is unavailable")
                    })?
                    .queue_anchor()?,
            };
            if anchor.operation() != terminal {
                return Err(Error::new(
                    "release_completion",
                    "completion anchor identity changed",
                ));
            }
            // A stored scoped error does not hide later corruption of its
            // authenticated outer seal, including on a disjoint query.
            anchor.validate_queue_envelope()?;
            let scope = anchor.queue_scope()?;
            let related = (scope.work().linear_endpoint == work.linear_endpoint
                && scope.work().linear_workspace == work.linear_workspace
                && scope.work().parent == work.parent)
                || !scope.resources().is_disjoint(resources);
            // A cached cap cannot erase an explicitly retained failed inventory
            // observation merely because one local validation now succeeds.
            if let Some(error) = self.completion_errors.get(&terminal) {
                if error.code == "unsafe_storage" || related {
                    return Err(error.clone());
                }
                continue;
            }
            let proof = match observed {
                Some(proof) => {
                    if proof.digest() != anchor.digest()
                        || proof.queue_anchor()?.queue_scope()? != scope
                    {
                        return Err(Error::new(
                            "release_completion",
                            "completion differs from its authenticated scope",
                        ));
                    }
                    proof
                }
                None => {
                    let error = self
                        .completion_errors
                        .get(&terminal)
                        .cloned()
                        .unwrap_or_else(|| {
                            Error::new(
                                "record_missing",
                                "authenticated completion lacks current verified references",
                            )
                        });
                    if error.code == "unsafe_storage" || related {
                        return Err(error);
                    }
                    continue;
                }
            };
            proof.validate_queue_envelope()?;
            match proof.validate() {
                Ok(ancestors) if related => {
                    for ancestor in ancestors.keys() {
                        if consumed.insert(*ancestor, terminal).is_some() {
                            return Err(Error::new(
                                "release_completion",
                                "ancestor has multiple terminal completions",
                            ));
                        }
                    }
                }
                Ok(_) => (),
                Err(error) if error.code == "unsafe_storage" || related => return Err(error),
                Err(_) => (),
            }
        }
        // Never-sealed producer errors are deliberately absent: they prove no
        // consumed ancestry, and their actual attempt holds remain in the queue.
        Ok(consumed)
    }
}

/// Historical failure state is preserved separately from its current resource
/// hold. A successful descendant consumes a hold without rewriting that state.
#[derive(Clone, Serialize)]
#[cfg(test)]
pub struct ReleaseHistory {
    pub parent: IssueKey,
    pub attempt: ReleaseAttempt,
    pub active_hold: bool,
    pub completed_by: Option<Uuid>,
    pub completion_error: Option<Error>,
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
    #[cfg(test)]
    completion_fixture_hooks: Mutex<FixtureCompletionHooks>,
    no_release_history: no_release::history::NoReleaseHistory,
    state: Arc<Mutex<ReleaseState>>,
    preparations: Mutex<Preparations>,
    live: Mutex<BTreeMap<Uuid, Arc<tokio::sync::Mutex<LiveRelease>>>>,
    progressing: tokio::sync::Mutex<()>,
}
// Per-facade, one-shot coordination only. These test hooks are reached by the
// unchanged production reader/commit paths and cannot supply a proof or permit.
#[cfg(test)]
#[derive(Default)]
struct FixtureCompletionHooks {
    first_anchor: Option<FixtureFirstCompletionAnchor>,
    commit_gate: Option<(Uuid, std::sync::mpsc::SyncSender<()>)>,
}
#[cfg(test)]
struct FixtureFirstCompletionAnchor {
    operation: Uuid,
    observed: std::sync::mpsc::SyncSender<()>,
    resume: std::sync::mpsc::Receiver<()>,
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
            "review_round",
            "review_round_preparation",
            "review_round_resume",
            "review_round_recovery",
            "session_resume",
            "session_recovery_preparation",
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
    completions: &BTreeMap<Uuid, VerifiedReleaseCompletion>,
    completion_anchors: &BTreeMap<Uuid, ReleaseCompletionAnchor>,
    completion_errors: &BTreeMap<Uuid, Error>,
    completion_inventory_error: Option<&Error>,
    historical_no_release: &NoReleaseQueueHistory,
) -> Result<ReleaseQueue> {
    if let Some(error) = completion_inventory_error {
        return Err(error.clone());
    }
    if let Some(error) = completion_errors
        .values()
        .find(|error| error.code == "unsafe_storage")
    {
        return Err(error.clone());
    }
    historical_no_release.require_available()?;
    let mut queue = ReleaseQueue::default();
    let mut quarantines = BTreeMap::new();
    let mut valid_completions = BTreeMap::new();
    let mut known_operations = BTreeSet::new();
    for (operation, anchor) in completion_anchors {
        if anchor.operation() != *operation {
            return Err(Error::new(
                "release_completion",
                "completion anchor key differs",
            ));
        }
        let scope = anchor.queue_scope()?;
        known_operations.extend(anchor.covered_operations());
        let error = completion_errors.get(operation).cloned().or_else(|| {
            (!completions.contains_key(operation)).then(|| {
                Error::new(
                    "record_missing",
                    "authenticated completion has no usable proof",
                )
            })
        });
        if let Some(error) = error {
            quarantines.insert(*operation, (scope, error));
        }
    }
    for (operation, proof) in completions {
        let anchor = proof.queue_anchor()?;
        let scope = anchor.queue_scope()?;
        if anchor.operation() != *operation
            || completion_anchors
                .get(operation)
                .map(|prior| prior.queue_scope())
                .transpose()?
                .as_ref()
                .is_some_and(|prior| prior != &scope)
        {
            return Err(Error::new(
                "release_completion",
                "completion proof differs from authenticated scope",
            ));
        }
        known_operations.extend(anchor.covered_operations());
        if quarantines.contains_key(operation) {
            continue;
        }
        proof.validate_queue_envelope()?;
        match proof.validate() {
            Ok(_) => {
                valid_completions.insert(*operation, proof);
            }
            Err(error) => {
                quarantines.insert(*operation, (scope, error));
            }
        }
    }
    let mut valid_no_release = Vec::new();
    for (operation, error) in &historical_no_release.unavailable_sealed {
        let anchor = historical_no_release
            .anchors
            .get(operation)
            .ok_or_else(|| error.clone())?;
        quarantines.insert(
            *operation,
            (domain::CompletionScope::no_release(anchor)?, error.clone()),
        );
    }
    for (operation, proof) in &historical_no_release.proofs {
        if quarantines.contains_key(operation) {
            continue;
        }
        proof.validate_queue_envelope()?;
        match proof.validate() {
            Ok(()) => valid_no_release.push(proof),
            Err(error) => {
                quarantines.insert(
                    *operation,
                    (domain::CompletionScope::no_release(proof)?, error),
                );
            }
        }
    }
    for (scope, error) in quarantines.values() {
        if error.code == "unsafe_storage" {
            return Err(error.clone());
        }
        queue.quarantine_completion(scope.clone(), error.clone())?;
    }
    let affected = |resources: &BTreeSet<domain::ResourceKey>,
                    dependencies: &[domain::DependencyRequirement]| {
        quarantines
            .values()
            .any(|(scope, _)| scope.affects(resources, dependencies))
    };
    // These were admitted against the then-current queue. Retain only unsealed,
    // unaffected ephemeral facts, before reconstructing later owners/holds.
    for (operation, proof) in no_release {
        let record = proof.record();
        let resources: BTreeSet<_> = record
            .baselines
            .iter()
            .map(|b| domain::ResourceKey::Repository {
                repository: b.repository.clone(),
            })
            .chain(record.targets.iter().map(|t| domain::ResourceKey::Target {
                provider: t.provider.clone(),
                id: t.target.clone(),
            }))
            .collect();
        if !historical_no_release.is_sealed(operation) && !affected(&resources, &[]) {
            queue.record_no_release_required(proof)?;
        }
    }
    for proof in valid_no_release {
        queue.record_no_release_completion(proof)?;
    }
    // Reconstruct actual retained owners and holds. Invalid completion evidence
    // cannot remove them; an authenticated protective scope cannot grant credit.
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
    for proof in valid_completions.values() {
        let mut conflicting = false;
        for (scope, _) in quarantines.values() {
            conflicting |= scope.conflicts_consumption(proof)?;
        }
        if !conflicting {
            queue.apply_completion(proof)?;
        }
        // Valid historical A credit remains true if a later overlapping B is
        // unavailable. This insertion does not clear any current owner or hold.
        queue.record_release_completion_history(proof)?;
    }
    for record in attempts
        .values()
        .filter(|r| r.attempt.completed() && r.pending_receipts.is_empty() && r.local_retired)
    {
        let plan = &record.attempt.plan;
        if !known_operations.contains(&plan.operation_id)
            && record.attempt.hold_reason.is_none()
            && !affected(&plan.resources, &plan.dependencies)
        {
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
    fn read_release_attempts(&self) -> Result<BTreeMap<Uuid, AttemptRecord>> {
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
            let record = signed.payload;
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
            records.insert(record.attempt.plan.operation_id, record);
        }
        Ok(records)
    }
    fn validate_release_admission(&self, record: &AttemptRecord) -> Result<()> {
        let plan = &record.attempt.plan;
        if record.admission_confirmed {
            let receipt = self.runtime.control.operation(record.admission_operation)?;
            let expected_scope = admission_scope(&receipt, plan)?;
            if receipt.kind != "admission"
                || receipt.runtime_id != record.runtime
                || receipt.expected.get("scope") != Some(&expected_scope)
                || receipt.owner_generation != record.attempt.owner_generation
                || receipt.canonical_targets != vec![issue(record)?.identity()?]
            {
                return Err(Error::new(
                    "ownership_conflict",
                    "release record lacks exact actual plan admission",
                ));
            }
        }
        Ok(())
    }
    fn restore_releases(&self) -> Result<()> {
        // All callers enter before their own admission/observation callbacks.
        // Local signed-record reads and first-anchor registration must be atomic
        // relative to commit_completion's known-seal check and protected write.
        self.runtime.control.serialize_observation(|_| {
            self.runtime.registration.verify_control_owner()?;
            #[derive(Clone, Copy)]
            enum FailedInventory {
                Release,
                NoRelease,
            }
            let mut failed_inventory = FailedInventory::Release;
            let mut full_inventory = false;
            let restored = (|| {
                let known = self.runtime.known_release_completion_anchors()?;
                let known_no_release = self.runtime.known_no_release_completions()?;
                let runtime = self.runtime.configuration.snapshot()?.runtime.id;
                let no_release_status = self.runtime.require_no_release_inventory_available();
                // A restored facade must not use its old queue after another
                // facade detected an ordinary global inventory failure. A real
                // cold/full read below may recover and clear that status.
                let shared_status = self.runtime.require_released_inventory_available();
                let known = {
                    let mut state = self.releases.state.lock().map_err(|_| unavailable())?;
                    let mut learned_anchor = state
                        .historical_no_release
                        .retain_shared_anchors(runtime, known_no_release)
                        .inspect_err(|_| failed_inventory = FailedInventory::NoRelease)?;
                    if let Err(error) = no_release_status {
                        // Shared fatal observation is never erased by this
                        // identity-only merge or by ordinary queue restoration.
                        state.historical_no_release.fatal_error = Some(error);
                        learned_anchor = true;
                    }
                    // Refuse an unavailable no-release inventory in its own
                    // category before the ordinary restore/rebuild. Successful
                    // no-release synchronization must be able to repair this
                    // refusal without clearing an independent release failure.
                    state
                        .historical_no_release
                        .require_available()
                        .inspect_err(|_| failed_inventory = FailedInventory::NoRelease)?;
                    for anchor in known {
                        let scope = anchor.queue_scope()?;
                        if state
                            .completion_anchors
                            .get(&anchor.operation())
                            .map(|prior| prior.queue_scope())
                            .transpose()?
                            .is_some_and(|prior| prior != scope)
                        {
                            return Err(Error::new(
                                "release_completion",
                                "known completion anchor changed",
                            ));
                        }
                        learned_anchor |=
                            !state.completion_anchors.contains_key(&anchor.operation());
                        state.completion_anchors.insert(anchor.operation(), anchor);
                    }
                    if state.completion_anchors.len() > 4096 {
                        return Err(Error::new(
                            "release_completion",
                            "completion anchor inventory exceeds finite bound",
                        ));
                    }
                    if state.restored {
                        shared_status?;
                        if learned_anchor {
                            // Another facade learned an immutable scope. Its
                            // missing local proof must fence this queue now,
                            // before any ordinary release eligibility read.
                            // Use live attempt state; do not inject restart holds.
                            state.queue = rebuild_queue(
                                &state.attempts,
                                &state.no_release,
                                &state.completions,
                                &state.completion_anchors,
                                &state.completion_errors,
                                state.completion_inventory_error.as_ref(),
                                &state.historical_no_release,
                            )?;
                        }
                        return Ok(());
                    }
                    state.completion_anchors.clone()
                };
                // These are bounded local file/receipt readers; operation(s)
                // never lock Control.state. No state guard crosses the shared
                // RuntimeState anchor-registration call below.
                let mut records = self.read_release_attempts()?;
                let mut inventory = self.learn_release_completion_queue_inventory(&known)?;
                self.runtime.remember_release_completion_anchors(
                    inventory.anchors.values().cloned().collect(),
                )?;
                for record in records.values_mut() {
                    if let Err(error) = self.validate_release_admission(record) {
                        if error.code == "unsafe_storage" {
                            return Err(error);
                        }
                        let covering: Vec<_> = inventory
                            .anchors
                            .iter()
                            .filter(|(_, anchor)| {
                                anchor
                                    .covered_operations()
                                    .any(|id| id == record.attempt.plan.operation_id)
                            })
                            .map(|(operation, _)| *operation)
                            .collect();
                        if covering.is_empty() {
                            return Err(error);
                        }
                        // The authenticated full scope still fences this record.
                        // An unavailable owner cannot be adopted or used as credit.
                        for operation in covering {
                            inventory.proofs.remove(&operation);
                            inventory.errors.insert(operation, error.clone());
                        }
                    }
                    if !record.attempt.completed() && !record.attempt.resolved {
                        record.attempt.hold_reason.get_or_insert_with(|| {
                            "recovered release requires explicit known-outcome reconciliation"
                                .into()
                        });
                    }
                }
                let mut state = self.releases.state.lock().map_err(|_| unavailable())?;
                // Keep authenticated identities even if unrelated history makes
                // the queue unusable. They never confer completion by themselves.
                state.completion_anchors = inventory.anchors.clone();
                let queue = rebuild_queue(
                    &records,
                    &state.no_release,
                    &inventory.proofs,
                    &inventory.anchors,
                    &inventory.errors,
                    None,
                    &state.historical_no_release,
                )?;
                state.queue = queue;
                state.completions = inventory.proofs;
                state.completion_anchors = inventory.anchors;
                state.completion_errors = inventory.errors;
                state.completion_inventory_error = None;
                state.attempts = records;
                state.restored = true;
                full_inventory = true;
                Ok(())
            })();
            if let Err(error) = &restored {
                match failed_inventory {
                    FailedInventory::Release => {
                        self.runtime.invalidate_released_inventory(error.clone())?;
                    }
                    FailedInventory::NoRelease => {
                        self.runtime
                            .invalidate_no_release_inventory(error.clone())?;
                    }
                }
                let mut state = self.releases.state.lock().map_err(|_| unavailable())?;
                state.queue.invalidate_after_rebuild(error.clone());
                match failed_inventory {
                    FailedInventory::Release => {
                        state.completion_inventory_error = Some(error.clone());
                    }
                    FailedInventory::NoRelease => {
                        state.historical_no_release.fatal_error = Some(error.clone());
                    }
                }
            } else if full_inventory {
                // Only an actual successful full read clears shared status;
                // the already-restored quick path never fabricates recovery.
                self.runtime.clear_released_inventory_error()?;
            }
            restored
        })
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
            let queue = rebuild_queue(
                &proposed,
                &state.no_release,
                &state.completions,
                &state.completion_anchors,
                &state.completion_errors,
                state.completion_inventory_error.as_ref(),
                &state.historical_no_release,
            )?;
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
                let (runtime, evidence) = match DeliveryCoordinator::decode_parent_publication(
                    &comment.body,
                    &self.runtime.services.key,
                ) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if runtime == config.runtime.id && &evidence == parent.manifest() {
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
        if active.is_none() && crate::config::config_revision(&config)? != record.config_revision {
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
            crate::control_command::verify_current_actor(&config, actor, role)?;
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
        if record.attempt.plan.carried_acceptance.is_some() != record.carry_publication.is_some() {
            return Err(Error::new(
                "stale_result",
                "release lost exact carried publication identity",
            ));
        }
        for reference in [&record.parent_publication, &record.acceptance_publication]
            .into_iter()
            .chain(record.carry_publication.as_ref())
        {
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
        if let Some(reference) = &record.carry_publication {
            let comment = comments
                .iter()
                .find(|c| c.id == reference.id)
                .ok_or_else(|| Error::new("stale_result", "carry publication disappeared"))?;
            DeliveryCoordinator::verify_carried_publication_for_plan(
                &comment.body,
                &runtime.services.key,
                &record.attempt.plan,
            )?;
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
    #[cfg(test)]
    pub fn release_history(&self) -> Result<Vec<ReleaseHistory>> {
        Ok(self
            .release_inspection_rows()?
            .into_iter()
            .map(|row| ReleaseHistory {
                parent: row.parent,
                attempt: row.attempt,
                active_hold: row.active_hold,
                completed_by: row.completed_by,
                completion_error: row.diagnostic,
            })
            .collect())
    }
    pub fn release_inspection_rows(&self) -> Result<Vec<crate::inspection::ReleaseInspectionRow>> {
        // Inspection authenticates the attempt inventory without requiring a
        // completion proof to grant authority or mutating restart recovery state.
        let mut attempts = self.read_release_attempts()?;
        let mut ownership_errors = BTreeMap::new();
        for (operation, record) in &attempts {
            if let Err(error) = self.validate_release_admission(record) {
                if error.code == "unsafe_storage" {
                    return Err(error);
                }
                ownership_errors.insert(*operation, error
                    .detail(json!({"release_operation":operation,"completion_stage":"owner_reference"}))
                    .next("Inspect and restore the exact retained owner receipt; do not readmit or repeat uncertain effects."));
            }
        }
        let diagnostics = self.inspect_release_completions(&attempts, &ownership_errors)?;
        {
            let state = self.releases.state.lock().map_err(|_| unavailable())?;
            for (operation, record) in &mut attempts {
                if let Some(cached) = state.attempts.get(operation) {
                    let mut normalized = cached.clone();
                    normalized.attempt.hold_reason = record.attempt.hold_reason.clone();
                    if canonical_digest(1, &normalized)? == canonical_digest(1, record)? {
                        record.attempt.hold_reason = cached.attempt.hold_reason.clone();
                    }
                } else if !state.restored && !record.attempt.completed() && !record.attempt.resolved
                {
                    record.attempt.hold_reason.get_or_insert_with(|| {
                        "retained release requires explicit known-outcome reconciliation".into()
                    });
                }
            }
        }
        let mut late_errors = BTreeMap::new();
        let mut rows = attempts
            .values()
            .map(|record| {
                let completed_by = diagnostics
                    .consumed
                    .get(&record.attempt.plan.operation_id)
                    .copied();
                let mut diagnostic = diagnostics
                    .errors
                    .get(&record.attempt.plan.operation_id)
                    .cloned();
                let active_hold = ownership_errors.contains_key(&record.attempt.plan.operation_id)
                    || (completed_by.is_none()
                        && !record.attempt.resolved
                        && (record.attempt.hold_reason.is_some()
                            || !record.pending_receipts.is_empty()
                            || (record.attempt.completed() && !record.local_retired)));
                let completion = if diagnostic.is_none() && !active_hold {
                    diagnostics
                        .terminals
                        .get(&record.attempt.plan.operation_id)
                        .filter(|proof| {
                            proof.workspace_id() == record.workspace && proof.work() == &record.work
                        })
                        .map(
                            |proof| -> Result<crate::inspection::ReleaseCompletionInspection> {
                                // Presentation accepts a valid legacy completion too;
                                // it does not grant dependency credit to that record.
                                proof.validate()?;
                                let success =
                                    ReleaseQueue::default().record_success(proof.terminal())?;
                                Ok(crate::inspection::ReleaseCompletionInspection {
                                    operation: success.operation_id,
                                    work: proof.work().clone(),
                                    completed_at: success.completed_at,
                                })
                            },
                        )
                        .transpose()
                } else {
                    Ok(None)
                };
                let completion = match completion {
                    Ok(completion) => completion,
                    Err(error) if error.code == "unsafe_storage" => return Err(error),
                    Err(error) => {
                        late_errors.insert(record.attempt.plan.operation_id, error.clone());
                        diagnostic = Some(error);
                        None
                    }
                };
                Ok(crate::inspection::ReleaseInspectionRow {
                    workspace: record.workspace,
                    work: record.work.clone(),
                    parent: issue(record)?,
                    attempt: record.attempt.clone(),
                    completed_by,
                    diagnostic,
                    active_hold,
                    completion,
                    display: record
                        .display
                        .as_ref()
                        .map(ReleaseDisplay::inspection)
                        .unwrap_or_default(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        for row in &mut rows {
            if let Some(error) = row
                .completed_by
                .and_then(|operation| late_errors.get(&operation))
            {
                let record = &attempts[&row.attempt.plan.operation_id];
                row.completed_by = None;
                row.completion = None;
                row.diagnostic = Some(error.clone());
                row.active_hold = ownership_errors.contains_key(&row.attempt.plan.operation_id)
                    || (!record.attempt.resolved
                        && (record.attempt.hold_reason.is_some()
                            || !record.pending_receipts.is_empty()
                            || (record.attempt.completed() && !record.local_retired)));
            }
        }
        crate::inspection::sort_rows(&mut rows);
        Ok(rows)
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
                carry_phase: None,
                carry: None,
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
            if let Some(phase) = &prepared.carry_phase {
                self.retire_delivery_phase(
                    phase,
                    json!({"carried_acceptance":"terminal_failure","error":error.code}),
                )?;
                prepared.carry_phase = None;
            }
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
        if prepared.carry_phase.is_some() || prepared.carry.is_some() {
            return self
                .progress_prepared_carry(lease, projection, &mut prepared)
                .await;
        }
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
        self.progress_prepared_carry(lease, projection, &mut prepared)
            .await
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
    candidate: &kegari_git::CandidateManifest,
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
        let carried = current.published_carry().cloned();
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
            carried: carried.as_ref().map(PublishedCarry::verified),
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
                    == crate::config::config_revision(&workspace.configuration).unwrap_or_default()
        });
        let successor = self.successor_intent(&plan, gate.work(), &acceptance)?;
        let successor_holds = if successor.is_some() {
            let baseline = predecessor.clone().ok_or_else(|| {
                Error::new(
                    "release_hold",
                    "successor admission lacks verified predecessor lineage",
                )
            })?;
            Some(self.successor_holds(&plan, gate.work(), baseline)?)
        } else {
            None
        };

        if let Some(manual) = &manual {
            crate::control_command::verify_current_actor(
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
            carried: carried.as_ref().map(PublishedCarry::verified),
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
            let queue = successor_holds
                .as_ref()
                .map(|proof| state.queue.successor_view(&plan, proof))
                .transpose()?;
            domain::verify_release_eligibility(
                &plan,
                evidence(),
                facts(&snapshot),
                queue.as_ref().unwrap_or(&state.queue),
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
                if crate::config::config_revision(&current_config)?
                    != crate::config::config_revision(&workspace.configuration)?
                {
                    return Err(Error::new(
                        "stale_config",
                        "release configuration changed during admission",
                    ));
                }
                if let Some(manual) = &manual {
                    crate::control_command::verify_current_actor(
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
                if let Some(carried) = &carried {
                    let projection = self.runtime.observations.admission_snapshot()?;
                    carried.verify_current_projection(
                        &current_config,
                        projection.parent(selected.id, gate.work().parent)?,
                    )?;
                }
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
                if let Some(proof) = &successor_holds {
                    state
                        .queue
                        .successor_view(&plan, proof)?
                        .eligible(&plan, Utc::now())?;
                } else {
                    state.queue.eligible(&plan, Utc::now())?;
                }
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
                    carry_publication: carried
                        .as_ref()
                        .map(|proof| {
                            proof
                                .publication_reference()
                                .map(|(id, body_digest, writer)| PublicationRef {
                                    id,
                                    body_digest,
                                    writer,
                                })
                        })
                        .transpose()?,
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
                    display: Some(ReleaseDisplay::capture(&workspace.configuration, parent)?),
                };
                Self::check_release_publications(&self.runtime, &record, None)?;
                let mut proposed = state.attempts.clone();
                proposed.insert(operation, record.clone());
                let queue = rebuild_queue(
                    &proposed,
                    &state.no_release,
                    &state.completions,
                    &state.completion_anchors,
                    &state.completion_errors,
                    state.completion_inventory_error.as_ref(),
                    &state.historical_no_release,
                )?;
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
            carried,
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
        carry: Option<PublishedCarry>,
        admitted_configuration: Config,
    ) -> ReleaseStepCheck {
        let state = self.releases.state.clone();
        let runtime = self.runtime.clone();
        let authority = self.retained_gate();
        Arc::new(move |plan, step| {
            runtime.registration.verify_active_owner(&permit.lease)?;
            authority(&gate)?;
            if let Some(carry) = &carry {
                // This is the actual LiveRelease workspace captured at admission,
                // not a current configuration snapshot or an inferred revision.
                if crate::config::config_revision(&admitted_configuration)?
                    != gate.config_revision()
                {
                    return Err(Error::new(
                        "ownership_conflict",
                        "carried release lost its actual pinned admission configuration",
                    ));
                }
                let active = ActiveParentGrantObserver::new(
                    runtime.clone(),
                    permit.clone(),
                    gate.clone(),
                    admitted_configuration.clone(),
                )?;
                carry.verify_active_projection(&active)?;
            }
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
                carried: live.carried.as_ref().map(PublishedCarry::verified),
                evidence: repository.evidence,
                observer: &workspace.observer,
                verifier: &verifier,
                final_gate: self.release_final_gate(
                    operation,
                    live.gate.clone(),
                    live.permit.clone(),
                    live.carried.clone(),
                    live.workspace.configuration.clone(),
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
        let projection = self.runtime.observations.admission_snapshot()?;
        // Only the ordinary live publication/evidence hydration may populate
        // readiness. Retained terminal attempt state alone grants nothing.
        // Drain above remains independent of new-admission hydration failures.
        self.refresh(&projection).await?;
        // Draining an existing phase may have taken a full provider/Git bound.
        // Preserve candidate ordering, not an expired observation snapshot.
        let projection = self.runtime.observations.admission_snapshot()?;
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
                self.runtime.registration.verify_active_owner(lease)?;
                let projection = self.release_candidate_projection(&candidate)?;
                if self
                    .advance_no_release(
                        lease,
                        &projection,
                        candidate.workspace,
                        candidate.parent.issue,
                    )
                    .await?
                {
                    return Ok(());
                }
                let projection = self.release_candidate_projection(&candidate)?;
                let Some(prepared) = self
                    .prepare_release(
                        lease,
                        &projection,
                        candidate.workspace,
                        candidate.parent.issue,
                    )
                    .await?
                else {
                    return Ok(());
                };
                let projection = self.release_candidate_projection(&candidate)?;
                self.admit_release(lease, &projection, prepared, &candidate)
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
    fn release_candidate_projection(
        &self,
        candidate: &ReleaseCandidate,
    ) -> Result<RuntimeProjection> {
        let current = self.runtime.observations.admission_snapshot()?;
        if current
            .parent(candidate.workspace, candidate.parent.issue)?
            .key
            != candidate.parent
        {
            return Err(Error::new(
                "identity_mismatch",
                "release selection changed provider identity during progression",
            ));
        }
        Ok(current)
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
        if let Err(error) = self.maintain_release_completions(projection).await {
            first.get_or_insert(error);
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
        if let Err(error) = self.maintain_release_completions(projection).await {
            first.get_or_insert(error);
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
    pub(super) fn release_workspace(
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
    key: &kegari_secrets::AttestationKey,
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
    let matches = comments
        .iter()
        .filter(|comment| {
            !comment.bot
                && !comment.delegated
                && comment.user_id == Some(writer)
                && comment.issue_id == projection.key.issue
        })
        .filter(|comment| {
            DeliveryCoordinator::decode_parent_publication(&comment.body, key).is_ok_and(
                |(observed_runtime, evidence)| {
                    observed_runtime == runtime && &evidence == parent.manifest()
                },
            )
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
    key: &kegari_secrets::AttestationKey,
    runtime: Uuid,
    comments: &[kegari_providers::Comment],
    selected: &kegari_providers::Comment,
) -> Result<()> {
    DeliveryCoordinator::reject_superseding_parent_publication(key, runtime, comments, selected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kegari_digest::canonical_digest;

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
            carry_publication: None,
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
            display: None,
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
        let directory = kegari_storage::PrivateDir::create(
            &root.path().canonicalize().unwrap().join("records"),
        )
        .unwrap();
        let key = kegari_secrets::AttestationKey::new("fixture".into(), vec![7; 32]).unwrap();
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
    async fn runtime_progression_preserves_missing_release_owner_as_a_blocker() {
        let (_root, fixture, delivery, wire, owner) =
            super::super::tests::fixture_authorization_with_wire(true).await;
        let (lease, _ownership) = owner.start(&fixture.runtime);
        let (plan, work, _) = crate::delivery::observed_merge_for_runtime().await;
        let operation = plan.operation_id;
        // Authenticated retained state is not its missing actual admission.
        // The production facade must visit release recovery before admitting
        // more work, while never adopting this record as a live permit.
        delivery.save_release(&record(plan, work)).unwrap();
        let before = fixture.runtime.control.operations().unwrap().len();
        let mutations = || {
            wire.lock()
                .unwrap()
                .requests
                .iter()
                .filter(|(_, body)| {
                    body["query"]
                        .as_str()
                        .is_some_and(|q| q.contains("mutation "))
                })
                .count()
        };
        let writes = mutations();
        assert_eq!(
            crate::runtime_driver::RuntimeProgression::advance(delivery.as_ref(), &lease)
                .await
                .unwrap_err()
                .code,
            "record_missing"
        );
        assert_eq!(
            crate::runtime_driver::RuntimeProgression::maintain(delivery.as_ref())
                .await
                .unwrap_err()
                .code,
            "record_missing"
        );
        assert_eq!(fixture.runtime.control.operations().unwrap().len(), before);
        assert_eq!(mutations(), writes);
        assert_eq!(
            fixture
                .runtime
                .control
                .operation(operation)
                .unwrap_err()
                .code,
            "record_missing"
        );
        assert!(
            fixture
                .runtime
                .control
                .snapshot()
                .unwrap()
                .active_operations
                .is_empty()
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

#[path = "runtime_delivery_lineage.rs"]
mod lineage;
pub(crate) use lineage::{
    ReleaseCompletionAnchor, VerifiedReleaseCompletion, VerifiedSuccessorHolds,
};

#[cfg(test)]
#[path = "runtime_delivery_successor_fixture.rs"]
mod successor_fixture;

#[path = "runtime_delivery_carry.rs"]
mod carry;
