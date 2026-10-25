//! Historical accepted input is explicitly synthetic. Actual owner receipts,
//! supersede authorization, native wire, object import and assembly are exercised.
use super::*;
use crate::decisions::{
    AuthorizedDecision, AuthorizedOperationRecovery, DecisionEvidence, RetryTarget,
    VerifiedExplicitCorrection, VerifiedRetryGrant,
};
use crate::{control::Receipt, digest::canonical_digest};

pub(super) struct ChainFixture<'a> {
    pub delivery: &'a Arc<RuntimeDelivery>,
    pub built: &'a crate::runtime_fixture::RuntimeFixture,
    pub data: &'a Arc<std::sync::Mutex<wire::Fixture>>,
    pub lease: &'a Lease,
    pub source: &'a std::path::Path,
    pub github: &'a RuntimeSeedWire,
    pub native: &'a Arc<RuntimeLineageWire>,
    pub seed: &'a RuntimeCandidateSeed,
    pub held: &'a AttemptRecord,
    pub gate: &'a FreshGate,
    pub merged: &'a crate::git::Revision,
}

pub(super) fn fixture_phase<'a>(
    delivery: &'a RuntimeDelivery,
    workspace: &'a WorkspaceDelivery,
    gate: &'a FreshGate,
    lease: &'a Lease,
    revision: &'a crate::git::Revision,
    phase: DeliveryPhase,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ActivePhase> + 'a>> {
    Box::pin(async move {
        let contract = &workspace.contracts[0];
        delivery
            .admit_delivery_phase(
                workspace,
                gate,
                lease,
                PhaseRequest {
                    operation: Uuid::new_v4(),
                    scope: DeliveryScope {
                        schema_version: 1,
                        runtime_id: lease.runtime_id,
                        phase,
                        work: gate.work().clone(),
                        repository: contract.repository.clone(),
                        revision: evidence::RevisionBinding {
                            repository: contract.repository.clone(),
                            head: revision.commit.clone(),
                            base: revision.commit.clone(),
                            tested_commit: revision.commit.clone(),
                            tested_tree: revision.tree.clone(),
                        },
                        fresh_gate_digest: gate.digest().unwrap(),
                        contract_digest: contract.digest().unwrap(),
                        candidate_vector_digest: Some(
                            canonical_digest(1, &"explicit fixture initial target").unwrap(),
                        ),
                    },
                },
            )
            .await
            .unwrap()
    })
}
pub(super) async fn publish_origin(
    delivery: &RuntimeDelivery,
    workspace: &WorkspaceDelivery,
    gate: &FreshGate,
    lease: &Lease,
    manifest: evidence::ParentEvidence,
    slice: &IsolatedSlice,
    targets: &[evidence::ProductionTargetBaseline],
) -> (
    evidence::ParentEvidence,
    Signed<evidence::Acceptance>,
    Vec<PublicationRef>,
) {
    let permit = delivery
        .runtime
        .control
        .admit(
            lease,
            manifest.publication_operation,
            gate.work_key().unwrap(),
            canonical_digest(1, &"synthetic upstream publication").unwrap(),
            || (delivery.current_gate())(gate),
        )
        .unwrap();
    let writer = wire::id(5).to_string();
    let actor = format!("owner:{}", lease.owner_id);
    let accepted = delivery
        .runtime
        .services
        .key
        .sign(
            "human_acceptance",
            &evidence::Acceptance {
                schema_version: 1,
                runtime_id: lease.runtime_id,
                operation_id: Uuid::new_v4(),
                work: manifest.work.clone(),
                reviewed_token: manifest.token().unwrap(),
                control_actor: actor.clone(),
                linear_actor: writer.clone(),
                provider_writer: writer,
                accepted_at: Utc::now(),
            },
        )
        .unwrap();
    let body = workspace
        .coordinator
        .fixture_original_manifest_body(&manifest, slice, targets, lease.runtime_id)
        .unwrap();
    let mut refs = Vec::new();
    for (id, body) in [
        (Uuid::new_v4(), body),
        (
            accepted.payload.operation_id,
            evidence::encode_signed_publication("human_acceptance", &accepted).unwrap(),
        ),
    ] {
        let current_gate = delivery.current_gate();
        let admitted_gate = gate.clone();
        let r = delivery
            .provider_broker(&workspace.configuration, gate.workspace_id())
            .unwrap()
            .mutate(
                delivery.runtime.control.clone(),
                crate::control::MutationRequest {
                    request_id: id,
                    runtime_id: lease.runtime_id,
                    actor: crate::control::Actor {
                        identity: actor.clone(),
                        principal: None,
                        roles: BTreeSet::new(),
                    },
                    kind: String::new(),
                    canonical_targets: vec![],
                    expected: BTreeMap::new(),
                    payload: Value::Null,
                },
                Some(permit.clone()),
                None,
                crate::providers::ProviderMutation::LinearComment {
                    issue: gate.work().parent,
                    body: body.clone(),
                },
                move || current_gate(&admitted_gate),
            )
            .await
            .unwrap();
        refs.push(PublicationRef {
            id: r.remote_ids["comment_id"].parse().unwrap(),
            body_digest: crate::digest::bytes_digest(body.as_bytes()),
            writer: wire::id(5),
        });
    }
    delivery
        .runtime
        .control
        .retire_local(
            permit.operation_id,
            json!({"synthetic_upstream_publication":true}),
        )
        .unwrap();
    (manifest, accepted, refs)
}

pub(super) fn chained_after_merge(
    f: ChainFixture<'_>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + '_>> {
    Box::pin(async move {
        eprintln!("lineage fixture: A merge observed; acquiring B ancestry");
        let snapshot = f.built.runtime.observations.refresh().await.unwrap();
        let workspace = f.delivery.workspace(&snapshot, wire::id(2)).unwrap();
        let phase = fixture_phase(
            f.delivery,
            &workspace,
            f.gate,
            f.lease,
            f.merged,
            DeliveryPhase::CandidateAssembly,
        )
        .await;
        let predecessor = f
            .delivery
            .known_predecessor_baseline(&snapshot, f.gate)
            .await
            .unwrap()
            .unwrap();
        let integrations = phase
            .coordinator
            .acquire_successor_integrations(&phase.admission, predecessor)
            .await
            .unwrap();
        assert_eq!(integrations.len(), 1);
        assert_eq!(
            integrations[0].record().release_operation,
            f.held.attempt.plan.operation_id
        );
        let root = f.source.parent().unwrap().join("lineage-b");
        crate::storage::PrivateDir::create(&root).unwrap();
        let slice = phase
            .coordinator
            .assemble_isolated_slice(
                &phase.admission,
                SliceAssembly {
                    execution_root: &root,
                    staged: &[],
                    required_children: phase.admission.fresh_gate().required_children(),
                    child_dependencies: &BTreeMap::new(),
                    satisfied_children: &BTreeSet::new(),
                    production_integrations: &integrations,
                    identity: crate::git::CommitIdentity {
                        name: "Fixture".into(),
                        email: "fixture@example.test".into(),
                    },
                },
            )
            .await
            .unwrap();
        assert!(slice.manifest().ordered_delta_digests.is_empty());
        assert_eq!(slice.manifest().result.as_ref(), Some(f.merged));
        f.delivery
            .retire_delivery_phase(&phase, json!({"fixture":"actual no-replay B assembly"}))
            .unwrap();
        let snapshot = f.built.runtime.observations.refresh().await.unwrap();
        let predecessor = f
            .delivery
            .known_predecessor_baseline(&snapshot, f.gate)
            .await
            .unwrap()
            .unwrap();
        let observer = workspace.observer.with_successor(predecessor).unwrap();
        let baseline = observer.observe(&workspace.contracts[0]).await.unwrap();
        let mut manifest = f.seed.parent.manifest().clone();
        manifest.work = f.gate.work().clone();
        manifest.publication_operation = Uuid::new_v4();
        manifest.created_at = Utc::now();
        manifest.candidates = vec![slice.manifest().clone()];
        manifest.baselines = vec![baseline.baseline().clone()];
        manifest.production_targets = baseline
            .targets()
            .iter()
            .map(|t| t.target.clone())
            .collect();
        manifest.integrated_results[0].candidate_vector_digest =
            manifest.candidate_vector_digest().unwrap();
        let (manifest, acceptance, refs) = publish_origin(
            f.delivery,
            &workspace,
            f.gate,
            f.lease,
            manifest,
            &slice,
            baseline.targets(),
        )
        .await;
        let mut plan = f.held.attempt.plan.clone();
        plan.operation_id = Uuid::new_v4();
        plan.generation = f.gate.work().generation.clone();
        plan.evidence_token = manifest.token().unwrap();
        plan.acceptance_digest = acceptance.digest().unwrap();
        plan.steps = vec![domain::ReleaseStep {
            id: "deploy".into(),
            depends_on: BTreeSet::new(),
            kind: domain::StepKind::DeployExisting {
                repository: workspace.contracts[0].repository.clone(),
                target: domain::ResourceKey::Target {
                    provider: "github".into(),
                    id: "service-production".into(),
                },
                workflow_id: 3,
                workflow_revision: "1".repeat(40),
                dispatch_ref: "trusted".into(),
                source_input: "source_sha".into(),
                source: f.merged.clone(),
                expected_artifact: None,
            },
        }];
        plan.steps.push(domain::ReleaseStep {
            id: "verify".into(),
            depends_on: ["deploy".into()].into(),
            kind: domain::StepKind::VerifyDeployed {
                repository: workspace.contracts[0].repository.clone(),
                target: domain::ResourceKey::Target {
                    provider: "github".into(),
                    id: "service-production".into(),
                },
                source: f.merged.clone(),
                deployment_step: "deploy".into(),
                expected_artifact: None,
            },
        });
        plan.validate().unwrap();
        let permit = f
            .built
            .runtime
            .control
            .admit(
                f.lease,
                plan.operation_id,
                f.gate.work_key().unwrap(),
                plan.digest().unwrap(),
                || (f.delivery.current_gate())(f.gate),
            )
            .unwrap();
        f.built.runtime.control.fence(plan.operation_id).unwrap();
        f.built
            .runtime
            .control
            .retire_local(
                plan.operation_id,
                json!({"fixture":"B interrupted before deploy"}),
            )
            .unwrap();
        let mut held = super::super::tests::record(plan, f.gate.work().clone());
        held.attempt.owner_generation = f
            .built
            .runtime
            .control
            .operation(permit.operation_id)
            .unwrap()
            .owner_generation;
        held.attempt.control_generation = permit.control_generation;
        held.attempt.steps = held
            .attempt
            .plan
            .steps
            .iter()
            .map(|step| (step.id.clone(), StepState::NotStarted))
            .collect();
        held.attempt.hold_reason = Some("Fixture B deployment-only interrupted".into());
        held.local_retired = true;
        held.parent_publication = refs[0].clone();
        held.acceptance_publication = refs[1].clone();
        held.config_revision = f.gate.config_revision().into();
        held.workflow_digest = f.gate.workflow_digest().into();
        f.delivery.save_release(&held).unwrap();
        authorize_link(f.delivery, f.held, &held, acceptance.payload.accepted_at).await;
        eprintln!("lineage fixture: actual A-to-B authorization retained; restarting for C");
        let fresh = RuntimeDelivery::with_fixture(
            f.built.runtime.clone(),
            f.delivery.processes.clone(),
            f.delivery.git_path().unwrap(),
        )
        .unwrap();
        let corrected = correct_parent(f.built, f.data, f.lease).await;
        let projection = f.built.runtime.observations.refresh().await.unwrap();
        let predecessor = fresh
            .known_predecessor_baseline(&projection, &corrected)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(predecessor.predecessor(), held.attempt.plan.operation_id);
        assert_eq!(predecessor.effects().count(), 0);
        assert_eq!(predecessor.integration_sources().count(), 1);
        let workspace = fresh.workspace(&projection, wire::id(2)).unwrap();
        let phase = fixture_phase(
            &fresh,
            &workspace,
            &corrected,
            f.lease,
            f.merged,
            DeliveryPhase::CandidateAssembly,
        )
        .await;
        let integrations = phase
            .coordinator
            .acquire_successor_integrations(&phase.admission, predecessor.clone())
            .await
            .unwrap();
        assert_eq!(integrations.len(), 1);
        assert_eq!(
            integrations[0].record().release_operation,
            f.held.attempt.plan.operation_id
        );
        eprintln!("lineage fixture: original A integration recovered through held B");
        let root = f.source.parent().unwrap().join("lineage-c");
        crate::storage::PrivateDir::create(&root).unwrap();
        let slice = phase
            .coordinator
            .assemble_isolated_slice(
                &phase.admission,
                SliceAssembly {
                    execution_root: &root,
                    staged: &[],
                    required_children: phase.admission.fresh_gate().required_children(),
                    child_dependencies: &BTreeMap::new(),
                    satisfied_children: &BTreeSet::new(),
                    production_integrations: &integrations,
                    identity: crate::git::CommitIdentity {
                        name: "Fixture".into(),
                        email: "fixture@example.test".into(),
                    },
                },
            )
            .await
            .unwrap();
        assert!(slice.manifest().ordered_delta_digests.is_empty());
        assert_eq!(slice.manifest().production_integrations.len(), 1);
        assert_eq!(slice.manifest().result.as_ref(), Some(f.merged));
        let projection = f.built.runtime.observations.refresh().await.unwrap();
        let predecessor = fresh
            .known_predecessor_baseline(&projection, &corrected)
            .await
            .unwrap()
            .unwrap();
        let observer = workspace
            .observer
            .with_successor(predecessor.clone())
            .unwrap();
        let baseline = observer.observe(&workspace.contracts[0]).await.unwrap();
        let parent = projection.parent(wire::id(2), wire::id(100)).unwrap();
        let contract = phase
            .coordinator
            .parent_contract(corrected.clone(), parent, &workspace.workflow)
            .unwrap();
        let vector = phase
            .coordinator
            .prepare_parent_vector(&contract, &observer, &[&slice], &[&baseline])
            .await
            .unwrap();
        fresh
            .retire_delivery_phase(&phase, json!({"fixture":"C exact contribution assembly"}))
            .unwrap();
        let e2e = parent
            .fresh_gate(
                Some(wire::id(101)),
                GatePurpose::NewOperation,
                &f.built.runtime.dependency_outcomes().unwrap(),
                Utc::now(),
                Duration::seconds(30),
            )
            .unwrap();
        assert_eq!(
            workspace
                .coordinator
                .verify_integrated_checks(&contract, &e2e, &vector, &observer, &[])
                .await
                .err()
                .unwrap()
                .code,
            "integrated_gate"
        );
        assert!(
            fresh
                .parent_sources(wire::id(2), wire::id(100))
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            fresh
                .release_attempts()
                .unwrap()
                .iter()
                .all(|a| a.hold_reason.is_some())
        );
        // A final explicit C assignment is upstream fixture input, not a release
        // admission: genuine integrated evidence above is still missing. Exercise
        // only the queue's exact authenticated lineage projection.
        let mut c = held.clone();
        c.work = corrected.work().clone();
        c.attempt.plan.operation_id = Uuid::new_v4();
        c.attempt.plan.generation = corrected.work().generation.clone();
        c.attempt.plan.evidence_token = crate::digest::bytes_digest(b"synthetic C accepted token");
        c.attempt.plan.acceptance_digest = crate::digest::bytes_digest(b"synthetic C acceptance");
        c.admission_operation = c.attempt.plan.operation_id;
        c.config_revision = corrected.config_revision().into();
        c.workflow_digest = corrected.workflow_digest().into();
        authorize_link(&fresh, &held, &c, Utc::now()).await;
        eprintln!("lineage fixture: C integrated gate remains blocked; checking exact hold chain");
        let projection = f.built.runtime.observations.refresh().await.unwrap();
        let predecessor = fresh
            .known_predecessor_baseline(&projection, &corrected)
            .await
            .unwrap()
            .unwrap();
        let proof = fresh
            .successor_holds(&c.attempt.plan, corrected.work(), predecessor.clone())
            .unwrap();
        let queue = fresh.releases.state.lock().unwrap().queue.clone();
        let holds_before = format!("{queue:?}");
        assert_eq!(fresh.release_attempts().unwrap().len(), 2);
        assert_eq!(
            queue
                .eligible(&c.attempt.plan, Utc::now())
                .unwrap_err()
                .code,
            "release_hold"
        );
        queue
            .successor_view(&c.attempt.plan, &proof)
            .unwrap()
            .eligible(&c.attempt.plan, Utc::now())
            .unwrap();
        assert_eq!(format!("{queue:?}"), holds_before);
        let mut unrelated = c.attempt.plan.clone();
        unrelated.operation_id = Uuid::new_v4();
        assert_eq!(
            queue.successor_view(&unrelated, &proof).unwrap_err().code,
            "scope_violation"
        );
        let mut missing = ReleaseQueue::default();
        let original_a: Signed<AttemptRecord> = fresh
            .records
            .read(&attempt_leaf(f.held.attempt.plan.operation_id))
            .unwrap();
        missing.restore_hold(&original_a.payload.attempt).unwrap();
        assert_eq!(
            missing
                .successor_view(&c.attempt.plan, &proof)
                .unwrap_err()
                .code,
            "release_hold"
        );
        let mut unrelated_hold = held.attempt.clone();
        unrelated_hold.plan.operation_id = Uuid::new_v4();
        unrelated_hold.assigned_successor = None;
        let mut with_unrelated = queue.clone();
        with_unrelated.restore_hold(&unrelated_hold).unwrap();
        assert_eq!(
            with_unrelated
                .successor_view(&c.attempt.plan, &proof)
                .unwrap()
                .eligible(&c.attempt.plan, Utc::now())
                .unwrap_err()
                .code,
            "release_hold"
        );
        let mut changed = original_a.payload.clone();
        changed.attempt.hold_reason = Some("tampered historical lineage".into());
        fresh.save_release(&changed).unwrap();
        assert_eq!(predecessor.revalidate().err().unwrap().code, "stale_result");
        fresh.save_release(&original_a.payload).unwrap();
        let original_b: Signed<AttemptRecord> = fresh
            .records
            .read(&attempt_leaf(held.attempt.plan.operation_id))
            .unwrap();
        let mut uncertain = original_b.payload.clone();
        uncertain.attempt.steps.insert(
            "deploy".into(),
            StepState::SentUnknown {
                payload_digest: canonical_digest(1, &"unknown intermediate fixture dispatch")
                    .unwrap(),
            },
        );
        fresh.save_release(&uncertain).unwrap();
        assert!(queue.successor_view(&c.attempt.plan, &proof).is_err());
        assert!(
            fresh
                .known_predecessor_baseline(&projection, &corrected)
                .await
                .is_err()
        );
        fresh.save_release(&original_b.payload).unwrap();
        let intent_file = format!("release-successor-{}.json", held.attempt.plan.operation_id);
        let intent: Signed<Value> = fresh.records.read(&intent_file).unwrap();
        let mut broken = intent.payload.clone();
        broken["predecessor"] = json!(held.attempt.plan.operation_id);
        fresh
            .records
            .write(
                &intent_file,
                &f.built
                    .runtime
                    .services
                    .key
                    .sign("release_successor_intent", &broken)
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(predecessor.revalidate().unwrap_err().code, "stale_result");
        assert!(
            fresh
                .known_predecessor_baseline(&projection, &corrected)
                .await
                .is_err()
        );
        fresh.records.write(&intent_file, &intent).unwrap();
        // Deleting/editing an original native publication invalidates recovery even
        // when every host receipt remains intact.
        let original_comment = {
            let mut data = f.data.lock().unwrap();
            let comment = data
                .comments
                .get_mut(&wire::id(100))
                .unwrap()
                .iter_mut()
                .find(|comment| comment["id"] == json!(f.held.parent_publication.id))
                .unwrap();
            let original = comment.clone();
            comment["body"] = json!("edited original publication");
            original
        };
        assert!(
            fresh
                .known_predecessor_baseline(&projection, &corrected)
                .await
                .is_err()
        );
        {
            let mut data = f.data.lock().unwrap();
            *data
                .comments
                .get_mut(&wire::id(100))
                .unwrap()
                .iter_mut()
                .find(|comment| comment["id"] == json!(f.held.parent_publication.id))
                .unwrap() = original_comment;
        }
        assert!(
            f.built
                .runtime
                .control
                .operation(c.admission_operation)
                .is_err()
        );
        // The only external effects were an initial fixture deployment and A's
        // actual wire merge; C never dispatches without genuine integrated checks.
        assert_eq!(f.github.release_effect_count(), 2);
        let _ = &f.native;
    })
}

struct LinkEvidence {
    delivery: Arc<RuntimeDelivery>,
    original: AttemptRecord,
    child: AttemptRecord,
    accepted_at: chrono::DateTime<Utc>,
}
impl DecisionEvidence for LinkEvidence {
    fn current(&self, key: &IssueKey) -> Result<crate::reconcile::ParentDelivery> {
        self.delivery.runtime.current_delivery(key)
    }
    fn outcomes(&self) -> Result<crate::reconcile::DependencyOutcomes> {
        self.delivery.runtime.dependency_outcomes()
    }
    fn retry_target(&self, _: &WorkIdentity) -> Result<RetryTarget> {
        Err(Error::invalid("fixture only links historical releases"))
    }
    fn published_retry_target(&self, _: &VerifiedRetryGrant) -> Result<RetryTarget> {
        Err(Error::invalid("fixture only links historical releases"))
    }
    fn record_acceptance(&self, _: &IssueKey, _: VerifiedAcceptance) -> Result<()> {
        Err(Error::invalid("fixture only links historical releases"))
    }
    fn record_correction(&self, _: VerifiedExplicitCorrection) -> Result<()> {
        Err(Error::invalid("fixture only links historical releases"))
    }
    fn retry_committed(&self, _: VerifiedRetryGrant) -> Result<()> {
        Err(Error::invalid("fixture only links historical releases"))
    }
    fn release(&self, _: AuthorizedDecision) -> crate::app::HostFuture<'_, Receipt> {
        Box::pin(async { Err(Error::invalid("fixture only links historical releases")) })
    }
    fn reconcile(&self, _: AuthorizedOperationRecovery) -> crate::app::HostFuture<'_, Receipt> {
        Box::pin(async { Err(Error::invalid("fixture only links historical releases")) })
    }
    fn recover_release(&self, decision: AuthorizedDecision) -> crate::app::HostFuture<'_, Receipt> {
        Box::pin(async move {
            self.delivery.fixture_successor_transition(
                &decision,
                &self.original,
                &self.child,
                self.accepted_at,
            )
        })
    }
}
async fn authorize_link(
    delivery: &Arc<RuntimeDelivery>,
    original: &AttemptRecord,
    child: &AttemptRecord,
    accepted_at: chrono::DateTime<Utc>,
) {
    use crate::{
        api::{CommandName, CommandRequest},
        control::{Actor, Principal},
        decisions::{DecisionDependencies, DecisionManager},
    };
    let runtime = &delivery.runtime;
    let services = &runtime.services;
    let manager = DecisionManager::new(DecisionDependencies {
        configuration: services.configuration.clone(),
        control: services.control.clone(),
        observations: services.observations.clone(),
        secrets: services.secrets.clone(),
        key: services.key.clone(),
        evidence: Arc::new(LinkEvidence {
            delivery: delivery.clone(),
            original: original.clone(),
            child: child.clone(),
            accepted_at,
        }),
        housekeeping: services.housekeeping.clone(),
        inputs: services.inputs.clone(),
    });
    let actor = Actor {
        identity: "operator".into(),
        principal: Some(Principal::Unix { uid: 501 }),
        roles: ["recover".into(), "inspect".into(), "lifecycle".into()].into(),
    };
    let mut request = CommandRequest {
        api_version: 1,
        request_id: Uuid::new_v4(),
        runtime_id: original.runtime,
        command: CommandName::ReleaseRecover,
        args: json!({"parent":"TEST-100","operation":original.attempt.plan.operation_id,"action":"supersede","result":child.attempt.plan.evidence_token,"reason":"Explicit historical successor fixture; upstream acceptance is synthetic"}),
        expected_config_revision: Some(
            runtime
                .configuration
                .snapshot()
                .unwrap()
                .revision()
                .unwrap(),
        ),
        expected_control_generation: Some(runtime.control.snapshot().unwrap().intent.generation),
    };
    let mut mutation =
        crate::app::mutation_request(&actor, &request, manager.command_targets(&request).unwrap());
    mutation.kind = "command/ReleaseRecover".into();
    mutation.payload = json!({"arguments":request.args,"secret_digest":null});
    let (parent, fresh) = runtime
        .control
        .queue_command(mutation, "recover", || Ok(()))
        .unwrap();
    assert!(fresh);
    request.request_id = crate::app::command_effect_id(parent.operation_id);
    let result = manager
        .execute(actor, request, parent.clone())
        .await
        .unwrap();
    runtime
        .control
        .complete_command(
            parent.operation_id,
            Ok(serde_json::to_value(result).unwrap()),
        )
        .unwrap();
}
