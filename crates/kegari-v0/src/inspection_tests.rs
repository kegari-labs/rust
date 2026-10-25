//! Presentation-only records. No completion, release or admission capability is
//! minted here; the producer's authenticated cached/cold tests cover provenance.
use super::*;
use crate::{digest::bytes_digest, evidence::Generation};
use serde_json::json;

fn config() -> Config {
    let config: Config = serde_json::from_value(json!({
        "schema_version": 1,
        "runtime": {
            "id": Uuid::from_u128(1),
            "host_id": "inspection-fixture",
            "max_agents": 1
        },
        "connections": {},
        "workspaces": {},
        "control": {
            "local_users": {
                "501": {
                    "id": "inspection-operator",
                    "linear_user_id": Uuid::from_u128(4),
                    "roles": ["inspect"]
                }
            }
        }
    }))
    .unwrap();
    config.validate().unwrap();
    config
}
fn projection(config: &Config) -> RuntimeProjection {
    RuntimeProjection {
        runtime_id: config.runtime.id,
        config_revision: config.revision().unwrap(),
        observed_at: Utc::now(),
        workspaces: BTreeMap::new(),
    }
}
fn row(workspace: u128, operation: u128) -> ReleaseInspectionRow {
    let generation = Generation {
        authorization: bytes_digest(b"authorization"),
        definition: bytes_digest(b"definition"),
        correction: bytes_digest(b"correction"),
        scope: bytes_digest(b"scope"),
    };
    let parent = IssueKey::new(
        "https://linear.invalid",
        Uuid::from_u128(2),
        Uuid::from_u128(3),
    )
    .unwrap();
    let work = WorkIdentity {
        linear_endpoint: parent.endpoint.clone(),
        linear_workspace: parent.workspace,
        parent: parent.issue,
        child: None,
        generation: generation.clone(),
    };
    let plan = crate::release::ReleasePlan {
        schema_version: 1,
        runtime_id: Uuid::from_u128(1),
        operation_id: Uuid::from_u128(operation),
        parent_id: parent.issue,
        linear_workspace: parent.workspace,
        generation,
        evidence_token: bytes_digest(b"display only"),
        acceptance_digest: bytes_digest(b"display only acceptance"),
        carried_acceptance: None,
        resources: BTreeSet::new(),
        dependencies: vec![],
        spacing_seconds: 1,
        deployment_free: BTreeSet::new(),
        steps: vec![],
    };
    ReleaseInspectionRow {
        workspace: Uuid::from_u128(workspace),
        parent,
        work,
        attempt: ReleaseAttempt {
            plan,
            admitted_at: Utc::now(),
            control_generation: 1,
            owner_generation: 1,
            steps: BTreeMap::new(),
            step_history: BTreeMap::new(),
            cancellations: BTreeMap::new(),
            hold_reason: None,
            abandoned: false,
            resolved: true,
            assigned_successor: None,
            recovery_records: vec![],
        },
        active_hold: false,
        completed_by: None,
        completion: None,
        diagnostic: None,
        display: ParentDisplay {
            identifier: Some("OLD-3".into()),
            ..Default::default()
        },
    }
}

#[test]
fn historical_resolution_requires_exact_workspace_and_never_changes_current_resolution() {
    let config = config();
    let projection = projection(&config);
    let rows = vec![row(10, 20), row(11, 21)];
    let selected = Uuid::from_u128(10);
    assert_eq!(
        resolve_workspace(&config, &rows, Some(&selected.to_string())).unwrap(),
        Some(selected)
    );
    assert!(crate::app::resolve_workspace(&config, Some(&selected.to_string())).is_err());
    assert!(projection.resolve_parent("OLD-3").is_err());
    assert_eq!(
        resolve_parent(&config, &projection, &rows, "OLD-3", None)
            .unwrap_err()
            .code,
        "ambiguous_reference"
    );
    let resolved = resolve_parent(&config, &projection, &rows, "OLD-3", Some(selected)).unwrap();
    assert_eq!(resolved.workspace, selected);
    assert_eq!(parent_rows(&rows, &resolved).len(), 1);
    assert!(
        resolve_parent(
            &config,
            &projection,
            &rows,
            &format!("{}/OLD-3", Uuid::from_u128(11)),
            Some(selected)
        )
        .is_err()
    );
    let duplicate = vec![rows[0].clone(), rows[0].clone()];
    assert!(resolve_parent(&config, &projection, &duplicate, "OLD-3", None).is_ok());
    assert!(validate_rows(config.runtime.id, &duplicate).is_err());
}

#[test]
fn legacy_urls_stay_unknown_and_unsafe_or_conflicting_observed_urls_do_not_open() {
    let mut first = row(10, 20);
    assert_eq!(
        observed_url(None, &[&first]).unwrap_err().code,
        "unknown_resource"
    );
    for url in [
        "file:///private/tmp/secret",
        "javascript:alert(1)",
        "https://user:secret@linear.invalid/issue/3",
    ] {
        first.display.url = Some(url.into());
        assert_eq!(
            observed_url(None, &[&first]).unwrap_err().code,
            "unsafe_url"
        );
    }
    first.display.url = Some("https://linear.invalid/issue/3".into());
    assert_eq!(
        observed_url(None, &[&first]).unwrap(),
        "https://linear.invalid/issue/3"
    );
    let mut second = first.clone();
    second.display.url = Some("https://linear.invalid/issue/other".into());
    assert_eq!(
        observed_url(None, &[&first, &second]).unwrap_err().code,
        "ambiguous_reference"
    );
}

#[tokio::test]
async fn historical_completion_does_not_hide_a_later_current_correction() {
    let (_root, built, _delivery) =
        crate::runtime_delivery::tests::fixture_authorization(true).await;
    let config = built.runtime.configuration.snapshot().unwrap();
    let projection = built.runtime.observations.snapshot().unwrap();
    let parent = projection
        .parent(Uuid::from_u128(2), Uuid::from_u128(100))
        .unwrap();
    let mut historical = row(2, 20);
    historical.parent = parent.key.clone();
    historical.work = parent.work.as_ref().unwrap().clone();
    historical.work.generation.correction = bytes_digest(b"earlier recorded correction");
    historical.attempt.plan.runtime_id = config.runtime.id;
    historical.attempt.plan.parent_id = parent.key.issue;
    historical.attempt.plan.linear_workspace = parent.key.workspace;
    historical.attempt.plan.generation = historical.work.generation.clone();
    historical.completion = Some(ReleaseCompletionInspection {
        operation: historical.attempt.plan.operation_id,
        work: historical.work.clone(),
        completed_at: Utc::now(),
    });
    let view = RuntimeInspection {
        projection,
        delivery: BTreeMap::new(),
        outcomes: Default::default(),
        activities: vec![],
        history: vec![],
        release_details: BTreeMap::new(),
        release_history: vec![historical],
    };
    validate_rows(config.runtime.id, &view.release_history).unwrap();
    let control = built.runtime.control.snapshot().unwrap();
    let output = overview(&view, None, &control, Utc::now(), Duration::seconds(30)).unwrap();
    assert_eq!(output.current.history.len(), 1);
    assert_eq!(output.current.history[0].outcome, "released");
    assert!(output.current.ready_for_release.is_empty());
    assert!(
        output
            .current
            .blocked
            .iter()
            .chain(&output.current.running)
            .any(|row| row.parent.issue == Uuid::from_u128(100))
    );
    assert!(
        view.outcomes
            .parent_outcome(
                &view.release_history[0].parent,
                &view.release_history[0].work.generation
            )
            .is_none()
    );
    let mut duplicate = view.clone();
    duplicate.history = output.current.history.clone();
    let output = overview(
        &duplicate,
        None,
        &control,
        Utc::now(),
        Duration::seconds(30),
    )
    .unwrap();
    assert_eq!(output.current.history.len(), 1);
    let mut removed = duplicate;
    removed.projection.workspaces.clear();
    let output = overview(
        &removed,
        Some(Uuid::from_u128(2)),
        &control,
        Utc::now(),
        Duration::seconds(30),
    )
    .unwrap();
    assert_eq!(output.current.history.len(), 1);
    assert!(output.current.ready_for_release.is_empty());
    assert!(output.current.blocked.is_empty());
    let mut unavailable = view.clone();
    unavailable.release_history[0].completion = None;
    unavailable.release_history[0].completed_by = Some(Uuid::from_u128(99));
    unavailable.release_history[0].diagnostic = Some(Error::new(
        "record_missing",
        "actual completion reference unavailable",
    ));
    let output = overview(
        &unavailable,
        None,
        &control,
        Utc::now(),
        Duration::seconds(30),
    )
    .unwrap();
    assert!(
        output.current.history.is_empty(),
        "recorded attempt or consumed ancestor does not create successful history"
    );
    assert!(output.release_history[0].diagnostic.is_some());
    verify_shared_native_issue_workspace_filter(&config, &view, &control);
}

fn verify_shared_native_issue_workspace_filter(
    config: &Config,
    view: &RuntimeInspection,
    control: &ControlSnapshot,
) {
    // Presentation copies only: a second runtime mapping does not mint a new
    // work, dependency, completion or admission capability.
    let first = Uuid::from_u128(2);
    let second = Uuid::from_u128(90);
    let mut config = config.clone();
    let mut configured = config
        .workspaces
        .values()
        .find(|w| w.id == first)
        .unwrap()
        .clone();
    configured.id = second;
    config
        .workspaces
        .insert("second-inspection-workspace".into(), configured);
    config.validate().unwrap();
    let mut scoped = view.clone();
    scoped.release_history.clear();
    let mut observed = scoped.projection.workspaces[&first].clone();
    observed.workspace_id = second;
    observed.workspace_name = "second-inspection-workspace".into();
    for parent in observed.parents.values_mut() {
        parent.workspace_id = second;
        parent.workspace_name = observed.workspace_name.clone();
        parent.parent_reference = format!("{second}/{}", parent.key.issue);
    }
    scoped.projection.workspaces.insert(second, observed);
    scoped.projection.config_revision = config.revision().unwrap();
    for workspace in scoped.projection.workspaces.values_mut() {
        workspace.config_revision = scoped.projection.config_revision.clone();
        for parent in workspace.parents.values_mut() {
            parent.config_revision = scoped.projection.config_revision.clone();
        }
    }
    let parent = scoped
        .projection
        .parent(first, Uuid::from_u128(100))
        .unwrap();
    let key = parent.key.clone();
    let generation = parent.work.as_ref().unwrap().generation.clone();
    scoped.history = [first, second]
        .map(|workspace| HistoryRow {
            parent_reference: format!("{workspace}/{}", key.issue),
            parent: key.clone(),
            outcome: "no-release-required".into(),
            generation: generation.clone(),
            completed_at: Utc::now(),
            links: vec![],
        })
        .into();
    for selected in [first, second] {
        let output = overview(
            &scoped,
            Some(selected),
            control,
            Utc::now(),
            Duration::seconds(30),
        )
        .unwrap();
        assert_eq!(output.current.history.len(), 1);
        assert_eq!(
            output.current.history[0].parent_reference,
            format!("{selected}/{}", key.issue)
        );
        let exact = history_for_parent(&scoped.history, selected, &key);
        assert_eq!(exact.len(), 1);
        assert_eq!(
            exact[0].parent_reference,
            output.current.history[0].parent_reference
        );
    }
    let output = overview(&scoped, None, control, Utc::now(), Duration::seconds(30)).unwrap();
    assert_eq!(output.current.history.len(), 2);
    assert!(scoped.outcomes.parent_outcome(&key, &generation).is_none());
}
