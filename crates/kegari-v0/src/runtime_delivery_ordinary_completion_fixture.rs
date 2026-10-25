//! Actual one-link merge/readback/retirement and signed history. Upstream
//! acceptance uses the existing explicit synthetic fixture, never a confined
//! check or a fabricated joined worker completion.
use super::*;

pub(super) struct OrdinaryFixture<'a> {
    pub built: &'a crate::runtime_fixture::RuntimeFixture,
    pub delivery: &'a Arc<RuntimeDelivery>,
    pub github: &'a RuntimeSeedWire,
    pub data: &'a Arc<std::sync::Mutex<wire::Fixture>>,
    pub observation_barrier: &'a Arc<tokio::sync::Mutex<()>>,
    pub permit: &'a Permit,
    pub plan: ReleasePlan,
    pub gate: &'a FreshGate,
    pub references: Vec<PublicationRef>,
    pub merged: (crate::git::Revision, ObservedReleaseStep),
    pub seed: &'a crate::delivery::RuntimeCandidateSeed,
    pub native: &'a RuntimeLineageWire,
    pub source: &'a std::path::Path,
    pub auxiliary: &'a extra_completion_fixture::Auxiliary,
}
pub(super) fn complete_ordinary(
    fixture: OrdinaryFixture<'_>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + '_>> {
    Box::pin(async move {
        let OrdinaryFixture {
            built,
            delivery,
            github,
            data,
            observation_barrier,
            permit,
            plan,
            gate,
            references,
            merged,
            seed,
            native,
            source,
            auxiliary,
        } = fixture;
        let operation = plan.operation_id;
        let mut record = super::super::tests::record(plan, gate.work().clone());
        record.attempt.owner_generation = permit.lease.owner_generation;
        record.attempt.control_generation = permit.control_generation;
        record.config_revision = gate.config_revision().into();
        record.workflow_digest = gate.workflow_digest().into();
        record.parent_publication = references[0].clone();
        record.acceptance_publication = references[1].clone();
        let legacy_value = serde_json::to_value(&record).unwrap();
        assert!(legacy_value.get("display").is_none());
        let legacy_signed = built
            .runtime
            .services
            .key
            .sign("runtime_release_attempt", &legacy_value)
            .unwrap();
        let legacy_typed: Signed<AttemptRecord> =
            serde_json::from_value(serde_json::to_value(&legacy_signed).unwrap()).unwrap();
        built
            .runtime
            .services
            .key
            .verify("runtime_release_attempt", &legacy_typed)
            .unwrap();
        let actual = built.runtime.observations.refresh().await.unwrap();
        record.display = Some(
            ReleaseDisplay::capture(
                &built.configuration.config.lock().unwrap(),
                actual.parent(record.workspace, record.work.parent).unwrap(),
            )
            .unwrap(),
        );
        delivery.save_release(&record).unwrap();
        {
            let mut state = delivery.releases.state.lock().unwrap();
            state.attempts.insert(operation, record);
            state.queue =
                rebuild_queue(&state.attempts, &state.no_release, &state.completions).unwrap();
        }
        delivery
            .record_release_observation(operation, &merged.1)
            .unwrap();
        let projection = built.runtime.observations.refresh().await.unwrap();
        delivery.complete_release_chains(&projection).await.unwrap();
        assert!(
            delivery
                .releases
                .state
                .lock()
                .unwrap()
                .completions
                .is_empty(),
            "successful effect alone is not retired completion"
        );
        let record = delivery.releases.state.lock().unwrap().attempts[&operation].clone();
        delivery.retire_release(operation, &record).unwrap();
        let projection = built.runtime.observations.refresh().await.unwrap();
        delivery.complete_release_chains(&projection).await.unwrap();
        let proof = delivery.releases.state.lock().unwrap().completions[&operation].clone();
        assert!(
            proof.validate().unwrap().is_empty(),
            "ordinary success consumes no ancestor hold"
        );
        assert_eq!(proof.work(), &record.work);
        assert_eq!(proof.workspace_id(), record.workspace);
        let configuration = built.configuration.config.lock().unwrap().clone();
        assert_eq!(proof.dependency_repositories().unwrap().len(), 2);
        assert!(
            record
                .attempt
                .plan
                .steps
                .iter()
                .all(|step| step.kind.repository().id != "84")
        );
        assert!(
            record
                .attempt
                .plan
                .deployment_free
                .iter()
                .all(|repo| repo.id != "84")
        );
        let success = proof.validate_for_configuration(&configuration).unwrap();
        assert_eq!(success.operation_id, operation);
        assert_eq!(
            success.outcome_digest,
            canonical_digest(1, &record.attempt.steps).unwrap()
        );
        let key = issue(&record).unwrap();
        assert_eq!(
            built
                .runtime
                .dependency_outcomes()
                .unwrap()
                .parent_outcome(&key, &record.work.generation),
            Some(success.outcome_digest.as_str())
        );
        let rows = delivery.release_inspection_rows().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].completion.as_ref().unwrap().work, record.work);
        assert!(rows[0].completed_by.is_none() && !rows[0].active_hold);

        // Identity remains pinned; unrelated configuration settings are not the
        // historical provider identity, nor is the current deployed pointer.
        let mut changed = configuration.clone();
        changed.runtime.poll_ms += 1;
        proof.validate_for_configuration(&changed).unwrap();
        for field in 0..6 {
            let mut changed = configuration.clone();
            let workspace = changed
                .workspaces
                .values()
                .find(|w| w.id == record.workspace)
                .unwrap()
                .clone();
            match field {
                0 => changed.runtime.id = Uuid::new_v4(),
                1 => {
                    changed
                        .connections
                        .get_mut(&workspace.linear_connection)
                        .unwrap()
                        .account_id = Uuid::new_v4().to_string()
                }
                2 => {
                    changed
                        .connections
                        .get_mut(&workspace.linear_connection)
                        .unwrap()
                        .endpoint = Some("https://other-linear.invalid/graphql".into())
                }
                3 => {
                    changed
                        .connections
                        .get_mut(&workspace.github_connection)
                        .unwrap()
                        .account_id = "another-account".into()
                }
                4 => {
                    changed
                        .connections
                        .get_mut(&workspace.github_connection)
                        .unwrap()
                        .endpoint = Some("https://other-github.invalid/".into())
                }
                _ => {
                    changed
                        .workspaces
                        .values_mut()
                        .find(|w| w.id == record.workspace)
                        .unwrap()
                        .repositories
                        .get_mut("42")
                        .unwrap()
                        .hostname = "other-github.invalid".into()
                }
            }
            assert!(
                proof.validate_for_configuration(&changed).is_err(),
                "identity field {field}"
            );
        }
        // A complete workspace does not imply pull access to an unchanged
        // baseline repository omitted from the effectful release plan.
        native.auxiliary_pull(false);
        let inaccessible = built.runtime.observations.refresh().await.unwrap();
        assert!(inaccessible.workspaces[&record.workspace].complete);
        assert!(
            built
                .runtime
                .dependency_outcomes()
                .unwrap()
                .parent_outcome(&key, &record.work.generation)
                .is_none()
        );
        native.auxiliary_pull(true);
        built.runtime.observations.refresh().await.unwrap();
        assert_eq!(
            built
                .runtime
                .dependency_outcomes()
                .unwrap()
                .parent_outcome(&key, &record.work.generation),
            Some(success.outcome_digest.as_str())
        );
        let _tamper = observation_barrier.lock().await;
        let completion_leaf = format!("release-completion-{operation}.json");
        let completion: Signed<Value> = delivery.records.read(&completion_leaf).unwrap();
        assert_eq!(completion.payload["schema_version"], 2);
        assert_eq!(completion.payload["attempts"].as_array().unwrap().len(), 1);
        let reference = completion.payload["effects"][0]["leaf"].as_str().unwrap();
        let store =
            PrivateDir::open(&delivery.records.path().join(record.workspace.to_string())).unwrap();
        let bytes = store.read_bytes(reference, 1024 * 1024).unwrap();
        std::fs::remove_file(store.path().join(reference)).unwrap();
        assert!(proof.validate_for_configuration(&configuration).is_err());
        assert!(
            built
                .runtime
                .dependency_outcomes()
                .unwrap()
                .parent_outcome(&key, &record.work.generation)
                .is_none(),
            "installed capability rechecks missing immutable evidence"
        );
        delivery.sync_released_completions().unwrap();
        let cold = RuntimeDelivery::with_fixture(
            built.runtime.clone(),
            delivery.processes.clone(),
            delivery.git_path().unwrap(),
        )
        .unwrap();
        assert!(cold.restore_releases().is_err());
        cold.sync_released_completions().unwrap();
        assert!(
            built
                .runtime
                .dependency_outcomes()
                .unwrap()
                .parent_outcome(&key, &record.work.generation)
                .is_none()
        );
        let row = cold.release_inspection_rows().unwrap().remove(0);
        assert!(row.completion.is_none() && row.diagnostic.is_some());
        assert!(
            !row.active_hold,
            "historical missing evidence is not an invented ordinary resource hold"
        );
        store.write_bytes(reference, &bytes).unwrap();

        // The old schema stays verifiable for queue history. It cannot silently
        // derive an old account from today's configuration hash.
        let mut legacy = completion.payload.clone();
        legacy["schema_version"] = json!(1);
        legacy.as_object_mut().unwrap().remove("dependency_binding");
        let legacy = built
            .runtime
            .services
            .key
            .sign("release_chain_completion", &legacy)
            .unwrap();
        delivery.records.write(&completion_leaf, &legacy).unwrap();
        let legacy_cold = RuntimeDelivery::with_fixture(
            built.runtime.clone(),
            delivery.processes.clone(),
            delivery.git_path().unwrap(),
        )
        .unwrap();
        legacy_cold.restore_releases().unwrap();
        let legacy_proof =
            legacy_cold.releases.state.lock().unwrap().completions[&operation].clone();
        assert_eq!(
            legacy_proof
                .validate_for_configuration(&configuration)
                .unwrap_err()
                .code,
            "dependency_binding_unavailable"
        );
        delivery
            .records
            .write(&completion_leaf, &completion)
            .unwrap();
        drop(_tamper);

        github.set_production(&crate::git::Revision {
            commit: "d".repeat(40),
            tree: "c".repeat(40),
        });
        built.runtime.observations.refresh().await.unwrap();
        let cold = RuntimeDelivery::with_fixture(
            built.runtime.clone(),
            delivery.processes.clone(),
            delivery.git_path().unwrap(),
        )
        .unwrap();
        cold.restore_releases().unwrap();
        cold.sync_released_completions().unwrap();
        assert_eq!(
            built
                .runtime
                .dependency_outcomes()
                .unwrap()
                .parent_outcome(&key, &record.work.generation),
            Some(success.outcome_digest.as_str())
        );
        assert_eq!(
            cold.releases.state.lock().unwrap().completions[&operation].digest(),
            proof.digest()
        );
        // The same old generation remains a dependency fact after an actual
        // authorized native correction. It cannot satisfy the new generation.
        // Explicit external fixture rollback: synchronize actual disposable Git
        // main with the simulated remote before making a new corrected release.
        // This is fixture input, not controller rollback authority.
        let original = seed.slice.manifest().production.clone();
        git(
            &delivery.git_path().unwrap(),
            source,
            &["reset", "--hard", &original.commit],
        );
        github.set_production(&original);
        built.runtime.observations.refresh().await.unwrap();
        let corrected = correct_parent(built, data, &permit.lease).await;
        built.runtime.observations.refresh().await.unwrap();
        assert_ne!(corrected.work().generation, record.work.generation);
        let outcomes = built.runtime.dependency_outcomes().unwrap();
        assert_eq!(
            outcomes.parent_outcome(&key, &record.work.generation),
            Some(success.outcome_digest.as_str())
        );
        assert!(
            outcomes
                .parent_outcome(&key, &corrected.work().generation)
                .is_none()
        );

        extra_completion_fixture::complete_second(extra_completion_fixture::SecondRelease {
            built,
            delivery,
            github,
            native,
            seed,
            source,
            gate: &corrected,
            lease: &permit.lease,
            auxiliary,
            first: &record,
            first_proof: &proof,
            observation_barrier,
        })
        .await;
        let _tamper = observation_barrier.lock().await;
        let effect_count = github.release_effect_count();
        let receipt_count = built.runtime.control.operations().unwrap().len();
        data.lock().unwrap().set_selected(false);
        for retained in [&cold, delivery] {
            assert_historical_facade(retained, &record, false).await;
        }
        assert_eq!(
            built
                .runtime
                .dependency_outcomes()
                .unwrap()
                .parent_outcome(&key, &record.work.generation),
            Some(success.outcome_digest.as_str()),
            "discovery exclusion does not hide the still-accessible native prerequisite"
        );
        built
            .configuration
            .config
            .lock()
            .unwrap()
            .workspaces
            .clear();
        let removed = RuntimeDelivery::with_fixture(
            built.runtime.clone(),
            delivery.processes.clone(),
            delivery.git_path().unwrap(),
        )
        .unwrap();
        for retained in [&cold, &removed] {
            assert_historical_facade(retained, &record, true).await;
        }
        assert!(
            built
                .runtime
                .dependency_outcomes()
                .unwrap()
                .parent_outcome(&key, &record.work.generation)
                .is_none()
        );
        assert_eq!(github.release_effect_count(), effect_count);
        assert_eq!(
            built.runtime.control.operations().unwrap().len(),
            receipt_count,
            "historical inspection must not admit or publish effects"
        );
        *built.configuration.config.lock().unwrap() = configuration;
        data.lock().unwrap().set_selected(true);
        drop(_tamper);
        github.set_production(&merged.0);
    })
}

async fn assert_historical_facade(
    delivery: &Arc<RuntimeDelivery>,
    record: &AttemptRecord,
    removed_workspace: bool,
) {
    let workflow = crate::runtime_workflow::RuntimeWorkflow::new(
        delivery.runtime.clone(),
        delivery.clone(),
        delivery.clone(),
    );
    let view = crate::app::WorkflowControl::inspect(&workflow)
        .await
        .unwrap();
    if !removed_workspace {
        let parent = view
            .projection
            .parent(record.workspace, record.work.parent)
            .unwrap();
        assert!(!parent.included, "discovery exclusion stops new work");
        assert!(
            parent.issue.is_some(),
            "owned native identity remains observed"
        );
    }
    let config = delivery.runtime.configuration.snapshot().unwrap();
    let selected = crate::inspection::resolve_parent(
        &config,
        &view.projection,
        &view.release_history,
        &format!("{}/{}", record.workspace, record.work.parent),
        None,
    )
    .unwrap();
    let rows = crate::inspection::parent_rows(&view.release_history, &selected);
    assert_eq!(rows.len(), 2);
    let original = rows
        .iter()
        .find(|row| row.attempt.plan.operation_id == record.attempt.plan.operation_id)
        .unwrap();
    assert_eq!(original.completion.as_ref().unwrap().work, record.work);
    let observed = record.display.as_ref().unwrap();
    assert_eq!(
        original.display.title.as_deref(),
        Some(observed.title.as_str())
    );
    assert_eq!(
        crate::inspection::observed_url(None, &rows).unwrap(),
        observed.url
    );
    if removed_workspace {
        assert!(view.projection.workspaces.is_empty());
        assert_eq!(
            crate::inspection::resolve_workspace(
                &config,
                &view.release_history,
                Some(&record.workspace.to_string())
            )
            .unwrap(),
            Some(record.workspace)
        );
    }
    let overview = crate::inspection::overview(
        &view,
        Some(record.workspace),
        &delivery.runtime.control.snapshot().unwrap(),
        Utc::now(),
        Duration::seconds(30),
    )
    .unwrap();
    let overview = serde_json::to_value(overview).unwrap();
    assert!(
        overview["history"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["outcome"] == "released"
                && row["generation"] == json!(record.work.generation))
    );
}
