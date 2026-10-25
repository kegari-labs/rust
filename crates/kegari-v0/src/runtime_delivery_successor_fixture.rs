//! Actual runtime registration, Git assembly and immutable lineage, with
//! explicitly synthetic upstream provider/check facts. No confined check or
//! native agent qualification is manufactured by these fixtures.
use super::*;
use crate::corrections::wire_fixture as wire;
use crate::runtime_delivery::worker_fixture::git;
use crate::test_process_probe::FixtureBootProbe;

struct ObservationPoll(tokio::task::JoinHandle<()>);
impl Drop for ObservationPoll {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn corrected_runtime_assembly_retains_known_predecessor_and_requires_integrated_evidence() {
    let (root, built, original, data, owner) =
        super::super::tests::fixture_authorization_with_wire(true).await;
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
    let github = RuntimeSeedWire::new(&production);
    data.lock().unwrap().github_transport = Some(github.transport());
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
    let _poll = ObservationPoll(tokio::spawn(async move {
        for _ in 0..600 {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            observations.refresh().await.unwrap();
        }
    }));
    let repository = &workspace.contracts[0];
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
    let contract = phase
        .coordinator
        .parent_contract(gate.clone(), parent, &workspace.workflow)
        .unwrap();
    let vector = phase
        .coordinator
        .prepare_parent_vector(&contract, &workspace.observer, &[&seed.slice], &[&baseline])
        .await
        .unwrap();
    assert_eq!(vector.candidates(), &[seed.slice.manifest().clone()]);
    assert_eq!(vector.baselines(), &[baseline.baseline().clone()]);
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
    let plan = ReleasePlan {
        schema_version: 1,
        runtime_id: lease.runtime_id,
        operation_id: Uuid::new_v4(),
        parent_id: gate.work().parent,
        linear_workspace: gate.work().linear_workspace,
        generation: gate.work().generation.clone(),
        evidence_token: seed.parent.token().into(),
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
    held.attempt.owner_generation = admission.owner_generation;
    held.attempt.control_generation = plan_permit.control_generation;
    held.attempt
        .steps
        .insert("merge".into(), StepState::NotStarted);
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
    let mut changed_record = held.clone();
    changed_record.attempt.hold_reason = Some("retained state changed after observation".into());
    delivery.save_release(&changed_record).unwrap();
    assert_eq!(predecessor.revalidate().err().unwrap().code, "stale_result");
    delivery.save_release(&held).unwrap();
}

async fn correct_parent(
    built: &crate::runtime_fixture::RuntimeFixture,
    data: &Arc<std::sync::Mutex<wire::Fixture>>,
    lease: &Lease,
) -> FreshGate {
    let comment = wire::id(960);
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
    let (_claim_root, completed) =
        crate::runtime_claims::test_coordination_fixture(gate, claim, "codex-fixture");
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
