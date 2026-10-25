//! Actual host/broker publication and readback with domain-verified synthetic
//! upstream checks and an owned real Git candidate; no live/native qualification.
use super::*;
use crate::evidence::{AcceptanceActor, AcceptanceRequest, VerifiedChild};
use crate::reconcile::DependencyOutcomes;
use crate::runtime_fixture::RuntimeFixture;
use std::sync::{Arc, Mutex};

#[path = "runtime_no_release_parent_fixture.rs"]
mod parent_fixture;

struct Harness {
    _root: tempfile::TempDir,
    _git: tempfile::TempDir,
    _owner: crate::storage::ExclusiveLock,
    owner: super::super::super::tests::FixtureOwner,
    built: RuntimeFixture,
    delivery: Arc<RuntimeDelivery>,
    wire: Arc<Mutex<crate::corrections::wire_fixture::Fixture>>,
    parent: VerifiedParent,
    child: VerifiedChild,
    acceptance: VerifiedAcceptance,
    lease: Lease,
}
impl Harness {
    async fn new() -> Self {
        let (root, built, delivery, wire, owner) =
            super::super::super::tests::fixture_authorization_with_wire(true).await;
        let projection = built.runtime.observations.snapshot().unwrap();
        let current = projection
            .parent(Uuid::from_u128(2), Uuid::from_u128(100))
            .unwrap();
        let workspace = delivery.workspace(&projection, Uuid::from_u128(2)).unwrap();
        let (git, parent, child) = parent_fixture::parent(
            current.work.as_ref().unwrap(),
            current.children[&Uuid::from_u128(101)]
                .work
                .as_ref()
                .unwrap(),
            &built.runtime.services.key,
            &delivery.git_path().unwrap(),
            workspace.contracts[0].repository.clone(),
        );
        let revision = &parent.manifest().baselines[0].production;
        {
            let mut data = wire.lock().unwrap();
            data.github_values.insert(
                "/repos/fixture/repo/git/ref/heads/main".into(),
                json!({"ref":"refs/heads/main","object":{"type":"commit","sha":revision.commit}}),
            );
            data.github_values.insert(
                format!("/repos/fixture/repo/git/commits/{}", revision.commit),
                json!({"sha":revision.commit,"tree":{"sha":revision.tree},"parents":[]}),
            );
        }
        let signed_parent=built.runtime.services.key.sign("parent_result",&json!({"runtime_id":workspace.configuration.runtime.id,"evidence":parent.manifest()})).unwrap();
        let parent_body =
            evidence::encode_signed_publication("parent_result", &signed_parent).unwrap();
        publish(
            &delivery,
            &built,
            &parent,
            parent.manifest().publication_operation,
            parent_body,
        )
        .await;
        let allowed = BTreeSet::from([Uuid::from_u128(5).to_string()]);
        let actor = Uuid::from_u128(5).to_string();
        let accepted = evidence::prepare_acceptance(
            &parent,
            AcceptanceRequest {
                supplied_token: parent.token(),
                current_work: &parent.manifest().work,
                pending_correction: false,
                actor: AcceptanceActor {
                    control_actor: "fixture-operator",
                    linear_actor: &actor,
                    provider_writer: &actor,
                    has_accept_role: true,
                    permitted_acceptors: &allowed,
                },
                runtime_id: workspace.configuration.runtime.id,
                operation_id: Uuid::new_v4(),
                now: Utc::now(),
            },
            &built.runtime.services.key,
        )
        .unwrap();
        let body = evidence::encode_signed_publication("human_acceptance", &accepted).unwrap();
        let receipt = publish(
            &delivery,
            &built,
            &parent,
            accepted.payload.operation_id,
            body,
        )
        .await;
        let publication = Publication {
            provider: "linear".into(),
            record_id: receipt.remote_ids["comment_id"].clone(),
            writer_account_id: actor,
            payload_digest: canonical_digest(1, &accepted).unwrap(),
            observed_at: Utc::now(),
            url: format!(
                "https://linear.invalid/comment/{}",
                receipt.remote_ids["comment_id"]
            ),
        };
        let acceptance = evidence::verify_acceptance(
            &accepted,
            &publication,
            &parent,
            &parent.manifest().work,
            false,
            &allowed,
            &built.runtime.services.key,
        )
        .unwrap();
        let (lease, lock) = owner.start(&built.runtime);
        let result = Self {
            _root: root,
            _git: git,
            _owner: lock,
            owner,
            built,
            delivery,
            wire,
            parent,
            child,
            acceptance,
            lease,
        };
        result.install_current().await;
        result
    }
    async fn snapshot(&self) -> (RuntimeProjection, VerifiedDeliverySnapshot) {
        let projection = self.built.runtime.observations.refresh().await.unwrap();
        let current = projection
            .parent(Uuid::from_u128(2), self.parent.manifest().work.parent)
            .unwrap();
        let gate = current
            .fresh_gate(
                None,
                GatePurpose::NewOperation,
                &DependencyOutcomes::default(),
                Utc::now(),
                Duration::seconds(30),
            )
            .unwrap();
        let snapshot = VerifiedDeliverySnapshot::fixture(
            gate,
            self.parent.clone(),
            self.acceptance.clone(),
            vec![self.child.clone()],
        )
        .unwrap();
        (projection, snapshot)
    }
    async fn install_current(&self) -> RuntimeProjection {
        let (projection, snapshot) = self.snapshot().await;
        self.built
            .runtime
            .retain_delivery_snapshot(snapshot)
            .unwrap();
        projection
    }
    fn stop(&self) {
        let snapshot = self.built.runtime.control.snapshot().unwrap();
        self.built
            .runtime
            .control
            .lifecycle(
                MutationRequest {
                    request_id: Uuid::new_v4(),
                    runtime_id: snapshot.intent.runtime_id,
                    actor: Actor {
                        identity: "fixture-operator".into(),
                        principal: None,
                        roles: ["lifecycle".into()].into(),
                    },
                    kind: "lifecycle".into(),
                    canonical_targets: vec![snapshot.intent.runtime_id.to_string()],
                    expected: BTreeMap::new(),
                    payload: json!(null),
                },
                crate::control::Lifecycle::Stop,
                snapshot.intent.generation,
            )
            .unwrap();
    }
    fn writes(&self) -> usize {
        self.wire
            .lock()
            .unwrap()
            .requests
            .iter()
            .filter(|(_, body)| {
                body["query"]
                    .as_str()
                    .is_some_and(|q| q.contains("mutation KegariComment"))
            })
            .count()
    }
}
async fn publish(
    delivery: &RuntimeDelivery,
    built: &RuntimeFixture,
    parent: &VerifiedParent,
    id: Uuid,
    body: String,
) -> Receipt {
    let config = built.runtime.configuration.snapshot().unwrap();
    let broker = delivery
        .provider_broker(&config, Uuid::from_u128(2))
        .unwrap();
    broker
        .mutate_observed(
            built.runtime.control.clone(),
            MutationRequest {
                request_id: id,
                runtime_id: config.runtime.id,
                actor: Actor {
                    identity: "fixture-operator".into(),
                    principal: None,
                    roles: ["accept".into()].into(),
                },
                kind: "linear_comment".into(),
                canonical_targets: vec![format!(
                    "linear:{}:{}:{}",
                    parent.manifest().work.linear_endpoint,
                    parent.manifest().work.linear_workspace,
                    parent.manifest().work.parent
                )],
                expected: BTreeMap::new(),
                payload: json!({}),
            },
            None,
            Some("accept".into()),
            ProviderMutation::LinearComment {
                issue: parent.manifest().work.parent,
                body,
            },
            |_| Ok(()),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn accepted_unchanged_parent_publishes_once_and_readback_supplies_stopped_dependency() {
    let h = Harness::new().await;
    let projection = h.install_current().await;
    assert_eq!(h.writes(), 2);
    assert!(
        h.delivery
            .advance_no_release(
                &h.lease,
                &projection,
                Uuid::from_u128(2),
                h.parent.manifest().work.parent
            )
            .await
            .unwrap()
    );
    assert_eq!(h.writes(), 3);
    assert!(
        h.built
            .runtime
            .control
            .snapshot()
            .unwrap()
            .active_operations
            .is_empty()
    );
    h.stop();
    let (projection, mut snapshot) = h.snapshot().await;
    let current = projection
        .parent(Uuid::from_u128(2), h.parent.manifest().work.parent)
        .unwrap();
    let workspace = h
        .delivery
        .workspace(&projection, Uuid::from_u128(2))
        .unwrap();
    h.delivery
        .hydrate_no_release(&workspace, current, &mut snapshot)
        .await
        .unwrap();
    let outcome = snapshot
        .parent_delivery()
        .no_release_required()
        .unwrap()
        .digest()
        .to_owned();
    assert_eq!(
        snapshot
            .outcomes()
            .parent_outcome(&current.key, &h.parent.manifest().work.generation),
        Some(outcome.as_str())
    );
    h.built
        .runtime
        .retain_delivery_snapshot(snapshot.clone())
        .unwrap();
    assert_eq!(
        h.built
            .runtime
            .dependency_outcomes()
            .unwrap()
            .parent_outcome(&current.key, &h.parent.manifest().work.generation),
        Some(outcome.as_str())
    );
    assert_eq!(h.writes(), 3);
    assert_eq!(
        h.built.runtime.control.snapshot().unwrap().intent.desired,
        crate::control::Intent::Stopped
    );
    let inspection = h.built.runtime.inspection().unwrap();
    let history: Vec<_> = inspection
        .history
        .iter()
        .filter(|row| row.parent == current.key && row.outcome == "no-release-required")
        .collect();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].links.len(), 1);
    assert!(history[0].links[0].starts_with("https://linear.invalid/comment/"));
    let control = h.built.runtime.control.snapshot().unwrap();
    let overview = inspection
        .projection
        .overview(crate::reconcile::OverviewInputs {
            workspace: None,
            delivery: &inspection.delivery,
            outcomes: &inspection.outcomes,
            activities: &inspection.activities,
            history: &inspection.history,
            control: &control,
            now: Utc::now(),
            max_age: Duration::seconds(30),
        })
        .unwrap();
    assert!(
        !overview
            .blocked
            .iter()
            .chain(&overview.running)
            .chain(&overview.ready_for_release)
            .chain(&overview.ready_for_review)
            .any(|row| row.parent == current.key)
    );
    // A changed acceptance cannot retain old dependency credit or display link.
    snapshot.acceptance(h.acceptance.clone()).unwrap();
    assert!(snapshot.parent_delivery().no_release_required().is_none());
    assert!(snapshot.no_release_link().is_none());
    assert!(
        snapshot
            .outcomes()
            .parent_outcome(&current.key, &h.parent.manifest().work.generation)
            .is_none()
    );
    h.built.runtime.retain_delivery_snapshot(snapshot).unwrap();
    assert!(
        h.built
            .runtime
            .dependency_outcomes()
            .unwrap()
            .parent_outcome(&current.key, &h.parent.manifest().work.generation)
            .is_none()
    );
    let inspection = h.built.runtime.inspection().unwrap();
    assert_eq!(
        inspection
            .history
            .iter()
            .filter(|row| row.parent == current.key && row.outcome == "no-release-required")
            .count(),
        1
    );
    let overview = inspection
        .projection
        .overview(crate::reconcile::OverviewInputs {
            workspace: None,
            delivery: &inspection.delivery,
            outcomes: &inspection.outcomes,
            activities: &inspection.activities,
            history: &inspection.history,
            control: &control,
            now: Utc::now(),
            max_age: Duration::seconds(30),
        })
        .unwrap();
    assert!(
        overview
            .blocked
            .iter()
            .chain(&overview.running)
            .chain(&overview.ready_for_release)
            .chain(&overview.ready_for_review)
            .any(|row| row.parent == current.key),
        "history alone cannot hide invalidated current work"
    );
}

#[tokio::test]
async fn changed_or_deleted_native_publication_cannot_supply_no_release_credit() {
    for delete in [false, true] {
        let h = Harness::new().await;
        let projection = h.install_current().await;
        h.delivery
            .advance_no_release(
                &h.lease,
                &projection,
                Uuid::from_u128(2),
                h.parent.manifest().work.parent,
            )
            .await
            .unwrap();
        let (_, mut snapshot) = h.snapshot().await;
        {
            let mut data = h.wire.lock().unwrap();
            let comments = data
                .comments
                .get_mut(&h.parent.manifest().work.parent)
                .unwrap();
            let index = comments
                .iter()
                .position(|c| {
                    c["body"]
                        .as_str()
                        .is_some_and(|b| b.starts_with("<!-- kegari:v0:no_release_required -->"))
                })
                .unwrap();
            if delete {
                comments.remove(index);
            } else {
                comments[index]["body"] = json!("edited no-release claim");
            }
        }
        h.stop();
        let projection = h.built.runtime.observations.refresh().await.unwrap();
        let current = projection
            .parent(Uuid::from_u128(2), h.parent.manifest().work.parent)
            .unwrap();
        let workspace = h
            .delivery
            .workspace(&projection, Uuid::from_u128(2))
            .unwrap();
        let result = h
            .delivery
            .hydrate_no_release(&workspace, current, &mut snapshot)
            .await;
        assert!(result.is_err() || snapshot.parent_delivery().no_release_required().is_none());
        assert!(
            snapshot
                .outcomes()
                .parent_outcome(&current.key, &h.parent.manifest().work.generation)
                .is_none()
        );
        assert_eq!(h.writes(), 3);
    }
}

#[tokio::test]
async fn production_movement_or_current_resource_owner_blocks_no_release() {
    let h = Harness::new().await;
    let projection = h.install_current().await;
    h.delivery
        .advance_no_release(
            &h.lease,
            &projection,
            Uuid::from_u128(2),
            h.parent.manifest().work.parent,
        )
        .await
        .unwrap();
    let (projection, mut snapshot) = h.snapshot().await;
    {
        let mut data = h.wire.lock().unwrap();
        data.github_values.insert(
            "/repos/fixture/repo/git/ref/heads/main".into(),
            json!({"ref":"refs/heads/main","object":{"type":"commit","sha":"e".repeat(40)}}),
        );
        data.github_values.insert(
            format!("/repos/fixture/repo/git/commits/{}", "e".repeat(40)),
            json!({"sha":"e".repeat(40),"tree":{"sha":"f".repeat(40)},"parents":[]}),
        );
    }
    let current = projection
        .parent(Uuid::from_u128(2), h.parent.manifest().work.parent)
        .unwrap();
    let workspace = h
        .delivery
        .workspace(&projection, Uuid::from_u128(2))
        .unwrap();
    assert_eq!(
        h.delivery
            .hydrate_no_release(&workspace, current, &mut snapshot)
            .await
            .unwrap_err()
            .code,
        "stale_result"
    );
    assert!(snapshot.parent_delivery().no_release_required().is_none());
    assert_eq!(h.writes(), 3);

    let held = Harness::new().await;
    held.delivery.restore_releases().unwrap();
    let (plan, _, _) = crate::delivery::observed_merge_for_runtime().await;
    held.delivery
        .releases
        .state
        .lock()
        .unwrap()
        .queue
        .reserve(&plan, Utc::now())
        .unwrap();
    let projection = held.install_current().await;
    assert_eq!(
        held.delivery
            .advance_no_release(
                &held.lease,
                &projection,
                Uuid::from_u128(2),
                held.parent.manifest().work.parent
            )
            .await
            .unwrap_err()
            .code,
        "release_hold"
    );
    assert_eq!(held.writes(), 2);
    assert!(
        held.built
            .runtime
            .control
            .snapshot()
            .unwrap()
            .active_operations
            .is_empty()
    );
}

async fn reconcile(h: &Harness, target: Uuid) -> Result<Receipt> {
    use crate::app::WorkflowControl;
    let workflow = crate::runtime_workflow::RuntimeWorkflow::new(
        h.built.runtime.clone(),
        h.delivery.clone(),
        h.delivery.clone(),
    );
    let actor = Actor {
        identity: "operator".into(),
        principal: Some(crate::control::Principal::Unix { uid: 501 }),
        roles: ["recover".into(), "inspect".into(), "lifecycle".into()].into(),
    };
    let mut request = crate::api::CommandRequest {
        api_version: 1,
        request_id: Uuid::new_v4(),
        runtime_id: h.lease.runtime_id,
        command: crate::api::CommandName::OperationReconcile,
        args: json!({"operation":target}),
        expected_config_revision: Some(h.built.runtime.configuration.snapshot()?.revision()?),
        expected_control_generation: Some(h.built.runtime.control.snapshot()?.intent.generation),
    };
    let mut mutation =
        crate::app::mutation_request(&actor, &request, workflow.command_targets(&request)?);
    mutation.kind = format!("command/{:?}", request.command);
    mutation.payload = json!({"arguments":request.args,"secret_digest":null});
    let (parent, fresh) = h
        .built
        .runtime
        .control
        .queue_command(mutation, "recover", || Ok(()))?;
    assert!(fresh);
    request.request_id = crate::app::command_effect_id(parent.operation_id);
    workflow.execute(actor, request, parent).await
}

#[tokio::test]
async fn lost_publication_reply_requires_actual_authorized_read_only_reconciliation() {
    let h = Harness::new().await;
    let projection = h.install_current().await;
    h.wire.lock().unwrap().comment_response_lost = true;
    let result = h
        .delivery
        .advance_no_release(
            &h.lease,
            &projection,
            Uuid::from_u128(2),
            h.parent.manifest().work.parent,
        )
        .await;
    assert!(result.is_err());
    assert_eq!(h.writes(), 3);
    assert!(
        h.built
            .runtime
            .control
            .snapshot()
            .unwrap()
            .active_operations
            .is_empty()
    );
    let effect = h
        .built
        .runtime
        .control
        .operations()
        .unwrap()
        .into_iter()
        .find(|r| r.kind == "linear_comment" && r.expected.contains_key("no_release_digest"))
        .unwrap();
    assert_eq!(effect.phase, ReceiptPhase::Sent);
    let admission: Uuid = effect.expected["admission_operation"].parse().unwrap();
    let original_intent = h
        .delivery
        .records
        .read_bytes(&intent_leaf(admission), 1024 * 1024)
        .unwrap();
    assert!(
        h.delivery
            .advance_no_release(
                &h.lease,
                &projection,
                Uuid::from_u128(2),
                h.parent.manifest().work.parent
            )
            .await
            .is_err()
    );
    assert_eq!(
        h.writes(),
        3,
        "unknown publication must never be retransmitted"
    );
    assert_eq!(
        h.delivery
            .records
            .read_bytes(&intent_leaf(admission), 1024 * 1024)
            .unwrap(),
        original_intent,
        "admitted unknown intent cannot be rewritten"
    );
    h.stop();
    h.wire.lock().unwrap().comment_response_lost = false;
    reconcile(&h, effect.operation_id).await.unwrap();
    assert_eq!(
        h.built
            .runtime
            .control
            .operation(effect.operation_id)
            .unwrap()
            .phase,
        ReceiptPhase::Reconciled
    );
    reconcile(&h, admission).await.unwrap();
    let (projection, mut snapshot) = h.snapshot().await;
    let current = projection
        .parent(Uuid::from_u128(2), h.parent.manifest().work.parent)
        .unwrap();
    let workspace = h
        .delivery
        .workspace(&projection, Uuid::from_u128(2))
        .unwrap();
    h.delivery
        .hydrate_no_release(&workspace, current, &mut snapshot)
        .await
        .unwrap();
    assert!(snapshot.parent_delivery().no_release_required().is_some());
    assert_eq!(h.writes(), 3);
    assert_eq!(
        h.built.runtime.control.snapshot().unwrap().intent.desired,
        crate::control::Intent::Stopped
    );
}

#[test]
fn equivalent_target_observation_keeps_semantics_and_changed_delivery_does_not() {
    let original = ProductionTargetBaseline {
        provider: "github".into(),
        target: "fixture-target".into(),
        repository: RepositoryId {
            hostname: "github.com".into(),
            id: "42".into(),
        },
        source: crate::git::Revision {
            commit: "1".repeat(40),
            tree: "2".repeat(40),
        },
        artifact_digest: bytes_digest(b"original artifact"),
        provider_record: "deployment-1/status-1".into(),
        observed_at: Utc::now(),
    };
    let mut current = original.clone();
    current.provider_record = "deployment-2/status-2".into();
    current.observed_at += Duration::seconds(1);
    assert!(same_targets(
        std::slice::from_ref(&original),
        std::slice::from_ref(&current)
    ));
    for field in [
        "provider",
        "target",
        "repository",
        "commit",
        "tree",
        "artifact",
    ] {
        let mut changed = current.clone();
        match field {
            "provider" => changed.provider = "other".into(),
            "target" => changed.target = "another".into(),
            "repository" => changed.repository.id = "43".into(),
            "commit" => changed.source.commit = "3".repeat(40),
            "tree" => changed.source.tree = "4".repeat(40),
            "artifact" => changed.artifact_digest = bytes_digest(b"different artifact"),
            _ => unreachable!(),
        }
        assert!(
            !same_targets(
                std::slice::from_ref(&original),
                std::slice::from_ref(&changed)
            ),
            "changed {field} must invalidate delivery equivalence"
        );
    }
}

#[tokio::test]
async fn stale_unadmitted_intent_replans_without_resetting_budget_or_losing_history() {
    let h = Harness::new().await;
    let projection = h.install_current().await;
    let current = projection
        .parent(Uuid::from_u128(2), h.parent.manifest().work.parent)
        .unwrap();
    let workspace = h
        .delivery
        .workspace(&projection, Uuid::from_u128(2))
        .unwrap();
    let gate = current
        .fresh_gate(
            None,
            GatePurpose::NewOperation,
            &DependencyOutcomes::default(),
            Utc::now(),
            Duration::seconds(30),
        )
        .unwrap();
    let identity = canonical_digest(
        1,
        &(
            Uuid::from_u128(2),
            gate.work(),
            h.parent.token(),
            h.acceptance.digest(),
        ),
    )
    .unwrap();
    let plan = crate::preparation::metadata_publication_attempt(
        "no_release_required",
        h.lease.runtime_id,
        &identity,
        &gate.work_key().unwrap(),
        &h.built.runtime.control.operations().unwrap(),
        &workspace.configuration.runtime,
        Utc::now(),
    )
    .unwrap();
    let record = evidence::prepare_no_release_required(
        NoReleaseRequired {
            schema_version: 1,
            runtime_id: h.lease.runtime_id,
            operation_id: plan.admission,
            work: gate.work().clone(),
            evidence_token: h.parent.token().into(),
            acceptance_digest: h.acceptance.digest().into(),
            baselines: h.parent.manifest().baselines.clone(),
            targets: vec![],
            recorded_at: Utc::now(),
        },
        &h.parent,
        &h.acceptance,
        NoReleaseRequirements {
            current_work: gate.work(),
            pending_correction: false,
            targets: &BTreeMap::new(),
        },
        &h.built.runtime.services.key,
    )
    .unwrap();
    let (parent_publication, acceptance_publication) = h
        .delivery
        .publication_refs(&workspace.configuration, current, &h.parent, &h.acceptance)
        .unwrap();
    let mut prior_budget = plan.reference.clone();
    assert!(prior_budget.max_attempts > 1);
    prior_budget.max_attempts -= 1;
    let original = NoReleaseIntent {
        schema_version: 1,
        publication_operation: plan.operation,
        attempt: prior_budget,
        workspace: Uuid::from_u128(2),
        config_revision: canonical_digest(1, &"prior configuration before admission").unwrap(),
        workflow_digest: gate.workflow_digest().into(),
        parent_publication,
        acceptance_publication,
        contracts: selected_contracts(&workspace, &h.parent).unwrap(),
        record,
    };
    let original_scope = original.scope().unwrap();
    h.delivery
        .records
        .write(
            &intent_leaf(plan.admission),
            &h.built
                .runtime
                .services
                .key
                .sign("runtime_no_release_intent", &original)
                .unwrap(),
        )
        .unwrap();
    assert_eq!(
        h.built
            .runtime
            .control
            .operation(plan.admission)
            .unwrap_err()
            .code,
        "record_missing"
    );
    assert_eq!(
        h.built
            .runtime
            .control
            .operation(plan.operation)
            .unwrap_err()
            .code,
        "record_missing"
    );
    assert!(
        h.delivery
            .advance_no_release(
                &h.lease,
                &projection,
                Uuid::from_u128(2),
                h.parent.manifest().work.parent
            )
            .await
            .unwrap()
    );
    assert_eq!(h.writes(), 3);
    let current = h.delivery.load_no_release_intent(plan.admission).unwrap();
    assert_eq!(current.config_revision, gate.config_revision());
    assert_eq!(
        canonical_digest(1, &current.attempt).unwrap(),
        canonical_digest(1, &original.attempt).unwrap()
    );
    assert_eq!(current.effect().unwrap(), original.effect().unwrap());
    let admission = h.built.runtime.control.operation(plan.admission).unwrap();
    let actual_budget = crate::control::MetadataReference::from_receipt(&admission)
        .unwrap()
        .unwrap();
    assert_eq!(actual_budget.max_attempts, original.attempt.max_attempts);
    assert_eq!(actual_budget.attempt, original.attempt.attempt);
    assert_ne!(current.scope().unwrap(), original_scope);
    assert!(
        h.delivery
            .records
            .records_bounded(65536)
            .unwrap()
            .into_iter()
            .filter(|leaf| leaf != &intent_leaf(plan.admission))
            .any(|leaf| h
                .delivery
                .records
                .read::<Signed<NoReleaseIntent>>(&leaf)
                .is_ok_and(|prior| prior.payload.scope().unwrap() == original_scope)),
        "superseded unadmitted intent remains immutable history"
    );
}

#[tokio::test]
async fn orchestration_owner_lost_after_admission_cannot_send_publication() {
    let h = Harness::new().await;
    let projection = h.install_current().await;
    let control = h.built.runtime.control.clone();
    let owner = h.owner.clone();
    let revoked = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let observed = revoked.clone();
    h.wire.lock().unwrap().before_issue_read = Some(Arc::new(move || {
        if !control.snapshot()?.active_operations.is_empty() {
            owner.revoke_orchestration();
            observed.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        Ok(())
    }));
    let error = h
        .delivery
        .advance_no_release(
            &h.lease,
            &projection,
            Uuid::from_u128(2),
            h.parent.manifest().work.parent,
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "ownership_conflict");
    assert!(revoked.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(h.writes(), 2, "preflight owner loss cannot publish");
    assert!(
        h.built
            .runtime
            .control
            .snapshot()
            .unwrap()
            .active_operations
            .is_empty()
    );
    assert!(
        !h.built
            .runtime
            .control
            .operations()
            .unwrap()
            .iter()
            .any(|receipt| receipt.kind == "linear_comment"
                && receipt.expected.contains_key("no_release_digest")
                && receipt.phase == ReceiptPhase::Sent)
    );
}
