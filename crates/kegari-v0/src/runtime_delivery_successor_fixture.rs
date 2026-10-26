//! Actual runtime registration, Git assembly and immutable lineage, with
//! explicitly synthetic upstream provider/check facts. No confined check or
//! native agent qualification is manufactured by these fixtures.
use super::*;
use crate::corrections::wire_fixture as wire;
use crate::runtime_delivery::worker_fixture::git;
use crate::test_process_probe::FixtureBootProbe;

struct ObservationPoll(tokio::task::JoinHandle<()>);
impl ObservationPoll {
    async fn stop(mut self) {
        self.0.abort();
        match (&mut self.0).await {
            Ok(()) => (),
            Err(error) if error.is_cancelled() => (),
            Err(error) => panic!("fixture observation poll failed: {error}"),
        }
    }
}
impl Drop for ObservationPoll {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn corrected_runtime_assembly_retains_known_predecessor_and_requires_integrated_evidence() {
    run_successor_fixture(false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chained_runtime_retains_merged_contribution_through_deployment_only_predecessor() {
    run_successor_fixture(true).await;
}
fn run_successor_fixture(
    chained: bool,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()>>> {
    Box::pin(run_release_fixture(chained, false))
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ordinary_release_seals_exact_historical_dependency_and_survives_later_production() {
    Box::pin(run_release_fixture(false, true)).await;
}
async fn run_release_fixture(chained: bool, ordinary: bool) {
    let (root, built, original, data, owner) =
        super::super::tests::fixture_authorization_with_provider_fixture(
            true,
            |configuration| {
                if chained || ordinary {
                    RuntimeLineageWire::configure(configuration);
                    if ordinary {
                        configuration
                            .workspaces
                            .get_mut("example")
                            .unwrap()
                            .repositories
                            .get_mut("42")
                            .unwrap()
                            .deployment = crate::config::Deployment::None("none".into());
                        extra_completion_fixture::configure_auxiliary(configuration);
                    }
                }
            },
            |_, data| {
                if ordinary {
                    extra_completion_fixture::bootstrap_auxiliary(data);
                }
            },
        )
        .await;
    let root = Arc::new(root);
    let binary = original.git_path().unwrap();
    let source = root.path().canonicalize().unwrap().join("source");
    git(&binary, &source, &["init", "-b", "main"]);
    std::fs::write(source.join("source.txt"), "production\n").unwrap();
    git(&binary, &source, &["add", "source.txt"]);
    git(
        &binary,
        &source,
        &[
            "commit",
            "--signoff",
            "-m",
            "test: initialize owned successor source",
        ],
    );
    let tool = Git::new(&binary).unwrap();
    let production = tool.revision(&source, "HEAD").unwrap();
    let auxiliary_revision = ordinary.then(|| {
        extra_completion_fixture::initialize_auxiliary(&binary, &source.with_file_name("source84"))
    });
    let github = RuntimeSeedWire::new(&production);
    let native =
        (chained || ordinary).then(|| RuntimeLineageWire::new(&github, production.clone()));
    if let Some(revision) = &auxiliary_revision {
        native.as_ref().unwrap().add_auxiliary(revision.clone());
    }
    data.lock().unwrap().github_transport = Some(
        native
            .as_ref()
            .map(|wire| wire.clone() as Arc<dyn crate::providers::HttpTransport>)
            .unwrap_or_else(|| github.transport()),
    );
    let processes = Arc::new(
        ProcessJournal::with_probe(
            built
                .runtime
                .registration
                .root()
                .child("successor-git-processes")
                .unwrap(),
            wire::id(1),
            Arc::new(FixtureBootProbe::new()),
        )
        .unwrap(),
    );
    let delivery = RuntimeDelivery::with_fixture(built.runtime.clone(), processes, binary).unwrap();
    let (lease, _owner_lock) = owner.start(&built.runtime);
    let projection = built.runtime.observations.refresh().await.unwrap();
    let parent = projection.parent(wire::id(2), wire::id(100)).unwrap();
    let gate = parent
        .fresh_gate(
            None,
            GatePurpose::NewOperation,
            &built.runtime.dependency_outcomes().unwrap(),
            Utc::now(),
            Duration::seconds(30),
        )
        .unwrap();
    let workspace = delivery.workspace(&projection, wire::id(2)).unwrap();
    // Long real Git operations retain their original gate but require actual
    // fresh provider observations, just as an active runtime poller does.
    let observations = built.runtime.observations.clone();
    let poll_root = root.clone();
    let observation_barrier = Arc::new(tokio::sync::Mutex::new(()));
    let poll_barrier = observation_barrier.clone();
    let _poll = ObservationPoll(tokio::spawn(async move {
        // A failed joined phase can unwind before cancellation is polled. Keep
        // owned storage alive until this task has actually stopped observing it.
        let _root = poll_root;
        for _ in 0..1800 {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            // Tamper tests wait for any in-flight refresh before changing a
            // retained reference. Their explicit reads still see missing data.
            let _observation = poll_barrier.lock().await;
            match observations.refresh().await {
                Ok(_) => (),
                // Signed publication can change between the two strict read
                // passes. Keep the failed observation unavailable and poll
                // again; operation gates still require a complete fresh view.
                Err(error) if error.code == "unstable_observation" => (),
                Err(error) => panic!("fixture observation poll failed: {error}"),
            }
        }
    }));
    let repository = &workspace.contracts[0];
    if let Some(native) = &native
        && !ordinary
    {
        native.capture_objects(
            &delivery.git_path().unwrap(),
            &source,
            std::slice::from_ref(&production),
        );
        let setup = fixture_phase(
            &delivery,
            &workspace,
            &gate,
            &lease,
            &production,
            DeliveryPhase::Release,
        )
        .await;
        native
            .seed_target(&setup.coordinator, &workspace.observer, &setup.admission)
            .await;
        delivery
            .retire_delivery_phase(&setup, json!({"fixture":"authenticated initial target"}))
            .unwrap();
    }
    let phase = delivery
        .admit_delivery_phase(
            &workspace,
            &gate,
            &lease,
            PhaseRequest {
                operation: Uuid::new_v4(),
                scope: DeliveryScope {
                    schema_version: 1,
                    runtime_id: lease.runtime_id,
                    phase: DeliveryPhase::CandidateAssembly,
                    work: gate.work().clone(),
                    repository: repository.repository.clone(),
                    revision: evidence::RevisionBinding {
                        repository: repository.repository.clone(),
                        head: production.commit.clone(),
                        base: production.commit.clone(),
                        tested_commit: production.commit.clone(),
                        tested_tree: production.tree.clone(),
                    },
                    fresh_gate_digest: gate.digest().unwrap(),
                    contract_digest: repository.digest().unwrap(),
                    candidate_vector_digest: None,
                },
            },
        )
        .await
        .unwrap();
    let seed = runtime_candidate_seed(
        phase.coordinator.clone(),
        phase.admission.clone(),
        parent.clone(),
        &github,
    )
    .await;
    assert_eq!(seed.slice.operation_id(), phase.permit.operation_id);
    let effects = built
        .runtime
        .control
        .operations()
        .unwrap()
        .into_iter()
        .filter(|receipt| receipt.kind == "candidate_assembly")
        .collect::<Vec<_>>();
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].phase, ReceiptPhase::Acknowledged);
    assert_eq!(
        effects[0].expected["admission_operation"],
        phase.permit.operation_id.to_string()
    );
    assert_eq!(
        effects[0].observed_result.as_ref().unwrap()["candidate"],
        serde_json::to_value(seed.slice.manifest()).unwrap()
    );
    let baseline = workspace.observer.observe(repository).await.unwrap();
    let auxiliary = if let Some(revision) = auxiliary_revision {
        // Each repository slice owns the canonical parent work exclusively.
        // Finish the main assembly before admitting the auxiliary slice.
        delivery
            .retire_delivery_phase(&phase, json!({"candidate":seed.slice.manifest()}))
            .unwrap();
        Some(
            extra_completion_fixture::assemble_auxiliary(
                &delivery, &workspace, &gate, &lease, &revision,
            )
            .await,
        )
    } else {
        None
    };
    let phase = if auxiliary.is_some() {
        // Reopen scoped reads under a real new vector phase after both slice
        // owners retired; a retired assembly permit cannot read the candidates.
        extra_completion_fixture::vector_phase(&delivery, &workspace, &gate, &lease, &production)
            .await
    } else {
        phase
    };
    let mut slices = vec![&seed.slice];
    let mut baselines = vec![&baseline];
    if let Some(auxiliary) = &auxiliary {
        slices.push(&auxiliary.slice);
        baselines.push(&auxiliary.baseline);
    }

    let contract = phase
        .coordinator
        .parent_contract(gate.clone(), parent, &workspace.workflow)
        .unwrap();
    let vector = phase
        .coordinator
        .prepare_parent_vector(&contract, &workspace.observer, &slices, &baselines)
        .await
        .unwrap();
    assert_eq!(
        vector.candidates(),
        slices
            .iter()
            .map(|slice| slice.manifest().clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        vector.baselines(),
        baselines
            .iter()
            .map(|baseline| baseline.baseline().clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(vector.candidates()[0].ordered_delta_digests.len(), 1);
    delivery
        .retire_delivery_phase(&phase, json!({"candidate_vector":vector.digest()}))
        .unwrap();
    let final_receipt = built
        .runtime
        .control
        .operation(phase.permit.operation_id)
        .unwrap();
    assert_eq!(final_receipt.phase, ReceiptPhase::Reconciled);
    assert_eq!(
        final_receipt.observed_result.unwrap()["unresolved_effects"],
        json!([])
    );
    // The runtime cannot turn synthetic upstream integrated claims into a
    // completed parent, acceptance or a release plan.
    assert!(
        delivery
            .parent_sources(wire::id(2), wire::id(100))
            .await
            .unwrap()
            .is_none()
    );
    assert!(delivery.release_attempts().unwrap().is_empty());

    // Establish actual native publication/receipt lineage for a retained,
    // accepted predecessor. Its upstream review/check observations are the
    // explicitly synthetic seed above, not a newly completed runtime parent.
    let (manifest, accepted, references) = if chained || ordinary {
        let mut manifest = seed.parent.manifest().clone();
        manifest.baselines = baselines
            .iter()
            .map(|baseline| baseline.baseline().clone())
            .collect();
        manifest.candidates = slices
            .iter()
            .map(|slice| slice.manifest().clone())
            .collect();
        manifest.production_targets = baseline
            .targets()
            .iter()
            .map(|target| target.target.clone())
            .collect();
        manifest.integrated_results[0].candidate_vector_digest =
            manifest.candidate_vector_digest().unwrap();
        Box::pin(publish_origin_with_slices(
            &delivery,
            &workspace,
            &gate,
            &lease,
            manifest,
            &slices,
            baseline.targets(),
        ))
        .await
    } else {
        let publication = built
            .runtime
            .control
            .admit(
                &lease,
                seed.parent.manifest().publication_operation,
                gate.work_key().unwrap(),
                crate::digest::canonical_digest(1, &"fixture prior publication").unwrap(),
                || (delivery.current_gate())(&gate),
            )
            .unwrap();
        let writer = wire::id(5).to_string();
        let actor = format!("owner:{}", lease.owner_id);
        let accepted = evidence::prepare_acceptance(
            &seed.parent,
            evidence::AcceptanceRequest {
                supplied_token: seed.parent.token(),
                current_work: gate.work(),
                pending_correction: false,
                actor: evidence::AcceptanceActor {
                    control_actor: &actor,
                    linear_actor: &writer,
                    provider_writer: &writer,
                    has_accept_role: true,
                    permitted_acceptors: &BTreeSet::from([writer.clone()]),
                },
                runtime_id: lease.runtime_id,
                operation_id: Uuid::new_v4(),
                now: Utc::now(),
            },
            &built.runtime.services.key,
        )
        .unwrap();
        let mut references = Vec::new();
        for (request, body) in [
            (
                Uuid::new_v4(),
                seed.original_parent_body(&workspace.coordinator),
            ),
            (
                accepted.payload.operation_id,
                evidence::encode_signed_publication("human_acceptance", &accepted).unwrap(),
            ),
        ] {
            let receipt = delivery
                .provider_broker(&workspace.configuration, wire::id(2))
                .unwrap()
                .mutate(
                    built.runtime.control.clone(),
                    crate::control::MutationRequest {
                        request_id: request,
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
                    Some(publication.clone()),
                    None,
                    crate::providers::ProviderMutation::LinearComment {
                        issue: gate.work().parent,
                        body: body.clone(),
                    },
                    || Ok(()),
                )
                .await
                .unwrap();
            references.push(PublicationRef {
                id: receipt.remote_ids["comment_id"].parse().unwrap(),
                body_digest: crate::digest::bytes_digest(body.as_bytes()),
                writer: wire::id(5),
            });
        }
        built
            .runtime
            .control
            .retire_local(publication.operation_id, json!({"published":true}))
            .unwrap();
        (seed.parent.manifest().clone(), accepted, references)
    };
    let mut plan = ReleasePlan {
        schema_version: 1,
        runtime_id: lease.runtime_id,
        operation_id: Uuid::new_v4(),
        parent_id: gate.work().parent,
        linear_workspace: gate.work().linear_workspace,
        generation: gate.work().generation.clone(),
        evidence_token: manifest.token().unwrap(),
        acceptance_digest: accepted.digest().unwrap(),
        carried_acceptance: None,
        resources: BTreeSet::from([domain::ResourceKey::Repository {
            repository: repository.repository.clone(),
        }]),
        dependencies: vec![],
        spacing_seconds: 0,
        deployment_free: BTreeSet::from([repository.repository.clone()]),
        steps: vec![domain::ReleaseStep {
            id: "merge".into(),
            depends_on: BTreeSet::new(),
            kind: domain::StepKind::Merge {
                repository: repository.repository.clone(),
                pull_request: 17,
                production_base: seed.slice.manifest().production.clone(),
                candidate: seed.slice.manifest().result.clone().unwrap(),
                selected_delta_digests: seed.slice.manifest().ordered_delta_digests.clone(),
            },
        }],
    };
    if chained {
        let route = fixture_deployment_route(repository);
        plan.deployment_free.clear();
        plan.resources.insert(domain::ResourceKey::Target {
            provider: "github".into(),
            id: "service-production".into(),
        });
        plan.steps.push(domain::ReleaseStep {
            id: "deploy".into(),
            depends_on: BTreeSet::from(["merge".into()]),
            kind: domain::StepKind::Deploy {
                repository: repository.repository.clone(),
                target: domain::ResourceKey::Target {
                    provider: "github".into(),
                    id: "service-production".into(),
                },
                workflow_id: route.workflow_id,
                workflow_revision: route.workflow_revision.clone(),
                dispatch_ref: fixture_dispatch_ref(route),
                source_input: route.source_input.clone(),
                source_merge_step: "merge".into(),
                expected_tree: seed.slice.manifest().result.as_ref().unwrap().tree.clone(),
                expected_artifact: None,
            },
        });
    }
    if chained {
        plan.steps.push(domain::ReleaseStep {
            id: "verify".into(),
            depends_on: ["deploy".into()].into(),
            kind: domain::StepKind::Verify {
                repository: repository.repository.clone(),
                target: domain::ResourceKey::Target {
                    provider: "github".into(),
                    id: "service-production".into(),
                },
                source_merge_step: "merge".into(),
                deployment_step: "deploy".into(),
                expected_tree: seed.slice.manifest().result.as_ref().unwrap().tree.clone(),
                expected_artifact: None,
            },
        });
    }
    plan.validate().unwrap();
    // Publishing the two host records can invalidate an in-flight strict poll.
    // Finish a complete read-only observation before admission; a failed cache
    // still cannot pass the serialized production current-authority gate.
    for attempt in 0..3 {
        match built.runtime.observations.refresh().await {
            Ok(_) => break,
            Err(error) if error.code == "unstable_observation" && attempt < 2 => (),
            Err(error) => panic!("fixture publication observation failed: {error}"),
        }
    }
    let plan_permit = built
        .runtime
        .control
        .admit(
            &lease,
            plan.operation_id,
            gate.work_key().unwrap(),
            plan.digest().unwrap(),
            || (delivery.current_gate())(&gate),
        )
        .unwrap();
    let merged = if let Some(native) = &native {
        let merged = native.prepare_merge(&delivery.git_path().unwrap(), &source, &seed.slice);
        let mut scope = phase.admission.scope().clone();
        scope.phase = DeliveryPhase::Release;
        scope.revision = evidence::RevisionBinding {
            repository: repository.repository.clone(),
            head: seed
                .slice
                .manifest()
                .result
                .as_ref()
                .unwrap()
                .commit
                .clone(),
            base: production.commit.clone(),
            tested_commit: seed
                .slice
                .manifest()
                .result
                .as_ref()
                .unwrap()
                .commit
                .clone(),
            tested_tree: seed.slice.manifest().result.as_ref().unwrap().tree.clone(),
        };
        scope.candidate_vector_digest = Some(vector.digest().into());
        native
            .send_merge(&workspace.coordinator, scope, &plan_permit, &plan, &gate)
            .await;
        let observed = workspace
            .coordinator
            .recover_owned_release_step(&plan, "merge", 1, &workspace.observer)
            .await
            .unwrap();
        assert!(matches!(observed.outcome(), ReleaseStepOutcome::Succeeded));
        Some((merged, observed))
    } else {
        None
    };
    if ordinary {
        ordinary_completion_fixture::complete_ordinary(
            ordinary_completion_fixture::OrdinaryFixture {
                built: &built,
                delivery: &delivery,
                github: &github,
                data: &data,
                observation_barrier: &observation_barrier,
                permit: &plan_permit,
                plan,
                gate: &gate,
                references,
                merged: merged.unwrap(),
                seed: &seed,
                native: native.as_ref().unwrap(),
                source: &source,
                auxiliary: auxiliary.as_ref().unwrap(),
            },
        )
        .await;
        _poll.stop().await;
        return;
    }
    built.runtime.control.fence(plan.operation_id).unwrap();
    built
        .runtime
        .control
        .retire_local(
            plan.operation_id,
            json!({"interrupted_before_dispatch":true}),
        )
        .unwrap();
    let mut held = super::tests::record(plan.clone(), gate.work().clone());
    let admission = built.runtime.control.operation(plan.operation_id).unwrap();
    held.attempt.steps = plan
        .steps
        .iter()
        .map(|step| (step.id.clone(), StepState::NotStarted))
        .collect();
    held.attempt.owner_generation = admission.owner_generation;
    held.attempt.control_generation = plan_permit.control_generation;
    held.attempt
        .steps
        .insert("merge".into(), StepState::NotStarted);
    if let Some((_, observed)) = &merged {
        held.attempt.steps.insert(
            "merge".into(),
            StepState::Succeeded {
                effect: Box::new(observed.effect().unwrap().clone()),
                observed_at: Utc::now(),
            },
        );
        held.attempt
            .steps
            .insert("deploy".into(), StepState::NotStarted);
    }
    held.attempt.hold_reason = Some("Corrected work requires an explicit successor".into());
    held.config_revision = gate.config_revision().into();
    held.workflow_digest = gate.workflow_digest().into();
    held.parent_publication = references[0].clone();
    held.acceptance_publication = references[1].clone();
    held.local_retired = true;
    delivery.save_release(&held).unwrap();
    let delivery = RuntimeDelivery::with_fixture(
        built.runtime.clone(),
        delivery.processes.clone(),
        delivery.git_path().unwrap(),
    )
    .unwrap();
    if let Some((_, observed)) = &merged {
        // A native acknowledgement plus a synthetic upstream successful state
        // is insufficient historical evidence. Commit the actual observed merge
        // through the same receipt reconciliation used by runtime progression.
        let error = workspace
            .coordinator
            .release_history_references(&held.attempt)
            .err()
            .expect("unreconciled merge acknowledgement must not seal history");
        assert_eq!(error.code, "production_validation");
        assert_eq!(
            error.message,
            "archived release effect lacks its exact reconciled outcome"
        );
        delivery.restore_releases().unwrap();
        delivery
            .record_release_observation(plan.operation_id, observed)
            .unwrap();
        held = delivery.releases.state.lock().unwrap().attempts[&plan.operation_id].clone();
        let actual = built
            .runtime
            .control
            .operation(observed.receipt().unwrap().operation_id)
            .unwrap();
        assert_eq!(actual.phase, ReceiptPhase::Reconciled);
        assert_eq!(
            actual.observed_result,
            Some(
                json!({"release_plan":observed.plan_digest(),"step":observed.step_id(),
                "outcome":observed.outcome(),"effect":observed.effect()})
            )
        );
        workspace
            .coordinator
            .release_history_references(&held.attempt)
            .unwrap();
    }
    let corrected = correct_parent(&built, &data, &lease).await;
    assert_ne!(
        corrected.work().generation.correction,
        gate.work().generation.correction
    );
    let latest = built.runtime.observations.refresh().await.unwrap();
    let predecessor = delivery
        .known_predecessor_baseline(&latest, &corrected)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(predecessor.predecessor(), plan.operation_id);
    if let Some((merged, _)) = &merged {
        chained_after_merge(ChainFixture {
            delivery: &delivery,
            built: &built,
            data: &data,
            lease: &lease,
            source: &source,
            github: &github,
            native: native.as_ref().unwrap(),
            seed: &seed,
            held: &held,
            gate: &corrected,
            merged,
            observation_barrier: &observation_barrier,
        })
        .await;
        _poll.stop().await;
        return;
    }
    assert_eq!(predecessor.effects().count(), 0);
    assert_eq!(predecessor.origin().manifest(), seed.parent.manifest());
    predecessor.revalidate().unwrap();
    assert!(
        delivery.release_attempts().unwrap()[0]
            .hold_reason
            .is_some()
    );
    assert!(!delivery.release_attempts().unwrap()[0].resolved);

    let current_parent = latest.parent(wire::id(2), wire::id(100)).unwrap();
    let current_workspace = delivery.workspace(&latest, wire::id(2)).unwrap();
    let current_contract = current_workspace
        .coordinator
        .parent_contract(
            corrected.clone(),
            current_parent,
            &current_workspace.workflow,
        )
        .unwrap();
    let observer = current_workspace
        .observer
        .with_successor(predecessor.clone())
        .unwrap();
    let retained_baseline = observer.observe(repository).await.unwrap();
    assert_eq!(
        retained_baseline.baseline().successor_recovery_operation,
        Some(plan.operation_id)
    );
    assert_eq!(
        current_workspace
            .coordinator
            .prepare_parent_vector(
                &current_contract,
                &observer,
                &[&seed.slice],
                &[&retained_baseline],
            )
            .await
            .err()
            .unwrap()
            .code,
        "candidate_provenance",
        "the previous generation's isolated candidate cannot be adopted",
    );
    let corrected_phase = delivery
        .admit_delivery_phase(
            &current_workspace,
            &corrected,
            &lease,
            PhaseRequest {
                operation: Uuid::new_v4(),
                scope: DeliveryScope {
                    schema_version: 1,
                    runtime_id: lease.runtime_id,
                    phase: DeliveryPhase::CandidateAssembly,
                    work: corrected.work().clone(),
                    repository: repository.repository.clone(),
                    revision: evidence::RevisionBinding {
                        repository: repository.repository.clone(),
                        head: production.commit.clone(),
                        base: production.commit.clone(),
                        tested_commit: production.commit.clone(),
                        tested_tree: production.tree.clone(),
                    },
                    fresh_gate_digest: corrected.digest().unwrap(),
                    contract_digest: repository.digest().unwrap(),
                    candidate_vector_digest: None,
                },
            },
        )
        .await
        .unwrap();
    let corrected_slice = seed
        .corrected_slice(
            &corrected_phase.coordinator,
            &corrected_phase.admission,
            &github,
        )
        .await;
    assert_eq!(
        corrected_slice.manifest().correction_generation,
        corrected.work().generation.correction
    );
    assert_ne!(
        corrected_slice.manifest().ordered_delta_digests,
        seed.slice.manifest().ordered_delta_digests
    );
    assert_eq!(
        corrected_slice.manifest().result.as_ref().unwrap().tree,
        seed.slice.manifest().result.as_ref().unwrap().tree
    );
    // A real refresh after the bounded Git phase reobserves ownership and the
    // complete baseline. No timestamp is manufactured on the original seal.
    let fresh = built.runtime.observations.refresh().await.unwrap();
    let predecessor = delivery
        .known_predecessor_baseline(&fresh, &corrected)
        .await
        .unwrap()
        .unwrap();
    let observer = current_workspace
        .observer
        .with_successor(predecessor)
        .unwrap();
    let baseline = observer.observe(repository).await.unwrap();
    let vector = corrected_phase
        .coordinator
        .prepare_parent_vector(
            &current_contract,
            &observer,
            &[&corrected_slice],
            &[&baseline],
        )
        .await
        .unwrap();
    delivery
        .retire_delivery_phase(
            &corrected_phase,
            json!({"candidate_vector":vector.digest()}),
        )
        .unwrap();
    let e2e = fresh
        .parent(wire::id(2), wire::id(100))
        .unwrap()
        .fresh_gate(
            Some(wire::id(101)),
            GatePurpose::NewOperation,
            &built.runtime.dependency_outcomes().unwrap(),
            Utc::now(),
            Duration::seconds(30),
        )
        .unwrap();
    let fresh = built.runtime.observations.refresh().await.unwrap();
    let predecessor = delivery
        .known_predecessor_baseline(&fresh, &corrected)
        .await
        .unwrap()
        .unwrap();
    let observer = current_workspace
        .observer
        .with_successor(predecessor.clone())
        .unwrap();
    // The origin must retain the actual upstream broker receipt. Removing its
    // protected bytes blocks assignment creation; restoring the exact bytes
    // permits the existing path without minting a new upstream effect.
    let delta_digest = &corrected_slice.manifest().ordered_delta_digests[0];
    let delta: crate::evidence::Signed<Value> = current_workspace
        .records
        .read(&format!("delta-{}.json", &delta_digest[7..]))
        .unwrap();
    let merge_operation: Uuid =
        serde_json::from_value(delta.payload["merge_request"].clone()).unwrap();
    let merge = built.runtime.control.operation(merge_operation).unwrap();
    assert_eq!(merge.kind, "github_merge");
    assert_eq!(merge.phase, ReceiptPhase::Acknowledged);
    assert_eq!(
        merge.expected["admission_operation"],
        corrected_phase.permit.operation_id.to_string()
    );
    let receipts = canonical_digest(1, &built.runtime.control.operations().unwrap()).unwrap();
    let receipt_dir = crate::storage::PrivateDir::open(
        &built.runtime.registration.root().path().join("receipts"),
    )
    .unwrap();
    let merge_leaf = format!("{merge_operation}.json");
    let merge_bytes = receipt_dir.read_bytes(&merge_leaf, 256 * 1024).unwrap();
    let tamper_window = observation_barrier.lock().await;
    std::fs::remove_file(receipt_dir.path().join(&merge_leaf)).unwrap();
    let refused = current_workspace
        .coordinator
        .integrated_worker_assignment(
            &current_contract,
            &e2e,
            &vector,
            &observer,
            &[&corrected_slice],
            json!({"purpose":"validate the exact corrected vector"}),
        )
        .await;
    receipt_dir.write_bytes(&merge_leaf, &merge_bytes).unwrap();
    assert_eq!(
        refused
            .err()
            .expect("missing upstream receipt must block parent origin")
            .code,
        "record_missing"
    );
    assert_eq!(
        canonical_digest(1, &built.runtime.control.operations().unwrap()).unwrap(),
        receipts
    );
    drop(tamper_window);
    let assignment = current_workspace
        .coordinator
        .integrated_worker_assignment(
            &current_contract,
            &e2e,
            &vector,
            &observer,
            &[&corrected_slice],
            json!({"purpose":"validate the exact corrected vector"}),
        )
        .await
        .unwrap();
    assert_eq!(assignment.phase(), "parent_integrated_validation");
    assert_eq!(
        assignment.source().unwrap().revision(),
        corrected_slice.manifest().result.as_ref().unwrap()
    );
    let fresh = built.runtime.observations.refresh().await.unwrap();
    let predecessor = delivery
        .known_predecessor_baseline(&fresh, &corrected)
        .await
        .unwrap()
        .unwrap();
    let observer = current_workspace
        .observer
        .with_successor(predecessor.clone())
        .unwrap();
    assert_eq!(
        current_workspace
            .coordinator
            .verify_integrated_checks(&current_contract, &e2e, &vector, &observer, &[],)
            .await
            .err()
            .unwrap()
            .code,
        "integrated_gate"
    );
    assert!(
        delivery
            .parent_sources(wire::id(2), wire::id(100))
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(github.release_effect_count(), 0);
    assert!(
        delivery.release_attempts().unwrap()[0]
            .hold_reason
            .is_some()
    );
    assert!(!delivery.release_attempts().unwrap()[0].resolved);

    // Current source movement that is absent from the retained owned-effect
    // history must block recovery; knowing an old plan UUID grants no bypass.
    let mut unrelated = production.clone();
    unrelated.commit = "f".repeat(40);
    github.set_production(&unrelated);
    assert!(
        delivery
            .known_predecessor_baseline(&fresh, &corrected)
            .await
            .is_err()
    );
    github.set_production(&production);
    let tamper_window = observation_barrier.lock().await;
    let mut changed_record = held.clone();
    changed_record.attempt.hold_reason = Some("retained state changed after observation".into());
    delivery.save_release(&changed_record).unwrap();
    assert_eq!(predecessor.revalidate().err().unwrap().code, "stale_result");
    delivery.save_release(&held).unwrap();
    drop(tamper_window);
    _poll.stop().await;
}

fn fixture_deployment_route(
    contract: &RepositoryContract,
) -> &crate::config::ExactRevisionWorkflow {
    let crate::config::Deployment::Steps { steps } = &contract.configuration.deployment else {
        panic!("lineage fixture requires an actual configured deployment route")
    };
    let selected = steps
        .iter()
        .filter(|step| step.id == "deploy" && step.target_id == "service-production")
        .collect::<Vec<_>>();
    assert_eq!(selected.len(), 1);
    &selected[0].workflow
}

fn fixture_dispatch_ref(route: &crate::config::ExactRevisionWorkflow) -> String {
    route
        .proof
        .workflow_ref
        .rsplit_once('@')
        .and_then(|(_, reference)| reference.strip_prefix("refs/"))
        .filter(|reference| reference.starts_with("heads/") || reference.starts_with("tags/"))
        .expect("configured fixture workflow must have a pinned canonical dispatch reference")
        .into()
}

async fn correct_parent(
    built: &crate::runtime_fixture::RuntimeFixture,
    data: &Arc<std::sync::Mutex<wire::Fixture>>,
    lease: &Lease,
) -> FreshGate {
    let comment = Uuid::new_v4();
    let body = "Validate the already-present behavior and keep this correction within the existing test responsibility.";
    data.lock().unwrap().comments.entry(wire::id(100)).or_default().push(json!({"id":comment,"issue":{"id":wire::id(100)},
        "body":body,"createdAt":Utc::now(),"updatedAt":Utc::now(),"editedAt":null,"user":{"id":wire::id(5)},
        "onBehalfOf":null,"botActor":null,"url":"https://linear.invalid/comment/correction"}));
    let projection = built.runtime.observations.refresh().await.unwrap();
    let parent = projection.parent(wire::id(2), wire::id(100)).unwrap();
    let gate = parent
        .fresh_gate(
            None,
            GatePurpose::Coordination,
            &built.runtime.dependency_outcomes().unwrap(),
            Utc::now(),
            Duration::seconds(30),
        )
        .unwrap();
    let claim = json!({"kind":"correction_classification","classifications":[{
        "source_comment":comment,"source_revision":crate::digest::bytes_digest(body.as_bytes()),"class":"corrective",
        "rationale":"The native request changes validation expectations within existing scope",
        "routes":[{"child":wire::id(101),"instructions":"Validate existing behavior without adding an unrelated source change.",
            "validation_expectations":["Run configured local and exact revision checks; report actual evidence"]}]}]});
    let (_claim_root, completed) = crate::runtime_claims::test_coordination_fixture_with_proposal(
        gate,
        claim,
        "codex-fixture",
        Uuid::new_v4(),
    );
    let owner = built.runtime.registration.clone();
    let control_owner = owner.clone();
    let services = &built.runtime.services;
    Arc::new(crate::corrections::CorrectionPublisher::new(
        crate::corrections::CorrectionDependencies {
            configuration: services.configuration.clone(),
            control: services.control.clone(),
            observations: services.observations.clone(),
            secrets: services.secrets.clone(),
            key: services.key.clone(),
            inputs: services.inputs.clone(),
            owner_check: Arc::new(move |lease| owner.verify_active_owner(lease)),
            control_owner_check: Arc::new(move || control_owner.verify_control_owner()),
        },
    ))
    .publish(lease, &completed)
    .await
    .unwrap();
    built
        .runtime
        .observations
        .refresh()
        .await
        .unwrap()
        .parent(wire::id(2), wire::id(100))
        .unwrap()
        .fresh_gate(
            None,
            GatePurpose::NewOperation,
            &built.runtime.dependency_outcomes().unwrap(),
            Utc::now(),
            Duration::seconds(30),
        )
        .unwrap()
}

#[path = "runtime_delivery_chain_fixture.rs"]
mod chain;
use chain::*;
#[path = "runtime_delivery_ordinary_completion_fixture.rs"]
mod ordinary_completion_fixture;

#[path = "runtime_delivery_extra_completion_fixture.rs"]
mod extra_completion_fixture;
