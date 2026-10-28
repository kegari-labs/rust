//! Actual one-link merge/readback/retirement and signed history. Upstream
//! acceptance uses the existing explicit synthetic fixture, never a confined
//! check or a fabricated joined worker completion.
use super::*;

#[path = "runtime_delivery_completion_race_fixture.rs"]
mod first_reader_race;

/// Genuine completed ordinary release; candidate plans are domain queries only.
/// No disjoint source/admission/readiness capability is constructed here.
async fn assert_scoped_ordinary_queue(
    built: &crate::runtime_fixture::RuntimeFixture,
    delivery: &Arc<RuntimeDelivery>,
    barrier: &Arc<tokio::sync::Mutex<()>>,
    proof: &crate::runtime_delivery::VerifiedReleaseCompletion,
    record: &AttemptRecord,
) {
    let _poll = barrier.lock().await;
    let projection = built.runtime.observations.refresh().await.unwrap();
    let workspace = delivery.workspace(&projection, record.workspace).unwrap();
    let scope = domain::CompletionScope::release(proof).unwrap();
    let mut overlap = record.attempt.plan.clone();
    overlap.operation_id = Uuid::new_v4();
    overlap.parent_id = Uuid::new_v4();
    overlap.dependencies.clear();
    overlap.spacing_seconds = 0;
    overlap.validate().unwrap();
    let mut disjoint = overlap.clone();
    disjoint.operation_id = Uuid::new_v4();
    assert_eq!(disjoint.steps.len(), 1);
    let domain::StepKind::Merge { repository, .. } = &mut disjoint.steps[0].kind else {
        panic!("ordinary fixture has one actual merge")
    };
    repository.id = "9002".into();
    disjoint.resources = BTreeSet::from([domain::ResourceKey::Repository {
        repository: repository.clone(),
    }]);
    disjoint.deployment_free = BTreeSet::from([repository.clone()]);
    disjoint.validate().unwrap();
    assert!(scope.resources().is_disjoint(&disjoint.resources));
    let success = proof
        .validate_for_configuration(&workspace.configuration)
        .unwrap();
    let mut dependent = disjoint.clone();
    dependent.operation_id = Uuid::new_v4();
    dependent.dependencies = vec![domain::DependencyRequirement {
        parent_id: record.work.parent,
        generation: record.work.generation.clone(),
        outcome_digest: success.outcome_digest.clone(),
    }];
    dependent.validate().unwrap();
    // Query a time after the genuine prior spacing interval; no dispatch or
    // observation timestamp is changed by this pure queue scheduling argument.
    let at = success.completed_at
        + Duration::seconds(i64::try_from(success.spacing_seconds).unwrap() + 1);
    {
        let state = delivery.releases.state.lock().unwrap();
        for plan in [&overlap, &disjoint, &dependent] {
            state.queue.eligible(plan, at).unwrap();
        }
    }
    let mut owned = delivery.releases.state.lock().unwrap().queue.clone();
    owned.reserve(&disjoint, at).unwrap();
    let mut held = owned.clone();
    let mut synthetic = super::super::tests::record(disjoint.clone(), record.work.clone());
    synthetic.attempt.hold_reason = Some("explicit domain-only unrelated hold".into());
    held.restore_hold(&synthetic.attempt).unwrap();
    let receipts = canonical_digest(1, &built.runtime.control.operations().unwrap()).unwrap();
    let signed: Signed<Value> = delivery
        .records
        .read(&format!(
            "release-completion-{}.json",
            proof.terminal().plan.operation_id
        ))
        .unwrap();
    let leaf = signed.payload["effects"][0]["leaf"].as_str().unwrap();
    let bytes = workspace.records.read_bytes(leaf, 1024 * 1024).unwrap();
    std::fs::remove_file(workspace.records.path().join(leaf)).unwrap();
    let missing = proof.validate().unwrap_err();
    assert_eq!(missing.code, "release_completion");
    assert_eq!(
        missing.message,
        "completed chain omitted a required prior step attempt"
    );
    let check = |queue: &domain::ReleaseQueue| {
        for plan in [&overlap, &dependent] {
            let denied = queue.eligible(plan, at).unwrap_err();
            assert_eq!(denied.code, missing.code);
            assert_eq!(denied.message, missing.message);
        }
        queue.eligible(&disjoint, at).unwrap();
        assert_eq!(queue.earliest_start(&disjoint, at).unwrap(), at);
    };
    check(&delivery.releases.state.lock().unwrap().queue);
    assert!(owned.owns_all(&disjoint));
    owned
        .quarantine_completion(scope.clone(), missing.clone())
        .unwrap();
    held.quarantine_completion(scope, missing.clone()).unwrap();
    assert!(owned.owns_all(&disjoint));
    assert_eq!(
        held.eligible(&disjoint, at).unwrap_err().code,
        "release_hold"
    );
    delivery.sync_released_completions().unwrap();
    check(&delivery.releases.state.lock().unwrap().queue);
    let cold = RuntimeDelivery::with_fixture(
        built.runtime.clone(),
        delivery.processes.clone(),
        delivery.git_path().unwrap(),
    )
    .unwrap();
    cold.sync_released_completions().unwrap();
    cold.restore_releases().unwrap();
    check(&cold.releases.state.lock().unwrap().queue);
    // A has only protective quarantine after this scan, not a valid proof-map
    // entry. Its actual outer seal must still be authenticated on a disjoint
    // queue query before another scan; losing success credit is not a waiver.
    for facade in [delivery.as_ref(), &cold] {
        assert!(
            !facade
                .releases
                .state
                .lock()
                .unwrap()
                .completions
                .contains_key(&proof.terminal().plan.operation_id)
        );
    }
    let quarantined_seal = format!(
        "release-completion-{}.json",
        proof.terminal().plan.operation_id
    );
    let quarantined_bytes = delivery
        .records
        .read_bytes(&quarantined_seal, 1024 * 1024)
        .unwrap();
    for malformed in [false, true] {
        let (code, message) = if malformed {
            delivery
                .records
                .write(&quarantined_seal, &json!({"not_a_signed_seal":true}))
                .unwrap();
            (
                "record_corrupt",
                "control record is invalid JSON or has unknown fields",
            )
        } else {
            let mut altered: Signed<Value> = delivery.records.read(&quarantined_seal).unwrap();
            altered.signature = "00".repeat(32);
            delivery.records.write(&quarantined_seal, &altered).unwrap();
            (
                "invalid_attestation",
                "authentication tag verification failed",
            )
        };
        for facade in [delivery.as_ref(), &cold] {
            let denied = facade
                .releases
                .state
                .lock()
                .unwrap()
                .queue
                .eligible(&disjoint, at)
                .unwrap_err();
            assert_eq!(denied.code, code);
            assert_eq!(denied.message, message);
        }
        delivery
            .records
            .write_bytes(&quarantined_seal, &quarantined_bytes)
            .unwrap();
        for facade in [delivery.as_ref(), &cold] {
            check(&facade.releases.state.lock().unwrap().queue);
        }
    }
    workspace.records.write_bytes(leaf, &bytes).unwrap();
    delivery.sync_released_completions().unwrap();
    delivery
        .releases
        .state
        .lock()
        .unwrap()
        .queue
        .eligible(&dependent, at)
        .unwrap();

    // A cold reader without any prior authenticated anchors cannot infer
    // scope from a completion filename when a signed AttemptRecord is missing.
    // This is the actual bounded reader, not a claim of restarting the runtime:
    // a new facade sharing this runtime may now retain genuine cached anchors.
    let attempt = attempt_leaf(record.attempt.plan.operation_id);
    let attempt_bytes = delivery.records.read_bytes(&attempt, 1024 * 1024).unwrap();
    std::fs::remove_file(delivery.records.path().join(&attempt)).unwrap();
    let unavailable = match delivery.release_completion_queue_inventory(&BTreeMap::new()) {
        Ok(_) => panic!("unknown cold scope requires every authenticated attempt"),
        Err(error) => error,
    };
    assert_eq!(unavailable.code, "record_missing");
    delivery
        .records
        .write_bytes(&attempt, &attempt_bytes)
        .unwrap();
    delivery.sync_released_completions().unwrap();
    built.runtime.observations.refresh().await.unwrap();
    assert_eq!(
        canonical_digest(1, &built.runtime.control.operations().unwrap()).unwrap(),
        receipts
    );
}

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
    pub merged: (kegari_git::Revision, ObservedReleaseStep),
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
        // A genuinely new runtime restores only the actual retired attempt,
        // before the first seal exists; no prior anchor/cache is transferred.
        let cold_readers = first_reader_race::prepare(delivery, operation).await;
        let proof = {
            // Retain the actual fixture poll barrier from this first successful
            // observation through the sequential missing-own-seal commit. No
            // timestamp, signed payload or native receipt is manufactured.
            let _poll = observation_barrier.lock().await;
            let projection = built.runtime.observations.refresh().await.unwrap();
            delivery.complete_release_chains(&projection).await.unwrap();
            let proof = delivery.releases.state.lock().unwrap().completions[&operation].clone();
            let provider_reads = data.lock().unwrap().requests.len();
            let effects = github.release_effect_count();
            first_reader_race::assert_serialized_first_reader(&cold_readers, &proof);
            assert_eq!(data.lock().unwrap().requests.len(), provider_reads);
            assert_eq!(github.release_effect_count(), effects);
            let seal_leaf = format!("release-completion-{operation}.json");
            let seal_bytes = delivery
                .records
                .read_bytes(&seal_leaf, 1024 * 1024)
                .unwrap();
            let commit = delivery
                .prepare_fixture_completion_commit(operation)
                .unwrap();
            std::fs::remove_file(delivery.records.path().join(&seal_leaf)).unwrap();
            let missing = proof.validate().unwrap_err();
            assert_eq!(missing.code, "record_missing");
            let refused = commit().unwrap_err();
            assert_eq!(refused.code, missing.code);
            assert_eq!(refused.message, missing.message);
            assert_eq!(
                delivery
                    .records
                    .read_bytes(&seal_leaf, 1024 * 1024)
                    .unwrap_err()
                    .code,
                "record_missing"
            );
            let key = issue(&record).unwrap();
            assert!(
                built
                    .runtime
                    .dependency_outcomes()
                    .unwrap()
                    .parent_outcome(&key, &record.work.generation)
                    .is_none()
            );
            delivery
                .records
                .write_bytes(&seal_leaf, &seal_bytes)
                .unwrap();
            proof.validate().unwrap();
            proof
        };
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
        Box::pin(assert_scoped_ordinary_queue(
            built,
            delivery,
            observation_barrier,
            &proof,
            &record,
        ))
        .await;

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
        cold.restore_releases().unwrap();
        let unavailable = proof.validate().unwrap_err();
        let denied = cold
            .releases
            .state
            .lock()
            .unwrap()
            .queue
            .eligible(&record.attempt.plan, Utc::now())
            .unwrap_err();
        assert_eq!(denied.code, unavailable.code);
        assert_eq!(denied.message, unavailable.message);
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
        // This facade retains the genuine original schema-2 identity. An
        // authenticated schema rewrite cannot replace it or supply credit.
        assert!(
            !legacy_cold
                .releases
                .state
                .lock()
                .unwrap()
                .completions
                .contains_key(&operation)
        );
        assert_eq!(
            legacy_cold
                .releases
                .state
                .lock()
                .unwrap()
                .completion_anchors[&operation]
                .digest(),
            proof.digest()
        );
        // Independently exercise the cold legacy decoder with no prior anchor.
        // This is a read-only component check, not a new process or cache reset.
        let legacy_inventory = legacy_cold
            .release_completion_queue_inventory(&BTreeMap::new())
            .unwrap();
        let legacy_proof = legacy_inventory.proofs[&operation].clone();
        let legacy_history = domain::ReleaseQueue::default()
            .record_release_completion_history(&legacy_proof)
            .unwrap();
        assert_eq!(legacy_history.outcome_digest, success.outcome_digest);
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

        github.set_production(&kegari_git::Revision {
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
    let view = crate::runtime_host::WorkflowControl::inspect(&workflow)
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
