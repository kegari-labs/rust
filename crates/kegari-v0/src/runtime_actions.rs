//! Select real finite host actions from current authority and evidence.
use crate::{
    authority::IssueKey,
    decisions::VerifiedRetryGrant,
    reconcile::{
        AuthorizationGate, DependencyOutcomes, GatePurpose, ParentDelivery, RuntimeProjection,
    },
    runtime_schedule::RunSchedule,
    scheduler::{ReadyWork, WorkKind},
    worker::WorkerAuthority,
};
use chrono::{Duration, Utc};
use kegari_config::Config;
use kegari_digest::canonical_digest;
use kegari_types::error::{Error, Result};
use kegari_types::run::PhaseKey;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

#[derive(Clone, Debug)]
pub enum RuntimeAction {
    Authorize(Box<AuthorizationGate>),
    Worker(Box<WorkerAction>),
}
#[derive(Clone, Debug)]
pub struct WorkerAction {
    pub authority: WorkerAuthority,
    pub phase: PhaseKey,
    pub schedule: RunSchedule,
    pub ready: ReadyWork,
}
pub(crate) struct Selection {
    pub actions: Vec<RuntimeAction>,
    pub blockers: BTreeMap<String, Error>,
}
pub(crate) struct SelectionInput<'a> {
    pub config: &'a Config,
    pub projection: &'a RuntimeProjection,
    pub outcomes: &'a DependencyOutcomes,
    pub delivery: &'a BTreeMap<IssueKey, ParentDelivery>,
    pub grants: &'a BTreeMap<Uuid, VerifiedRetryGrant>,
}
pub(crate) fn select(
    input: SelectionInput<'_>,
    plan: impl Fn(&PhaseKey, Uuid, &str) -> Result<RunSchedule>,
) -> Result<Selection> {
    let SelectionInput {
        config,
        projection,
        outcomes,
        delivery,
        grants,
    } = input;
    if projection.runtime_id != config.runtime.id
        || projection.config_revision != crate::config::config_revision(config)?
    {
        return Err(Error::new(
            "stale_config",
            "action selection requires the current configuration projection",
        ));
    }
    let mut selected = Selection {
        actions: Vec::new(),
        blockers: BTreeMap::new(),
    };
    let now = Utc::now();
    for workspace in projection.workspaces.values() {
        if !workspace.complete {
            selected.blockers.insert(
                workspace.workspace_id.to_string(),
                workspace.error.clone().unwrap_or_else(|| {
                    Error::new(
                        "incomplete_snapshot",
                        "workspace provider reads are incomplete",
                    )
                }),
            );
            continue;
        }
        let configured = config
            .workspaces
            .values()
            .find(|w| w.id == workspace.workspace_id)
            .ok_or_else(|| {
                Error::new("stale_config", "observed workspace is no longer configured")
            })?;
        // Declared evaluation subjects require their actual explicit local task.
        if configured.evaluation.is_some() {
            continue;
        }
        let account = &config.connections[&configured.codex_connection].account_id;
        for parent in workspace.parents.values() {
            let parent_key = parent.key.identity()?;
            if !parent.authorization_published {
                match parent.authorization_gate(now, Duration::seconds(30)) {
                    Ok(gate) => selected
                        .actions
                        .push(RuntimeAction::Authorize(Box::new(gate))),
                    Err(error) => {
                        selected.blockers.insert(parent_key, error);
                    }
                }
                continue;
            }
            if !parent.corrections.pending_ids().is_empty() {
                let correction_digest = parent.corrections.pending_revision_digest()?;
                let suffix = correction_digest
                    .rsplit(':')
                    .next()
                    .ok_or_else(|| Error::invalid("pending revision digest is invalid"))?;
                crate::evidence::validate_digest(&correction_digest)?;
                let phase = format!("correction_classification_{}", &suffix[..32]);
                let result = parent
                    .fresh_gate(
                        None,
                        GatePurpose::Coordination,
                        outcomes,
                        now,
                        Duration::seconds(30),
                    )
                    .and_then(|gate| {
                        let issue = parent.issue.as_ref().ok_or_else(|| {
                            Error::new("incomplete_snapshot", "coordination issue is missing")
                        })?;
                        worker_action(
                            WorkerAuthority::Authorized(gate),
                            &phase,
                            WorkerOrder::issue(WorkKind::Coordination, issue),
                            account,
                            grants,
                            &plan,
                        )
                    });
                selected.record(parent_key.clone(), result);
            }
            for child in parent.children.values() {
                let key = child.key.identity()?;
                if child.work.as_ref().is_some_and(|work| {
                    delivery
                        .get(&parent.key)
                        .and_then(|p| p.verified_children().get(&child.key.issue))
                        .is_some_and(|proof| &proof.evidence().work == work)
                }) {
                    continue;
                }
                let result = parent
                    .fresh_gate(
                        Some(child.key.issue),
                        GatePurpose::NewOperation,
                        outcomes,
                        now,
                        Duration::seconds(30),
                    )
                    .and_then(|gate| {
                        let issue = child.issue.as_ref().ok_or_else(|| {
                            Error::new("incomplete_snapshot", "child issue is missing")
                        })?;
                        worker_action(
                            WorkerAuthority::Authorized(gate),
                            "implementation",
                            WorkerOrder::issue(WorkKind::Child, issue),
                            account,
                            grants,
                            &plan,
                        )
                    });
                selected.record(key, result);
            }
        }
        for job in &workspace.independent_jobs {
            let authority = projection.independent_gate(
                workspace.workspace_id,
                job.repository_id,
                job.pull_request.number,
                now,
                Duration::seconds(30),
            );
            match authority {
                Ok(gate) => {
                    let authority = WorkerAuthority::Independent(gate);
                    let key = authority.work_key()?;
                    let result = worker_action(
                        authority,
                        &job.phase,
                        WorkerOrder {
                            kind: WorkKind::IndependentReview,
                            state: "independent_review".into(),
                            priority: None,
                            created_at: None,
                        },
                        account,
                        grants,
                        &plan,
                    );
                    selected.record(key, result);
                }
                Err(error) => {
                    selected.blockers.insert(
                        format!(
                            "{}:{}:{}",
                            workspace.workspace_id, job.repository_id, job.pull_request.number
                        ),
                        error,
                    );
                }
            }
        }
    }
    // Detect cross-workspace claim collisions before any launch task starts.
    let mut claims = BTreeMap::new();
    for action in &selected.actions {
        let (key, workspace) = match action {
            RuntimeAction::Authorize(gate) => (gate.work_key()?, gate.workspace_id()),
            RuntimeAction::Worker(worker) => {
                (worker.ready.canonical_key.clone(), worker.ready.workspace)
            }
        };
        if claims.insert(key, workspace).is_some() {
            return Err(Error::new(
                "ownership_conflict",
                "action selection produced duplicate canonical work",
            ));
        }
    }
    Ok(selected)
}
impl Selection {
    fn record(&mut self, key: String, result: Result<WorkerAction>) {
        match result {
            Ok(action) => self.actions.push(RuntimeAction::Worker(Box::new(action))),
            Err(error) => {
                self.blockers.insert(key, error);
            }
        }
    }
}

struct WorkerOrder {
    kind: WorkKind,
    state: String,
    priority: Option<i32>,
    created_at: Option<chrono::DateTime<Utc>>,
}
impl WorkerOrder {
    fn issue(kind: WorkKind, issue: &kegari_providers::Issue) -> Self {
        Self {
            kind,
            state: issue.state_name.trim().to_lowercase(),
            priority: issue.priority.and_then(|n| i32::try_from(n).ok()),
            created_at: issue.created_at,
        }
    }
}
fn worker_action(
    authority: WorkerAuthority,
    phase_name: &str,
    order: WorkerOrder,
    account: &str,
    grants: &BTreeMap<Uuid, VerifiedRetryGrant>,
    plan: &impl Fn(&PhaseKey, Uuid, &str) -> Result<RunSchedule>,
) -> Result<WorkerAction> {
    let (workspace, generation) = match &authority {
        WorkerAuthority::Authorized(gate) => (
            gate.workspace_id(),
            canonical_digest(1, &gate.work().generation)?,
        ),
        WorkerAuthority::Independent(gate) => (gate.workspace_id(), gate.review_identity().into()),
    };
    let mut phase = PhaseKey {
        work: authority.work_key()?,
        generation,
        phase: phase_name.into(),
        retry_grant: None,
    };
    let mut seen = BTreeSet::new();
    loop {
        let next = grants
            .values()
            .filter(|g| g.phase() == &phase)
            .collect::<Vec<_>>();
        match next.as_slice() {
            [] => break,
            [grant] if seen.insert(grant.record().operation_id) => {
                if grant.record().workspace_id != workspace
                    || authority.work() != Some(grant.work())
                {
                    return Err(Error::new(
                        "scope_violation",
                        "retry publication differs from current selected authority",
                    ));
                }
                phase = grant.phase_key();
            }
            _ => {
                return Err(Error::new(
                    "ownership_conflict",
                    "retry publications contain a cycle or competing successors",
                ));
            }
        }
    }
    let schedule = plan(&phase, workspace, account)?;
    let ready = ReadyWork {
        canonical_key: phase.work.clone(),
        workspace,
        account: account.into(),
        generation: phase.generation.clone(),
        state: order.state,
        priority: order.priority,
        created_at: order.created_at,
        kind: order.kind,
        trigger: Some(schedule.trigger.clone()),
        not_before: schedule.not_before,
        gate_blockers: vec![],
    };
    Ok(WorkerAction {
        authority,
        phase,
        schedule,
        ready,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        authority::{ChildScope, CommentRevision, Corrections, Responsibility},
        evidence::{Generation, WorkIdentity},
        reconcile::{ChildProjection, ParentProjection, WorkspaceProjection},
    };
    use kegari_config::{Profile, ReleaseMode, Speed};
    use kegari_digest::bytes_digest;
    use kegari_providers::Issue;
    use serde_json::json;
    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }
    fn fixture() -> (Config, RuntimeProjection) {
        let profile = Profile {
            tool: "codex".into(),
            model: "fixture-model".into(),
            reasoning: "high".into(),
            speed: Speed::Standard,
        };
        let config:Config=serde_json::from_value(json!({"schema_version":1,"runtime":{"id":id(1),"host_id":"fixture","max_agents":2},
            "connections":{"linear":{"kind":"linear","account_id":id(4),"endpoint":"https://linear.invalid/graphql","credential_ref":"keychain:linear"},
                "github":{"kind":"github","account_id":"7","endpoint":"https://github.invalid/","credential_ref":"keychain:github"},
                "codex":{"kind":"codex","account_id":"fixture-account","auth_home":"/private/tmp/fixture-auth","max_agents":2}},
            "workspaces":{"fixture":{"id":id(3),"linear_workspace_id":id(2),"linear_connection":"linear","github_connection":"github","codex_connection":"codex", "workflow":"/private/tmp/WORKFLOW.md",
                "linear":{"teams":"all","projects":"all","users":"all","authorization_label_id":id(5),"states":{}},"github":{"organizations":[],"repositories":[]},
                "actors":{"authorizers":[id(4)],"correctors":[id(4)],"acceptors":[id(4)]},"profiles":{"coordination":profile},
                "routing_labels":{"tool":{},"model":{},"reasoning":{},"speed":{}},"max_agents":2,"release":{"mode":"manual","spacing_seconds":0},"repositories":{}}},
            "control":{"local_users":{"501":{"id":"fixture","linear_user_id":id(4),"roles":["inspect"]}}}})).unwrap();
        let digest = bytes_digest(b"fixture-generation");
        let work = WorkIdentity {
            linear_endpoint: "https://linear.invalid/graphql".into(),
            linear_workspace: id(2),
            parent: id(10),
            child: None,
            generation: Generation {
                authorization: digest.clone(),
                definition: digest.clone(),
                correction: digest.clone(),
                scope: digest.clone(),
            },
        };
        let issue = |child: bool| Issue {
            id: if child { id(11) } else { id(10) },
            workspace_id: id(2),
            endpoint: work.linear_endpoint.clone(),
            identifier: if child { "TEST-11" } else { "TEST-10" }.into(),
            title: "fixture".into(),
            description: Some("fixture definition".into()),
            team_id: id(6),
            project_id: None,
            creator_id: Some(id(4)),
            assignee_id: None,
            parent_id: child.then_some(id(10)),
            state_id: id(7),
            state_name: "Prepared".into(),
            state_type: "unstarted".into(),
            labels: vec![],
            priority: Some(2),
            created_at: Some(Utc::now()),
            updated_at: None,
            url: "https://linear.invalid/fixture".into(),
            diagnostics: vec![],
        };
        let mut child_work = work.clone();
        child_work.child = Some(id(11));
        let child = ChildProjection {
            parent_reference: "fixture/10".into(),
            key: IssueKey::new(&work.linear_endpoint, id(2), id(11)).unwrap(),
            issue: Some(issue(true)),
            scope: Some(ChildScope {
                schema_version: 1,
                kind: "child".into(),
                responsibility: Responsibility::Documentation,
                write_repository: None,
                write_paths: vec![],
                read_repositories: vec![],
                test_resources: vec![],
            }),
            profile: Some(profile.clone()),
            work: Some(child_work),
            blockers: vec![],
            prerequisites: BTreeSet::new(),
        };
        let parent = ParentProjection {
            workspace_id: id(3),
            workspace_name: "fixture".into(),
            parent_reference: "fixture/10".into(),
            key: IssueKey::new(&work.linear_endpoint, id(2), id(10)).unwrap(),
            issue: Some(issue(false)),
            included: true,
            observed_at: Utc::now(),
            required_children: BTreeSet::from([id(11)]),
            children: BTreeMap::from([(id(11), child)]),
            observations: BTreeMap::new(),
            metadata: None,
            authorization_candidate: None,
            authorization_published: true,
            corrections: Corrections::default(),
            observation_revision: 1,
            disappeared_children: BTreeMap::new(),
            work: Some(work),
            blockers: vec![],
            prerequisites: BTreeSet::new(),
            source_digest: digest.clone(),
            config_revision: crate::config::config_revision(&config).unwrap(),
            workflow_digest: digest.clone(),
            coordination_profile: profile,
            drain_config_revision: digest.clone(),
            release_mode: ReleaseMode::Manual,
        };
        let projection = RuntimeProjection {
            runtime_id: id(1),
            config_revision: crate::config::config_revision(&config).unwrap(),
            observed_at: Utc::now(),
            workspaces: BTreeMap::from([(
                id(3),
                WorkspaceProjection {
                    workspace_id: id(3),
                    workspace_name: "fixture".into(),
                    complete: true,
                    observed_at: Utc::now(),
                    parents: BTreeMap::from([(id(10), parent)]),
                    repositories: BTreeMap::new(),
                    independent_jobs: vec![],
                    error: None,
                    config_revision: crate::config::config_revision(&config).unwrap(),
                    workflow_digest: digest,
                    independent_review_profile: None,
                },
            )]),
        };
        (config, projection)
    }
    fn choose(config: &Config, projection: &RuntimeProjection) -> Selection {
        select(
            SelectionInput {
                config,
                projection,
                outcomes: &DependencyOutcomes::default(),
                delivery: &BTreeMap::new(),
                grants: &BTreeMap::new(),
            },
            |phase, workspace, account| {
                assert_eq!(account, "fixture-account");
                crate::runtime_schedule::plan(&config.runtime, phase, workspace, &[], Utc::now())
            },
        )
        .unwrap()
    }
    #[test]
    fn selects_only_scoped_child_then_pending_classification_blocks_implementation() {
        let (config, mut projection) = fixture();
        let initial = choose(&config, &projection);
        let [RuntimeAction::Worker(child)] = initial.actions.as_slice() else {
            panic!("expected one child")
        };
        assert_eq!(child.ready.kind, WorkKind::Child);
        assert_eq!(child.phase.phase, "implementation");
        assert_eq!(child.authority.work().unwrap().child, Some(id(11)));
        let parent = projection
            .workspaces
            .get_mut(&id(3))
            .unwrap()
            .parents
            .get_mut(&id(10))
            .unwrap();
        parent
            .corrections
            .observe(
                CommentRevision {
                    id: id(20),
                    author: Some(id(4)),
                    body_digest: bytes_digest(b"change request"),
                    edited_at: None,
                    observed_revision: 1,
                },
                &BTreeSet::from([id(4)]),
                &BTreeSet::new(),
                None,
            )
            .unwrap();
        let selected = choose(&config, &projection);
        let [RuntimeAction::Worker(coordination)] = selected.actions.as_slice() else {
            panic!("expected only classification")
        };
        assert_eq!(coordination.ready.kind, WorkKind::Coordination);
        assert!(coordination.authority.work().unwrap().child.is_none());
        assert!(
            coordination
                .phase
                .phase
                .starts_with("correction_classification_")
        );
        assert!(coordination.phase.phase.len() <= 64);
        assert!(
            coordination
                .phase
                .phase
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
        );
        assert!(
            selected
                .blockers
                .values()
                .any(|e| e.code == "correction_classification_pending")
        );
        let repeat = choose(&config, &projection);
        let [RuntimeAction::Worker(repeated)] = repeat.actions.as_slice() else {
            panic!("classification should remain same pending action")
        };
        assert_eq!(repeated.phase, coordination.phase);
    }
    #[test]
    fn incomplete_authority_and_consumed_phase_cannot_create_replacement_work() {
        for case in ["incomplete", "unpublished", "stale"] {
            let (config, mut projection) = fixture();
            let workspace = projection.workspaces.get_mut(&id(3)).unwrap();
            match case {
                "incomplete" => workspace.complete = false,
                "unpublished" => {
                    workspace
                        .parents
                        .get_mut(&id(10))
                        .unwrap()
                        .authorization_published = false
                }
                "stale" => {
                    workspace.parents.get_mut(&id(10)).unwrap().observed_at =
                        Utc::now() - Duration::seconds(31)
                }
                _ => unreachable!(),
            }
            let selected = choose(&config, &projection);
            assert!(selected.actions.is_empty(), "{case}");
            assert!(!selected.blockers.is_empty(), "{case}");
        }
        let (config, projection) = fixture();
        let selected = select(
            SelectionInput {
                config: &config,
                projection: &projection,
                outcomes: &DependencyOutcomes::default(),
                delivery: &BTreeMap::new(),
                grants: &BTreeMap::new(),
            },
            |_, _, _| {
                Err(Error::new(
                    "no_work_trigger",
                    "actual completed phase cannot be retried by polling",
                ))
            },
        )
        .unwrap();
        assert!(selected.actions.is_empty());
        assert_eq!(
            selected.blockers.values().next().unwrap().code,
            "no_work_trigger"
        );
    }
}
