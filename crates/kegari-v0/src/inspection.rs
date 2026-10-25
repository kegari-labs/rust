//! Authenticated historical display is separate from current work resolution.
//! These rows never supply scheduling, acceptance, release or dependency proof.
use crate::{
    app::RuntimeInspection,
    authority::IssueKey,
    config::Config,
    control::ControlSnapshot,
    error::{Error, Result},
    evidence::WorkIdentity,
    reconcile::{HistoryRow, Overview, OverviewInputs, RuntimeProjection},
    release::ReleaseAttempt,
};
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

#[derive(Clone, Default, Serialize)]
pub struct ParentDisplay {
    pub workspace_name: Option<String>,
    pub identifier: Option<String>,
    pub title: Option<String>,
    pub url: Option<String>,
}

/// Supplied only after the release reader validates the actual terminal
/// completion for this exact work. It remains display data, not a capability.
#[derive(Clone, Serialize)]
pub struct ReleaseCompletionInspection {
    pub operation: Uuid,
    pub work: WorkIdentity,
    pub completed_at: DateTime<Utc>,
}

#[derive(Clone, Serialize)]
pub struct ReleaseInspectionRow {
    pub workspace: Uuid,
    pub parent: IssueKey,
    pub work: WorkIdentity,
    pub attempt: ReleaseAttempt,
    pub active_hold: bool,
    /// A descendant may consume an old resource hold without making that old
    /// attempt a successful release of its generation.
    pub completed_by: Option<Uuid>,
    pub completion: Option<ReleaseCompletionInspection>,
    pub diagnostic: Option<Error>,
    pub display: ParentDisplay,
}

impl ReleaseInspectionRow {
    pub fn parent_reference(&self) -> String {
        format!("{}/{}", self.workspace, self.parent.issue)
    }
}

/// Check presentation identity, not release authority. The producer must have
/// authenticated records and independently validated any completion marker.
pub(crate) fn validate_rows(runtime: Uuid, rows: &[ReleaseInspectionRow]) -> Result<()> {
    if rows.len() > 65_536 {
        return Err(Error::new(
            "record_limit",
            "release inspection exceeds its finite inventory",
        ));
    }
    let mut operations = BTreeSet::new();
    for row in rows {
        let plan = &row.attempt.plan;
        let parent = IssueKey::new(
            &row.work.linear_endpoint,
            row.work.linear_workspace,
            row.work.parent,
        )?;
        if row.workspace.is_nil()
            || row.parent != parent
            || row.work.child.is_some()
            || plan.runtime_id != runtime
            || plan.parent_id != row.parent.issue
            || plan.linear_workspace != row.parent.workspace
            || plan.generation != row.work.generation
            || !operations.insert(plan.operation_id)
            || row.completed_by.is_some_and(|operation| operation.is_nil())
            || row.completion.as_ref().is_some_and(|completion| {
                completion.operation != plan.operation_id
                    || completion.work != row.work
                    || row.diagnostic.is_some()
                    || row.active_hold
            })
        {
            return Err(Error::new(
                "record_corrupt",
                "historical inspection identity is inconsistent",
            ));
        }
    }
    Ok(())
}

pub(crate) fn sort_rows(rows: &mut [ReleaseInspectionRow]) {
    rows.sort_by(|a, b| {
        (a.workspace, &a.parent, a.attempt.plan.operation_id).cmp(&(
            b.workspace,
            &b.parent,
            b.attempt.plan.operation_id,
        ))
    });
}

/// Historical workspace names are not aliases for current configuration.
/// Removed workspaces can be selected only by their authenticated UUID.
pub(crate) fn resolve_workspace(
    config: &Config,
    rows: &[ReleaseInspectionRow],
    reference: Option<&str>,
) -> Result<Option<Uuid>> {
    let Some(reference) = reference else {
        return Ok(None);
    };
    let mut matches = config
        .workspaces
        .iter()
        .filter(|(name, workspace)| {
            reference == name.as_str() || reference == workspace.id.to_string()
        })
        .map(|(_, workspace)| workspace.id)
        .collect::<BTreeSet<_>>();
    if let Ok(id) = reference.parse::<Uuid>()
        && rows.iter().any(|row| row.workspace == id)
    {
        matches.insert(id);
    }
    match matches.iter().copied().collect::<Vec<_>>().as_slice() {
        [workspace] => Ok(Some(*workspace)),
        [] => Err(Error::new(
            "unknown_resource",
            "workspace is neither configured nor present in authenticated release history",
        )),
        _ => Err(Error::new(
            "ambiguous_reference",
            "inspection workspace reference is ambiguous",
        )),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct InspectionParent {
    pub workspace: Uuid,
    pub key: IssueKey,
}
impl InspectionParent {
    pub fn reference(&self) -> String {
        format!("{}/{}", self.workspace, self.key.issue)
    }
}

pub(crate) fn resolve_parent(
    config: &Config,
    projection: &RuntimeProjection,
    rows: &[ReleaseInspectionRow],
    reference: &str,
    workspace: Option<Uuid>,
) -> Result<InspectionParent> {
    let parts = reference.split('/').collect::<Vec<_>>();
    let (embedded, issue) = match parts.as_slice() {
        [issue] if !issue.trim().is_empty() => (None, *issue),
        [workspace, issue] if !workspace.is_empty() && !issue.trim().is_empty() => {
            (resolve_workspace(config, rows, Some(workspace))?, *issue)
        }
        _ => {
            return Err(Error::invalid(
                "parent reference must be workspace/UUID-or-key or a unique UUID-or-key",
            ));
        }
    };
    if embedded.zip(workspace).is_some_and(|(a, b)| a != b) {
        return Err(Error::new(
            "unknown_resource",
            "parent is outside the inspection workspace",
        ));
    }
    let selected = embedded.or(workspace);
    let mut candidates = BTreeSet::new();
    for current in projection
        .workspaces
        .values()
        .filter(|current| selected.is_none_or(|id| current.workspace_id == id))
    {
        for parent in current.parents.values() {
            if issue == parent.key.issue.to_string()
                || parent
                    .issue
                    .as_ref()
                    .is_some_and(|value| value.identifier == issue)
            {
                candidates.insert(InspectionParent {
                    workspace: current.workspace_id,
                    key: parent.key.clone(),
                });
            }
        }
    }
    for row in rows
        .iter()
        .filter(|row| selected.is_none_or(|id| row.workspace == id))
    {
        if issue == row.parent.issue.to_string() || row.display.identifier.as_deref() == Some(issue)
        {
            candidates.insert(InspectionParent {
                workspace: row.workspace,
                key: row.parent.clone(),
            });
        }
    }
    match candidates.into_iter().collect::<Vec<_>>().as_slice() {
        [parent] => Ok(parent.clone()),
        [] => Err(Error::new("not_found", "parent is absent from current observations and authenticated release history")),
        candidates => Err(Error::new("ambiguous_reference", "parent matches multiple inspection identities")
            .detail(serde_json::json!({"candidates": candidates.iter().map(|parent|
                serde_json::json!({"reference":parent.reference(),"workspace":parent.workspace,"parent":parent.key})
            ).collect::<Vec<_>>()}))),
    }
}

pub(crate) fn parent_rows<'a>(
    rows: &'a [ReleaseInspectionRow],
    parent: &InspectionParent,
) -> Vec<&'a ReleaseInspectionRow> {
    rows.iter()
        .filter(|row| row.workspace == parent.workspace && row.parent == parent.key)
        .collect()
}

pub(crate) fn history_for_parent<'a>(
    history: &'a [HistoryRow],
    workspace: Uuid,
    parent: &IssueKey,
) -> Vec<&'a HistoryRow> {
    let reference = format!("{workspace}/{}", parent.issue);
    history
        .iter()
        .filter(|row| row.parent == *parent && row.parent_reference == reference)
        .collect()
}

pub(crate) fn observed_url(
    current: Option<&crate::reconcile::ParentProjection>,
    rows: &[&ReleaseInspectionRow],
) -> Result<String> {
    let urls = if let Some(issue) = current.and_then(|parent| parent.issue.as_ref()) {
        [issue.url.clone()].into_iter().collect::<BTreeSet<_>>()
    } else {
        rows.iter()
            .filter_map(|row| row.display.url.clone())
            .collect()
    };
    let url = match urls.iter().collect::<Vec<_>>().as_slice() {
        [url] => (*url).clone(),
        [] => {
            return Err(Error::new(
                "unknown_resource",
                "no observed parent URL is retained",
            ));
        }
        _ => {
            return Err(Error::new(
                "ambiguous_reference",
                "retained parent URLs disagree; a current observation is required",
            ));
        }
    };
    let parsed = url::Url::parse(&url)
        .map_err(|_| Error::new("unsafe_url", "observed parent URL is malformed"))?;
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err(Error::new(
            "unsafe_url",
            "observed parent URL must be credential-free HTTPS",
        ));
    }
    Ok(url)
}

#[derive(Serialize)]
pub(crate) struct InspectionOverview {
    #[serde(flatten)]
    current: Overview,
    release_history: Vec<ReleaseInspectionRow>,
}

pub(crate) fn overview(
    view: &RuntimeInspection,
    workspace: Option<Uuid>,
    control: &ControlSnapshot,
    now: DateTime<Utc>,
    max_age: Duration,
) -> Result<InspectionOverview> {
    // A removed workspace has no current readiness rows. Filtering a local
    // presentation copy never inserts retained history into scheduling state.
    let mut projection = view.projection.clone();
    projection
        .workspaces
        .retain(|id, _| workspace.is_none_or(|selected| *id == selected));
    let release_history = view
        .release_history
        .iter()
        .filter(|row| workspace.is_none_or(|id| row.workspace == id))
        .cloned()
        .collect::<Vec<_>>();
    let mut completions = BTreeMap::new();
    for row in &release_history {
        if let Some(completion) = &row.completion {
            completions
                .entry((
                    row.workspace,
                    row.parent.clone(),
                    row.attempt.plan.operation_id,
                ))
                .or_insert_with(|| HistoryRow {
                    parent_reference: row.parent_reference(),
                    parent: row.parent.clone(),
                    outcome: "released".into(),
                    generation: completion.work.generation.clone(),
                    completed_at: completion.completed_at,
                    links: observed_url(None, &[row]).ok().into_iter().collect(),
                });
        }
    }
    let current_parents = projection
        .workspaces
        .values()
        .flat_map(|workspace| {
            workspace.parents.values().map(|parent| {
                (
                    format!("{}/{}", workspace.workspace_id, parent.key.issue),
                    parent.key.clone(),
                )
            })
        })
        .collect::<BTreeSet<_>>();
    let mut history = view
        .history
        .iter()
        .filter(|row| current_parents.contains(&(row.parent_reference.clone(), row.parent.clone())))
        .cloned()
        .collect::<Vec<_>>();
    history.extend(completions.values().cloned());
    deduplicate_history(&mut history)?;
    // Current-row suppression still requires the independent dependency outcome
    // and exact generation checked by overview. Display facts add no outcome.
    let mut current = projection.overview(OverviewInputs {
        workspace: None,
        delivery: &view.delivery,
        outcomes: &view.outcomes,
        activities: &view.activities,
        history: &history,
        control,
        now,
        max_age,
    })?;
    current.history.extend(
        completions
            .into_iter()
            .filter_map(|((workspace, parent, _), row)| {
                let present = projection
                    .workspaces
                    .get(&workspace)
                    .is_some_and(|workspace| {
                        workspace
                            .parents
                            .values()
                            .any(|current| current.key == parent)
                    });
                (!present).then_some(row)
            }),
    );
    deduplicate_history(&mut current.history)?;
    current.history.sort_by(|a, b| {
        b.completed_at
            .cmp(&a.completed_at)
            .then_with(|| a.parent_reference.cmp(&b.parent_reference))
            .then_with(|| a.parent.cmp(&b.parent))
    });
    Ok(InspectionOverview {
        current,
        release_history,
    })
}

fn deduplicate_history(history: &mut Vec<HistoryRow>) -> Result<()> {
    let mut seen = BTreeSet::new();
    let mut unique = Vec::with_capacity(history.len());
    for row in history.drain(..) {
        if seen.insert(crate::digest::canonical_digest(1, &row)?) {
            unique.push(row);
        }
    }
    *history = unique;
    Ok(())
}

#[cfg(test)]
#[path = "inspection_tests.rs"]
mod tests;
