//! Authoritative, bounded provider reconciliation and transient inspection views.
//! This module performs reads only. No issue body is placed in recovery receipts.
use crate::{
    authority::{
        self, ChildScope, CommentRevision, Corrections, IssueKey, LabelTransition, ParentMetadata,
        ScopeSnapshot,
    },
    config::{Config, Profile, ReleaseMode, Workspace},
    digest::canonical_digest,
    error::{Error, Result},
    evidence::{
        AttestationKey, Generation, Publication, Signed, VerifiedAcceptance, VerifiedChild,
        VerifiedParent, WorkIdentity,
    },
    providers::{
        Comment, Dependency, GithubClient, HistoryEntry, Issue, LinearClient, PullRequest,
        Repository,
    },
    workflow::Workflow,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

#[derive(Clone)]
pub struct WorkspaceClients {
    pub linear: LinearClient,
    pub github: GithubClient,
}

/// Small signed authoritative record, published to the parent before admission.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorizationSnapshot {
    pub schema_version: u32,
    pub parent: IssueKey,
    pub event: LabelTransition,
    pub required_children: BTreeSet<Uuid>,
    pub scope: ScopeSnapshot,
    pub dependency_ids: BTreeMap<Uuid, BTreeSet<Uuid>>,
    pub dependency_generations: BTreeMap<Uuid, Generation>,
}

#[derive(Clone, Debug)]
pub struct VerifiedAuthorization {
    snapshot: AuthorizationSnapshot,
    publication_id: Uuid,
    digest: String,
}
impl VerifiedAuthorization {
    pub fn verify(
        key: &AttestationKey,
        signed: &Signed<AuthorizationSnapshot>,
        publication: &Publication,
        writer: &str,
    ) -> Result<Self> {
        key.verify("parent_authorization", signed)?;
        publication.verify(signed, "linear", writer)?;
        let snapshot = &signed.payload;
        if snapshot.schema_version != 1
            || snapshot.required_children.is_empty()
            || snapshot.required_children != snapshot.scope.child_scopes.keys().copied().collect()
            || snapshot.required_children
                != snapshot.scope.child_definitions.keys().copied().collect()
            || snapshot.required_children.contains(&snapshot.parent.issue)
            || snapshot.required_children.iter().any(Uuid::is_nil)
            || snapshot
                .dependency_ids
                .values()
                .flatten()
                .copied()
                .collect::<BTreeSet<_>>()
                != snapshot.dependency_generations.keys().copied().collect()
        {
            return Err(Error::new(
                "authorization_provenance_unavailable",
                "published authorization has inconsistent required scope",
            ));
        }
        IssueKey::new(
            &snapshot.parent.endpoint,
            snapshot.parent.workspace,
            snapshot.parent.issue,
        )?;
        let publication_id = Uuid::parse_str(&publication.record_id).map_err(|_| {
            Error::invalid("authorization publication needs canonical comment UUID")
        })?;
        Ok(Self {
            snapshot: snapshot.clone(),
            publication_id,
            digest: signed.digest()?,
        })
    }
    pub fn snapshot(&self) -> &AuthorizationSnapshot {
        &self.snapshot
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
}

/// Only identity/digest/correction references, never provider issue bodies.
#[derive(Clone, Debug, Default)]
pub struct ParentRefs {
    /// Durable identity-only retention; a lower bound for inspection, never an
    /// authorization or completion proof when publications are unavailable.
    pub required_child_ids: BTreeSet<Uuid>,
    pub authorization: Option<VerifiedAuthorization>,
    pub corrections: Corrections,
    pub observation_revision: u64,
    pub host_comment_ids: BTreeSet<Uuid>,
    /// First observed disappearance. A later human label event is required to
    /// reconcile removal, even when the deleted issue's history is inaccessible.
    pub disappeared_children: BTreeMap<Uuid, DateTime<Utc>>,
    /// Provider-reconstructed prerequisite generations when preparing initial
    /// authorization; published pins take precedence on subsequent refreshes.
    pub dependency_generations: BTreeMap<Uuid, Generation>,
    pub definition_correction: Option<VerifiedDefinitionCorrection>,
    pub explicit_corrections: BTreeMap<Uuid, crate::decisions::VerifiedExplicitCorrection>,
}
pub type ReconcileRefs = BTreeMap<Uuid, BTreeMap<Uuid, ParentRefs>>;

/// Host-authenticated result of interpreting permitted existing-scope intent.
/// It cannot add children, paths, repositories or production targets.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DefinitionCorrection {
    pub schema_version: u32,
    pub parent: IssueKey,
    pub authorization_digest: String,
    pub before: ScopeSnapshot,
    pub after: ScopeSnapshot,
    pub correction_generation: String,
    pub correction_watermark: u64,
    pub source_comments: BTreeMap<Uuid, String>,
}
#[derive(Clone, Debug)]
pub struct VerifiedDefinitionCorrection {
    record: DefinitionCorrection,
    publication_id: Uuid,
}
impl VerifiedDefinitionCorrection {
    pub fn verify(
        key: &AttestationKey,
        signed: &Signed<DefinitionCorrection>,
        publication: &Publication,
        writer: &str,
    ) -> Result<Self> {
        key.verify("in_scope_definition_correction", signed)?;
        publication.verify(signed, "linear", writer)?;
        let record = &signed.payload;
        if record.schema_version != 1
            || record.correction_watermark == 0
            || record.source_comments.is_empty()
            || record.before.child_scopes != record.after.child_scopes
            || record.before.production_targets != record.after.production_targets
            || record.before.child_definitions.keys().collect::<Vec<_>>()
                != record.after.child_definitions.keys().collect::<Vec<_>>()
        {
            return Err(Error::new(
                "scope_violation",
                "correction cannot add or widen the published child/repository/path/target scope",
            ));
        }
        for digest in record
            .source_comments
            .values()
            .chain([&record.authorization_digest, &record.correction_generation])
        {
            crate::evidence::validate_digest(digest)?;
        }
        let publication_id = Uuid::parse_str(&publication.record_id).map_err(|_| {
            Error::invalid("correction publication requires canonical comment UUID")
        })?;
        Ok(Self {
            record: record.clone(),
            publication_id,
        })
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Blocker {
    pub code: String,
    pub reason: String,
    pub affected: IssueKey,
    pub impacted_gates: Vec<String>,
    pub last_verified_evidence: Option<String>,
    pub attempted_recovery: String,
    pub next_action: String,
    pub prerequisite: Option<IssueKey>,
}
fn blocker(key: &IssueKey, code: &str, reason: impl Into<String>, next: &str) -> Blocker {
    Blocker {
        code: code.into(),
        reason: reason.into(),
        affected: key.clone(),
        impacted_gates: vec!["admission".into(), "completion".into(), "release".into()],
        last_verified_evidence: None,
        attempted_recovery: "Complete authoritative provider refresh".into(),
        next_action: next.into(),
        prerequisite: None,
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct IssueObservation {
    pub issue: Issue,
    pub comments: Vec<Comment>,
    pub history: Vec<HistoryEntry>,
    pub dependencies: Vec<Dependency>,
}
#[derive(Clone, Debug, Serialize)]
pub struct ChildProjection {
    pub parent_reference: String,
    pub key: IssueKey,
    pub issue: Option<Issue>,
    pub scope: Option<ChildScope>,
    pub profile: Option<Profile>,
    pub work: Option<WorkIdentity>,
    pub blockers: Vec<Blocker>,
    pub prerequisites: BTreeSet<Uuid>,
}
#[derive(Clone, Debug, Serialize)]
pub struct ParentProjection {
    pub workspace_id: Uuid,
    pub workspace_name: String,
    pub parent_reference: String,
    pub key: IssueKey,
    pub issue: Option<Issue>,
    pub included: bool,
    pub observed_at: DateTime<Utc>,
    pub required_children: BTreeSet<Uuid>,
    pub children: BTreeMap<Uuid, ChildProjection>,
    pub observations: BTreeMap<Uuid, IssueObservation>,
    pub metadata: Option<ParentMetadata>,
    pub authorization_candidate: Option<AuthorizationSnapshot>,
    pub authorization_published: bool,
    pub corrections: Corrections,
    pub observation_revision: u64,
    pub disappeared_children: BTreeMap<Uuid, DateTime<Utc>>,
    pub work: Option<WorkIdentity>,
    pub blockers: Vec<Blocker>,
    pub prerequisites: BTreeSet<Uuid>,
    pub source_digest: String,
    pub config_revision: String,
    pub workflow_digest: String,
    pub coordination_profile: Profile,
    pub drain_config_revision: String,
    pub release_mode: ReleaseMode,
}
#[derive(Clone, Debug, Serialize)]
pub struct IndependentJob {
    pub workspace_id: Uuid,
    pub repository_id: u64,
    pub pull_request: PullRequest,
    pub review_identity: String,
    pub phase: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct WorkspaceProjection {
    pub workspace_id: Uuid,
    pub workspace_name: String,
    pub complete: bool,
    pub observed_at: DateTime<Utc>,
    pub parents: BTreeMap<Uuid, ParentProjection>,
    pub repositories: BTreeMap<u64, Repository>,
    pub independent_jobs: Vec<IndependentJob>,
    pub error: Option<Error>,
    pub config_revision: String,
    pub workflow_digest: String,
    pub independent_review_profile: Option<Profile>,
}
#[derive(Clone, Debug, Serialize)]
pub struct RuntimeProjection {
    pub runtime_id: Uuid,
    pub config_revision: String,
    pub observed_at: DateTime<Utc>,
    pub workspaces: BTreeMap<Uuid, WorkspaceProjection>,
}

#[derive(Clone, Serialize)]
struct Collected {
    roots: BTreeSet<Uuid>,
    selected: BTreeSet<Uuid>,
    hierarchy: BTreeMap<Uuid, BTreeSet<Uuid>>,
    issues: BTreeMap<Uuid, IssueObservation>,
    missing: BTreeSet<Uuid>,
    repositories: BTreeMap<u64, Repository>,
    independent: BTreeMap<u64, Vec<PullRequest>>,
}
#[derive(Clone, Debug)]
pub struct Reconciler {
    max_issues: usize,
}
impl Default for Reconciler {
    fn default() -> Self {
        Self {
            max_issues: 100_000,
        }
    }
}
impl Reconciler {
    pub fn bounded(max_issues: usize) -> Result<Self> {
        if !(1..=100_000).contains(&max_issues) {
            return Err(Error::invalid(
                "reconciliation issue bound must be 1..=100000",
            ));
        }
        Ok(Self { max_issues })
    }
    /// Each workspace either has a stable complete read or an explicit failed
    /// projection. A failing workspace cannot admit work from a partial list.
    pub async fn refresh(
        &self,
        config: &Config,
        clients: &BTreeMap<Uuid, WorkspaceClients>,
        workflows: &BTreeMap<Uuid, Workflow>,
        retained: &ReconcileRefs,
    ) -> Result<RuntimeProjection> {
        config.validate()?;
        let revision = config.revision()?;
        let mut output = RuntimeProjection {
            runtime_id: config.runtime.id,
            config_revision: revision.clone(),
            observed_at: Utc::now(),
            workspaces: BTreeMap::new(),
        };
        for (name, workspace) in &config.workspaces {
            let empty = BTreeMap::new();
            let prior = retained.get(&workspace.id).unwrap_or(&empty);
            let result = async {
                let bound = clients.get(&workspace.id).ok_or_else(|| Error::new("missing_secret", "workspace has no configured bound provider clients"))?;
                let workflow = workflows.get(&workspace.id).ok_or_else(|| Error::invalid("workspace workflow has not been loaded"))?;
                let first = self.collect(config, workspace, bound, prior).await?;
                let second = self.collect(config, workspace, bound, prior).await?;
                if canonical_digest(1, &first)? != canonical_digest(1, &second)? {
                    return Err(Error::new("unstable_observation", "provider records changed during complete reconciliation; refresh before admission"));
                }
                project(config, name, workspace, workflow, prior, second, Utc::now())
            }.await;
            let projection = match result {
                Ok(value) => value,
                Err(error) => failed_workspace(
                    config,
                    name,
                    workspace,
                    workflows.get(&workspace.id),
                    &revision,
                    prior,
                    error,
                )?,
            };
            output.workspaces.insert(workspace.id, projection);
        }
        // Canonical provider work matching two configurations is a conflict,
        // including selected-but-unpublished parents; never two independent jobs.
        let mut owners: BTreeMap<IssueKey, Vec<Uuid>> = BTreeMap::new();
        for workspace in output.workspaces.values() {
            for parent in workspace.parents.values().filter(|p| p.included) {
                owners
                    .entry(parent.key.clone())
                    .or_default()
                    .push(workspace.workspace_id);
            }
        }
        for workspace in output.workspaces.values_mut() {
            for parent in workspace.parents.values_mut() {
                if owners.get(&parent.key).is_some_and(|ids| ids.len() > 1) {
                    parent.blockers.push(blocker(
                        &parent.key,
                        "ownership_conflict",
                        "Canonical Linear parent matches multiple Kegari workspace configurations",
                        "Remove overlapping inclusion or reconcile the existing canonical owner",
                    ));
                }
            }
        }
        output.observed_at = Utc::now();
        Ok(output)
    }
    async fn collect(
        &self,
        config: &Config,
        workspace: &Workspace,
        clients: &WorkspaceClients,
        prior: &BTreeMap<Uuid, ParentRefs>,
    ) -> Result<Collected> {
        let linear_account = clients.linear.identity().await?;
        let github_account = clients.github.identity().await?;
        if linear_account.account_id != config.connections[&workspace.linear_connection].account_id
            || linear_account.workspace_id != Some(workspace.linear_workspace_id)
            || github_account.account_id
                != config.connections[&workspace.github_connection].account_id
        {
            return Err(Error::new(
                "identity_mismatch",
                "bound provider account differs from current workspace configuration",
            ));
        }
        let selected: BTreeMap<_, _> = clients
            .linear
            .discover_parents()
            .await?
            .into_iter()
            .map(|p| (p.id, p))
            .collect();
        let roots: BTreeSet<_> = selected.keys().chain(prior.keys()).copied().collect();
        if roots.len() > self.max_issues {
            return Err(Error::new(
                "provider_pagination",
                "reconciliation exceeds configured issue limit",
            ));
        }
        let mut current = clients.linear.refresh(&roots).await?;
        for (id, issue) in &selected {
            if current.issues.get(id).is_none_or(|other| {
                canonical_digest(1, other).ok() != canonical_digest(1, issue).ok()
            }) {
                return Err(Error::new(
                    "unstable_observation",
                    "selected parent changed or disappeared during refresh",
                ));
            }
        }
        let mut hierarchy = BTreeMap::new();
        let mut needed = BTreeSet::new();
        for parent in &roots {
            if let Some(issue) = current.issues.get(parent) {
                let children = clients.linear.children(*parent).await?;
                let ids: BTreeSet<_> = children.iter().map(|child| child.id).collect();
                needed.extend(&ids);
                if let Ok(meta) = authority::metadata::<ParentMetadata>(
                    issue.description.as_deref().unwrap_or(""),
                ) {
                    needed.extend(meta.required_children);
                }
                for child in children {
                    current.issues.insert(child.id, child);
                }
                hierarchy.insert(*parent, ids);
            }
            if let Some(authorization) = prior.get(parent).and_then(|p| p.authorization.as_ref()) {
                needed.extend(&authorization.snapshot.required_children);
            }
            if let Some(prior) = prior.get(parent) {
                needed.extend(&prior.required_child_ids);
            }
        }
        needed.extend(roots.iter().copied());
        let mut observations = BTreeMap::new();
        let endpoint = config.connections[&workspace.linear_connection]
            .endpoint
            .as_deref()
            .ok_or_else(|| Error::invalid("Linear endpoint missing"))?;
        while !needed.is_empty() {
            if observations.len() + current.missing.len() + needed.len() > self.max_issues {
                return Err(Error::new(
                    "provider_pagination",
                    "dependency closure exceeds configured issue limit",
                ));
            }
            let fetched = clients.linear.refresh(&needed).await?;
            current.missing.extend(fetched.missing);
            needed.clear();
            for (id, issue) in fetched.issues {
                if IssueKey::new(&issue.endpoint, issue.workspace_id, id)?
                    != IssueKey::new(endpoint, workspace.linear_workspace_id, id)?
                {
                    return Err(Error::new(
                        "identity_mismatch",
                        "issue endpoint/workspace differs from configured provider",
                    ));
                }
                if let Some(previous) = current.issues.get(&id)
                    && canonical_digest(1, previous)? != canonical_digest(1, &issue)?
                {
                    return Err(Error::new(
                        "unstable_observation",
                        "issue changed while enumerating hierarchy",
                    ));
                }
                let mut comments = clients.linear.comments(id).await?;
                let mut history = clients.linear.history(id).await?;
                let mut dependencies = clients.linear.dependencies(id).await?;
                comments.sort_by_key(|c| c.id);
                history.sort_by_key(|h| h.id);
                dependencies.sort_by_key(|d| d.id);
                for prerequisite in prerequisites(id, &dependencies)? {
                    if !observations.contains_key(&prerequisite)
                        && !current.missing.contains(&prerequisite)
                    {
                        needed.insert(prerequisite);
                    }
                }
                observations.insert(
                    id,
                    IssueObservation {
                        issue,
                        comments,
                        history,
                        dependencies,
                    },
                );
            }
            needed.retain(|id| !observations.contains_key(id) && !current.missing.contains(id));
        }
        let repositories: BTreeMap<_, _> = clients
            .github
            .discover_repositories()
            .await?
            .into_iter()
            .map(|r| (r.identity.id, r))
            .collect();
        let mut independent = BTreeMap::new();
        if workspace.independent_review.enabled {
            for (id, repository) in &repositories {
                if repository.archived || repository.disabled {
                    continue;
                }
                let mut prs = clients
                    .github
                    .independent_review_candidates(&repository.identity)
                    .await?;
                prs.sort_by_key(|pr| pr.number);
                independent.insert(*id, prs);
            }
        }
        Ok(Collected {
            roots,
            selected: selected.keys().copied().collect(),
            hierarchy,
            issues: observations,
            missing: current.missing,
            repositories,
            independent,
        })
    }
}

fn prerequisites(issue: Uuid, relations: &[Dependency]) -> Result<BTreeSet<Uuid>> {
    let mut result = BTreeSet::new();
    for relation in relations {
        match relation.relation_type.as_str() {
            "blocks" if relation.related_issue_id == issue => {
                result.insert(relation.issue_id);
            }
            "blocks" | "duplicate" | "related" | "similar" => {}
            _ => {
                return Err(Error::new(
                    "provider_response",
                    "unknown Linear relation type cannot establish dependency direction",
                ));
            }
        }
    }
    Ok(result)
}
fn history_events(history: &[HistoryEntry], label: Uuid) -> Result<Vec<LabelTransition>> {
    history
        .iter()
        .filter(|h| h.added_labels.contains(&label) || h.removed_labels.contains(&label))
        .map(|h| {
            let time = h.created_at.ok_or_else(|| {
                Error::new(
                    "authorization_provenance_unavailable",
                    "authorization event lacks a valid timestamp",
                )
            })?;
            let actors: BTreeSet<_> = h.actor_ids.iter().copied().chain(h.actor_id).collect();
            Ok(LabelTransition {
                event_id: h.id,
                occurred_at: time,
                actor: h.actor_id,
                added: h.added_labels.iter().copied().collect(),
                removed: h.removed_labels.iter().copied().collect(),
                unambiguous: !h.bot
                    && h.actor_id.is_some()
                    && actors.len() == 1
                    && h.updated_at.is_none_or(|updated| updated == time),
            })
        })
        .collect()
}
fn definition(issue: &Issue) -> Result<String> {
    // Routing and presentation state do not create a new work generation.
    canonical_digest(
        1,
        &serde_json::json!({"title":issue.title,"description":issue.description,"parent":issue.parent_id}),
    )
}
fn drain_config_revision(config: &Config, workspace: &Workspace) -> Result<String> {
    let mut scope = serde_json::to_value(workspace)
        .map_err(|_| Error::invalid("workspace cannot be canonicalized"))?;
    if let Some(selection) = scope
        .get_mut("linear")
        .and_then(serde_json::Value::as_object_mut)
    {
        for ordinary_filter in ["teams", "projects", "users"] {
            selection.remove(ordinary_filter);
        }
    }
    canonical_digest(
        1,
        &serde_json::json!({"workspace":scope,"runtime":config.runtime,"linear_connection":config.connections[&workspace.linear_connection],"github_connection":config.connections[&workspace.github_connection],"codex_connection":config.connections[&workspace.codex_connection]}),
    )
}
fn key(config: &Config, workspace: &Workspace, id: Uuid) -> Result<IssueKey> {
    IssueKey::new(
        config.connections[&workspace.linear_connection]
            .endpoint
            .as_deref()
            .ok_or_else(|| Error::invalid("Linear endpoint missing"))?,
        workspace.linear_workspace_id,
        id,
    )
}

fn project(
    config: &Config,
    name: &str,
    workspace: &Workspace,
    workflow: &Workflow,
    retained: &BTreeMap<Uuid, ParentRefs>,
    facts: Collected,
    now: DateTime<Utc>,
) -> Result<WorkspaceProjection> {
    let revision = config.revision()?;
    let mut output = WorkspaceProjection {
        workspace_id: workspace.id,
        workspace_name: name.into(),
        complete: true,
        observed_at: now,
        parents: BTreeMap::new(),
        repositories: facts.repositories.clone(),
        independent_jobs: Vec::new(),
        error: None,
        config_revision: revision.clone(),
        workflow_digest: workflow.digest.clone(),
        independent_review_profile: workspace.profiles.independent_review.clone(),
    };
    let allowed_repositories: BTreeSet<_> = workspace
        .repositories
        .keys()
        .filter(|id| {
            id.parse::<u64>().ok().is_some_and(|id| {
                facts.repositories.get(&id).is_some_and(|r| {
                    !r.archived
                        && !r.disabled
                        && r.permissions
                            .get("pull")
                            .and_then(serde_json::Value::as_bool)
                            == Some(true)
                })
            })
        })
        .cloned()
        .collect();
    let resources = workspace.test_resources.keys().cloned().collect();
    for parent_id in &facts.roots {
        let default = ParentRefs::default();
        let prior = retained.get(parent_id).unwrap_or(&default);
        let parent_key = key(config, workspace, *parent_id)?;
        let reference = format!("{}/{parent_id}", workspace.id);
        let issue = facts.issues.get(parent_id).map(|v| v.issue.clone());
        let mut parent = ParentProjection {
            workspace_id: workspace.id,
            workspace_name: name.into(),
            parent_reference: reference.clone(),
            key: parent_key.clone(),
            issue: issue.clone(),
            included: facts.selected.contains(parent_id),
            observed_at: now,
            required_children: BTreeSet::new(),
            children: BTreeMap::new(),
            observations: BTreeMap::new(),
            metadata: None,
            authorization_candidate: None,
            authorization_published: false,
            corrections: prior.corrections.clone(),
            observation_revision: prior
                .observation_revision
                .checked_add(1)
                .ok_or_else(|| Error::invalid("observation revision overflow"))?,
            disappeared_children: prior.disappeared_children.clone(),
            work: None,
            blockers: Vec::new(),
            prerequisites: BTreeSet::new(),
            source_digest: canonical_digest(1, &facts)?,
            config_revision: revision.clone(),
            workflow_digest: workflow.digest.clone(),
            coordination_profile: workspace.profiles.coordination.clone(),
            drain_config_revision: drain_config_revision(config, workspace)?,
            release_mode: workspace.release.mode,
        };
        if !parent.included {
            parent.blockers.push(blocker(
                &parent_key,
                "excluded",
                "Parent no longer matches ordinary selection filters",
                "Restore inclusion to admit new work; existing authorized work may drain",
            ));
        }
        let Some(issue) = issue else {
            parent.required_children = prior.required_child_ids.clone();
            if let Some(authorized) = &prior.authorization {
                parent
                    .required_children
                    .extend(&authorized.snapshot.required_children);
            }
            for child in &parent.required_children {
                parent.children.insert(
                    *child,
                    missing_child(config, workspace, &reference, *child)?,
                );
            }
            parent.blockers.push(blocker(&parent_key, "inaccessible_parent", "Previously owned parent is missing or inaccessible", "Restore access and reconcile authoritative scope; retained children remain required"));
            output.parents.insert(*parent_id, parent);
            continue;
        };
        if issue.parent_id.is_some() {
            parent.blockers.push(blocker(
                &parent_key,
                "reparented_parent",
                "Previously owned root is now a child",
                "Reconcile changed hierarchy through renewed parent authorization",
            ));
        }
        let meta =
            match authority::metadata::<ParentMetadata>(issue.description.as_deref().unwrap_or(""))
            {
                Ok(meta) => Some(meta),
                Err(error) => {
                    parent.blockers.push(blocker(
                        &parent_key,
                        &error.code,
                        error.message,
                        "Prepare the complete parent outcome and strict kegari metadata in Linear",
                    ));
                    None
                }
            };
        let observation = &facts.issues[parent_id];
        let labels = issue.labels.iter().map(|l| l.id).collect();
        let authorization = history_events(
            &observation.history,
            workspace.linear.authorization_label_id,
        )
        .and_then(|history| {
            authority::latest_authorization(
                workspace.linear.authorization_label_id,
                &labels,
                &history,
                true,
                &workspace.actors.authorizers,
            )
        });
        if let Err(error) = &authorization {
            parent.blockers.push(blocker(
                &parent_key,
                &error.code,
                &error.message,
                "Apply the authorization label with a currently permitted Linear authorizer",
            ));
        }
        let enumerated = facts.hierarchy.get(parent_id).cloned().unwrap_or_default();
        let previous: BTreeSet<_> = prior
            .authorization
            .as_ref()
            .map(|a| a.snapshot.required_children.clone())
            .unwrap_or_default()
            .union(&prior.required_child_ids)
            .copied()
            .collect();
        let declared: BTreeSet<_> = meta
            .as_ref()
            .map(|m| m.required_children.iter().copied().collect())
            .unwrap_or_default();
        let absent: BTreeSet<_> = previous
            .difference(&declared.union(&enumerated).copied().collect())
            .copied()
            .collect();
        for id in &absent {
            parent.disappeared_children.entry(*id).or_insert(now);
        }
        parent
            .disappeared_children
            .retain(|id, _| absent.contains(id));
        let removals: BTreeSet<_> = parent
            .disappeared_children
            .iter()
            .filter(|(_, observed)| {
                authorization
                    .as_ref()
                    .is_ok_and(|event| event.occurred_at > **observed)
            })
            .map(|(id, _)| *id)
            .collect();
        parent.required_children = declared
            .union(&enumerated)
            .copied()
            .chain(previous.difference(&removals).copied())
            .collect();
        if let Some(meta) = &meta {
            match authority::required_children(meta, &enumerated, &previous, &removals) {
                Ok(required) => parent.required_children = required,
                Err(error) => parent.blockers.push(blocker(
                    &parent_key,
                    &error.code,
                    error.message,
                    "Prepare at least one required child and an identifiable required E2E child",
                )),
            }
            let configured_targets: BTreeSet<_> = workspace
                .repositories
                .values()
                .flat_map(|r| match &r.deployment {
                    crate::config::Deployment::None(_) => Vec::new(),
                    crate::config::Deployment::Steps { steps } => {
                        steps.iter().map(|s| s.target_id.clone()).collect()
                    }
                })
                .collect();
            if meta
                .production_targets
                .iter()
                .collect::<BTreeSet<_>>()
                .len()
                != meta.production_targets.len()
                || meta
                    .production_targets
                    .iter()
                    .any(|target| !configured_targets.contains(target))
            {
                parent.blockers.push(blocker(
                    &parent_key,
                    "scope_violation",
                    "Parent production target is duplicate or not configured",
                    "Prepare explicit configured production target IDs",
                ));
            }
        }
        let mut scope = ScopeSnapshot {
            child_scopes: BTreeMap::new(),
            child_definitions: BTreeMap::new(),
            parent_definition: definition(&issue)?,
            production_targets: meta
                .as_ref()
                .map(|m| m.production_targets.iter().cloned().collect())
                .unwrap_or_default(),
        };
        let mut dependency_ids = BTreeMap::new();
        for child_id in &parent.required_children {
            let child_key = key(config, workspace, *child_id)?;
            let Some(child_observation) = facts.issues.get(child_id) else {
                parent.children.insert(
                    *child_id,
                    missing_child(config, workspace, &reference, *child_id)?,
                );
                continue;
            };
            let child_issue = &child_observation.issue;
            let mut child = ChildProjection {
                parent_reference: reference.clone(),
                key: child_key.clone(),
                issue: Some(child_issue.clone()),
                scope: None,
                profile: None,
                work: None,
                blockers: Vec::new(),
                prerequisites: prerequisites(*child_id, &child_observation.dependencies)?,
            };
            if child_issue.parent_id != Some(*parent_id) {
                child.blockers.push(blocker(&child_key, "required_child_reparented", "Required child no longer belongs to the authorized parent", "Restore hierarchy or renew authorization after reconciling the removed responsibility"));
            }
            match authority::metadata::<ChildScope>(
                child_issue.description.as_deref().unwrap_or(""),
            ) {
                Ok(value) => {
                    if let Err(error) = value.validate(&allowed_repositories, &resources) {
                        child.blockers.push(blocker(&child_key, &error.code, error.message, "Restore configured repository access or reconcile authorized child scope"));
                    }
                    if value.write_repository.as_ref().is_some_and(|id| {
                        id.parse::<u64>()
                            .ok()
                            .and_then(|id| facts.repositories.get(&id))
                            .is_none_or(|r| {
                                r.permissions
                                    .get("push")
                                    .and_then(serde_json::Value::as_bool)
                                    != Some(true)
                            })
                    }) {
                        child.blockers.push(blocker(&child_key, "access_denied", "Configured child write repository no longer grants push permission", "Restore actual repository write permission or reconcile the child scope"));
                    }
                    scope
                        .child_scopes
                        .insert(*child_id, canonical_digest(1, &value)?);
                    child.scope = Some(value);
                }
                Err(error) => child.blockers.push(blocker(
                    &child_key,
                    &error.code,
                    error.message,
                    "Prepare strict child responsibility, repository and write boundaries",
                )),
            }
            scope
                .child_definitions
                .insert(*child_id, definition(child_issue)?);
            match workspace
                .routing_labels
                .resolve(&child_issue.labels.iter().map(|l| l.id).collect())
            {
                Ok(profile) => child.profile = Some(profile),
                Err(error) => child.blockers.push(blocker(
                    &child_key,
                    &error.code,
                    error.message,
                    "Assign exactly one configured tool, model, reasoning and speed label",
                )),
            }
            validate_state(workspace, child_issue, &child_key, &mut child.blockers);
            dependency_ids.insert(*child_id, child.prerequisites.clone());
            parent.children.insert(*child_id, child);
        }
        validate_state(workspace, &issue, &parent_key, &mut parent.blockers);
        parent.prerequisites = prerequisites(*parent_id, &observation.dependencies)?;
        dependency_ids.insert(*parent_id, parent.prerequisites.clone());
        let mut relevant = parent.required_children.clone();
        relevant.insert(*parent_id);
        // Retain every transitive prerequisite in the in-memory detail view.
        let mut pending = relevant.clone();
        while let Some(id) = pending.pop_first() {
            if let Some(observed) = facts.issues.get(&id) {
                for dependency in prerequisites(id, &observed.dependencies)? {
                    if relevant.insert(dependency) {
                        pending.insert(dependency);
                    }
                }
                parent.observations.insert(id, observed.clone());
            }
        }
        for id in relevant {
            let affected = key(config, workspace, id)?;
            if let Some(observed) = facts.issues.get(&id) {
                // Comments on dependency work are context; only this parent and
                // its required children can supply its corrective instructions.
                if id == *parent_id || parent.required_children.contains(&id) {
                    for comment in &observed.comments {
                        let explicit =
                            prior.explicit_corrections.get(&comment.id).filter(|proof| {
                                proof.body_digest() == comment.body_digest
                                    && proof.record().parent == parent_key
                                    && workspace
                                        .actors
                                        .correctors
                                        .contains(&proof.record().linear_actor)
                            });
                        if explicit.is_none()
                            && (prior.host_comment_ids.contains(&comment.id)
                                || prior
                                    .authorization
                                    .as_ref()
                                    .is_some_and(|a| a.publication_id == comment.id)
                                || prior
                                    .definition_correction
                                    .as_ref()
                                    .is_some_and(|c| c.publication_id == comment.id))
                        {
                            continue;
                        }
                        if comment.bot || comment.delegated || comment.user_id.is_none() {
                            parent.blockers.push(blocker(&affected, "correction_provenance_unavailable", "Comment provenance is unknown, delegated or automated", "Reconcile host-owned comment IDs or restate corrective intent with a permitted native identity"));
                            continue;
                        }
                        parent.corrections.observe(
                            CommentRevision {
                                id: comment.id,
                                author: comment.user_id,
                                body_digest: comment.body_digest.clone(),
                                edited_at: comment.edited_at.or(comment.updated_at),
                                observed_revision: parent.observation_revision,
                            },
                            &workspace.actors.correctors,
                            &prior.host_comment_ids,
                            explicit.map(|proof| proof.record().linear_actor),
                        )?;
                    }
                }
                for dependency in prerequisites(id, &observed.dependencies)? {
                    if facts.missing.contains(&dependency) {
                        let mut reason = blocker(
                            &affected,
                            "missing_dependency",
                            "Required native Linear prerequisite is missing or inaccessible",
                            "Restore prerequisite access and verify its required outcome",
                        );
                        reason.prerequisite = Some(key(config, workspace, dependency)?);
                        parent.blockers.push(reason);
                    }
                }
            }
        }
        if !parent.corrections.pending_ids().is_empty() {
            parent.blockers.push(blocker(&parent_key, "correction_classification_pending", "Permitted comment revisions require current classification or correction processing", "Admit read-only coordination when runtime intent permits; preserve prior acceptance as history"));
        }
        let graph: BTreeMap<_, _> = parent
            .observations
            .iter()
            .filter(|(id, _)| {
                **id == *parent_id
                    || parent.required_children.contains(id)
                    || prior
                        .authorization
                        .as_ref()
                        .is_none_or(|a| !a.snapshot.dependency_generations.contains_key(id))
            })
            .map(|(id, o)| Ok((*id, prerequisites(*id, &o.dependencies)?)))
            .collect::<Result<_>>()?;
        if let Some(cycle) = dependency_cycle(&graph) {
            parent.blockers.push(blocker(
                &parent_key,
                "dependency_cycle",
                format!(
                    "Native blocking dependency cycle: {}",
                    cycle
                        .iter()
                        .map(Uuid::to_string)
                        .collect::<Vec<_>>()
                        .join(" -> ")
                ),
                "Resolve the native Linear dependency cycle",
            ));
        }
        if let (Some(meta), Ok(event)) = (&meta, authorization)
            && scope.child_scopes.keys().copied().collect::<BTreeSet<_>>()
                == parent.required_children
            && !parent.required_children.is_empty()
        {
            let generation = Generation {
                authorization: canonical_digest(1, &event)?,
                definition: canonical_digest(
                    1,
                    &serde_json::json!({"parent":scope.parent_definition,"children":scope.child_definitions}),
                )?,
                correction: parent.corrections.generation()?,
                scope: canonical_digest(1, &scope)?,
            };
            let dependency_generations = dependency_ids
                .values()
                .flatten()
                .copied()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .filter_map(|id| {
                    let pinned = prior
                        .authorization
                        .as_ref()
                        .and_then(|a| a.snapshot.dependency_generations.get(&id))
                        .or_else(|| prior.dependency_generations.get(&id));
                    pinned
                        .cloned()
                        .or_else(|| {
                            parent
                                .required_children
                                .contains(&id)
                                .then(|| generation.clone())
                        })
                        .map(|value| (id, value))
                })
                .collect::<BTreeMap<_, _>>();
            if dependency_generations
                .keys()
                .copied()
                .collect::<BTreeSet<_>>()
                != dependency_ids.values().flatten().copied().collect()
            {
                parent.blockers.push(blocker(&parent_key, "dependency_requirement_unresolved", "A prerequisite's required generation has not been reconstructed from authoritative records", "Resolve and publish the prerequisite generation required when authorizing this parent"));
            }
            let candidate = AuthorizationSnapshot {
                schema_version: 1,
                parent: parent_key.clone(),
                event: event.clone(),
                required_children: parent.required_children.clone(),
                scope: scope.clone(),
                dependency_ids,
                dependency_generations,
            };
            let matches = prior.authorization.as_ref().is_some_and(|old| {
                canonical_digest(1, &old.snapshot).ok() == canonical_digest(1, &candidate).ok()
            });
            if let Some(old) = &prior.authorization {
                if old.snapshot.parent != parent_key {
                    parent.blockers.push(blocker(
                        &parent_key,
                        "identity_mismatch",
                        "Retained authorization belongs to a different canonical parent",
                        "Restore matching authoritative authorization references",
                    ));
                }
                let changed_at = parent
                    .observations
                    .values()
                    .filter_map(|o| o.issue.updated_at)
                    .max()
                    .unwrap_or(now);
                let in_scope_correction =
                    prior
                        .definition_correction
                        .as_ref()
                        .is_some_and(|verified| {
                            let correction = &verified.record;
                            correction.parent == parent_key
                                && correction.authorization_digest == old.digest
                                && correction.before == old.snapshot.scope
                                && correction.after == scope
                                && correction.correction_generation == generation.correction
                                && parent
                                    .corrections
                                    .gates_current(correction.correction_watermark)
                                && correction.source_comments.iter().all(|(id, digest)| {
                                    parent.observations.values().flat_map(|o| &o.comments).any(
                                        |comment| {
                                            comment.id == *id
                                                && comment.body_digest == *digest
                                                && !comment.bot
                                                && !comment.delegated
                                                && comment.user_id.is_some_and(|actor| {
                                                    workspace.actors.correctors.contains(&actor)
                                                })
                                        },
                                    )
                                })
                        });
                if let Err(error) = authority::require_scope_authorization(
                    &old.snapshot.scope,
                    &scope,
                    &event,
                    changed_at,
                    in_scope_correction,
                ) {
                    parent.blockers.push(blocker(
                        &parent_key,
                        &error.code,
                        error.message,
                        "Renew parent authorization after reconciling changed definitions or scope",
                    ));
                }
                if old.snapshot.dependency_ids != candidate.dependency_ids
                    && event.occurred_at <= changed_at
                {
                    parent.blockers.push(blocker(
                        &parent_key,
                        "renewed_authorization_required",
                        "Declared prerequisite set changed without later authorization",
                        "Reconcile the required dependency outcome and renew parent authorization",
                    ));
                }
            } else {
                let definitions = parent
                    .observations
                    .iter()
                    .filter(|(id, _)| **id == *parent_id || parent.required_children.contains(id));
                let mut material = Vec::new();
                let mut unknown = false;
                for (_, observation) in definitions {
                    if let Some(created) = observation.issue.created_at {
                        material.push(created);
                    } else {
                        unknown = true;
                    }
                    for history in &observation.history {
                        if history.description_updated == Some(true)
                            || history.from_parent.is_some()
                            || history.to_parent.is_some()
                        {
                            if let Some(time) = history.created_at {
                                material.push(time);
                            } else {
                                unknown = true;
                            }
                        }
                    }
                }
                if unknown
                    || material
                        .into_iter()
                        .any(|change| change >= event.occurred_at)
                {
                    parent.blockers.push(blocker(&parent_key, "renewed_authorization_required", "Initial label authority does not demonstrably follow creation and material preparation of every required responsibility", "Apply authorization after preparing the complete current parent/child scope"));
                }
            }
            parent.authorization_published = matches;
            if !matches {
                parent.blockers.push(blocker(&parent_key, "authorization_unpublished", "Current authorization snapshot has not been verified on the parent", "Publish and reread the signed authorization snapshot through the host broker before admitting work"));
            }
            let work = WorkIdentity {
                linear_endpoint: parent_key.endpoint.clone(),
                linear_workspace: parent_key.workspace,
                parent: *parent_id,
                child: None,
                generation,
            };
            work.validate()?;
            for child in parent.children.values_mut() {
                if child.scope.is_some() {
                    let mut child_work = work.clone();
                    child_work.child = Some(child.key.issue);
                    child.work = Some(child_work);
                }
            }
            parent.work = Some(work);
            parent.authorization_candidate = Some(candidate);
            if !parent.required_children.contains(&meta.e2e_child) {
                parent.blockers.push(blocker(
                    &parent_key,
                    "incompletely_prepared",
                    "E2E child is outside required scope",
                    "Include the explicit E2E responsibility",
                ));
            }
        }
        parent.metadata = meta;
        let referenced_repositories: BTreeSet<_> = parent
            .children
            .values()
            .filter_map(|c| c.scope.as_ref())
            .flat_map(|s| s.write_repository.iter().chain(&s.read_repositories))
            .filter_map(|id| id.parse::<u64>().ok())
            .collect();
        let repository_facts: BTreeMap<_, _> = referenced_repositories
            .iter()
            .map(|id| (id, facts.repositories.get(id)))
            .collect();
        let own_observations: BTreeMap<_, _> = parent
            .observations
            .iter()
            .filter(|(id, _)| **id == *parent_id || parent.required_children.contains(id))
            .collect();
        parent.source_digest = canonical_digest(
            1,
            &serde_json::json!({"observations":own_observations,"required_children":parent.required_children,"hierarchy":facts.hierarchy.get(parent_id),"repositories":repository_facts}),
        )?;
        output.parents.insert(*parent_id, parent);
    }
    for (repository, prs) in facts.independent {
        for pr in prs {
            let review_identity = canonical_digest(
                1,
                &serde_json::json!({"repository":repository,"pr":pr.number,"head":pr.head_sha,"base":pr.base_sha}),
            )?;
            output.independent_jobs.push(IndependentJob {
                workspace_id: workspace.id,
                repository_id: repository,
                pull_request: pr,
                review_identity,
                phase: "candidate_requires_admission".into(),
            });
        }
    }
    Ok(output)
}

fn validate_state(
    workspace: &Workspace,
    issue: &Issue,
    key: &IssueKey,
    blockers: &mut Vec<Blocker>,
) {
    match workspace.linear.states.get(&issue.team_id.to_string()) {
        None => blockers.push(blocker(
            key,
            "incompletely_prepared",
            "Issue team has no explicit state mapping",
            "Configure stable Linear state IDs for the issue team",
        )),
        Some(mapping) if issue.state_id == mapping.cancelled => blockers.push(blocker(
            key,
            "cancelled",
            "Required work is explicitly cancelled and remains unresolved",
            "Reconcile the cancelled responsibility through renewed scope authorization",
        )),
        _ => {}
    }
}
fn missing_child(
    config: &Config,
    workspace: &Workspace,
    reference: &str,
    id: Uuid,
) -> Result<ChildProjection> {
    let key = key(config, workspace, id)?;
    Ok(ChildProjection {
        parent_reference: reference.into(),
        key: key.clone(),
        issue: None,
        scope: None,
        profile: None,
        work: None,
        blockers: vec![blocker(
            &key,
            "required_child_missing",
            "Required child is missing or inaccessible; it still counts toward completeness",
            "Restore access or renew authorization after explicit scope reconciliation",
        )],
        prerequisites: BTreeSet::new(),
    })
}
fn failed_workspace(
    config: &Config,
    name: &str,
    workspace: &Workspace,
    workflow: Option<&Workflow>,
    _revision: &str,
    prior: &BTreeMap<Uuid, ParentRefs>,
    error: Error,
) -> Result<WorkspaceProjection> {
    let facts = Collected {
        roots: prior.keys().copied().collect(),
        selected: BTreeSet::new(),
        hierarchy: BTreeMap::new(),
        issues: BTreeMap::new(),
        missing: prior.keys().copied().collect(),
        repositories: BTreeMap::new(),
        independent: BTreeMap::new(),
    };
    let fallback = Workflow::parse(
        "Unavailable workflow; no execution allowed",
        "/unavailable/WORKFLOW.md",
    )?;
    let mut output = project(
        config,
        name,
        workspace,
        workflow.unwrap_or(&fallback),
        prior,
        facts,
        Utc::now(),
    )?;
    output.complete = false;
    for parent in output.parents.values_mut() {
        parent.blockers.push(blocker(
            &parent.key,
            &error.code,
            &error.message,
            "Restore complete provider observations before any new admission or gated mutation",
        ));
    }
    output.error = Some(error);
    Ok(output)
}

/// Iterative DFS avoids stack overflow on long provider-supplied chains.
pub fn dependency_cycle(graph: &BTreeMap<Uuid, BTreeSet<Uuid>>) -> Option<Vec<Uuid>> {
    let mut done = BTreeSet::new();
    for start in graph.keys() {
        if done.contains(start) {
            continue;
        }
        let mut stack = vec![(*start, false)];
        let mut active = BTreeSet::new();
        let mut path = Vec::new();
        while let Some((node, exit)) = stack.pop() {
            if exit {
                active.remove(&node);
                done.insert(node);
                path.pop();
                continue;
            }
            if active.contains(&node) {
                let begin = path.iter().position(|id| *id == node).unwrap_or(0);
                let mut cycle = path[begin..].to_vec();
                cycle.push(node);
                return Some(cycle);
            }
            if done.contains(&node) {
                continue;
            }
            active.insert(node);
            path.push(node);
            stack.push((node, true));
            if let Some(deps) = graph.get(&node) {
                for dependency in deps.iter().rev() {
                    stack.push((*dependency, false));
                }
            }
        }
    }
    None
}

#[derive(Clone, Debug, Default)]
pub struct DependencyOutcomes {
    outcomes: BTreeMap<IssueKey, Vec<(Generation, String, bool)>>,
}
impl DependencyOutcomes {
    pub(crate) fn merge(&mut self, other: &Self) {
        for (work, entries) in &other.outcomes {
            let retained = self.outcomes.entry(work.clone()).or_default();
            for entry in entries {
                if !retained.contains(entry) {
                    retained.push(entry.clone());
                }
            }
        }
    }
    pub fn child(&mut self, verified: &VerifiedChild) -> Result<()> {
        let work = &verified.evidence().work;
        let child = work
            .child
            .ok_or_else(|| Error::invalid("child dependency evidence has no child identity"))?;
        self.outcomes
            .entry(IssueKey::new(
                &work.linear_endpoint,
                work.linear_workspace,
                child,
            )?)
            .or_default()
            .push((work.generation.clone(), verified.digest().into(), false));
        Ok(())
    }
    /// Only a complete immutable runtime seal can insert historical release
    /// credit. A raw attempt or terminal tracker state supplies no authority.
    pub(crate) fn released(
        &mut self,
        proof: &crate::runtime_delivery::VerifiedReleaseCompletion,
        config: &crate::config::Config,
    ) -> Result<()> {
        let success = proof.validate_for_configuration(config)?;
        let work = proof.work();
        let entries = self
            .outcomes
            .entry(IssueKey::new(
                &work.linear_endpoint,
                work.linear_workspace,
                success.parent_id,
            )?)
            .or_default();
        let outcome = (success.generation, success.outcome_digest, true);
        if !entries.contains(&outcome) {
            entries.push(outcome);
        }
        Ok(())
    }
    pub fn no_release_required(
        &mut self,
        verified: &crate::evidence::VerifiedNoReleaseRequired,
    ) -> Result<()> {
        let work = &verified.record().work;
        self.outcomes
            .entry(IssueKey::new(
                &work.linear_endpoint,
                work.linear_workspace,
                work.parent,
            )?)
            .or_default()
            .push((work.generation.clone(), verified.digest().into(), true));
        Ok(())
    }
    pub(crate) fn parent_outcome(&self, key: &IssueKey, generation: &Generation) -> Option<&str> {
        self.outcome(key, generation, true)
    }
    fn outcome(&self, key: &IssueKey, generation: &Generation, parent: bool) -> Option<&str> {
        self.outcomes
            .get(key)?
            .iter()
            .find(|(g, _, p)| g == generation && *p == parent)
            .map(|(_, digest, _)| digest.as_str())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GatePurpose {
    NewOperation,
    DrainExisting,
    Coordination,
}

/// A host-only preparation capability. It permits recording an already verified
/// human authorization; it cannot be used as a worker or delivery gate.
#[derive(Clone, Debug)]
pub struct AuthorizationGate {
    workspace_id: Uuid,
    snapshot: AuthorizationSnapshot,
    config_revision: String,
    workflow_digest: String,
    sources_digest: String,
    observed_at: DateTime<Utc>,
}
impl AuthorizationGate {
    pub fn workflow_digest(&self) -> &str {
        &self.workflow_digest
    }
    pub fn workspace_id(&self) -> Uuid {
        self.workspace_id
    }
    pub fn snapshot(&self) -> &AuthorizationSnapshot {
        &self.snapshot
    }
    pub fn config_revision(&self) -> &str {
        &self.config_revision
    }
    pub fn work_key(&self) -> Result<String> {
        self.snapshot.parent.identity()
    }
    pub fn digest(&self) -> Result<String> {
        canonical_digest(
            1,
            &serde_json::json!({"purpose":"authorization_preparation","workspace":self.workspace_id,"snapshot":self.snapshot,"configuration":self.config_revision,"workflow":self.workflow_digest,"sources":self.sources_digest}),
        )
    }
    pub fn compare_current(
        &self,
        current: &Self,
        now: DateTime<Utc>,
        max_age: Duration,
    ) -> Result<()> {
        fresh_time(current.observed_at, now, max_age)?;
        if self.digest()? != current.digest()? {
            return Err(Error::new(
                "stale_authorization",
                "authorization scope or its verified source changed before publication",
            ));
        }
        Ok(())
    }
}

impl ParentProjection {
    pub fn authorization_gate(
        &self,
        now: DateTime<Utc>,
        max_age: Duration,
    ) -> Result<AuthorizationGate> {
        fresh_time(self.observed_at, now, max_age)?;
        if self.authorization_published {
            return Err(Error::new(
                "already_published",
                "current authorization is already published",
            ));
        }
        // Publishing this record neither classifies a comment nor authorizes an
        // implementation session. All other preparation/authority blockers hold.
        if let Some(reason) = self.blockers.iter().find(|b| {
            !matches!(
                b.code.as_str(),
                "authorization_unpublished" | "correction_classification_pending"
            )
        }) {
            return Err(Error::new(&reason.code, &reason.reason));
        }
        if !self.included || self.required_children.is_empty() || self.work.is_none() {
            return Err(Error::new(
                "incompletely_prepared",
                "authorization publication requires complete included prepared work",
            ));
        }
        let snapshot = self.authorization_candidate.clone().ok_or_else(|| {
            Error::new(
                "authorization_provenance_unavailable",
                "verified human authorization is unavailable",
            )
        })?;
        if snapshot.parent != self.key || snapshot.required_children != self.required_children {
            return Err(Error::new(
                "scope_violation",
                "authorization candidate differs from current required work",
            ));
        }
        Ok(AuthorizationGate {
            workspace_id: self.workspace_id,
            snapshot,
            config_revision: self.config_revision.clone(),
            workflow_digest: self.workflow_digest.clone(),
            sources_digest: self.source_digest.clone(),
            observed_at: self.observed_at,
        })
    }
}

/// Opaque host observation. Serialize only digest-bound references into an
/// operation receipt; never deserialize this capability from a worker message.
#[derive(Clone, Debug, Serialize)]
pub struct FreshGate {
    workspace_id: Uuid,
    work: WorkIdentity,
    config_revision: String,
    drain_config_revision: String,
    workflow_digest: String,
    sources_digest: String,
    correction_watermark: u64,
    pending_correction_digest: String,
    profile_digest: String,
    profile: Profile,
    child_scope: Option<ChildScope>,
    required_children: BTreeSet<Uuid>,
    child_scope_digest: Option<String>,
    dependency_digest: String,
    purpose: GatePurpose,
    observed_at: DateTime<Utc>,
}
impl FreshGate {
    /// Recover historical provenance only from the host-authenticated pending
    /// outcome envelope. A current provider gate and live permit are still
    /// required before any effect. Worker JSON never deserializes this capability.
    pub fn recover_host_claim(
        key: &AttestationKey,
        signed: &Signed<serde_json::Value>,
        expected_digest: &str,
    ) -> Result<Self> {
        key.verify("pending_worker_claim", signed)?;
        let payload = signed.payload.as_object().ok_or_else(|| {
            Error::new(
                "invalid_attestation",
                "pending host claim envelope is malformed",
            )
        })?;
        if payload.len() != 3
            || payload.get("schema_version") != Some(&serde_json::json!(1))
            || !payload
                .get("outcome")
                .is_some_and(serde_json::Value::is_object)
        {
            return Err(Error::new(
                "invalid_attestation",
                "pending host claim envelope is not the exact versioned schema",
            ));
        }
        Self::recover_record(
            payload.get("authority").cloned().ok_or_else(|| {
                Error::new(
                    "invalid_attestation",
                    "pending host claim lacks authority provenance",
                )
            })?,
            expected_digest,
        )
    }
    pub(crate) fn recover_correction_record(
        key: &AttestationKey,
        signed: &Signed<serde_json::Value>,
        field: &str,
        expected_digest: &str,
    ) -> Result<Self> {
        key.verify("worker_correction_transition", signed)?;
        if !matches!(field, "original" | "current") {
            return Err(Error::new(
                "invalid_attestation",
                "unknown correction authority field",
            ));
        }
        Self::recover_record(signed.payload["transition"][field].clone(), expected_digest)
    }
    fn recover_record(value: serde_json::Value, expected_digest: &str) -> Result<Self> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct StoredGate {
            workspace_id: Uuid,
            work: WorkIdentity,
            config_revision: String,
            drain_config_revision: String,
            workflow_digest: String,
            sources_digest: String,
            correction_watermark: u64,
            pending_correction_digest: String,
            profile_digest: String,
            profile: Profile,
            child_scope: Option<ChildScope>,
            required_children: BTreeSet<Uuid>,
            child_scope_digest: Option<String>,
            dependency_digest: String,
            purpose: String,
            observed_at: DateTime<Utc>,
        }
        let stored: StoredGate = serde_json::from_value(value).map_err(|_| {
            Error::new(
                "invalid_attestation",
                "stored host authority schema is invalid",
            )
        })?;
        let purpose = match stored.purpose.as_str() {
            "new_operation" => GatePurpose::NewOperation,
            "drain_existing" => GatePurpose::DrainExisting,
            "coordination" => GatePurpose::Coordination,
            _ => {
                return Err(Error::new(
                    "invalid_attestation",
                    "stored host authority purpose is unknown",
                ));
            }
        };
        stored.work.validate()?;
        for digest in [
            &stored.config_revision,
            &stored.drain_config_revision,
            &stored.workflow_digest,
            &stored.sources_digest,
            &stored.profile_digest,
            &stored.pending_correction_digest,
            &stored.dependency_digest,
            expected_digest,
        ] {
            crate::evidence::validate_digest(digest)?;
        }
        if stored.workspace_id.is_nil()
            || stored.required_children.iter().any(Uuid::is_nil)
            || stored
                .work
                .child
                .is_some_and(|child| !stored.required_children.contains(&child))
            || canonical_digest(1, &stored.profile)? != stored.profile_digest
            || stored
                .child_scope
                .as_ref()
                .map(|scope| canonical_digest(1, scope))
                .transpose()?
                != stored.child_scope_digest
            || (stored.work.child.is_none() && stored.child_scope.is_some())
        {
            return Err(Error::new(
                "invalid_attestation",
                "stored host authority identities or embedded digests differ",
            ));
        }
        let recovered = Self {
            workspace_id: stored.workspace_id,
            work: stored.work,
            config_revision: stored.config_revision,
            drain_config_revision: stored.drain_config_revision,
            workflow_digest: stored.workflow_digest,
            sources_digest: stored.sources_digest,
            correction_watermark: stored.correction_watermark,
            pending_correction_digest: stored.pending_correction_digest,
            profile_digest: stored.profile_digest,
            profile: stored.profile,
            child_scope: stored.child_scope,
            required_children: stored.required_children,
            child_scope_digest: stored.child_scope_digest,
            dependency_digest: stored.dependency_digest,
            purpose,
            observed_at: stored.observed_at,
        };
        if recovered.digest()? != expected_digest {
            return Err(Error::new(
                "invalid_attestation",
                "stored host authority differs from immutable worker outcome digest",
            ));
        }
        Ok(recovered)
    }
    /// Canonical claim identity intentionally survives corrected generations.
    /// Exact generation/profile/scope remain bound separately in digest().
    pub fn work_key(&self) -> Result<String> {
        IssueKey::new(
            &self.work.linear_endpoint,
            self.work.linear_workspace,
            self.work.child.unwrap_or(self.work.parent),
        )?
        .identity()
    }
    pub fn purpose(&self) -> GatePurpose {
        self.purpose
    }
    pub fn workspace_id(&self) -> Uuid {
        self.workspace_id
    }
    pub fn work(&self) -> &WorkIdentity {
        &self.work
    }
    pub fn config_revision(&self) -> &str {
        &self.config_revision
    }
    pub fn workflow_digest(&self) -> &str {
        &self.workflow_digest
    }
    pub fn sources_digest(&self) -> &str {
        &self.sources_digest
    }
    pub fn correction_watermark(&self) -> u64 {
        self.correction_watermark
    }
    pub fn profile(&self) -> &Profile {
        &self.profile
    }
    pub fn child_scope(&self) -> Option<&ChildScope> {
        self.child_scope.as_ref()
    }
    pub fn required_children(&self) -> &BTreeSet<Uuid> {
        &self.required_children
    }
    pub fn dependency_digest(&self) -> &str {
        &self.dependency_digest
    }
    pub fn child_scope_digest(&self) -> Option<&str> {
        self.child_scope_digest.as_deref()
    }
    pub fn digest(&self) -> Result<String> {
        canonical_digest(
            1,
            &serde_json::json!({"workspace":self.workspace_id,"work":self.work,"config":self.config_revision,"workflow":self.workflow_digest,"sources":self.sources_digest,"watermark":self.correction_watermark,"pending_corrections":self.pending_correction_digest,"profile":self.profile_digest,"scope":self.child_scope_digest,"children":self.required_children,"dependencies":self.dependency_digest,"purpose":self.purpose}),
        )
    }
    pub fn compare_current(
        &self,
        current: &FreshGate,
        now: DateTime<Utc>,
        max_age: Duration,
    ) -> Result<()> {
        fresh_time(current.observed_at, now, max_age)?;
        self.compare_admitted_authority(current)
    }
    /// Compare immutable authority in retained proof/contract records. This
    /// supplies no current observation or admission permission; live callers
    /// must still use `compare_current` with freshly observed provider facts.
    pub(crate) fn compare_admitted_authority(&self, current: &FreshGate) -> Result<()> {
        let allowed_filter_drain = current.purpose == GatePurpose::DrainExisting
            && matches!(
                self.purpose,
                GatePurpose::NewOperation | GatePurpose::DrainExisting
            )
            && self.workspace_id == current.workspace_id
            && self.work == current.work
            && self.drain_config_revision == current.drain_config_revision
            && self.workflow_digest == current.workflow_digest
            && self.correction_watermark == current.correction_watermark
            && self.profile_digest == current.profile_digest
            && self.child_scope_digest == current.child_scope_digest
            && self.required_children == current.required_children
            && self.dependency_digest == current.dependency_digest;
        // The raw observation digest remains in the instruction/operation
        // provenance. Harmless provider timestamps and our own recorded comments
        // must not change the authorized work. Fresh gate construction already
        // rejects incomplete reads, revoked access and unclassified human input.
        let same_authority = self.workspace_id == current.workspace_id
            && self.work == current.work
            && self.config_revision == current.config_revision
            && self.workflow_digest == current.workflow_digest
            && self.correction_watermark == current.correction_watermark
            && self.profile_digest == current.profile_digest
            && self.child_scope_digest == current.child_scope_digest
            && self.required_children == current.required_children
            && self.dependency_digest == current.dependency_digest
            && self.purpose == current.purpose
            && (self.purpose != GatePurpose::Coordination
                || self.pending_correction_digest == current.pending_correction_digest);
        if !same_authority && !allowed_filter_drain {
            return Err(Error::new(
                "stale_result",
                "Fresh provider gate differs from the operation's exact work, authority, corrections, profile, scope or dependencies",
            ));
        }
        Ok(())
    }
    pub fn bind_permit(
        &self,
        permit: &crate::control::Permit,
        expected_scope_digest: &str,
    ) -> Result<()> {
        crate::evidence::validate_digest(expected_scope_digest)?;
        if permit.work_key != self.work_key()? || permit.scope_digest != expected_scope_digest {
            return Err(Error::new(
                "scope_violation",
                "Operation permit is not bound to the fresh canonical work and exact broker scope",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct IndependentReviewGate {
    workspace_id: Uuid,
    repository_id: u64,
    pull_request: PullRequest,
    review_identity: String,
    config_revision: String,
    workflow_digest: String,
    profile: Profile,
    repository_digest: String,
    observed_at: DateTime<Utc>,
}
impl IndependentReviewGate {
    pub fn workspace_id(&self) -> Uuid {
        self.workspace_id
    }
    pub fn repository_id(&self) -> u64 {
        self.repository_id
    }
    pub fn pull_request(&self) -> &PullRequest {
        &self.pull_request
    }
    pub fn review_identity(&self) -> &str {
        &self.review_identity
    }
    pub fn config_revision(&self) -> &str {
        &self.config_revision
    }
    pub fn workflow_digest(&self) -> &str {
        &self.workflow_digest
    }
    pub fn profile(&self) -> &Profile {
        &self.profile
    }
    pub fn digest(&self) -> Result<String> {
        canonical_digest(
            1,
            &serde_json::json!({"workspace":self.workspace_id,"repository":self.repository_id,"pr":self.pull_request,"review":self.review_identity,"config":self.config_revision,"workflow":self.workflow_digest,"profile":self.profile,"repository_digest":self.repository_digest}),
        )
    }
    pub fn compare_current(
        &self,
        current: &Self,
        now: DateTime<Utc>,
        max_age: Duration,
    ) -> Result<()> {
        fresh_time(current.observed_at, now, max_age)?;
        if self.digest()? != current.digest()? {
            return Err(Error::new(
                "stale_result",
                "Independent review no longer matches exact configured PR selection, head/base and permissions",
            ));
        }
        Ok(())
    }
}
fn fresh_time(observed: DateTime<Utc>, now: DateTime<Utc>, max_age: Duration) -> Result<()> {
    if max_age <= Duration::zero()
        || max_age > Duration::minutes(5)
        || observed > now + Duration::seconds(5)
        || now - observed > max_age
    {
        return Err(Error::new(
            "stale_result",
            "Authoritative observation is outside the bounded freshness interval",
        ));
    }
    Ok(())
}
impl ParentProjection {
    /// Dependency evidence is pinned to the required historical generation,
    /// never silently replaced by the prerequisite's newest generation.
    pub fn dependency_blockers(
        &self,
        child: Option<Uuid>,
        outcomes: &DependencyOutcomes,
    ) -> Result<Vec<Blocker>> {
        let ids = match child {
            Some(id) => {
                &self
                    .children
                    .get(&id)
                    .ok_or_else(|| {
                        Error::new("scope_violation", "child is not required by this parent")
                    })?
                    .prerequisites
            }
            None => &self.prerequisites,
        };
        let mut blockers = Vec::new();
        for id in ids {
            let dependency = IssueKey::new(&self.key.endpoint, self.key.workspace, *id)?;
            let expected = self
                .authorization_candidate
                .as_ref()
                .and_then(|a| a.dependency_generations.get(id));
            let parent_dependency = self
                .observations
                .get(id)
                .is_none_or(|o| o.issue.parent_id.is_none());
            if expected.is_none_or(|generation| {
                outcomes
                    .outcome(&dependency, generation, parent_dependency)
                    .is_none()
            }) {
                let mut reason = blocker(
                    &self.key,
                    "dependency_unsatisfied",
                    "Required prerequisite generation lacks verified completion or successful production outcome",
                    "Complete the required prerequisite outcome; Done state alone does not satisfy this gate",
                );
                reason.prerequisite = Some(dependency);
                blockers.push(reason);
            }
        }
        Ok(blockers)
    }
    pub fn fresh_gate(
        &self,
        child: Option<Uuid>,
        purpose: GatePurpose,
        outcomes: &DependencyOutcomes,
        now: DateTime<Utc>,
        max_age: Duration,
    ) -> Result<FreshGate> {
        fresh_time(self.observed_at, now, max_age)?;
        let ignored = |code: &str| {
            (purpose == GatePurpose::DrainExisting && code == "excluded")
                || (purpose == GatePurpose::Coordination
                    && code == "correction_classification_pending")
        };
        if let Some(reason) = self.blockers.iter().find(|b| !ignored(&b.code)) {
            return Err(Error::new(&reason.code, &reason.reason));
        }
        if !self.authorization_published {
            return Err(Error::new(
                "authorization_unpublished",
                "verified current authorization publication is required",
            ));
        }
        if purpose != GatePurpose::Coordination
            && !self.corrections.gates_current(self.corrections.watermark)
        {
            return Err(Error::new(
                "correction_classification_pending",
                "pending comment classification prevents gated effects",
            ));
        }
        let (work, profile, scope) = if let Some(id) = child {
            let child = self
                .children
                .get(&id)
                .ok_or_else(|| Error::new("scope_violation", "child is outside required scope"))?;
            if let Some(reason) = child.blockers.first() {
                return Err(Error::new(&reason.code, &reason.reason));
            }
            (
                child.work.as_ref(),
                child.profile.as_ref(),
                child
                    .scope
                    .as_ref()
                    .map(|s| canonical_digest(1, s))
                    .transpose()?,
            )
        } else {
            (self.work.as_ref(), Some(&self.coordination_profile), None)
        };
        let work = work.ok_or_else(|| {
            Error::new(
                "incompletely_prepared",
                "current work generation is unavailable",
            )
        })?;
        let profile =
            profile.ok_or_else(|| Error::invalid("current execution profile is unavailable"))?;
        let mut dependencies = BTreeMap::new();
        if purpose != GatePurpose::Coordination {
            if let Some(reason) = self.dependency_blockers(child, outcomes)?.first() {
                return Err(Error::new(&reason.code, &reason.reason));
            }
            let required = child
                .and_then(|id| self.children.get(&id))
                .map(|c| &c.prerequisites)
                .unwrap_or(&self.prerequisites);
            for id in required {
                let key = IssueKey::new(&self.key.endpoint, self.key.workspace, *id)?;
                let expected = &self
                    .authorization_candidate
                    .as_ref()
                    .ok_or_else(|| Error::invalid("authorization candidate missing"))?
                    .dependency_generations[id];
                let parent = self
                    .observations
                    .get(id)
                    .is_none_or(|o| o.issue.parent_id.is_none());
                dependencies.insert(
                    id,
                    outcomes.outcome(&key, expected, parent).ok_or_else(|| {
                        Error::new("dependency_unsatisfied", "dependency outcome disappeared")
                    })?,
                );
            }
        }
        Ok(FreshGate {
            workspace_id: self.workspace_id,
            work: work.clone(),
            config_revision: self.config_revision.clone(),
            drain_config_revision: self.drain_config_revision.clone(),
            workflow_digest: self.workflow_digest.clone(),
            sources_digest: self.source_digest.clone(),
            correction_watermark: self.corrections.watermark,
            pending_correction_digest: self.corrections.pending_revision_digest()?,
            profile_digest: canonical_digest(1, profile)?,
            profile: profile.clone(),
            child_scope: child
                .and_then(|id| self.children.get(&id))
                .and_then(|c| c.scope.clone()),
            required_children: self.required_children.clone(),
            child_scope_digest: scope,
            dependency_digest: canonical_digest(1, &dependencies)?,
            purpose,
            observed_at: self.observed_at,
        })
    }
}

/// Verified evidence belongs to host composition, never user/worker JSON.
#[derive(Clone, Debug, Default)]
pub struct ParentDelivery {
    children: BTreeMap<Uuid, VerifiedChild>,
    parent: Option<VerifiedParent>,
    acceptance: Option<VerifiedAcceptance>,
    release: Option<crate::release::VerifiedReleaseEligibility>,
    no_release: Option<crate::evidence::VerifiedNoReleaseRequired>,
}
impl ParentDelivery {
    pub fn verified_parent(&self) -> Option<&VerifiedParent> {
        self.parent.as_ref()
    }
    pub fn verified_acceptance(&self) -> Option<&VerifiedAcceptance> {
        self.acceptance.as_ref()
    }
    pub fn verified_children(&self) -> &BTreeMap<Uuid, VerifiedChild> {
        &self.children
    }
    pub fn release_eligibility(&self) -> Option<&crate::release::VerifiedReleaseEligibility> {
        self.release.as_ref()
    }
    pub fn no_release_required(&self) -> Option<&crate::evidence::VerifiedNoReleaseRequired> {
        self.no_release.as_ref()
    }
    pub(crate) fn record_no_release_required(
        &mut self,
        proof: crate::evidence::VerifiedNoReleaseRequired,
    ) -> Result<()> {
        let record = proof.record();
        if self.parent.as_ref().is_none_or(|parent| {
            parent.token() != record.evidence_token || parent.manifest().work != record.work
        }) || self.acceptance.as_ref().is_none_or(|acceptance| {
            acceptance.digest() != record.acceptance_digest
                || acceptance.acceptance().work != record.work
        }) {
            return Err(Error::new(
                "stale_result",
                "already-production outcome differs from current accepted evidence",
            ));
        }
        self.no_release = Some(proof);
        self.release = None;
        Ok(())
    }
    pub fn child(&mut self, child: VerifiedChild) -> Result<()> {
        let id = child
            .evidence()
            .work
            .child
            .ok_or_else(|| Error::invalid("verified child has no canonical child UUID"))?;
        if self.children.contains_key(&id) {
            return Err(Error::invalid("duplicate verified child outcome"));
        }
        self.children.insert(id, child);
        Ok(())
    }
    pub fn parent(&mut self, parent: VerifiedParent) {
        self.parent = Some(parent);
        self.acceptance = None;
        self.release = None;
        self.no_release = None;
    }
    pub fn acceptance(&mut self, acceptance: VerifiedAcceptance) -> Result<()> {
        let parent = self.parent.as_ref().ok_or_else(|| {
            Error::new(
                "evidence_missing",
                "current verified parent evidence is required",
            )
        })?;
        if acceptance.acceptance().work != parent.manifest().work
            || acceptance.acceptance().reviewed_token != parent.token()
        {
            return Err(Error::new(
                "stale_result",
                "acceptance names another work generation or evidence token",
            ));
        }
        self.acceptance = Some(acceptance);
        self.release = None;
        self.no_release = None;
        Ok(())
    }
    pub(crate) fn require_release_selection(
        &self,
        parent_token: &str,
        acceptance_digest: &str,
        accepted_at: DateTime<Utc>,
    ) -> Result<()> {
        if self
            .parent
            .as_ref()
            .is_none_or(|parent| parent.token() != parent_token)
            || self.acceptance.as_ref().is_none_or(|acceptance| {
                acceptance.digest() != acceptance_digest
                    || acceptance.acceptance().accepted_at != accepted_at
                    || acceptance.acceptance().reviewed_token != parent_token
            })
        {
            return Err(Error::new(
                "stale_result",
                "sorted release parent or exact acceptance changed before admission",
            ));
        }
        Ok(())
    }
    pub fn release(&mut self, proof: crate::release::VerifiedReleaseEligibility) -> Result<()> {
        if self
            .parent
            .as_ref()
            .is_none_or(|p| p.token() != proof.evidence_token())
            || self.acceptance.as_ref().is_none_or(|a| {
                a.digest() != proof.acceptance_digest()
                    || a.acceptance().reviewed_token != proof.evidence_token()
                    || self
                        .parent
                        .as_ref()
                        .is_none_or(|p| a.acceptance().work != p.manifest().work)
            })
        {
            return Err(Error::new(
                "stale_result",
                "release eligibility does not bind the accepted current evidence",
            ));
        }
        self.release = Some(proof);
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Activity {
    pub parent: IssueKey,
    pub child: Option<Uuid>,
    pub operation_id: Uuid,
    pub phase: String,
    pub queued: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct HistoryRow {
    pub parent_reference: String,
    pub parent: IssueKey,
    pub outcome: String,
    pub generation: Generation,
    pub completed_at: DateTime<Utc>,
    pub links: Vec<String>,
}
#[derive(Clone, Debug, Serialize)]
pub struct ChildRow {
    pub parent_reference: String,
    pub key: IssueKey,
    pub identifier: Option<String>,
    pub complete: bool,
    pub missing: bool,
    pub reasons: Vec<Blocker>,
}
#[derive(Clone, Debug, Serialize)]
pub struct ParentRow {
    pub workspace_id: Uuid,
    pub workspace_name: String,
    pub parent_reference: String,
    pub parent: IssueKey,
    pub identifier: Option<String>,
    pub title: Option<String>,
    pub url: Option<String>,
    pub required_children: usize,
    pub complete_children: usize,
    pub children: Vec<ChildRow>,
    pub phase: String,
    pub fresh: bool,
    pub observed_at: DateTime<Utc>,
    pub accepted: bool,
    pub evidence_token: Option<String>,
    pub release_mode: ReleaseMode,
    pub reasons: Vec<Blocker>,
    pub activities: Vec<Activity>,
    pub readiness_at: Option<DateTime<Utc>>,
    pub acceptance_at: Option<DateTime<Utc>>,
    pub priority: Option<i64>,
}
#[derive(Clone, Debug, Serialize)]
pub struct NextAction {
    pub action: String,
    pub parent_reference: Option<String>,
    pub reason: String,
    pub automatic: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct Overview {
    pub ready_for_release: Vec<ParentRow>,
    pub ready_for_review: Vec<ParentRow>,
    pub blocked: Vec<ParentRow>,
    pub running: Vec<ParentRow>,
    pub history: Vec<HistoryRow>,
    pub independent_jobs: Vec<IndependentJob>,
    pub runtime_health: Vec<serde_json::Value>,
    pub next_action: NextAction,
}
pub struct OverviewInputs<'a> {
    pub workspace: Option<Uuid>,
    pub delivery: &'a BTreeMap<IssueKey, ParentDelivery>,
    pub outcomes: &'a DependencyOutcomes,
    pub activities: &'a [Activity],
    pub history: &'a [HistoryRow],
    pub control: &'a crate::control::ControlSnapshot,
    pub now: DateTime<Utc>,
    pub max_age: Duration,
}
impl RuntimeProjection {
    pub fn independent_gate(
        &self,
        workspace_id: Uuid,
        repository_id: u64,
        pr_number: u64,
        now: DateTime<Utc>,
        max_age: Duration,
    ) -> Result<IndependentReviewGate> {
        let workspace = self
            .workspaces
            .get(&workspace_id)
            .ok_or_else(|| Error::new("not_found", "independent review workspace unavailable"))?;
        if !workspace.complete {
            return Err(Error::new(
                "incomplete_snapshot",
                "independent review requires complete provider reads",
            ));
        }
        fresh_time(workspace.observed_at, now, max_age)?;
        let job = workspace
            .independent_jobs
            .iter()
            .find(|job| job.repository_id == repository_id && job.pull_request.number == pr_number)
            .ok_or_else(|| {
                Error::new(
                    "excluded",
                    "PR is not a currently selected independent review candidate",
                )
            })?;
        let repository = workspace.repositories.get(&repository_id).ok_or_else(|| {
            Error::new(
                "access_denied",
                "independent review repository access is unavailable",
            )
        })?;
        let profile = workspace
            .independent_review_profile
            .clone()
            .ok_or_else(|| Error::invalid("independent review profile missing"))?;
        Ok(IndependentReviewGate {
            workspace_id,
            repository_id,
            pull_request: job.pull_request.clone(),
            review_identity: job.review_identity.clone(),
            config_revision: workspace.config_revision.clone(),
            workflow_digest: workspace.workflow_digest.clone(),
            profile,
            repository_digest: canonical_digest(1, repository)?,
            observed_at: workspace.observed_at,
        })
    }
    /// Install only small observation references. Existing verified publication
    /// references remain host-owned; refresh never forges a published record.
    pub fn retain_references(&self, retained: &mut ReconcileRefs) {
        for workspace in self.workspaces.values().filter(|w| w.complete) {
            for parent in workspace.parents.values() {
                let prior = retained
                    .entry(workspace.workspace_id)
                    .or_default()
                    .entry(parent.key.issue)
                    .or_default();
                prior.corrections = parent.corrections.clone();
                prior.required_child_ids = parent.required_children.clone();
                prior.observation_revision = parent.observation_revision;
                prior.disappeared_children = parent.disappeared_children.clone();
            }
        }
    }
    pub fn parent(&self, workspace: Uuid, parent: Uuid) -> Result<&ParentProjection> {
        self.workspaces
            .get(&workspace)
            .and_then(|w| w.parents.get(&parent))
            .ok_or_else(|| {
                Error::new(
                    "not_found",
                    "canonical parent is not present in the targeted runtime projection",
                )
            })
    }
    pub fn resolve_parent(&self, reference: &str) -> Result<(Uuid, IssueKey)> {
        if reference.trim().is_empty() {
            return Err(Error::invalid("parent reference cannot be empty"));
        }
        let pieces: Vec<_> = reference.split('/').collect();
        let (workspace, issue) = match pieces.as_slice() {
            [issue] => (None, *issue),
            [workspace, issue] if !workspace.is_empty() && !issue.is_empty() => {
                (Some(*workspace), *issue)
            }
            _ => {
                return Err(Error::invalid(
                    "parent reference must be workspace/UUID-or-key or a unique UUID-or-key",
                ));
            }
        };
        let mut candidates = Vec::new();
        for ws in self.workspaces.values().filter(|w| {
            workspace.is_none_or(|filter| {
                filter == w.workspace_id.to_string() || filter == w.workspace_name
            })
        }) {
            for parent in ws.parents.values() {
                if issue == parent.key.issue.to_string()
                    || parent.issue.as_ref().is_some_and(|p| p.identifier == issue)
                {
                    candidates.push((
                        ws.workspace_id,
                        parent.key.clone(),
                        parent.parent_reference.clone(),
                    ));
                }
            }
        }
        match candidates.as_slice() {
            [(workspace, key, _)] => Ok((*workspace, key.clone())),
            [] => Err(Error::new(
                "not_found",
                "parent reference does not resolve in configured runtime scope",
            )),
            _ => Err(Error::new(
                "ambiguous_reference",
                "parent reference matches multiple configured workspaces",
            )
            .detail(serde_json::json!({"candidates":
                candidates
                    .into_iter()
                    .map(|(_, _, reference)| reference)
                    .collect::<Vec<_>>()
            }))),
        }
    }
    /// Pure projection: never publishes, dispatches, accepts or releases work.
    pub fn overview(&self, input: OverviewInputs<'_>) -> Result<Overview> {
        if input
            .workspace
            .is_some_and(|id| !self.workspaces.contains_key(&id))
        {
            return Err(Error::new(
                "not_found",
                "workspace filter does not resolve in this runtime",
            ));
        }
        let mut result = Overview {
            ready_for_release: Vec::new(),
            ready_for_review: Vec::new(),
            blocked: Vec::new(),
            running: Vec::new(),
            history: Vec::new(),
            independent_jobs: Vec::new(),
            runtime_health: Vec::new(),
            next_action: NextAction {
                action: "none".into(),
                parent_reference: None,
                reason: "No actionable parent is currently observed".into(),
                automatic: false,
            },
        };
        let running_intent = input.control.intent.desired == crate::control::Intent::Running;
        for workspace in self
            .workspaces
            .values()
            .filter(|w| input.workspace.is_none_or(|id| id == w.workspace_id))
        {
            if let Some(error) = &workspace.error {
                result.runtime_health.push(serde_json::json!({"workspace_id":workspace.workspace_id,"healthy":false,"code":error.code,"reason":error.message}));
            }
            result
                .independent_jobs
                .extend(workspace.independent_jobs.clone());
            for parent in workspace.parents.values() {
                let proof = input.delivery.get(&parent.key);
                let verified = proof.and_then(|p| p.parent.as_ref()).filter(|p| {
                    Some(&p.manifest().work) == parent.work.as_ref()
                        && p.manifest().required_children == parent.required_children
                });
                let accepted = proof
                    .and_then(|p| p.acceptance.as_ref())
                    .filter(|a| verified.is_some_and(|p| a.acceptance().work == p.manifest().work));
                let mut reasons = parent.blockers.clone();
                reasons.extend(parent.dependency_blockers(None, input.outcomes)?);
                let mut children = Vec::new();
                for child in parent.children.values() {
                    let complete = proof
                        .and_then(|p| p.children.get(&child.key.issue))
                        .is_some_and(|v| Some(&v.evidence().work) == child.work.as_ref());
                    let mut child_reasons = child.blockers.clone();
                    child_reasons
                        .extend(parent.dependency_blockers(Some(child.key.issue), input.outcomes)?);
                    reasons.extend(child_reasons.clone());
                    children.push(ChildRow {
                        parent_reference: parent.parent_reference.clone(),
                        key: child.key.clone(),
                        identifier: child.issue.as_ref().map(|i| i.identifier.clone()),
                        complete,
                        missing: child.issue.is_none(),
                        reasons: child_reasons,
                    });
                }
                let complete_children = children.iter().filter(|c| c.complete).count();
                let fresh = workspace.complete
                    && fresh_time(parent.observed_at, input.now, input.max_age).is_ok();
                if !fresh {
                    reasons.push(blocker(
                        &parent.key,
                        "stale_observation",
                        "Provider projection is stale or incomplete",
                        "Complete a fresh authoritative reconciliation",
                    ));
                }
                let released = input.history.iter().any(|h| {
                    h.parent == parent.key
                        && h.parent_reference == parent.parent_reference
                        && parent.work.as_ref().is_some_and(|w| {
                            w.generation == h.generation
                                && input
                                    .outcomes
                                    .parent_outcome(&parent.key, &w.generation)
                                    .is_some()
                        })
                });
                if !running_intent && !released && (verified.is_none() || accepted.is_some()) {
                    reasons.push(blocker(
                        &parent.key,
                        "runtime_not_running",
                        "Runtime intent does not permit new execution or release",
                        "Resume or start the runtime when work should continue",
                    ));
                }
                let activities: Vec<_> = input
                    .activities
                    .iter()
                    .filter(|a| a.parent == parent.key)
                    .cloned()
                    .collect();
                if released && activities.is_empty() && reasons.is_empty() {
                    continue;
                }
                let mut row = ParentRow {
                    workspace_id: parent.workspace_id,
                    workspace_name: parent.workspace_name.clone(),
                    parent_reference: parent.parent_reference.clone(),
                    parent: parent.key.clone(),
                    identifier: parent.issue.as_ref().map(|i| i.identifier.clone()),
                    title: parent.issue.as_ref().map(|i| i.title.clone()),
                    url: parent.issue.as_ref().map(|i| i.url.clone()),
                    required_children: parent.required_children.len(),
                    complete_children,
                    children,
                    phase: "waiting".into(),
                    fresh,
                    observed_at: parent.observed_at,
                    accepted: accepted.is_some(),
                    evidence_token: verified.map(|p| p.token().into()),
                    release_mode: parent.release_mode,
                    reasons,
                    activities,
                    readiness_at: verified.map(|p| p.manifest().created_at),
                    acceptance_at: accepted.map(|a| a.acceptance().accepted_at),
                    priority: parent.issue.as_ref().and_then(|i| i.priority),
                };
                let active = row.activities.iter().any(|a| !a.queued);
                if active {
                    row.phase = row
                        .activities
                        .iter()
                        .filter(|a| !a.queued)
                        .map(|a| a.phase.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    result.running.push(row);
                    continue;
                }
                let release_ready = proof.and_then(|p| p.release.as_ref()).is_some_and(|r| {
                    verified.is_some_and(|p| p.token() == r.evidence_token())
                        && fresh_time(r.checked_at(), input.now, input.max_age).is_ok()
                });
                if row.reasons.is_empty() && accepted.is_some() && release_ready {
                    row.phase = if parent.release_mode == ReleaseMode::Manual {
                        "awaiting_manual_release"
                    } else {
                        "automatic_release_queued"
                    }
                    .into();
                    result.ready_for_release.push(row);
                } else if row.reasons.is_empty()
                    && verified.is_some()
                    && accepted.is_none()
                    && complete_children == parent.required_children.len()
                    && complete_children > 0
                {
                    row.phase = "awaiting_human_acceptance".into();
                    result.ready_for_review.push(row);
                } else if row.reasons.is_empty() && accepted.is_none() && verified.is_none() {
                    row.phase = if row.activities.is_empty() {
                        "eligible_work_queued"
                    } else {
                        "queued_for_execution_slot"
                    }
                    .into();
                    result.running.push(row);
                } else {
                    if row.reasons.is_empty() {
                        row.reasons.push(blocker(&parent.key, "release_validation_pending", "Accepted result requires current release eligibility verification", "Refresh production validation, policy, dependencies, holds and spacing"));
                    }
                    row.phase = "blocked_or_waiting".into();
                    result.blocked.push(row);
                }
            }
            result.history.extend(
                input
                    .history
                    .iter()
                    .filter(|h| {
                        workspace
                            .parents
                            .values()
                            .any(|p| p.key == h.parent && p.parent_reference == h.parent_reference)
                    })
                    .cloned(),
            );
        }
        for rows in [
            &mut result.ready_for_release,
            &mut result.ready_for_review,
            &mut result.blocked,
            &mut result.running,
        ] {
            rows.sort_by_key(|r| {
                (
                    r.acceptance_at.or(r.readiness_at).is_none(),
                    r.acceptance_at.or(r.readiness_at),
                    r.priority.filter(|p| *p > 0).unwrap_or(i64::MAX),
                    r.parent.clone(),
                    r.workspace_id,
                )
            });
        }
        result.runtime_health.push(serde_json::json!({"runtime_id":self.runtime_id,"intent":input.control.intent.desired,"control_generation":input.control.intent.generation,"reconciled":input.control.reconciled,"admission_open":input.control.admission_open,"persistence_error":input.control.persistence_error,"unresolved_operations":input.control.unresolved_operations}));
        result.next_action = next_action(&result);
        Ok(result)
    }
}
fn next_action(overview: &Overview) -> NextAction {
    if let Some(row) = overview
        .ready_for_release
        .iter()
        .find(|r| r.release_mode == ReleaseMode::Manual)
    {
        return NextAction {
            action: "release".into(),
            parent_reference: Some(row.parent_reference.clone()),
            reason: "Accepted current candidate is eligible for one explicit manual release".into(),
            automatic: false,
        };
    }
    if let Some(row) = overview.ready_for_review.first() {
        return NextAction {
            action: "review".into(),
            parent_reference: Some(row.parent_reference.clone()),
            reason: "Review current complete evidence and explicitly accept its exact token".into(),
            automatic: false,
        };
    }
    if let Some(start) = overview.blocked.iter().find(|r| r.accepted) {
        let mut current = start;
        let mut visited = BTreeSet::new();
        loop {
            if !visited.insert(current.parent.clone()) {
                return NextAction {
                    action: "resolve_dependency_cycle".into(),
                    parent_reference: Some(current.parent_reference.clone()),
                    reason:
                        "A dependency cycle prevents identifying a deliverable root prerequisite"
                            .into(),
                    automatic: false,
                };
            }
            if let Some(dependency) = current.reasons.iter().find_map(|r| r.prerequisite.as_ref()) {
                if let Some(prerequisite) =
                    overview.blocked.iter().find(|r| &r.parent == dependency)
                {
                    current = prerequisite;
                    continue;
                }
                return NextAction { action: "inspect_dependency".into(), parent_reference: Some(start.parent_reference.clone()), reason: "Root cause is unknown: the required prerequisite has no actionable blocked projection".into(), automatic: false };
            }
            if let Some(reason) = current.reasons.first() {
                return NextAction {
                    action: reason.next_action.clone(),
                    parent_reference: Some(current.parent_reference.clone()),
                    reason: reason.reason.clone(),
                    automatic: false,
                };
            }
            break;
        }
    }
    if let Some(row) = overview.ready_for_release.first() {
        return NextAction {
            action: "automatic_release_queued".into(),
            parent_reference: Some(row.parent_reference.clone()),
            reason: "Eligible automatic release is queued by the coordinator".into(),
            automatic: true,
        };
    }
    NextAction { action: "none".into(), parent_reference: None, reason: "No delivery-ready operator action is established; root cause is unknown for unfinished work".into(), automatic: false }
}

impl ParentProjection {
    /// A correction may replace work only through verified native routing. This
    /// is not a new admission gate and never changes retained execution choices.
    pub(crate) fn retain_corrected_gate(
        &self,
        admitted: &FreshGate,
        routes: &crate::corrections::VerifiedCorrectionRoutes,
        outcomes: &DependencyOutcomes,
        now: DateTime<Utc>,
        age: Duration,
    ) -> Result<FreshGate> {
        if admitted.work.child.is_none() || admitted.purpose == GatePurpose::Coordination {
            return Err(Error::new(
                "scope_violation",
                "incremental corrections require an admitted child",
            ));
        }
        let mut parent = self.clone();
        parent.blockers.retain(|blocker| blocker.code != "excluded");
        let child = parent
            .children
            .get_mut(&admitted.work.child.unwrap())
            .ok_or_else(|| Error::new("scope_violation", "corrected child disappeared"))?;
        child.profile = Some(admitted.profile.clone());
        child.blockers.retain(|blocker| {
            blocker.next_action
                != "Assign exactly one configured tool, model, reasoning and speed label"
        });
        let mut current = parent.fresh_gate(
            admitted.work.child,
            GatePurpose::DrainExisting,
            outcomes,
            now,
            age,
        )?;
        if current.workspace_id != admitted.workspace_id
            || current.child_scope_digest != admitted.child_scope_digest
            || current.required_children != admitted.required_children
            || current.dependency_digest != admitted.dependency_digest
        {
            return Err(Error::new(
                "scope_violation",
                "correction changed the admitted scope, child inventory or dependencies",
            ));
        }
        current.config_revision = admitted.config_revision.clone();
        current.drain_config_revision = admitted.drain_config_revision.clone();
        current.workflow_digest = admitted.workflow_digest.clone();
        current.profile = admitted.profile.clone();
        current.profile_digest = admitted.profile_digest.clone();
        current.purpose = admitted.purpose;
        routes.validate_successor(admitted, &current, &self.source_digest, now, age)?;
        Ok(current)
    }
    /// Used only by the active snapshot registry after current connection/path/
    /// repository/allowlist checks. It cannot create a new admission snapshot.
    pub(crate) fn retain_active_gate(
        &self,
        admitted: &FreshGate,
        outcomes: &DependencyOutcomes,
        now: DateTime<Utc>,
        age: Duration,
    ) -> Result<FreshGate> {
        let mut parent = self.clone();
        parent.blockers.retain(|b| b.code != "excluded");
        if let Some(id) = admitted.work.child {
            let child = parent
                .children
                .get_mut(&id)
                .ok_or_else(|| Error::new("scope_violation", "admitted child disappeared"))?;
            child.profile = Some(admitted.profile.clone());
            child.blockers.retain(|b| {
                b.next_action
                    != "Assign exactly one configured tool, model, reasoning and speed label"
            });
        }
        let purpose = if admitted.purpose == GatePurpose::Coordination {
            GatePurpose::Coordination
        } else {
            GatePurpose::DrainExisting
        };
        let mut current = parent.fresh_gate(admitted.work.child, purpose, outcomes, now, age)?;
        if current.workspace_id != admitted.workspace_id
            || current.work != admitted.work
            || current.correction_watermark != admitted.correction_watermark
            || current.child_scope_digest != admitted.child_scope_digest
            || current.required_children != admitted.required_children
            || current.dependency_digest != admitted.dependency_digest
        {
            return Err(Error::new(
                "stale_result",
                "active work authority, scope, correction or dependency changed",
            ));
        }
        current.config_revision = admitted.config_revision.clone();
        current.drain_config_revision = admitted.drain_config_revision.clone();
        current.workflow_digest = admitted.workflow_digest.clone();
        current.profile = admitted.profile.clone();
        current.profile_digest = admitted.profile_digest.clone();
        current.purpose = admitted.purpose;
        admitted.compare_current(&current, now, age)?;
        Ok(current)
    }
}
impl IndependentReviewGate {
    pub(crate) fn retain_active(
        &self,
        mut current: Self,
        now: DateTime<Utc>,
        age: Duration,
    ) -> Result<Self> {
        fresh_time(current.observed_at, now, age)?;
        if current.workspace_id != self.workspace_id
            || current.repository_id != self.repository_id
            || current.review_identity != self.review_identity
            || canonical_digest(1, &current.pull_request)?
                != canonical_digest(1, &self.pull_request)?
        {
            return Err(Error::new(
                "stale_result",
                "active independent review identity changed",
            ));
        }
        current.config_revision = self.config_revision.clone();
        current.workflow_digest = self.workflow_digest.clone();
        current.profile = self.profile.clone();
        current.repository_digest = self.repository_digest.clone();
        current.pull_request = self.pull_request.clone();
        self.compare_current(&current, now, age)?;
        Ok(current)
    }
}

#[cfg(test)]
mod historical_authority_tests {
    use super::*;

    #[tokio::test]
    async fn historical_compatibility_preserves_semantics_without_supplying_freshness() {
        let (_root, transition) = crate::corrections::test_transition_fixture().await;
        let original = transition.current_authority();
        let mut observed = original.clone();
        observed.sources_digest = crate::digest::bytes_digest(b"new raw native observation");
        observed.observed_at = Utc::now() - Duration::minutes(10);
        assert_ne!(original.digest().unwrap(), observed.digest().unwrap());
        original.compare_admitted_authority(&observed).unwrap();
        assert!(
            original
                .compare_current(&observed, Utc::now(), Duration::seconds(30))
                .is_err()
        );
        observed.observed_at = Utc::now();
        original
            .compare_current(&observed, Utc::now(), Duration::seconds(30))
            .unwrap();

        let mut profile = observed.clone();
        profile.profile.model = "another-model".into();
        profile.profile_digest = canonical_digest(1, &profile.profile).unwrap();
        let mut work = observed.clone();
        work.work.generation.definition = crate::digest::bytes_digest(b"new definition");
        let mut scope = observed.clone();
        scope
            .child_scope
            .as_mut()
            .unwrap()
            .write_paths
            .push("other/".into());
        scope.child_scope_digest =
            Some(canonical_digest(1, scope.child_scope.as_ref().unwrap()).unwrap());
        for changed in [profile, work, scope] {
            assert!(original.compare_admitted_authority(&changed).is_err());
            assert!(
                original
                    .compare_current(&changed, Utc::now(), Duration::seconds(30))
                    .is_err()
            );
        }
    }
}
