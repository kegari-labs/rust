//! Additional real Git/vector/Control paths around explicitly synthetic accepted
//! upstream validation. No local-check or joined worker proof is manufactured.
use super::*;

pub(super) fn configure_auxiliary(config: &mut Config) {
    let workspace = config.workspaces.get_mut("example").unwrap();
    let original = &workspace.repositories["42"];
    let source = std::path::Path::new(&original.source).with_file_name("source84");
    let encoded = serde_json::to_string(original)
        .unwrap()
        .replace("fixture/repo/", "fixture/aux/");
    let mut auxiliary: crate::config::Repository = serde_json::from_str(&encoded).unwrap();
    auxiliary.source = source.to_str().unwrap().into();
    workspace.repositories.insert("84".into(), auxiliary);
    workspace.github.repositories.push("84".into());
}
pub(super) fn bootstrap_auxiliary(data: &mut wire::Fixture) {
    data.set_read_repositories(&["84"]);
    let auxiliary = RuntimeLineageWire::auxiliary_repository(true);
    let mut main = auxiliary.clone();
    main["id"] = json!(42);
    main["name"] = json!("repo");
    data.github_values
        .insert("/user/repos".into(), json!([main, auxiliary]));
    data.github_values.insert(
        "/repos/fixture/aux".into(),
        RuntimeLineageWire::auxiliary_repository(true),
    );
    data.github_values
        .insert("/repos/fixture/aux/pulls".into(), json!([]));
}
pub(super) fn initialize_auxiliary(
    binary: &std::path::Path,
    source: &std::path::Path,
) -> crate::git::Revision {
    std::fs::create_dir(source).unwrap();
    git(binary, source, &["init", "-b", "main"]);
    std::fs::write(
        source.join("auxiliary.txt"),
        "unchanged dependency repository\n",
    )
    .unwrap();
    git(binary, source, &["add", "auxiliary.txt"]);
    git(
        binary,
        source,
        &[
            "commit",
            "--signoff",
            "-m",
            "test: initialize unchanged auxiliary source",
        ],
    );
    Git::new(binary).unwrap().revision(source, "HEAD").unwrap()
}
pub(super) struct Auxiliary {
    pub slice: IsolatedSlice,
    pub baseline: crate::deployment::VerifiedProductionBaseline,
    pub revision: crate::git::Revision,
}
pub(super) fn assemble_auxiliary<'a>(
    delivery: &'a RuntimeDelivery,
    workspace: &'a WorkspaceDelivery,
    gate: &'a FreshGate,
    lease: &'a Lease,
    revision: &'a crate::git::Revision,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Auxiliary> + 'a>> {
    Box::pin(async move {
        let contract = workspace
            .contracts
            .iter()
            .find(|contract| contract.repository.id == "84")
            .unwrap();
        let phase = delivery
            .admit_delivery_phase(
                workspace,
                gate,
                lease,
                PhaseRequest {
                    operation: Uuid::new_v4(),
                    scope: DeliveryScope {
                        schema_version: 1,
                        runtime_id: lease.runtime_id,
                        phase: DeliveryPhase::CandidateAssembly,
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
                        candidate_vector_digest: None,
                    },
                },
            )
            .await
            .unwrap();
        let root = std::path::Path::new(&contract.configuration.source)
            .parent()
            .unwrap()
            .join(format!("aux-candidate-{}", phase.permit.operation_id));
        PrivateDir::create(&root).unwrap();
        let slice = phase
            .coordinator
            .assemble_isolated_slice(
                &phase.admission,
                SliceAssembly {
                    execution_root: &root,
                    staged: &[],
                    required_children: gate.required_children(),
                    child_dependencies: &BTreeMap::new(),
                    satisfied_children: gate.required_children(),
                    production_integrations: &[],
                    identity: crate::git::CommitIdentity {
                        name: "Fixture".into(),
                        email: "fixture@example.test".into(),
                    },
                },
            )
            .await
            .unwrap();
        assert!(slice.manifest().ordered_delta_digests.is_empty());
        assert_eq!(slice.manifest().result.as_ref(), Some(revision));
        delivery.retire_delivery_phase(&phase, json!({"fixture":"actual unchanged auxiliary assembly; upstream child satisfaction synthetic"})).unwrap();
        let baseline = workspace.observer.observe(contract).await.unwrap();
        Auxiliary {
            slice,
            baseline,
            revision: revision.clone(),
        }
    })
}

pub(super) fn vector_phase<'a>(
    delivery: &'a RuntimeDelivery,
    workspace: &'a WorkspaceDelivery,
    gate: &'a FreshGate,
    lease: &'a Lease,
    revision: &'a crate::git::Revision,
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
                        phase: DeliveryPhase::CandidateAssembly,
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
                        candidate_vector_digest: None,
                    },
                },
            )
            .await
            .unwrap()
    })
}

pub(super) struct SecondRelease<'a> {
    pub built: &'a crate::runtime_fixture::RuntimeFixture,
    pub delivery: &'a Arc<RuntimeDelivery>,
    pub github: &'a RuntimeSeedWire,
    pub native: &'a RuntimeLineageWire,
    pub seed: &'a crate::delivery::RuntimeCandidateSeed,
    pub source: &'a std::path::Path,
    pub gate: &'a FreshGate,
    pub lease: &'a Lease,
    pub auxiliary: &'a Auxiliary,
    pub first: &'a AttemptRecord,
    pub first_proof: &'a VerifiedReleaseCompletion,
    pub observation_barrier: &'a Arc<tokio::sync::Mutex<()>>,
}
pub(super) fn complete_second(
    f: SecondRelease<'_>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + '_>> {
    Box::pin(async move {
        let projection = f.built.runtime.observations.refresh().await.unwrap();
        let workspace = f
            .delivery
            .workspace(&projection, f.first.workspace)
            .unwrap();
        let original = f.seed.slice.manifest().production.clone();
        let phase = fixture_phase(
            f.delivery,
            &workspace,
            f.gate,
            f.lease,
            &original,
            DeliveryPhase::CandidateAssembly,
        )
        .await;
        let slice = f
            .seed
            .corrected_slice(&phase.coordinator, &phase.admission, f.github)
            .await;
        f.delivery
            .retire_delivery_phase(&phase, json!({"candidate":slice.manifest()}))
            .unwrap();
        let auxiliary = assemble_auxiliary(
            f.delivery,
            &workspace,
            f.gate,
            f.lease,
            &f.auxiliary.revision,
        )
        .await;
        // The auxiliary owner has also retired. Vector inspection gets its
        // own actual phase, matching the production parent stage ordering.
        let phase = vector_phase(f.delivery, &workspace, f.gate, f.lease, &original).await;
        let baseline = workspace
            .observer
            .observe(&workspace.contracts[0])
            .await
            .unwrap();
        let parent = projection
            .parent(f.first.workspace, f.first.work.parent)
            .unwrap();
        let contract = phase
            .coordinator
            .parent_contract(f.gate.clone(), parent, &workspace.workflow)
            .unwrap();
        let vector = phase
            .coordinator
            .prepare_parent_vector(
                &contract,
                &workspace.observer,
                &[&slice, &auxiliary.slice],
                &[&baseline, &auxiliary.baseline],
            )
            .await
            .unwrap();
        f.delivery
            .retire_delivery_phase(&phase, json!({"candidate_vector":vector.digest()}))
            .unwrap();
        let mut manifest = f.seed.parent.manifest().clone();
        manifest.work = f.gate.work().clone();
        manifest.publication_operation = Uuid::new_v4();
        manifest.created_at = Utc::now();
        manifest.candidates = vector.candidates().to_vec();
        manifest.baselines = vector.baselines().to_vec();
        manifest.required_contributions.insert(
            wire::id(101),
            slice
                .manifest()
                .selected_contributions
                .values()
                .flatten()
                .cloned()
                .collect(),
        );
        manifest.integrated_results[0].candidate_vector_digest =
            manifest.candidate_vector_digest().unwrap();
        let (manifest, accepted, references) = {
            let _publication =
                origin_publication_window(f.delivery, f.gate, f.observation_barrier).await;
            Box::pin(publish_origin_with_slices(
                f.delivery,
                &workspace,
                f.gate,
                f.lease,
                manifest,
                &[&slice, &auxiliary.slice],
                &[],
            ))
            .await
        };
        let mut plan = f.first.attempt.plan.clone();
        plan.operation_id = Uuid::new_v4();
        plan.generation = f.gate.work().generation.clone();
        plan.evidence_token = manifest.token().unwrap();
        plan.acceptance_digest = accepted.digest().unwrap();
        let domain::StepKind::Merge {
            candidate,
            selected_delta_digests,
            production_base,
            ..
        } = &mut plan.steps[0].kind
        else {
            unreachable!()
        };
        *candidate = slice.manifest().result.clone().unwrap();
        *selected_delta_digests = slice.manifest().ordered_delta_digests.clone();
        *production_base = original;
        let permit = {
            let _poll = f.observation_barrier.lock().await;
            f.built.runtime.observations.refresh().await.unwrap();
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
                .unwrap()
        };
        let merged = f
            .native
            .prepare_merge(&f.delivery.git_path().unwrap(), f.source, &slice);
        let mut scope = phase.admission.scope().clone();
        scope.phase = DeliveryPhase::Release;
        scope.revision = candidate_revision(slice.manifest()).unwrap();
        scope.candidate_vector_digest = Some(vector.digest().into());
        f.native
            .send_merge(&workspace.coordinator, scope, &permit, &plan, f.gate)
            .await;
        let observed = workspace
            .coordinator
            .recover_owned_release_step(&plan, "merge", 1, &workspace.observer)
            .await
            .unwrap();
        let mut record = super::super::tests::record(plan, f.gate.work().clone());
        record.attempt.owner_generation = permit.lease.owner_generation;
        record.attempt.control_generation = permit.control_generation;
        record.config_revision = f.gate.config_revision().into();
        record.workflow_digest = f.gate.workflow_digest().into();
        record.parent_publication = references[0].clone();
        record.acceptance_publication = references[1].clone();
        record.display = Some(ReleaseDisplay::capture(&workspace.configuration, parent).unwrap());
        f.delivery.save_release(&record).unwrap();
        {
            let mut state = f.delivery.releases.state.lock().unwrap();
            state
                .attempts
                .insert(record.attempt.plan.operation_id, record.clone());
            state.queue = rebuild_queue(
                &state.attempts,
                &state.no_release,
                &state.completions,
                &state.completion_anchors,
                &state.completion_errors,
                state.completion_inventory_error.as_ref(),
                &state.historical_no_release,
            )
            .unwrap();
        }
        f.delivery
            .record_release_observation(record.attempt.plan.operation_id, &observed)
            .unwrap();
        let record = f.delivery.releases.state.lock().unwrap().attempts
            [&record.attempt.plan.operation_id]
            .clone();
        f.delivery
            .retire_release(record.attempt.plan.operation_id, &record)
            .unwrap();
        let projection = f.built.runtime.observations.refresh().await.unwrap();
        f.delivery
            .complete_release_chains(&projection)
            .await
            .unwrap();
        let second = f.delivery.releases.state.lock().unwrap().completions
            [&record.attempt.plan.operation_id]
            .clone();
        assert_ne!(second.work().generation, f.first_proof.work().generation);
        let first_success = f
            .first_proof
            .validate_for_configuration(&workspace.configuration)
            .unwrap();
        let second_success = second
            .validate_for_configuration(&workspace.configuration)
            .unwrap();
        let key = issue(f.first).unwrap();
        let outcomes = f.built.runtime.dependency_outcomes().unwrap();
        assert_eq!(
            outcomes.parent_outcome(&key, &f.first.work.generation),
            Some(first_success.outcome_digest.as_str())
        );
        assert_eq!(
            outcomes.parent_outcome(&key, &record.work.generation),
            Some(second_success.outcome_digest.as_str())
        );
        let _poll = f.observation_barrier.lock().await;
        let mut disjoint = record.attempt.plan.clone();
        disjoint.operation_id = Uuid::new_v4();
        disjoint.parent_id = Uuid::new_v4();
        disjoint.spacing_seconds = 0;
        assert_eq!(disjoint.steps.len(), 1);
        let domain::StepKind::Merge { repository, .. } = &mut disjoint.steps[0].kind else {
            panic!("two genuine ordinary completions retain a single merge each")
        };
        repository.id = "9003".into();
        disjoint.resources = BTreeSet::from([domain::ResourceKey::Repository {
            repository: repository.clone(),
        }]);
        disjoint.deployment_free = BTreeSet::from([repository.clone()]);
        disjoint.dependencies = vec![domain::DependencyRequirement {
            parent_id: f.first.work.parent,
            generation: f.first.work.generation.clone(),
            outcome_digest: first_success.outcome_digest.clone(),
        }];
        disjoint.validate().unwrap();
        let mut depends_on_damaged = disjoint.clone();
        depends_on_damaged.operation_id = Uuid::new_v4();
        depends_on_damaged.dependencies[0].generation = record.work.generation.clone();
        depends_on_damaged.dependencies[0].outcome_digest = second_success.outcome_digest.clone();
        depends_on_damaged.validate().unwrap();
        assert_eq!(
            f.first_proof.terminal().plan.resources,
            second.terminal().plan.resources
        );
        assert!(
            disjoint
                .resources
                .is_disjoint(&second.terminal().plan.resources)
        );
        let completion: Signed<Value> = f
            .delivery
            .records
            .read(&format!(
                "release-completion-{}.json",
                second.terminal().plan.operation_id
            ))
            .unwrap();
        let leaf = completion.payload["effects"][0]["leaf"].as_str().unwrap();
        let bytes = workspace.records.read_bytes(leaf, 1024 * 1024).unwrap();
        std::fs::remove_file(workspace.records.path().join(leaf)).unwrap();
        // Both successful generation capabilities are actual. A's exact credit
        // remains usable by this disjoint domain query when only later B is
        // damaged, even though A and B originally used the same resource set.
        f.delivery
            .releases
            .state
            .lock()
            .unwrap()
            .queue
            .eligible(&disjoint, Utc::now())
            .unwrap();
        assert_eq!(
            f.delivery
                .releases
                .state
                .lock()
                .unwrap()
                .queue
                .eligible(&depends_on_damaged, Utc::now())
                .unwrap_err()
                .code,
            "release_completion"
        );
        for delivery in [
            f.delivery.clone(),
            RuntimeDelivery::with_fixture(
                f.built.runtime.clone(),
                f.delivery.processes.clone(),
                f.delivery.git_path().unwrap(),
            )
            .unwrap(),
        ] {
            delivery.sync_released_completions().unwrap();
            f.built.runtime.observations.refresh().await.unwrap();
            let outcomes = f.built.runtime.dependency_outcomes().unwrap();
            assert_eq!(
                outcomes.parent_outcome(&key, &f.first.work.generation),
                Some(first_success.outcome_digest.as_str()),
                "one missing newer effect must not erase another complete generation"
            );
            assert!(
                outcomes
                    .parent_outcome(&key, &record.work.generation)
                    .is_none()
            );
            if !delivery.releases.state.lock().unwrap().restored {
                delivery.restore_releases().unwrap();
            }
            let error = delivery
                .releases
                .state
                .lock()
                .unwrap()
                .queue
                .eligible(&record.attempt.plan, Utc::now())
                .unwrap_err();
            {
                let state = delivery.releases.state.lock().unwrap();
                state.queue.eligible(&disjoint, Utc::now()).unwrap();
                let dependent_error = state
                    .queue
                    .eligible(&depends_on_damaged, Utc::now())
                    .unwrap_err();
                assert_eq!(dependent_error.code, error.code);
                assert_eq!(dependent_error.message, error.message);
            }
            assert_eq!(error.code, "release_completion");
            assert_eq!(
                error.message,
                "completed chain omitted a required prior step attempt"
            );
        }
        workspace.records.write_bytes(leaf, &bytes).unwrap();
        f.delivery.sync_released_completions().unwrap();
        f.built.runtime.observations.refresh().await.unwrap();
        assert_eq!(
            f.built
                .runtime
                .dependency_outcomes()
                .unwrap()
                .parent_outcome(&key, &record.work.generation),
            Some(second_success.outcome_digest.as_str())
        );
        // Deterministic enumerate/open race: these names came from the actual
        // bounded store, then exactly one own completion disappeared.
        let names = f.delivery.records.records_bounded(65536).unwrap();
        let own_leaf = format!(
            "release-completion-{}.json",
            second.terminal().plan.operation_id
        );
        let own_bytes = f
            .delivery
            .records
            .read_bytes(&own_leaf, 1024 * 1024)
            .unwrap();
        std::fs::remove_file(f.delivery.records.path().join(&own_leaf)).unwrap();
        let (proofs, errors) = f
            .delivery
            .released_completion_inventory_from_names(names)
            .unwrap();
        assert_eq!(proofs.len(), 1);
        assert_eq!(proofs[0].digest(), f.first_proof.digest());
        assert_eq!(errors.len(), 1);
        assert_eq!(
            errors[&second.terminal().plan.operation_id].code,
            "record_missing"
        );
        f.built
            .runtime
            .replace_released_completions(proofs, errors)
            .unwrap();
        f.built.runtime.observations.refresh().await.unwrap();
        let outcomes = f.built.runtime.dependency_outcomes().unwrap();
        assert_eq!(
            outcomes.parent_outcome(&key, &f.first.work.generation),
            Some(first_success.outcome_digest.as_str())
        );
        assert!(
            outcomes
                .parent_outcome(&key, &record.work.generation)
                .is_none()
        );
        f.delivery
            .records
            .write_bytes(&own_leaf, &own_bytes)
            .unwrap();
        f.delivery.sync_released_completions().unwrap();
        f.github.set_production(&merged);
    })
}
