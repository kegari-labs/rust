//! Synthetic provider wire; production dispatch, receipts, signatures and recovery.
use super::wire_fixture::*;
use super::*;
use crate::control::Lifecycle;
use crate::control::MutationRequest;
use crate::observations::ObservationCache;
use crate::reconcile::DependencyOutcomes;
use crate::reconcile::GatePurpose;
use kegari_control_api::identity::Actor;
use kegari_storage::PrivateDir;
use kegari_types::control::Lease;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
struct Harness {
    _temp: tempfile::TempDir,
    config: Arc<CacheConfiguration>,
    wire: Arc<Mutex<Fixture>>,
    cache: Arc<ObservationCache>,
    host: Arc<ControlHost>,
    publisher: Arc<CorrectionPublisher>,
    key: Arc<AttestationKey>,
    lease: Lease,
    references: std::path::PathBuf,
}
impl Harness {
    async fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let workflow = root.join("WORKFLOW.md");
        std::fs::write(&workflow,"---\nchecks:\n  local:\n    local:\n      argv: ['/usr/bin/true']\n      result_artifact: '{{ paths.scratch }}/local.json'\n  integrated:\n    e2e:\n      argv: ['/usr/bin/true']\n      result_artifact: '{{ paths.scratch }}/e2e.json'\n---\nFixture correction coordination\n").unwrap();
        let mut config = config();
        let workspace = config.workspaces.get_mut("example").unwrap();
        workspace.workflow = Some(workflow.to_str().unwrap().into());
        workspace.actors.correctors = BTreeSet::from([id(6)]);
        let config = Arc::new(CacheConfiguration(config));
        let wire = Fixture::new();
        let key = Arc::new(AttestationKey::new("fixture".into(), vec![7; 32]).unwrap());
        let host = Arc::new(
            ControlHost::setup(
                PrivateDir::create(&root.join("control")).unwrap(),
                id(1),
                id(700),
            )
            .unwrap(),
        );
        host.lifecycle(
            MutationRequest {
                request_id: id(701),
                runtime_id: id(1),
                actor: Actor {
                    identity: "fixture".into(),
                    principal: None,
                    roles: BTreeSet::from(["lifecycle".into()]),
                },
                kind: "start".into(),
                canonical_targets: vec![format!("runtime:{}", id(1))],
                expected: BTreeMap::new(),
                payload: Value::Null,
            },
            Lifecycle::Start,
            0,
        )
        .unwrap();
        let lease = Lease {
            runtime_id: id(1),
            owner_generation: 1,
            owner_id: id(702),
        };
        host.attach_reconciled(lease.clone()).unwrap();
        let references = root.join("refs");
        let cache = ObservationCache::with_inputs(
            config.clone(),
            Arc::new(CacheSecrets),
            key.clone(),
            PrivateDir::create(&references).unwrap(),
            Arc::new(CacheInputs(wire.clone())),
            host.clone(),
        )
        .await
        .unwrap();
        let cache = Arc::new(cache);
        let first = cache.refresh().await.unwrap();
        let signed = key
            .sign(
                "parent_authorization",
                first
                    .parent(id(2), id(100))
                    .unwrap()
                    .authorization_candidate
                    .as_ref()
                    .unwrap(),
            )
            .unwrap();
        let body =
            crate::evidence::encode_signed_publication("parent_authorization", &signed).unwrap();
        wire.lock().unwrap().comments.insert(
            id(100),
            vec![
                native_json(id(950), id(100), id(5), body),
                native_json(
                    id(960),
                    id(100),
                    id(6),
                    "Please clarify the expected outcome".into(),
                ),
            ],
        );
        cache.refresh().await.unwrap();
        let publisher = Arc::new(CorrectionPublisher::new(CorrectionDependencies {
            configuration: config.clone(),
            control: host.clone(),
            observations: cache.clone(),
            secrets: Arc::new(CacheSecrets),
            key: key.clone(),
            inputs: Arc::new(CacheInputs(wire.clone())),
            owner_check: Arc::new(|_| Ok(())),
            control_owner_check: Arc::new(|| Ok(())),
        }));
        Self {
            _temp: temp,
            config,
            wire,
            cache,
            host,
            publisher,
            key,
            lease,
            references,
        }
    }
    async fn completed(
        &self,
        class: CommentClass,
    ) -> (
        tempfile::TempDir,
        crate::runtime::VerifiedCoordinationOutcome,
    ) {
        let p = self.cache.refresh().await.unwrap();
        let parent = p.parent(id(2), id(100)).unwrap();
        let gate = parent
            .fresh_gate(
                None,
                GatePurpose::Coordination,
                &DependencyOutcomes::default(),
                Utc::now(),
                chrono::Duration::seconds(30),
            )
            .unwrap();
        let source = parent.observations[&id(100)]
            .comments
            .iter()
            .find(|c| c.id == id(960))
            .unwrap();
        let routes = if class == CommentClass::Corrective {
            json!([{"child":id(101),"instructions":"Assert the required error response in tests; preserve existing scope.","validation_expectations":["Run the named local test and record its actual result"]}])
        } else {
            json!([])
        };
        let claim = json!({"kind":"correction_classification","classifications":[{"source_comment":id(960),"source_revision":source.body_digest,"class":class,"rationale":"Interpret the native comment in authorized parent context","routes":routes}]});
        let digest = canonical_digest(1, &claim).unwrap();
        let bytes = hex::decode(digest.rsplit(':').next().unwrap()).unwrap();
        let proposal = Uuid::from_bytes(bytes[..16].try_into().unwrap());
        crate::runtime_claims::test_coordination_fixture_with_proposal(
            gate,
            claim,
            "codex-fixture",
            proposal,
        )
    }
    fn posts(&self) -> Vec<Value> {
        self.wire
            .lock()
            .unwrap()
            .requests
            .iter()
            .filter_map(|(_, v)| {
                v["query"]
                    .as_str()
                    .is_some_and(|q| q.contains("mutation KegariComment"))
                    .then_some(v.clone())
            })
            .collect()
    }
    async fn restart(&self) -> Arc<ObservationCache> {
        ObservationCache::with_inputs(
            self.config.clone(),
            Arc::new(CacheSecrets),
            self.key.clone(),
            PrivateDir::open(&self.references).unwrap(),
            Arc::new(CacheInputs(self.wire.clone())),
            self.host.clone(),
        )
        .await
        .unwrap()
        .into()
    }
}
fn native_json(comment: Uuid, issue: Uuid, writer: Uuid, body: String) -> Value {
    json!({"id":comment,"body":body,"createdAt":"2026-01-04T00:00:00Z","updatedAt":"2026-01-04T00:00:00Z","editedAt":null,"url":format!("https://linear.invalid/comment/{comment}"),"user":{"id":writer},"onBehalfOf":null,"botActor":null,"issue":{"id":issue}})
}
#[tokio::test]
async fn informational_and_ambiguous_publish_no_routes_or_generation() {
    for class in [CommentClass::Informational, CommentClass::Ambiguous] {
        let h = Harness::new().await;
        let before = h
            .cache
            .admission_snapshot()
            .unwrap()
            .parent(id(2), id(100))
            .unwrap()
            .corrections
            .generation()
            .unwrap();
        let (_temp, completed) = h.completed(class).await;
        let result = h.publisher.publish(&h.lease, &completed).await.unwrap();
        assert_eq!(h.posts().len(), 1);
        assert_eq!(result.generation(), before);
        assert_eq!(result.watermark(), 0);
        assert_eq!(
            !result.remaining_pending().is_empty(),
            class == CommentClass::Ambiguous
        );
        let restarted = h.restart().await;
        let p = restarted.refresh().await.unwrap();
        let parent = p.parent(id(2), id(100)).unwrap();
        assert_eq!(parent.corrections.generation().unwrap(), before);
        assert_eq!(
            !parent.corrections.pending_ids().is_empty(),
            class == CommentClass::Ambiguous
        );
        h.publisher.publish(&h.lease, &completed).await.unwrap();
        assert_eq!(h.posts().len(), 1);
    }
}
#[tokio::test]
async fn corrective_routes_precede_final_and_recover_without_new_generation() {
    let h = Harness::new().await;
    let (_temp, completed) = h.completed(CommentClass::Corrective).await;
    let result = h.publisher.publish(&h.lease, &completed).await.unwrap();
    let posts = h.posts();
    assert_eq!(posts.len(), 2);
    assert_eq!(posts[0]["variables"]["input"]["issueId"], json!(id(101)));
    assert!(
        posts[0]["variables"]["input"]["body"]
            .as_str()
            .unwrap()
            .starts_with("<!-- kegari:v0:correction_route")
    );
    assert_eq!(posts[1]["variables"]["input"]["issueId"], json!(id(100)));
    assert!(result.remaining_pending().is_empty());
    assert_eq!(result.watermark(), 1);
    let restored = h.restart().await.refresh().await.unwrap();
    let parent = restored.parent(id(2), id(100)).unwrap();
    assert!(parent.corrections.pending_ids().is_empty());
    assert_eq!(
        parent.corrections.generation().unwrap(),
        result.generation()
    );
    assert!(parent.blockers.is_empty(), "{:?}", parent.blockers);
    let repeat = h.publisher.publish(&h.lease, &completed).await.unwrap();
    assert_eq!(repeat.digest(), result.digest());
    assert_eq!(h.posts().len(), 2);
}
#[tokio::test]
async fn changed_or_missing_native_route_or_final_never_clears_after_restart() {
    for variant in [
        "route_edit",
        "route_delete",
        "final_edit",
        "final_delete",
        "source_edit",
        "writer",
    ] {
        let h = Harness::new().await;
        let (_temp, completed) = h.completed(CommentClass::Corrective).await;
        h.publisher.publish(&h.lease, &completed).await.unwrap();
        {
            let mut wire = h.wire.lock().unwrap();
            match variant {
                "route_edit" => {
                    wire.comments.get_mut(&id(101)).unwrap()[0]["body"] = json!("changed route")
                }
                "route_delete" => wire.comments.get_mut(&id(101)).unwrap().clear(),
                "final_edit" => {
                    wire.comments.get_mut(&id(100)).unwrap().last_mut().unwrap()["body"] =
                        json!("changed final")
                }
                "final_delete" => {
                    wire.comments.get_mut(&id(100)).unwrap().pop();
                }
                "source_edit" => {
                    let source = wire
                        .comments
                        .get_mut(&id(100))
                        .unwrap()
                        .iter_mut()
                        .find(|v| v["id"] == json!(id(960)))
                        .unwrap();
                    source["body"] = json!("Change again");
                    source["editedAt"] = json!("2026-01-05T00:00:00Z");
                }
                "writer" => {
                    wire.comments.get_mut(&id(101)).unwrap()[0]["user"]["id"] = json!(id(6))
                }
                _ => unreachable!(),
            }
        }
        let restored = h.restart().await.refresh().await.unwrap();
        let parent = restored.parent(id(2), id(100)).unwrap();
        assert!(
            !parent.corrections.pending_ids().is_empty() || !parent.blockers.is_empty(),
            "{variant}"
        );
        assert!(
            parent
                .fresh_gate(
                    Some(id(101)),
                    GatePurpose::NewOperation,
                    &DependencyOutcomes::default(),
                    Utc::now(),
                    chrono::Duration::seconds(30)
                )
                .is_err(),
            "{variant}"
        );
        assert!(
            h.publisher.publish(&h.lease, &completed).await.is_err(),
            "{variant}"
        );
        assert_eq!(h.posts().len(), 2);
    }
}
#[tokio::test]
async fn lost_route_response_is_observed_without_retransmission() {
    let h = Harness::new().await;
    let (_temp, completed) = h.completed(CommentClass::Corrective).await;
    h.wire.lock().unwrap().comment_response_lost = true;
    assert!(h.publisher.publish(&h.lease, &completed).await.is_err());
    assert_eq!(h.posts().len(), 1);
    h.wire.lock().unwrap().comment_response_lost = false;
    let recovered = h.publisher.publish(&h.lease, &completed).await.unwrap();
    assert!(recovered.remaining_pending().is_empty());
    assert_eq!(h.posts().len(), 2);
}
#[test]
fn proposal_contract_requires_exact_source_and_actionable_validation() {
    let base = json!({"kind":"correction_classification","classifications":[{"source_comment":id(960),"source_revision":bytes_digest(b"native"),"class":"corrective","rationale":"Fix bounded error behavior","routes":[{"child":id(101),"instructions":"Assert error response","validation_expectations":["Named local test passes"]}]}]});
    validate_proposal(&base).unwrap();
    for change in [
        "unknown",
        "empty_validation",
        "no_routes",
        "informational_route",
        "duplicate",
        "missing_source",
    ] {
        let mut proposal = base.clone();
        match change {
            "unknown" => proposal["classifications"][0]["trust_me"] = json!(true),
            "empty_validation" => {
                proposal["classifications"][0]["routes"][0]["validation_expectations"] = json!([])
            }
            "no_routes" => proposal["classifications"][0]["routes"] = json!([]),
            "informational_route" => {
                proposal["classifications"][0]["class"] = json!("informational")
            }
            "duplicate" => {
                let item = proposal["classifications"][0].clone();
                proposal["classifications"]
                    .as_array_mut()
                    .unwrap()
                    .push(item);
            }
            "missing_source" => proposal["classifications"][0]["source_revision"] = json!(""),
            _ => unreachable!(),
        }
        assert!(validate_proposal(&proposal).is_err(), "{change}");
    }
}
#[tokio::test]
async fn scope_expansion_and_stale_source_fail_before_any_publication() {
    for variant in ["child", "source"] {
        let h = Harness::new().await;
        let (_temp, original) = h.completed(CommentClass::Corrective).await;
        let mut claim = original.outcome().claim.clone();
        if variant == "child" {
            claim["classifications"][0]["routes"][0]["child"] = json!(id(999));
        } else {
            claim["classifications"][0]["source_revision"] = json!(bytes_digest(b"wrong source"));
        }
        let (_other, completed) = crate::runtime_claims::test_coordination_fixture(
            original.original_authority().clone(),
            claim,
            "codex-fixture",
        );
        assert!(h.publisher.publish(&h.lease, &completed).await.is_err());
        assert!(h.posts().is_empty());
    }
}

#[tokio::test]
async fn changed_native_revision_after_completion_invalidates_classifier() {
    for edit in ["same_body_timestamp", "new_pending"] {
        let h = Harness::new().await;
        let (_temp, completed) = h.completed(CommentClass::Corrective).await;
        {
            let mut wire = h.wire.lock().unwrap();
            if edit == "same_body_timestamp" {
                let source = wire
                    .comments
                    .get_mut(&id(100))
                    .unwrap()
                    .iter_mut()
                    .find(|c| c["id"] == json!(id(960)))
                    .unwrap();
                source["editedAt"] = json!("2026-01-05T00:00:00Z");
            } else {
                wire.comments.get_mut(&id(100)).unwrap().push(native_json(
                    id(961),
                    id(100),
                    id(6),
                    "Additional clarification".into(),
                ));
            }
        }
        assert!(
            h.publisher.publish(&h.lease, &completed).await.is_err(),
            "{edit}"
        );
        assert!(h.posts().is_empty());
    }
}
#[tokio::test]
async fn duplicate_or_changed_route_prevents_final_intake() {
    for edit in [false, true] {
        let h = Harness::new().await;
        let (_temp, completed) = h.completed(CommentClass::Corrective).await;
        if edit {
            h.wire.lock().unwrap().edit_route_on_read = true;
        } else {
            h.wire.lock().unwrap().duplicate_publication = true;
        }
        assert!(h.publisher.publish(&h.lease, &completed).await.is_err());
        assert_eq!(h.posts().len(), 1);
        assert!(
            h.posts()[0]["variables"]["input"]["body"]
                .as_str()
                .unwrap()
                .contains("correction_route")
        );
        assert!(h.publisher.publish(&h.lease, &completed).await.is_err());
        assert_eq!(h.posts().len(), 1);
    }
}
#[tokio::test]
async fn unknown_final_can_be_observed_while_stopped_without_resending() {
    let h = Harness::new().await;
    let (_temp, completed) = h.completed(CommentClass::Informational).await;
    h.wire.lock().unwrap().comment_response_lost = true;
    assert!(h.publisher.publish(&h.lease, &completed).await.is_err());
    h.wire.lock().unwrap().comment_response_lost = false;
    let generation = h.host.snapshot().unwrap().intent.generation;
    h.host
        .lifecycle(
            MutationRequest {
                request_id: id(703),
                runtime_id: id(1),
                actor: Actor {
                    identity: "fixture".into(),
                    principal: None,
                    roles: BTreeSet::from(["lifecycle".into()]),
                },
                kind: "stop".into(),
                canonical_targets: vec![format!("runtime:{}", id(1))],
                expected: BTreeMap::new(),
                payload: Value::Null,
            },
            Lifecycle::Stop,
            generation,
        )
        .unwrap();
    let recovered = h.publisher.recover(&completed).await.unwrap().unwrap();
    assert!(recovered.remaining_pending().is_empty());
    assert_eq!(h.posts().len(), 1);
}

#[tokio::test]
async fn stopped_prefix_recovery_observes_routes_without_publishing_missing_final() {
    let h = Harness::new().await;
    let (_temp, completed) = h.completed(CommentClass::Corrective).await;
    assert!(h.publisher.recover(&completed).await.unwrap().is_none());
    assert!(h.posts().is_empty());
    h.wire.lock().unwrap().comment_response_lost = true;
    assert!(h.publisher.publish(&h.lease, &completed).await.is_err());
    h.wire.lock().unwrap().comment_response_lost = false;
    let generation = h.host.snapshot().unwrap().intent.generation;
    h.host
        .lifecycle(
            MutationRequest {
                request_id: id(704),
                runtime_id: id(1),
                actor: Actor {
                    identity: "fixture".into(),
                    principal: None,
                    roles: BTreeSet::from(["lifecycle".into()]),
                },
                kind: "stop".into(),
                canonical_targets: vec![format!("runtime:{}", id(1))],
                expected: BTreeMap::new(),
                payload: Value::Null,
            },
            Lifecycle::Stop,
            generation,
        )
        .unwrap();
    let progress = h.publisher.recover_progress(&completed).await.unwrap();
    assert!(progress.publication().is_none());
    assert_eq!(progress.stages().len(), 1);
    assert_eq!(progress.stage_limit(), 2);
    let fingerprint = progress.fingerprint().unwrap();
    assert_eq!(
        h.publisher
            .recover_progress(&completed)
            .await
            .unwrap()
            .fingerprint()
            .unwrap(),
        fingerprint
    );
    assert_eq!(h.posts().len(), 1);
    let operation = operation_id(
        id(1),
        completed.outcome().proposal_id,
        "route",
        Some(id(960)),
        Some(id(101)),
    )
    .unwrap();
    assert_eq!(
        h.host.operation(operation).unwrap().phase,
        ReceiptPhase::Reconciled
    );
    let observed = h.cache.admission_snapshot().unwrap();
    assert_eq!(
        observed
            .parent(id(2), id(100))
            .unwrap()
            .corrections
            .pending_ids(),
        vec![id(960)]
    );
    assert!(h.publisher.publish(&h.lease, &completed).await.is_err());
    assert_eq!(h.posts().len(), 1);
}

async fn admitted_child(
    h: &Harness,
) -> (
    crate::observations::ActiveSnapshot,
    crate::reconcile::FreshGate,
) {
    let (_temp, informational) = h.completed(CommentClass::Informational).await;
    h.publisher.publish(&h.lease, &informational).await.unwrap();
    let projection = h.cache.refresh().await.unwrap();
    let parent = projection.parent(id(2), id(100)).unwrap();
    let gate = parent
        .fresh_gate(
            Some(id(101)),
            GatePurpose::NewOperation,
            &DependencyOutcomes::default(),
            Utc::now(),
            chrono::Duration::seconds(30),
        )
        .unwrap();
    let active = h
        .cache
        .capture_active(id(2), gate.config_revision(), gate.workflow_digest())
        .unwrap();
    (active, gate)
}
fn edit_correction(h: &Harness, body: &str, day: u32) {
    let mut wire = h.wire.lock().unwrap();
    let source = wire
        .comments
        .get_mut(&id(100))
        .unwrap()
        .iter_mut()
        .find(|value| value["id"] == json!(id(960)))
        .unwrap();
    source["body"] = json!(body);
    source["editedAt"] = json!(format!("2026-01-{day:02}T00:00:00Z"));
}
#[tokio::test]
async fn correction_transition_retains_original_gate_and_exact_signed_predecessor_chain() {
    let h = Harness::new().await;
    let (_active, admitted) = admitted_child(&h).await;
    let original = admitted.digest().unwrap();
    edit_correction(&h, "Assert the bounded error response", 5);
    let (_temp, correction) = h.completed(CommentClass::Corrective).await;
    h.publisher.publish(&h.lease, &correction).await.unwrap();
    let transition = Arc::new(
        h.publisher
            .transition(&admitted, None, &DependencyOutcomes::default())
            .await
            .unwrap(),
    );
    assert_eq!(transition.watermark(), 1);
    assert_eq!(transition.original_authority().digest().unwrap(), original);
    assert_ne!(transition.original_work(), transition.effective_work());
    assert!(
        transition
            .input()
            .contains("Assert the required error response")
    );
    assert!(
        h.cache
            .active_gate(&admitted, &DependencyOutcomes::default())
            .is_err()
    );
    assert!(
        h.cache
            .active_gate(
                transition.current_authority(),
                &DependencyOutcomes::default()
            )
            .is_ok()
    );
    let posts = h.posts().len();
    // Polling the same history does not create another effective work epoch.
    assert!(
        h.publisher
            .transition(
                &admitted,
                Some(transition.clone()),
                &DependencyOutcomes::default()
            )
            .await
            .is_err()
    );
    assert_eq!(h.posts().len(), posts);

    edit_correction(&h, "Also assert the new follow-up error", 6);
    let (_temp, correction) = h.completed(CommentClass::Corrective).await;
    h.publisher.publish(&h.lease, &correction).await.unwrap();
    let next = h
        .publisher
        .transition(
            &admitted,
            Some(transition.clone()),
            &DependencyOutcomes::default(),
        )
        .await
        .unwrap();
    assert_eq!(next.watermark(), 2);
    assert_eq!(next.previous().unwrap().digest(), transition.digest());
    assert_eq!(next.original_authority().digest().unwrap(), original);
    assert!(
        h.cache
            .active_gate(
                transition.current_authority(),
                &DependencyOutcomes::default()
            )
            .is_err()
    );
    assert_eq!(next.current_authority().profile(), admitted.profile());
    assert_eq!(
        next.current_authority().child_scope_digest(),
        admitted.child_scope_digest()
    );
    assert_eq!(
        next.current_authority().dependency_digest(),
        admitted.dependency_digest()
    );
    let input: Value = serde_json::from_str(next.input()).unwrap();
    assert_eq!(input["classifications"].as_array().unwrap().len(), 1);
    assert_eq!(
        input["classifications"][0]["source_revision"],
        json!(bytes_digest(b"Also assert the new follow-up error"))
    );
}
#[tokio::test]
async fn correction_transition_rejects_edited_native_routes_and_racing_revisions() {
    for route_edit in [false, true] {
        let h = Harness::new().await;
        let (_active, admitted) = admitted_child(&h).await;
        edit_correction(&h, "Assert corrected behavior", 5);
        let (_temp, correction) = h.completed(CommentClass::Corrective).await;
        h.publisher.publish(&h.lease, &correction).await.unwrap();
        let routes = h.publisher.current_routes(&admitted).await.unwrap();
        let posts = h.posts().len();
        if route_edit {
            h.wire.lock().unwrap().comments.get_mut(&id(101)).unwrap()[0]["body"] =
                json!("altered published instruction");
        } else {
            edit_correction(&h, "A newer unclassified correction", 6);
        }
        h.cache.refresh().await.unwrap();
        assert!(
            h.cache
                .active_corrected_gate(&admitted, &routes, &DependencyOutcomes::default())
                .is_err()
        );
        assert!(
            h.publisher
                .transition(&admitted, None, &DependencyOutcomes::default())
                .await
                .is_err()
        );
        assert_eq!(h.posts().len(), posts);
    }
}

/// Exact native signed correction proof for execution boundary regressions.
pub(crate) async fn test_transition_fixture()
-> (tempfile::TempDir, Arc<VerifiedCorrectionTransition>) {
    let h = Harness::new().await;
    let (_active, admitted) = admitted_child(&h).await;
    edit_correction(&h, "Assert the corrected bounded behavior", 5);
    let (_temp, correction) = h.completed(CommentClass::Corrective).await;
    h.publisher.publish(&h.lease, &correction).await.unwrap();
    let transition = h
        .publisher
        .transition(&admitted, None, &DependencyOutcomes::default())
        .await
        .unwrap();
    (h._temp, Arc::new(transition))
}

pub(crate) async fn test_pending_transition_fixture() -> (
    tempfile::TempDir,
    Arc<VerifiedCorrectionTransition>,
    crate::reconcile::ParentProjection,
) {
    let h = Harness::new().await;
    let (_active, admitted) = admitted_child(&h).await;
    edit_correction(&h, "Assert the corrected bounded behavior", 5);
    let (_temp, correction) = h.completed(CommentClass::Corrective).await;
    h.publisher.publish(&h.lease, &correction).await.unwrap();
    let transition = Arc::new(
        h.publisher
            .transition(&admitted, None, &DependencyOutcomes::default())
            .await
            .unwrap(),
    );
    edit_correction(&h, "A second revision needs classification", 6);
    h.cache.refresh().await.unwrap();
    let (_, parent) = h
        .cache
        .active_correction_context(transition.current_authority())
        .unwrap();
    (h._temp, transition, parent)
}
