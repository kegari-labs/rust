//! Historical accepted input is explicitly synthetic. Actual owner receipts,
//! supersede authorization, native wire, object import and assembly are exercised.
use super::*;
use crate::control::Receipt;
use crate::decisions::{
    AuthorizedDecision, AuthorizedOperationRecovery, DecisionEvidence, RetryTarget,
    VerifiedExplicitCorrection, VerifiedRetryGrant,
};
use crate::runtime_host::ConfigurationControl;
use kegari_digest::canonical_digest;

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
    pub merged: &'a kegari_git::Revision,
    pub observation_barrier: &'a Arc<tokio::sync::Mutex<()>>,
}

struct BaselineReload {
    configuration: kegari_config::Config,
    workflow: std::path::PathBuf,
    contents: Vec<u8>,
    admissions: String,
    release_effects: usize,
}
type ReloadedPredecessor = (Arc<KnownPredecessorBaseline>, BaselineReload);

/// Compare provenance and actual product identity, excluding only observation
/// timestamps that necessarily change after a fresh complete native read.
fn baseline_identity(proof: &KnownPredecessorBaseline) -> String {
    let origin = |origin: &VerifiedReleaseOrigin| {
        json!({"plan":origin.plan(),"manifest":origin.manifest(),"targets":origin.targets(),
            "admission":origin.admission_digest(),"retained":origin.retained_references()})
    };
    let effect = |effect: &ObservedReleaseStep| {
        json!({"plan":effect.plan_digest(),"step":effect.step_id(),"attempt":effect.attempt(),
            "receipt":effect.receipt(),"outcome":effect.outcome(),"effect":effect.effect()})
    };
    canonical_digest(1, &json!({
        "predecessor":proof.predecessor(),"record":proof.record_digest(),"work":proof.work(),
        "origin":origin(proof.origin()),
        "effects":proof.effects().map(effect).collect::<Vec<_>>(),
        "merges":proof.integration_sources().map(|(o,e)| (origin(o),effect(e))).collect::<Vec<_>>(),
        "baselines":proof.baselines().iter().map(|b| json!({
            "repository":b.repository(),"production":b.production(),"contract":b.contract_digest(),
            "coherent":b.coherent(),"targets":b.targets().iter().map(|target| json!({
                "provider":target.provider,"target":target.target,"repository":target.repository,
                "source":target.source,"artifact":target.artifact_digest,"record":target.provider_record
            })).collect::<Vec<_>>()
        })).collect::<Vec<_>>()
    })).unwrap()
}

fn reloaded_predecessor<'a>(
    f: &'a ChainFixture<'_>,
    phase: &'a ActivePhase,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ReloadedPredecessor> + 'a>> {
    Box::pin(async move {
        tokio::time::timeout(std::time::Duration::from_secs(60), async {
            let snapshot = f.built.runtime.observations.admission_snapshot().unwrap();
            let original = f
                .delivery
                .known_predecessor_baseline(&snapshot, f.gate)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(original.predecessor(), f.held.attempt.plan.operation_id);
            assert_eq!(original.integration_sources().count(), 1);
            let original_contract = f
                .delivery
                .active_parent_contract(phase)
                .unwrap()
                .digest()
                .unwrap();
            let before = baseline_identity(&original);
            let receipts =
                canonical_digest(1, &f.built.runtime.control.operations().unwrap()).unwrap();
            let admissions = canonical_digest(
                1,
                &f.built
                    .runtime
                    .control
                    .operations()
                    .unwrap()
                    .into_iter()
                    .filter(|receipt| receipt.kind == "admission")
                    .collect::<Vec<_>>(),
            )
            .unwrap();
            let permit = canonical_digest(1, &phase.permit).unwrap();
            let release_effects = f.github.release_effect_count();
            let configuration = f.built.configuration.snapshot().unwrap();
            let workflow = std::path::PathBuf::from(
                configuration.workspaces["example"]
                    .workflow
                    .as_ref()
                    .unwrap(),
            );
            let contents = std::fs::read(&workflow).unwrap();
            {
                let _refresh = f.observation_barrier.lock().await;
                let mut changed = configuration.clone();
                changed.runtime.max_agents += 1;
                changed.workspaces.get_mut("example").unwrap().linear.teams =
                    kegari_config::Selector::Ids(vec![wire::id(999).to_string()]);
                *f.built.configuration.config.lock().unwrap() = changed;
                let mut changed_workflow = contents.clone();
                changed_workflow
                    .extend_from_slice(b"\nHarmless reload applies only to new work.\n");
                std::fs::write(&workflow, changed_workflow).unwrap();
                f.data.lock().unwrap().set_selected(false);
                let current = f.built.runtime.observations.refresh().await.unwrap();
                assert!(
                    current.workspaces[&f.gate.workspace_id()].complete,
                    "{:?}",
                    current.workspaces[&f.gate.workspace_id()].error
                );
                assert!(
                    !current
                        .parent(f.gate.workspace_id(), f.gate.work().parent)
                        .unwrap()
                        .included
                );
                assert_ne!(current.config_revision, f.gate.config_revision());
                assert_ne!(
                    current.workspaces[&f.gate.workspace_id()].workflow_digest,
                    f.gate.workflow_digest()
                );
            }
            let parent = f
                .built
                .runtime
                .observations
                .active_parent(f.gate, &f.built.runtime.dependency_outcomes().unwrap())
                .unwrap();
            let active = parent
                .retain_active_gate(
                    f.gate,
                    &f.built.runtime.dependency_outcomes().unwrap(),
                    Utc::now(),
                    Duration::seconds(30),
                )
                .unwrap();
            assert_ne!(parent.config_revision, active.config_revision());
            assert_eq!(
                phase
                    .workspace
                    .coordinator
                    .parent_contract(active, &parent, &phase.workspace.workflow,)
                    .err()
                    .unwrap()
                    .code,
                "stale_result"
            );
            assert_eq!(
                f.delivery
                    .active_parent_contract(phase)
                    .unwrap()
                    .digest()
                    .unwrap(),
                original_contract
            );
            let observed = f
                .delivery
                .active_predecessor_baseline(phase)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(baseline_identity(&observed), before);
            assert_eq!(observed.gate().config_revision(), f.gate.config_revision());
            assert_eq!(observed.gate().workflow_digest(), f.gate.workflow_digest());
            assert_eq!(canonical_digest(1, &phase.permit).unwrap(), permit);
            f.built
                .runtime
                .control
                .validate_permit(&phase.permit, || f.delivery.retained_gate()(f.gate))
                .unwrap();
            assert_eq!(
                canonical_digest(1, &f.built.runtime.control.operations().unwrap()).unwrap(),
                receipts
            );
            assert_eq!(f.github.release_effect_count(), release_effects);
            (
                observed,
                BaselineReload {
                    configuration,
                    workflow,
                    contents,
                    admissions,
                    release_effects,
                },
            )
        })
        .await
        .expect("retained predecessor reload read completes within its fixture bound")
    })
}

fn restore_baseline_reload<'a>(
    f: &'a ChainFixture<'_>,
    reload: BaselineReload,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + 'a>> {
    Box::pin(async move {
        tokio::time::timeout(std::time::Duration::from_secs(60), async {
            assert_eq!(
                canonical_digest(
                    1,
                    &f.built
                        .runtime
                        .control
                        .operations()
                        .unwrap()
                        .into_iter()
                        .filter(|receipt| receipt.kind == "admission")
                        .collect::<Vec<_>>()
                )
                .unwrap(),
                reload.admissions
            );
            assert_eq!(f.github.release_effect_count(), reload.release_effects);
            let _refresh = f.observation_barrier.lock().await;
            *f.built.configuration.config.lock().unwrap() = reload.configuration;
            std::fs::write(reload.workflow, reload.contents).unwrap();
            f.data.lock().unwrap().set_selected(true);
            let current = f.built.runtime.observations.refresh().await.unwrap();
            assert!(current.workspaces[&f.gate.workspace_id()].complete);
            assert!(
                current
                    .parent(f.gate.workspace_id(), f.gate.work().parent)
                    .unwrap()
                    .included
            );
            f.delivery.current_gate()(f.gate).unwrap();
        })
        .await
        .expect("restored fixture configuration receives an actual complete observation")
    })
}

pub(super) fn fixture_phase<'a>(
    delivery: &'a RuntimeDelivery,
    workspace: &'a WorkspaceDelivery,
    gate: &'a FreshGate,
    lease: &'a Lease,
    revision: &'a kegari_git::Revision,
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
/// The synthetic accepted-origin setup is one declared fixture interval.
/// Wait for the existing poller's in-flight read, then obtain a real complete
/// observation before its unchanged admission/mutation gates. This prevents a
/// publication from splitting the poller's two strict reconstruction reads.
/// The guard is held only for this sequence, never for assembly or release work.
pub(super) fn origin_publication_window<'a>(
    delivery: &'a RuntimeDelivery,
    gate: &'a FreshGate,
    barrier: &'a Arc<tokio::sync::Mutex<()>>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = tokio::sync::MutexGuard<'a, ()>> + 'a>> {
    Box::pin(async move {
        let guard = barrier.lock().await;
        delivery
            .runtime
            .observations
            .refresh()
            .await
            .unwrap_or_else(|error| {
                panic!(
                    "origin publication window refresh failed: {error:?}; health={:?}",
                    delivery.runtime.observations.health()
                )
            });
        delivery.current_gate()(gate).unwrap_or_else(|error| {
            panic!(
                "origin publication window current gate failed: {error:?}; health={:?}",
                delivery.runtime.observations.health()
            )
        });
        guard
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
    Box::pin(publish_origin_with_slices(
        delivery,
        workspace,
        gate,
        lease,
        manifest,
        &[slice],
        targets,
    ))
    .await
}
pub(super) async fn publish_origin_with_slices(
    delivery: &RuntimeDelivery,
    workspace: &WorkspaceDelivery,
    gate: &FreshGate,
    lease: &Lease,
    manifest: evidence::ParentEvidence,
    slices: &[&IsolatedSlice],
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
        .fixture_original_manifest_body_with_slices(&manifest, slices, targets, lease.runtime_id)
        .unwrap();
    let mut refs = Vec::new();
    for (kind, id, body) in [
        ("parent_result", Uuid::new_v4(), body),
        (
            "human_acceptance",
            accepted.payload.operation_id,
            evidence::encode_signed_publication("human_acceptance", &accepted).unwrap(),
        ),
    ] {
        let current_gate = delivery.current_gate();
        let admitted_gate = gate.clone();
        let observations = delivery.runtime.observations.clone();
        let publication = manifest.publication_operation;
        let r = delivery
            .provider_broker(&workspace.configuration, gate.workspace_id())
            .unwrap()
            .mutate(
                delivery.runtime.control.clone(),
                crate::control::MutationRequest {
                    request_id: id,
                    runtime_id: lease.runtime_id,
                    actor: kegari_control_api::identity::Actor {
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
                move || {
                    current_gate(&admitted_gate).map_err(|error| {
                        // Called inside the provider's existing Control gate; only
                        // read cache health here, never reacquire Control or refresh.
                        let original_details = error.details.clone();
                        error.detail(
                            json!({"publication_kind":kind,"effect":id,"publication":publication,
                        "health":observations.health().ok(),"original_details":original_details}),
                        )
                    })
                },
            )
            .await
            .unwrap_or_else(|error| panic!("origin {kind} mutation failed for {id}: {error:?}"));
        refs.push(PublicationRef {
            id: r.remote_ids["comment_id"].parse().unwrap(),
            body_digest: kegari_digest::bytes_digest(body.as_bytes()),
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
        let (predecessor, reload) = reloaded_predecessor(&f, &phase).await;
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
        kegari_storage::PrivateDir::create(&root).unwrap();
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
                    identity: kegari_git::CommitIdentity {
                        name: "Fixture".into(),
                        email: "fixture@example.test".into(),
                    },
                },
            )
            .await
            .unwrap();
        assert!(slice.manifest().ordered_delta_digests.is_empty());
        assert_eq!(slice.manifest().result.as_ref(), Some(f.merged));
        restore_baseline_reload(&f, reload).await;
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
        let (manifest, acceptance, refs) = {
            let _publication =
                origin_publication_window(f.delivery, f.gate, f.observation_barrier).await;
            publish_origin(
                f.delivery,
                &workspace,
                f.gate,
                f.lease,
                manifest,
                &slice,
                baseline.targets(),
            )
            .await
        };
        let route = fixture_deployment_route(&workspace.contracts[0]);
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
                workflow_id: route.workflow_id,
                workflow_revision: route.workflow_revision.clone(),
                dispatch_ref: fixture_dispatch_ref(route),
                source_input: route.source_input.clone(),
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
        let permit = {
            // The two acknowledged origin publications can invalidate a
            // concurrent two-pass poll. A new admission needs a complete
            // observation after both writes, not whichever poll last finished.
            // Keep the fixture poll out until this read and admission finish;
            // the real native reads and final current_gate remain mandatory.
            let _observation = f.observation_barrier.lock().await;
            let current = f
                .built
                .runtime
                .observations
                .refresh_bounded(std::time::Duration::from_secs(30))
                .await
                .unwrap_or_else(|error| {
                    panic!(
                        "B interrupted admission refresh: {error:?}; health={:?}",
                        f.built.runtime.observations.health()
                    )
                });
            assert!(
                current.workspaces[&f.gate.workspace_id()].complete,
                "B interrupted admission workspace incomplete: {:?}; health={:?}",
                current.workspaces[&f.gate.workspace_id()].error,
                f.built.runtime.observations.health()
            );
            f.built
                .runtime
                .control
                .admit(
                    f.lease,
                    plan.operation_id,
                    f.gate.work_key().unwrap(),
                    plan.digest().unwrap(),
                    || (f.delivery.current_gate())(f.gate),
                )
                .unwrap_or_else(|error| {
                    panic!(
                        "B interrupted admission after complete refresh: {error:?}; health={:?}",
                        f.built.runtime.observations.health()
                    )
                })
        };
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
        kegari_storage::PrivateDir::create(&root).unwrap();
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
                    identity: kegari_git::CommitIdentity {
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
        // Explicit historical accepted input for C. This does not grant the
        // runtime genuine integrated evidence; that barrier was asserted above.
        let mut c_manifest = f.seed.parent.manifest().clone();
        c_manifest.work = corrected.work().clone();
        c_manifest.publication_operation = Uuid::new_v4();
        c_manifest.created_at = Utc::now();
        c_manifest.candidates = vec![slice.manifest().clone()];
        c_manifest.baselines = vec![baseline.baseline().clone()];
        c_manifest.production_targets = baseline
            .targets()
            .iter()
            .map(|t| t.target.clone())
            .collect();
        c_manifest.integrated_results[0].candidate_vector_digest =
            c_manifest.candidate_vector_digest().unwrap();
        let (c_manifest, c_acceptance, c_refs) = {
            let _publication =
                origin_publication_window(&fresh, &corrected, f.observation_barrier).await;
            publish_origin(
                &fresh,
                &workspace,
                &corrected,
                f.lease,
                c_manifest,
                &slice,
                baseline.targets(),
            )
            .await
        };
        let mut c = held.clone();
        c.work = corrected.work().clone();
        c.attempt.plan.operation_id = Uuid::new_v4();
        c.attempt.plan.generation = corrected.work().generation.clone();
        c.attempt.plan.evidence_token = c_manifest.token().unwrap();
        c.attempt.plan.acceptance_digest = c_acceptance.digest().unwrap();
        c.attempt.plan.spacing_seconds = 60;
        c.parent_publication = c_refs[0].clone();
        c.acceptance_publication = c_refs[1].clone();
        c.admission_operation = c.attempt.plan.operation_id;
        c.config_revision = corrected.config_revision().into();
        c.workflow_digest = corrected.workflow_digest().into();
        authorize_link(&fresh, &held, &c, c_acceptance.payload.accepted_at).await;
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
        let tamper_window = f.observation_barrier.lock().await;
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
        drop(tamper_window);
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
        completion_fixture::complete_and_recover(
            &f,
            &fresh,
            &workspace,
            &corrected,
            c,
            &c_manifest,
        )
        .await
        .expect("owned terminal fixture task must complete");
    })
}

#[path = "runtime_delivery_completion_fixture.rs"]
mod completion_fixture;

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
    fn release(&self, _: AuthorizedDecision) -> crate::runtime_host::HostFuture<'_, Receipt> {
        Box::pin(async { Err(Error::invalid("fixture only links historical releases")) })
    }
    fn reconcile(
        &self,
        _: AuthorizedOperationRecovery,
    ) -> crate::runtime_host::HostFuture<'_, Receipt> {
        Box::pin(async { Err(Error::invalid("fixture only links historical releases")) })
    }
    fn recover_release(
        &self,
        decision: AuthorizedDecision,
    ) -> crate::runtime_host::HostFuture<'_, Receipt> {
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
    use crate::decisions::DecisionDependencies;
    use crate::decisions::DecisionManager;
    use kegari_control_api::identity::Actor;
    use kegari_control_api::identity::Principal;
    use kegari_types::api::{CommandName, CommandRequest};
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
            crate::config::config_revision(&runtime.configuration.snapshot().unwrap()).unwrap(),
        ),
        expected_control_generation: Some(runtime.control.snapshot().unwrap().intent.generation),
    };
    let mut mutation = crate::control_command::mutation_request(
        &actor,
        &request,
        manager.command_targets(&request).unwrap(),
    );
    mutation.kind = "command/ReleaseRecover".into();
    mutation.payload = json!({"arguments":request.args,"secret_digest":null});
    let (parent, fresh) = runtime
        .control
        .queue_command(mutation, "recover", || Ok(()))
        .unwrap();
    assert!(fresh);
    request.request_id = crate::control_command::command_effect_id(parent.operation_id);
    // Operator commands are separate owned tasks in HostDispatcher. Preserve
    // that boundary instead of polling its provider refresh on the deep Git
    // fixture's task stack.
    let command_parent = parent.clone();
    eprintln!("lineage fixture: starting owned successor command task");
    let result = tokio::spawn(async move {
        tokio::time::timeout(
            std::time::Duration::from_secs(300),
            manager.execute(actor, request, command_parent),
        )
        .await
        .unwrap_or_else(|_| {
            Err(Error::new(
                "operation_uncertain",
                "fixture owned command exceeded its production bound",
            ))
        })
    })
    .await
    .unwrap_or_else(|_| {
        Err(Error::new(
            "operation_uncertain",
            "fixture owned command ended without a verified result",
        ))
    });
    eprintln!("lineage fixture: owned successor command task completed");
    runtime
        .control
        .complete_command(
            parent.operation_id,
            result
                .clone()
                .map(|result| json!({"effect_operation_id":result.operation_id,"receipt":result})),
        )
        .unwrap();
    result.unwrap();
}
